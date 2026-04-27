//! Network client — Sprint 2: lobby etkileşimi + GameStart bekleme.
//!
//! `--connect <addr>` ile bağlanır, lobide rol seçer, ready basar, oyun
//! başlayınca state hash'ini gösterir. TUI yok — stdout demo. Sprint 3'te
//! bu modül `Backend` trait'i üzerinden ratatui uygulamasına bağlanacak.
//!
//! ## Komutlar (stdin)
//!
//! - `t` → role = Tüccar
//! - `s` → role = Sanayici
//! - `r` → ready toggle
//! - `q` → çık (Bye)
//!
//! Her satır basıldıktan sonra Enter ile gönderilir.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use moneywar_domain::{GameState, Role};
use moneywar_net::{
    ClientMessage, LobbyEntry, PROTOCOL_VERSION, ServerMessage, decode_server, encode_client,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{interval, timeout};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");
const HEARTBEAT_PERIOD_MS: u64 = 5_000;
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
        let my_id = match welcome {
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
                player_id
            }
            ServerMessage::Reject { reason } => {
                println!("[net] ✗ Reject: {reason:?}");
                return Ok(());
            }
            other => {
                return Err(anyhow!("Welcome yerine {other:?} geldi"));
            }
        };

        println!("[net] Lobiye girildi. Komutlar:");
        println!("       t → Tüccar    s → Sanayici    r → ready toggle    q → çık");
        println!();

        // ---- Lobi + oyun döngüsü ----
        let mut stdin = BufReader::new(tokio::io::stdin()).lines();
        let mut heartbeat = interval(Duration::from_millis(HEARTBEAT_PERIOD_MS));
        let mut nonce: u64 = 0;
        let mut ready_state = false;

        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    nonce = nonce.wrapping_add(1);
                    if let Err(e) = send(&mut framed, &ClientMessage::Ping { nonce }).await {
                        println!("[net] ping yollanamadı: {e}");
                        break;
                    }
                }

                line = stdin.next_line() => match line {
                    Ok(Some(l)) => {
                        let cmd = l.trim();
                        match cmd {
                            "t" | "T" => {
                                send(&mut framed, &ClientMessage::SelectRole { role: Role::Tuccar }).await?;
                                println!("[me] → Tüccar seçildi");
                                ready_state = false;
                            }
                            "s" | "S" => {
                                send(&mut framed, &ClientMessage::SelectRole { role: Role::Sanayici }).await?;
                                println!("[me] → Sanayici seçildi");
                                ready_state = false;
                            }
                            "r" | "R" => {
                                ready_state = !ready_state;
                                send(&mut framed, &ClientMessage::Ready { ready: ready_state }).await?;
                                println!("[me] → Ready = {ready_state}");
                            }
                            "q" | "Q" => {
                                let _ = send(&mut framed, &ClientMessage::Bye).await;
                                println!("[me] çıkılıyor");
                                break;
                            }
                            "" => {}
                            other => {
                                println!("[?] bilinmeyen komut '{other}' — t/s/r/q");
                            }
                        }
                    }
                    Ok(None) => {
                        // stdin EOF (Ctrl-D) → temiz çıkış
                        let _ = send(&mut framed, &ClientMessage::Bye).await;
                        break;
                    }
                    Err(e) => {
                        println!("[net] stdin hatası: {e}");
                        break;
                    }
                },

                msg = framed.next() => match msg {
                    Some(Ok(frame)) => match decode_server(&frame) {
                        Ok(ServerMessage::Pong { .. }) => { /* sessiz */ }
                        Ok(ServerMessage::LobbyState { entries, host }) => {
                            print_lobby(&entries, my_id, host);
                        }
                        Ok(ServerMessage::PlayerLeft { player_id, clean }) => {
                            let tag = if clean { "temiz" } else { "kopuk" };
                            println!("[lobby] player_id={} ayrıldı ({tag})", player_id.value());
                        }
                        Ok(ServerMessage::GameStart { initial_state, tick_ms }) => {
                            print_game_start(&initial_state, tick_ms);
                            // Sprint 2: oyun başladığında demo modu burada bitiyor.
                            // Sprint 3'te gerçek tick döngüsü başlayacak.
                            println!("[net] Sprint 2 demo: GameStart alındı, çıkılıyor.");
                            let _ = send(&mut framed, &ClientMessage::Bye).await;
                            break;
                        }
                        Ok(ServerMessage::Reject { reason }) => {
                            println!("[net] ✗ Reject: {reason:?}");
                            break;
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
                    let _ = send(&mut framed, &ClientMessage::Bye).await;
                    println!("\n[net] Ctrl-C — çıkılıyor");
                    break;
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    })
}

fn print_lobby(
    entries: &[LobbyEntry],
    me: moneywar_domain::PlayerId,
    host: moneywar_domain::PlayerId,
) {
    println!("┌── 🎮 LOBBY ({} kişi) ──", entries.len());
    for e in entries {
        let me_marker = if e.player_id == me { "👈 (sen)" } else { "" };
        let host_marker = if e.player_id == host { " 👑" } else { "" };
        let role_label = e
            .role
            .map(|r| match r {
                Role::Tuccar => "Tüccar",
                Role::Sanayici => "Sanayici",
            })
            .unwrap_or("?");
        let ready_label = if e.ready { "✓ ready" } else { "  bekliyor" };
        println!(
            "│ #{:<2} {:<14} {:<10} {} {}{}",
            e.player_id.value(),
            e.player_name,
            role_label,
            ready_label,
            host_marker,
            me_marker
        );
    }
    println!("└─ komutlar: t/s rol, r ready, q çık");
}

fn print_game_start(state: &GameState, tick_ms: u64) {
    let humans = state.players.values().filter(|p| !p.is_npc).count();
    let npcs = state.players.values().filter(|p| p.is_npc).count();
    let hash = simple_state_hash(state);
    println!();
    println!("════════════════════════════════════════════════════════");
    println!("🎮 GAME START — tick_ms={tick_ms}");
    println!("   {} insan + {} NPC", humans, npcs);
    println!("   room_id = {}", state.room_id.value());
    println!("   state_hash = 0x{:016x}", hash);
    println!("════════════════════════════════════════════════════════");
}

/// Hızlı determinism check için basit fnv-1a tabanlı hash. Postcard
/// canonical encoding aynı state'e aynı bytes garantisi verir.
fn simple_state_hash(state: &GameState) -> u64 {
    let bytes = postcard::to_allocvec(state).unwrap_or_default();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in &bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

async fn send(
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    msg: &ClientMessage,
) -> Result<()> {
    let bytes = encode_client(msg).context("encode")?;
    framed.send(Bytes::from(bytes)).await.context("send")?;
    Ok(())
}
