//! Multiplayer oturumu — network thread + ana TUI thread arasında köprü.
//!
//! `--connect <addr>` ile çağrıldığında bu modülün `MpSession::start()`
//! fonksiyonu:
//!
//! 1. Bir tokio runtime'ı arka plan thread'inde spawn eder.
//! 2. Server'a `Hello` yollar, `Welcome` ile `PlayerId` alır.
//! 3. Server'dan gelen mesajları (LobbyState, GameStart, TickAdvanced,
//!    Pong, ...) `mpsc::Receiver<MpEvent>` üzerinden ana thread'e iletir.
//! 4. Ana thread `MpCommand`'lar yazar (SelectRole, Ready, SubmitCommand)
//!    ve `mpsc::Sender<MpCommand>` üzerinden network thread'i tetikler.
//!
//! Ana thread blocking değil — `try_recv` ile mesaj kontrolü yapar, render
//! 60fps'i tutar.

#![allow(dead_code)]

use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use moneywar_domain::{Command, GameState, PlayerId, Role, RoomId};
use moneywar_net::{
    ClientMessage, LobbyEntry, PROTOCOL_VERSION, RejectReason, ServerMessage, decode_server,
    encode_client,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc as tokio_mpsc;
use tokio::time::{interval, timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

const HELLO_TIMEOUT_SECS: u64 = 5;
const HEARTBEAT_PERIOD_MS: u64 = 5_000;
const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Ana thread'e iletilen network olayları.
#[derive(Debug, Clone)]
pub enum MpEvent {
    /// Bağlantı kuruldu, sunucu bizi tanıdı.
    Connected {
        player_id: PlayerId,
        room_id: RoomId,
        server_version: String,
    },
    /// Sunucu reddetti (versiyon, isim, vb).
    Rejected(RejectReason),
    /// Lobide oyuncu durumu değişti.
    Lobby {
        entries: Vec<LobbyEntry>,
        host: PlayerId,
    },
    /// Oyun başladı, ilk state geldi.
    GameStart {
        initial_state: Box<GameState>,
        tick_ms: u64,
    },
    /// Yeni tick — state güncel.
    TickAdvanced { state: Box<GameState> },
    /// Server bir komutu reddetti (kullanıcı feedback için).
    CommandRejected { reason: String },
    /// Bir oyuncu ayrıldı.
    PlayerLeft { player_id: PlayerId, clean: bool },
    /// Network bağlantısı koptu / hata.
    Disconnected { reason: String },
}

/// Ana thread'in network thread'ine yazdığı komutlar.
#[derive(Debug, Clone)]
pub enum MpCommand {
    SelectRole(Role),
    SetReady(bool),
    Submit(Command),
    Bye,
}

/// MP oturumu handle'ı — ana thread bunu tutar, `events()` ile olay okur,
/// `send()` ile komut yazar.
pub struct MpSession {
    pub events: std_mpsc::Receiver<MpEvent>,
    pub commands: tokio_mpsc::UnboundedSender<MpCommand>,
    pub player_name: String,
}

impl MpSession {
    /// Yeni bağlantı kur. Tokio runtime'ı arka plan thread'inde spawn olur.
    /// Bağlantı başarılı/başarısız `MpEvent::Connected` ya da `Disconnected`
    /// ile bildirilir; bu fonksiyon hemen döner (non-blocking).
    pub fn start(addr: String, player_name: String) -> Self {
        let (event_tx, event_rx) = std_mpsc::channel::<MpEvent>();
        let (cmd_tx, cmd_rx) = tokio_mpsc::unbounded_channel::<MpCommand>();
        let name_clone = player_name.clone();

        thread::Builder::new()
            .name("mw-net".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = event_tx.send(MpEvent::Disconnected {
                            reason: format!("runtime: {e}"),
                        });
                        return;
                    }
                };
                runtime.block_on(network_task(addr, name_clone, event_tx, cmd_rx));
            })
            .expect("spawn mw-net thread");

        Self {
            events: event_rx,
            commands: cmd_tx,
            player_name,
        }
    }

    /// Non-blocking olay drain — ana loop her tick'te çağırır.
    pub fn drain_events(&self) -> Vec<MpEvent> {
        let mut out = Vec::new();
        while let Ok(e) = self.events.try_recv() {
            out.push(e);
        }
        out
    }

    /// Bir komut yolla; network thread tokio kanalından alır.
    pub fn send(&self, cmd: MpCommand) {
        let _ = self.commands.send(cmd);
    }
}

async fn network_task(
    addr: String,
    player_name: String,
    event_tx: std_mpsc::Sender<MpEvent>,
    mut cmd_rx: tokio_mpsc::UnboundedReceiver<MpCommand>,
) {
    let socket = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            let _ = event_tx.send(MpEvent::Disconnected {
                reason: format!("{addr} bağlanamadı: {e}"),
            });
            return;
        }
    };
    socket.set_nodelay(true).ok();
    let mut framed = Framed::new(socket, LengthDelimitedCodec::new());

    // ---- Hello ----
    let hello = ClientMessage::Hello {
        protocol_version: PROTOCOL_VERSION,
        client_version: CLI_VERSION.to_string(),
        player_name,
    };
    if let Err(e) = send(&mut framed, &hello).await {
        let _ = event_tx.send(MpEvent::Disconnected {
            reason: format!("Hello yollanamadı: {e}"),
        });
        return;
    }

    // ---- Welcome ----
    let welcome_frame = match timeout(Duration::from_secs(HELLO_TIMEOUT_SECS), framed.next()).await
    {
        Ok(Some(Ok(f))) => f,
        Ok(Some(Err(e))) => {
            let _ = event_tx.send(MpEvent::Disconnected {
                reason: format!("Welcome stream: {e}"),
            });
            return;
        }
        Ok(None) | Err(_) => {
            let _ = event_tx.send(MpEvent::Disconnected {
                reason: format!("Welcome {HELLO_TIMEOUT_SECS}s içinde gelmedi"),
            });
            return;
        }
    };

    match decode_server(&welcome_frame) {
        Ok(ServerMessage::Welcome {
            player_id,
            room_id,
            server_version,
            ..
        }) => {
            let _ = event_tx.send(MpEvent::Connected {
                player_id,
                room_id,
                server_version,
            });
        }
        Ok(ServerMessage::Reject { reason }) => {
            let _ = event_tx.send(MpEvent::Rejected(reason));
            return;
        }
        Ok(other) => {
            let _ = event_tx.send(MpEvent::Disconnected {
                reason: format!("Welcome yerine {other:?} geldi"),
            });
            return;
        }
        Err(e) => {
            let _ = event_tx.send(MpEvent::Disconnected {
                reason: format!("Welcome decode: {e}"),
            });
            return;
        }
    }

    // ---- Ana döngü: command/event/heartbeat ----
    let mut heartbeat = interval(Duration::from_millis(HEARTBEAT_PERIOD_MS));
    heartbeat.tick().await; // ilk tick'i atla
    let mut nonce: u64 = 0;

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(MpCommand::SelectRole(role)) => {
                    let _ = send(&mut framed, &ClientMessage::SelectRole { role }).await;
                }
                Some(MpCommand::SetReady(ready)) => {
                    let _ = send(&mut framed, &ClientMessage::Ready { ready }).await;
                }
                Some(MpCommand::Submit(command)) => {
                    let _ = send(&mut framed, &ClientMessage::SubmitCommand { command }).await;
                }
                Some(MpCommand::Bye) => {
                    let _ = send(&mut framed, &ClientMessage::Bye).await;
                    return;
                }
                None => {
                    // command channel kapandı = ana thread çıktı
                    let _ = send(&mut framed, &ClientMessage::Bye).await;
                    return;
                }
            },

            _ = heartbeat.tick() => {
                nonce = nonce.wrapping_add(1);
                if let Err(e) = send(&mut framed, &ClientMessage::Ping { nonce }).await {
                    let _ = event_tx.send(MpEvent::Disconnected {
                        reason: format!("ping: {e}"),
                    });
                    return;
                }
            },

            msg = framed.next() => match msg {
                Some(Ok(frame)) => match decode_server(&frame) {
                    Ok(ServerMessage::Pong { .. }) => {}
                    Ok(ServerMessage::LobbyState { entries, host }) => {
                        let _ = event_tx.send(MpEvent::Lobby { entries, host });
                    }
                    Ok(ServerMessage::GameStart { initial_state, tick_ms }) => {
                        let _ = event_tx.send(MpEvent::GameStart { initial_state, tick_ms });
                    }
                    Ok(ServerMessage::TickAdvanced { state, .. }) => {
                        let _ = event_tx.send(MpEvent::TickAdvanced { state });
                    }
                    Ok(ServerMessage::CommandRejected { reason, .. }) => {
                        let _ = event_tx.send(MpEvent::CommandRejected { reason });
                    }
                    Ok(ServerMessage::PlayerLeft { player_id, clean }) => {
                        let _ = event_tx.send(MpEvent::PlayerLeft { player_id, clean });
                    }
                    Ok(ServerMessage::Reject { reason }) => {
                        let _ = event_tx.send(MpEvent::Rejected(reason));
                        return;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        let _ = event_tx.send(MpEvent::Disconnected {
                            reason: format!("decode: {e}"),
                        });
                        return;
                    }
                },
                Some(Err(e)) => {
                    let _ = event_tx.send(MpEvent::Disconnected {
                        reason: format!("stream: {e}"),
                    });
                    return;
                }
                None => {
                    let _ = event_tx.send(MpEvent::Disconnected {
                        reason: "server bağlantıyı kapattı".into(),
                    });
                    return;
                }
            },
        }
    }
}

async fn send(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    msg: &ClientMessage,
) -> std::io::Result<()> {
    match encode_client(msg) {
        Ok(bytes) => framed
            .send(Bytes::from(bytes))
            .await
            .map_err(|e| std::io::Error::other(e.to_string())),
        Err(e) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.to_string(),
        )),
    }
}
