//! `MoneyWar` LAN multiplayer server — Sprint 2: lobby + GameStart.
//!
//! ## Çalıştırma
//!
//! ```text
//! cargo run -p moneywar-server -- --port 7878
//! ```
//!
//! ## Mesaj akışı (Sprint 2)
//!
//! ```text
//! Client                            Server
//! ──────                            ──────
//! Hello{v=1, name}                   →
//!                                    ← Welcome{player_id, room_id}
//!                                    ← LobbyState{entries, host}      (broadcast)
//! SelectRole{role}                   →
//!                                    ← LobbyState{...}                (broadcast)
//! Ready{ready=true}                  →
//!                                    ← LobbyState{...}                (broadcast)
//!  ... tüm oyuncular Ready basınca ...
//!                                    ← GameStart{initial_state, ...} (broadcast)
//!  ... Sprint 3'te tick döngüsü ...
//! ```
//!
//! Heartbeat ve idle timeout aynı; lobby ve aktif oyun fazlarında 5sn
//! sessizlikten sonra connection kapanır.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::doc_markdown
)]

mod game;
mod lobby;
mod world;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use moneywar_domain::{Command, GameState, PlayerId, RoomId};
use moneywar_net::{
    ClientMessage, DEFAULT_TICK_MS, PROTOCOL_VERSION, RejectReason, ServerMessage, decode_client,
    encode_server,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, mpsc};
use tokio::time::timeout;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info, warn};

use lobby::{JoinError, Lobby};

const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const IDLE_TIMEOUT_SECS: u64 = 30; // lobide insan biraz gecikebilir
const DEFAULT_PORT: u16 = 7878;
const OUTBOUND_CHANNEL_CAP: usize = 32;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let port = parse_port_arg().unwrap_or(DEFAULT_PORT);
    let addr: SocketAddr = format!("0.0.0.0:{port}")
        .parse()
        .with_context(|| format!("invalid bind address (port {port})"))?;

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("port {port} bind edilemedi"))?;
    info!("MoneyWar server v{SERVER_VERSION} dinliyor: {addr}");
    info!("protocol_version = {PROTOCOL_VERSION}, idle_timeout = {IDLE_TIMEOUT_SECS}s");

    let state = Arc::new(Mutex::new(ServerState::new()));

    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                error!("accept hatası: {e}");
                continue;
            }
        };
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(socket, peer, state).await {
                warn!("{peer} bağlantı sonlandı: {e}");
            }
        });
    }
}

/// Server'ın paylaşılan durumu — tek room.
#[derive(Debug)]
pub struct ServerState {
    pub next_player_id: u64,
    pub lobby: Lobby,
    /// Aktif oyun state'i — `start_game` set eder, tick döngüsü mutate eder.
    /// `None` → henüz lobide.
    pub game: Option<GameState>,
    /// Tick batch'ine girmeyi bekleyen insan komutları. Game loop drain eder.
    pub pending_commands: Vec<Command>,
}

impl ServerState {
    fn new() -> Self {
        Self {
            next_player_id: 1,
            lobby: Lobby::new(RoomId::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(1)
                    .max(1),
            )),
            game: None,
            pending_commands: Vec::new(),
        }
    }

    fn assign_player_id(&mut self) -> PlayerId {
        let id = PlayerId::new(self.next_player_id);
        self.next_player_id += 1;
        id
    }
}

/// Connection lifecycle:
/// 1. Hello + protokol kontrolü → Welcome
/// 2. Lobby'ye katıl → LobbyState broadcast
/// 3. Mesaj döngüsü: SelectRole/Ready/Ping/Bye, lobby state güncellemeleri
/// 4. Disconnect → lobby'den çıkar, PlayerLeft broadcast
async fn handle_connection(
    socket: TcpStream,
    peer: SocketAddr,
    state: Arc<Mutex<ServerState>>,
) -> Result<()> {
    info!("{peer} bağlandı");
    socket.set_nodelay(true).ok();

    let mut framed = Framed::new(socket, LengthDelimitedCodec::new());

    // ---- Phase 1: Hello ----
    let hello_frame = match timeout(Duration::from_secs(5), framed.next()).await {
        Ok(Some(Ok(frame))) => frame,
        Ok(Some(Err(e))) => {
            warn!("{peer}: ilk frame okuma hatası: {e}");
            return Ok(());
        }
        Ok(None) => {
            warn!("{peer}: Hello gelmeden bağlantı kapandı");
            return Ok(());
        }
        Err(_) => {
            warn!("{peer}: 5s içinde Hello gelmedi");
            return Ok(());
        }
    };

    let (client_protocol, client_version, player_name) = match decode_client(&hello_frame) {
        Ok(ClientMessage::Hello {
            protocol_version,
            client_version,
            player_name,
        }) => (protocol_version, client_version, player_name),
        Ok(other) => {
            warn!("{peer}: Hello yerine {other:?}");
            send_one(
                &mut framed,
                &ServerMessage::Reject {
                    reason: RejectReason::Other {
                        message: "expected Hello as first message".into(),
                    },
                },
            )
            .await
            .ok();
            return Ok(());
        }
        Err(e) => {
            warn!("{peer}: Hello decode hatası: {e}");
            send_one(
                &mut framed,
                &ServerMessage::Reject {
                    reason: RejectReason::Other {
                        message: format!("Hello decode failed: {e}"),
                    },
                },
            )
            .await
            .ok();
            return Ok(());
        }
    };

    if client_protocol != PROTOCOL_VERSION {
        warn!("{peer}: protokol uyuşmazlığı (server={PROTOCOL_VERSION}, client={client_protocol})");
        send_one(
            &mut framed,
            &ServerMessage::Reject {
                reason: RejectReason::ProtocolMismatch {
                    expected: PROTOCOL_VERSION,
                    got: client_protocol,
                },
            },
        )
        .await
        .ok();
        return Ok(());
    }

    // ---- Phase 2: Welcome + Lobby join ----
    let (player_id, room_id, mut outbound_rx) = {
        let mut state = state.lock().await;
        let player_id = state.assign_player_id();
        let room_id = state.lobby.room_id;
        let (tx, rx) = mpsc::channel::<ServerMessage>(OUTBOUND_CHANNEL_CAP);
        match state.lobby.add_player(player_id, player_name.clone(), tx) {
            Ok(()) => (player_id, room_id, rx),
            Err(e) => {
                drop(state);
                let reason = match e {
                    JoinError::RoomFull => RejectReason::RoomFull {
                        capacity: moneywar_net::MAX_HUMAN_PLAYERS,
                    },
                    JoinError::NameTaken => RejectReason::NameTaken,
                    JoinError::GameAlreadyStarted => RejectReason::GameAlreadyStarted,
                };
                send_one(&mut framed, &ServerMessage::Reject { reason })
                    .await
                    .ok();
                return Ok(());
            }
        }
    };

    info!(
        "{peer}: Hello kabul → player_id={} room_id={} (client v{client_version}, name='{player_name}')",
        player_id.value(),
        room_id.value(),
    );

    send_one(
        &mut framed,
        &ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            server_version: SERVER_VERSION.into(),
            player_id,
            room_id,
        },
    )
    .await?;

    // İlk LobbyState broadcast — yeni gelenin de görmesi için.
    broadcast_lobby_state(&state).await;

    // ---- Phase 3: Mesaj döngüsü ----
    let result = run_message_loop(&mut framed, &mut outbound_rx, player_id, &state).await;

    // ---- Phase 4: Cleanup ----
    {
        let mut s = state.lock().await;
        s.lobby.remove_player(player_id);
    }
    broadcast_lobby_state(&state).await;
    broadcast(
        &state,
        &ServerMessage::PlayerLeft {
            player_id,
            clean: result.is_ok(),
        },
    )
    .await;
    info!("{peer}: lobby'den çıktı (player_id={})", player_id.value());

    result
}

async fn run_message_loop(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    outbound_rx: &mut mpsc::Receiver<ServerMessage>,
    player_id: PlayerId,
    state: &Arc<Mutex<ServerState>>,
) -> Result<()> {
    loop {
        tokio::select! {
            // Server → client: outbound queue'dan al, frame'le yolla.
            msg = outbound_rx.recv() => {
                if let Some(m) = msg {
                    send_one(framed, &m).await?;
                } else {
                    warn!("outbound channel kapandı, connection sonlandırılıyor");
                    return Ok(());
                }
            }

            // Client → server: framed stream'den oku.
            frame = timeout(Duration::from_secs(IDLE_TIMEOUT_SECS), framed.next()) => match frame {
                Ok(Some(Ok(bytes))) => {
                    if !handle_client_message(&bytes, player_id, state).await? {
                        return Ok(()); // Bye
                    }
                }
                Ok(Some(Err(e))) => {
                    warn!("stream hatası: {e}");
                    return Ok(());
                }
                Ok(None) => {
                    info!("player_id={}: bağlantı kapandı", player_id.value());
                    return Ok(());
                }
                Err(_) => {
                    warn!("player_id={}: idle timeout", player_id.value());
                    return Ok(());
                }
            },
        }
    }
}

/// Bir client mesajını handle et. `false` → Bye, döngü sonlanmalı.
async fn handle_client_message(
    bytes: &[u8],
    player_id: PlayerId,
    state: &Arc<Mutex<ServerState>>,
) -> Result<bool> {
    let msg = match decode_client(bytes) {
        Ok(m) => m,
        Err(e) => {
            warn!("decode hatası: {e}");
            return Ok(true);
        }
    };

    match msg {
        ClientMessage::Hello { .. } => {
            warn!(
                "player_id={}: tekrar Hello — yok sayılıyor",
                player_id.value()
            );
        }
        ClientMessage::SelectRole { role } => {
            {
                let mut s = state.lock().await;
                s.lobby.select_role(player_id, role);
            }
            broadcast_lobby_state(state).await;
        }
        ClientMessage::Ready { ready } => {
            let should_start = {
                let mut s = state.lock().await;
                s.lobby.set_ready(player_id, ready);
                ready && s.lobby.all_ready() && !s.lobby.game_started
            };
            broadcast_lobby_state(state).await;
            if should_start {
                start_game(state).await;
            }
        }
        ClientMessage::SubmitCommand { command } => {
            // DispatchCaravan'ın requester'ı placeholder=0 → engine validate eder.
            // Diğer komutlar için actor mismatch'te reddet.
            let actor = command.requester();
            if actor.value() != 0 && actor != player_id {
                warn!(
                    "player_id={}: başkasının (#{}) adına emir reddedildi",
                    player_id.value(),
                    actor.value()
                );
                send_to(
                    state,
                    player_id,
                    ServerMessage::CommandRejected {
                        command,
                        reason: "actor mismatch".into(),
                    },
                )
                .await;
                return Ok(true);
            }
            game::enqueue_command(state, command).await;
        }
        ClientMessage::Ping { nonce } => {
            send_to(state, player_id, ServerMessage::Pong { nonce }).await;
        }
        ClientMessage::Bye => {
            info!("player_id={}: Bye", player_id.value());
            return Ok(false);
        }
    }
    Ok(true)
}

async fn broadcast_lobby_state(state: &Arc<Mutex<ServerState>>) {
    let snap = {
        let s = state.lock().await;
        s.lobby.snapshot()
    };
    broadcast(state, &snap).await;
}

async fn broadcast(state: &Arc<Mutex<ServerState>>, msg: &ServerMessage) {
    let s = state.lock().await;
    s.lobby.broadcast(msg).await;
}

async fn send_to(state: &Arc<Mutex<ServerState>>, player_id: PlayerId, msg: ServerMessage) {
    let s = state.lock().await;
    if let Some(slot) = s.lobby.slots.get(&player_id) {
        let _ = slot.tx.send(msg).await;
    }
}

/// Tüm oyuncular ready basınca initial state üret + GameStart broadcast et +
/// game loop'u spawn et.
async fn start_game(state: &Arc<Mutex<ServerState>>) {
    let initial_state = {
        let mut s = state.lock().await;
        s.lobby.game_started = true;
        world::build_initial_state(&s.lobby)
    };
    info!(
        "🎮 GameStart broadcast — tick=0, players={}, npcs={}",
        initial_state.players.values().filter(|p| !p.is_npc).count(),
        initial_state.players.values().filter(|p| p.is_npc).count(),
    );
    game::install_initial_state(state, initial_state.clone()).await;
    broadcast(
        state,
        &ServerMessage::GameStart {
            initial_state: Box::new(initial_state),
            tick_ms: DEFAULT_TICK_MS,
        },
    )
    .await;
    game::spawn_game_loop(state.clone(), DEFAULT_TICK_MS);
}

/// Tek bir mesajı framed stream'e yaz — encode hatasını yutmaz.
async fn send_one(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    msg: &ServerMessage,
) -> Result<()> {
    let bytes = encode_server(msg).context("ServerMessage encode")?;
    framed
        .send(Bytes::from(bytes))
        .await
        .context("frame send")?;
    Ok(())
}

/// `--port <N>` argümanını parse et. Yoksa `None`.
fn parse_port_arg() -> Option<u16> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--port" {
            return args.next().and_then(|v| v.parse().ok());
        }
        if let Some(rest) = arg.strip_prefix("--port=") {
            return rest.parse().ok();
        }
    }
    None
}
