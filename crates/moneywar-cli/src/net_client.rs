//! Network client — Sprint 3: lobby + tick döngüsü + emir gönderme.
//!
//! `--connect <addr>` ile bağlanır, lobide rol seçer, ready basar, oyun
//! başlayınca tick'leri canlı izler ve stdin'den emir gönderebilir.
//! TUI yok — stdout demo. Sprint 4'te bu modül ratatui'ye bağlanacak.
//!
//! ## Komutlar
//!
//! ### Lobi
//! - `t` → role = Tüccar
//! - `s` → role = Sanayici
//! - `r` → ready toggle
//! - `q` → çık (Bye)
//!
//! ### Oyun (GameStart sonrası)
//! - `i` → kendi durumumu yazdır (cash + envanter özet)
//! - `l` → leaderboard (skor)
//! - `b <city> <product> <qty> <price>` → BUY emri
//! - `s <city> <product> <qty> <price>` → SELL emri
//! - `f <city> <product>` → BuildFactory (Sanayici)
//! - `c <city>` → BuyCaravan
//! - `q` → Bye + çık
//!
//! Şehir/ürün kısaltmaları: `ist`/`izm`/`ank`, `pamuk`/`bugday`/`zeytin`/
//! `kumas`/`un`/`yag`.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, OrderId, OrderSide, PlayerId, ProductKind,
    Role, Tick,
};
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
        let mut current_state: Option<GameState> = None;
        let mut order_seq: u64 = 1;

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
                        let trimmed = l.trim();
                        if trimmed.is_empty() {
                            // skip
                        } else if current_state.is_none() {
                            handle_lobby_command(trimmed, &mut framed, &mut ready_state).await?;
                        } else {
                            // Game phase
                            let state = current_state.as_ref().unwrap();
                            handle_game_command(
                                trimmed,
                                &mut framed,
                                state,
                                my_id,
                                &mut order_seq,
                            )
                            .await?;
                        }
                        if matches!(trimmed, "q" | "Q") {
                            let _ = send(&mut framed, &ClientMessage::Bye).await;
                            println!("[me] çıkılıyor");
                            break;
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
                            print_help_game();
                            current_state = Some(*initial_state);
                        }
                        Ok(ServerMessage::TickAdvanced { tick, state, .. }) => {
                            print_tick_summary(tick, &state, my_id);
                            current_state = Some(*state);
                        }
                        Ok(ServerMessage::CommandRejected { command: _, reason }) => {
                            println!("[server] emir reddedildi: {reason}");
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

async fn handle_lobby_command(
    cmd: &str,
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    ready_state: &mut bool,
) -> Result<()> {
    match cmd {
        "t" | "T" => {
            send(framed, &ClientMessage::SelectRole { role: Role::Tuccar }).await?;
            println!("[me] → Tüccar seçildi");
            *ready_state = false;
        }
        "s" | "S" => {
            send(
                framed,
                &ClientMessage::SelectRole {
                    role: Role::Sanayici,
                },
            )
            .await?;
            println!("[me] → Sanayici seçildi");
            *ready_state = false;
        }
        "r" | "R" => {
            *ready_state = !*ready_state;
            send(
                framed,
                &ClientMessage::Ready {
                    ready: *ready_state,
                },
            )
            .await?;
            println!("[me] → Ready = {ready_state}");
        }
        "q" | "Q" => { /* outer loop break */ }
        other => println!("[?] bilinmeyen lobi komutu '{other}' — t/s/r/q"),
    }
    Ok(())
}

async fn handle_game_command(
    cmd: &str,
    framed: &mut Framed<TcpStream, LengthDelimitedCodec>,
    state: &GameState,
    my_id: PlayerId,
    order_seq: &mut u64,
) -> Result<()> {
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    if parts.is_empty() {
        return Ok(());
    }
    match parts[0] {
        "i" | "I" => print_my_status(state, my_id),
        "l" | "L" => print_leaderboard(state),
        "?" | "h" | "help" => print_help_game(),
        "b" | "B" | "s" | "S" => {
            // b/s <city> <product> <qty> <price>
            if parts.len() != 5 {
                println!(
                    "[?] kullanım: {} <ist|izm|ank> <pamuk|...> <qty> <price>",
                    parts[0]
                );
                return Ok(());
            }
            let Some(city) = parse_city(parts[1]) else {
                println!("[?] geçersiz şehir '{}'. ist/izm/ank", parts[1]);
                return Ok(());
            };
            let Some(product) = parse_product(parts[2]) else {
                println!("[?] geçersiz ürün '{}'", parts[2]);
                return Ok(());
            };
            let Ok(qty) = parts[3].parse::<u32>() else {
                println!("[?] qty sayı olmalı");
                return Ok(());
            };
            let Ok(price) = parts[4].parse::<f64>() else {
                println!("[?] price sayı olmalı");
                return Ok(());
            };
            let side = if parts[0].eq_ignore_ascii_case("b") {
                OrderSide::Buy
            } else {
                OrderSide::Sell
            };
            let order_id = OrderId::new(client_order_id(my_id, *order_seq));
            *order_seq += 1;
            let price_money = Money::from_cents((price * 100.0).round() as i64);
            let next_tick = state.current_tick.next();
            let order = match MarketOrder::new(
                order_id,
                my_id,
                city,
                product,
                side,
                qty,
                price_money,
                next_tick,
            ) {
                Ok(o) => o,
                Err(e) => {
                    println!("[?] emir oluşturulamadı: {e}");
                    return Ok(());
                }
            };
            send(
                framed,
                &ClientMessage::SubmitCommand {
                    command: Command::SubmitOrder(order),
                },
            )
            .await?;
            println!(
                "[me] {:?} {qty} {product} @ {price_money} {city:?} (sonraki tick'te)",
                side
            );
        }
        "f" | "F" => {
            if parts.len() != 3 {
                println!("[?] kullanım: f <city> <finished_product>");
                return Ok(());
            }
            let Some(city) = parse_city(parts[1]) else {
                return Ok(());
            };
            let Some(product) = parse_product(parts[2]) else {
                return Ok(());
            };
            send(
                framed,
                &ClientMessage::SubmitCommand {
                    command: Command::BuildFactory {
                        owner: my_id,
                        city,
                        product,
                    },
                },
            )
            .await?;
            println!("[me] BuildFactory {product} @ {city:?}");
        }
        "c" | "C" => {
            if parts.len() != 2 {
                println!("[?] kullanım: c <city>");
                return Ok(());
            }
            let Some(city) = parse_city(parts[1]) else {
                return Ok(());
            };
            send(
                framed,
                &ClientMessage::SubmitCommand {
                    command: Command::BuyCaravan {
                        owner: my_id,
                        starting_city: city,
                    },
                },
            )
            .await?;
            println!("[me] BuyCaravan @ {city:?}");
        }
        "q" | "Q" => { /* outer break */ }
        other => println!("[?] bilinmeyen oyun komutu '{other}' — `?` ile yardım"),
    }
    Ok(())
}

fn parse_city(s: &str) -> Option<CityId> {
    match s.to_lowercase().as_str() {
        "ist" | "istanbul" => Some(CityId::Istanbul),
        "izm" | "izmir" => Some(CityId::Izmir),
        "ank" | "ankara" => Some(CityId::Ankara),
        _ => None,
    }
}

fn parse_product(s: &str) -> Option<ProductKind> {
    match s.to_lowercase().as_str() {
        "pamuk" => Some(ProductKind::Pamuk),
        "bugday" => Some(ProductKind::Bugday),
        "zeytin" => Some(ProductKind::Zeytin),
        "kumas" => Some(ProductKind::Kumas),
        "un" => Some(ProductKind::Un),
        "yag" | "zeytinyagi" => Some(ProductKind::Zeytinyagi),
        _ => None,
    }
}

/// Client-tarafı OrderId üreteci. NPC offset'iyle çakışmasın diye düşük aralık.
fn client_order_id(player_id: PlayerId, seq: u64) -> u64 {
    player_id.value().saturating_mul(1_000_000) + seq
}

fn print_help_game() {
    println!();
    println!("┌── 🎮 OYUN KOMUTLARI ──");
    println!("│  i              durumumu göster (cash + envanter)");
    println!("│  l              skor tablosu");
    println!("│  b ist pamuk 50 6.0   → BUY 50 pamuk @ 6.0₺ İstanbul");
    println!("│  s ank kumas 30 18.0  → SELL 30 kumaş @ 18.0₺ Ankara");
    println!("│  f ist kumas    → BuildFactory (Sanayici)");
    println!("│  c izm          → BuyCaravan İzmir'den başlat");
    println!("│  q              çık");
    println!("└─");
}

fn print_my_status(state: &GameState, my_id: PlayerId) {
    let Some(p) = state.players.get(&my_id) else {
        println!("[?] state'te oyuncum yok");
        return;
    };
    println!("[me] {} | nakit: {} | rol: {:?}", p.name, p.cash, p.role);
    let entries: Vec<(CityId, ProductKind, u32)> =
        p.inventory.entries().filter(|(_, _, q)| *q > 0).collect();
    if entries.is_empty() {
        println!("       envanter boş");
    } else {
        for (c, prod, q) in entries.iter().take(8) {
            println!("       {c:?} × {prod}: {q}");
        }
    }
}

fn print_leaderboard(state: &GameState) {
    let mut entries: Vec<(&str, i64)> = state
        .players
        .values()
        .map(|p| (p.name.as_str(), p.cash.as_cents()))
        .collect();
    entries.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    println!("┌── 🏆 SKOR (cash) ──");
    for (i, (name, cents)) in entries.iter().take(10).enumerate() {
        let lira = *cents / 100;
        println!("│ #{:<2} {:<18} {:>8}₺", i + 1, name, lira);
    }
    println!("└─");
}

fn print_tick_summary(tick: Tick, state: &GameState, my_id: PlayerId) {
    let me = state
        .players
        .get(&my_id)
        .map(|p| (p.cash.as_cents() / 100, p.inventory.total_units()))
        .unwrap_or((0, 0));
    let players_alive = state.players.len();
    println!(
        "[tick {:>3}/{}] cash={}₺ stok={} (toplam {} oyuncu)",
        tick.value(),
        state.config.season_ticks,
        me.0,
        me.1,
        players_alive,
    );
}
