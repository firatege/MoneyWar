//! Network client — Sprint 1: minimal connect demo.
//!
//! `--connect <addr>` ile çağrıldığında TUI'yi açmadan server'a bağlanır,
//! `Hello` yollar, `Welcome` bekler, sonra 1 Hz heartbeat (`Ping`/`Pong`)
//! döngüsüne girer. Stdout'a düz log basar (TUI yok). Ctrl-C ile temiz çıkış.
//!
//! Sprint 2'de bu modül lobby + game state akışına genişler ve TUI'ye
//! `Backend` trait'i üzerinden enjekte edilir.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use moneywar_net::{ClientMessage, PROTOCOL_VERSION, ServerMessage, decode_server, encode_client};
use tokio::net::TcpStream;
use tokio::time::{interval, timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const HEARTBEAT_PERIOD_MS: u64 = 1000;
const HELLO_TIMEOUT_SECS: u64 = 5;

/// `--connect <addr>` modunun ana fonksiyonu. Tokio runtime'ı burada kurar.
pub fn run_connect_demo(addr: &str) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("tokio runtime")?;

    runtime.block_on(async move {
        println!("[net] {addr} adresine bağlanılıyor…");
        let socket = TcpStream::connect(addr)
            .await
            .with_context(|| format!("{addr} bağlanamadı"))?;
        socket.set_nodelay(true).ok();
        println!("[net] TCP bağlantısı kuruldu");

        let mut framed = Framed::new(socket, LengthDelimitedCodec::new());

        // ---- Hello ----
        let player_name = std::env::var("MONEYWAR_NAME").unwrap_or_else(|_| "Sen".to_string());
        let hello = ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            client_version: CLI_VERSION.to_string(),
            player_name: player_name.clone(),
        };
        send(&mut framed, &hello).await?;
        println!("[net] Hello yollandı (protocol={PROTOCOL_VERSION}, name='{player_name}')");

        // ---- Welcome bekle ----
        let welcome_frame = timeout(Duration::from_secs(HELLO_TIMEOUT_SECS), framed.next())
            .await
            .map_err(|_| anyhow!("Welcome {HELLO_TIMEOUT_SECS}s içinde gelmedi"))?
            .ok_or_else(|| anyhow!("server Welcome'dan önce bağlantıyı kapattı"))?
            .context("Welcome frame okuma")?;

        let welcome = decode_server(&welcome_frame).context("Welcome decode")?;
        match welcome {
            ServerMessage::Welcome {
                protocol_version,
                server_version,
                player_id,
                room_id,
            } => {
                println!(
                    "[net] ✓ Welcome — server v{server_version} protocol={protocol_version} \
                    player_id={} room_id={}",
                    player_id.value(),
                    room_id.value()
                );
            }
            ServerMessage::Reject { reason } => {
                println!("[net] ✗ Reject: {reason:?}");
                return Ok(());
            }
            other => {
                return Err(anyhow!("Welcome yerine {other:?} geldi"));
            }
        }

        // ---- Heartbeat döngüsü ----
        println!("[net] heartbeat döngüsü başladı (Ctrl-C ile çık)");
        let mut ticker = interval(Duration::from_millis(HEARTBEAT_PERIOD_MS));
        let mut nonce: u64 = 0;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    nonce = nonce.wrapping_add(1);
                    let ping = ClientMessage::Ping { nonce };
                    if let Err(e) = send(&mut framed, &ping).await {
                        println!("[net] ping yollanamadı: {e}");
                        break;
                    }
                }
                msg = framed.next() => match msg {
                    Some(Ok(frame)) => match decode_server(&frame) {
                        Ok(ServerMessage::Pong { nonce: n }) => {
                            println!("[net] ← Pong nonce={n}");
                        }
                        Ok(other) => {
                            println!("[net] ← {other:?}");
                        }
                        Err(e) => {
                            println!("[net] decode hatası: {e}");
                            break;
                        }
                    },
                    Some(Err(e)) => {
                        println!("[net] stream hatası: {e}");
                        break;
                    }
                    None => {
                        println!("[net] server bağlantıyı kapattı");
                        break;
                    }
                },
                _ = tokio::signal::ctrl_c() => {
                    println!("\n[net] Ctrl-C — Bye yollanıyor");
                    let _ = send(&mut framed, &ClientMessage::Bye).await;
                    break;
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    })
}

async fn send(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    msg: &ClientMessage,
) -> Result<()> {
    let bytes = encode_client(msg).context("encode")?;
    framed.send(Bytes::from(bytes)).await.context("send")?;
    Ok(())
}
