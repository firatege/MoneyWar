//! `MoneyWar` LAN multiplayer server — Sprint 1: TCP echo + heartbeat.
//!
//! Bu sprint için sadece bağlantı kurma + protokol versiyonu doğrulama.
//! Lobby ve oyun döngüsü Sprint 2+.
//!
//! ## Çalıştırma
//!
//! ```text
//! cargo run -p moneywar-server -- --port 7878
//! ```
//!
//! ## Mesaj akışı (Sprint 1)
//!
//! ```text
//! Client            Server
//! ──────            ──────
//! Hello{v=1, ...}    →
//!                    ←  Welcome{player_id, room_id}
//! Ping{nonce}        →
//!                    ←  Pong{nonce}
//! ```
//!
//! 5 saniye boyunca paket gelmezse server bağlantıyı kapatır (idle timeout).

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::too_many_lines,
    clippy::doc_markdown
)]

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use moneywar_domain::{PlayerId, RoomId};
use moneywar_net::{
    ClientMessage, PROTOCOL_VERSION, RejectReason, ServerMessage, decode_client, encode_server,
};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_util::codec::{Framed, LengthDelimitedCodec};
use tracing::{error, info, warn};

const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const IDLE_TIMEOUT_SECS: u64 = 5;
const DEFAULT_PORT: u16 = 7878;

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

    // Sprint 1: tek room, monoton player_id sayacı. Sprint 2'de proper Room state.
    let state = std::sync::Arc::new(Mutex::new(ServerState::new()));

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

/// Server'ın paylaşılan durumu. Sprint 1'de minimal — Sprint 2+'da `Room`,
/// command queue, tick advancing buraya gelir.
struct ServerState {
    next_player_id: u64,
    room_id: RoomId,
}

impl ServerState {
    fn new() -> Self {
        Self {
            next_player_id: 1,
            // Sprint 1: random epoch seed; Sprint 2'de host config'inden gelir.
            room_id: RoomId::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(1)
                    .max(1),
            ),
        }
    }

    fn assign_player_id(&mut self) -> PlayerId {
        let id = PlayerId::new(self.next_player_id);
        self.next_player_id += 1;
        id
    }
}

/// Bir client bağlantısının yaşam döngüsü:
/// 1. `Hello` bekle (timeout altında)
/// 2. Versiyon kontrolü → `Welcome` veya `Reject`
/// 3. Heartbeat döngüsü: `Ping` → `Pong`, idle timeout → kapan
async fn handle_connection(
    socket: TcpStream,
    peer: SocketAddr,
    state: std::sync::Arc<Mutex<ServerState>>,
) -> Result<()> {
    info!("{peer} bağlandı");
    socket.set_nodelay(true).ok();

    let mut framed = Framed::new(socket, LengthDelimitedCodec::new());

    // ---- Aşama 1: Hello bekle ----
    let hello_frame = match timeout(Duration::from_secs(IDLE_TIMEOUT_SECS), framed.next()).await {
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
            warn!("{peer}: {IDLE_TIMEOUT_SECS}s içinde Hello gelmedi");
            return Ok(());
        }
    };

    let hello = match decode_client(&hello_frame) {
        Ok(msg) => msg,
        Err(e) => {
            warn!("{peer}: Hello decode hatası: {e}");
            send(
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

    let (client_protocol, client_version, player_name) = match hello {
        ClientMessage::Hello {
            protocol_version,
            client_version,
            player_name,
        } => (protocol_version, client_version, player_name),
        other => {
            warn!("{peer}: Hello yerine {other:?} geldi");
            send(
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
    };

    if client_protocol != PROTOCOL_VERSION {
        warn!("{peer}: protokol uyuşmazlığı (server={PROTOCOL_VERSION}, client={client_protocol})");
        send(
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

    // ---- Aşama 2: Welcome ----
    let (player_id, room_id) = {
        let mut state = state.lock().await;
        (state.assign_player_id(), state.room_id)
    };

    info!(
        "{peer}: Hello kabul → player_id={} room_id={} (client v{client_version}, name='{player_name}')",
        player_id.value(),
        room_id.value()
    );

    send(
        &mut framed,
        &ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            server_version: SERVER_VERSION.into(),
            player_id,
            room_id,
        },
    )
    .await?;

    // ---- Aşama 3: Heartbeat döngüsü ----
    loop {
        let next = timeout(Duration::from_secs(IDLE_TIMEOUT_SECS), framed.next()).await;
        match next {
            Ok(Some(Ok(frame))) => match decode_client(&frame) {
                Ok(ClientMessage::Ping { nonce }) => {
                    send(&mut framed, &ServerMessage::Pong { nonce }).await?;
                }
                Ok(ClientMessage::Bye) => {
                    info!("{peer}: temiz çıkış (Bye)");
                    return Ok(());
                }
                Ok(other) => {
                    // Sprint 1: diğer mesajlar henüz desteklenmiyor — log ve geç.
                    info!("{peer}: bekleyen mesaj türü {other:?} (Sprint 2+)");
                }
                Err(e) => {
                    warn!("{peer}: decode hatası: {e}");
                    return Ok(());
                }
            },
            Ok(Some(Err(e))) => {
                warn!("{peer}: stream hatası: {e}");
                return Ok(());
            }
            Ok(None) => {
                info!("{peer}: bağlantı kapandı");
                return Ok(());
            }
            Err(_) => {
                warn!("{peer}: idle timeout ({IDLE_TIMEOUT_SECS}s)");
                return Ok(());
            }
        }
    }
}

/// `ServerMessage`'ı çerçeveli olarak yolla. Encode hatası iletilir; IO
/// hatası `?` ile yukarı.
async fn send(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    msg: &ServerMessage,
) -> Result<()> {
    let bytes = encode_server(msg).context("ServerMessage encode")?;
    framed
        .send(bytes::Bytes::from(bytes))
        .await
        .context("frame send")?;
    Ok(())
}

/// `--port <N>` argümanını parse et. Yoksa `None`.
/// Sprint 1: minimal env::args. Sprint 2+'da clap'e geçilebilir.
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
