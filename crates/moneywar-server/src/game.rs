//! Server-side oyun döngüsü — Sprint 3: tick advance + broadcast.
//!
//! `start_game_loop` çağrıldığında bir tokio task spawn olur, her
//! `DEFAULT_TICK_MS` ms'de:
//!
//! 1. Bekleyen insan emirlerini al (PlayerId → Vec<Command>)
//! 2. NPC'lerin emirlerini hesapla (`decide_all_npcs`)
//! 3. `advance_tick(state, all_commands)` çağır
//! 4. Yeni state'i `TickAdvanced` ile tüm peer'lara broadcast et
//! 5. Sezon bitti mi kontrol et — bitti ise döngüden çık
//!
//! State paylaşımı `Arc<Mutex<ServerState>>` üzerinden — connection
//! task'ları bekleyen komutları queue'ya yazıyor, game loop her tick'te
//! drain ediyor.

use std::sync::Arc;
use std::time::Duration;

use moneywar_domain::{Command, GameState};
use moneywar_engine::{advance_tick, rng_for};
use moneywar_net::ServerMessage;
use moneywar_npc::{Difficulty, decide_all_npcs};
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::ServerState;

/// Server difficulty — Sprint 3 default Hard. Sprint 4'te lobi config'inden gelecek.
const SERVER_DIFFICULTY: Difficulty = Difficulty::Hard;

/// Tick advance modu.
#[derive(Debug, Clone, Copy)]
pub enum TickMode {
    /// Server her `ms` ms'de tick advance eder (timer-driven).
    Auto { ms: u64 },
    /// Tüm bağlı oyuncular `AdvanceReady` yollayınca tick advance.
    /// Solo'daki SPACE deneyimi gibi — kimse acele etmiyor.
    Manual,
}

/// Game loop'u arka plan task olarak spawn eder. State `Arc<Mutex<ServerState>>`
/// üzerinden paylaşılır; connection task'ları aynı state'e SubmitCommand ekler.
pub fn spawn_game_loop(state: Arc<Mutex<ServerState>>, mode: TickMode) {
    tokio::spawn(async move {
        run_game_loop(state, mode).await;
    });
}

async fn run_game_loop(state: Arc<Mutex<ServerState>>, mode: TickMode) {
    info!("⏰ game loop başladı (mode={mode:?})");
    match mode {
        TickMode::Auto { ms } => run_auto_loop(state, ms).await,
        TickMode::Manual => run_manual_loop(state).await,
    }
}

async fn run_auto_loop(state: Arc<Mutex<ServerState>>, tick_ms: u64) {
    let mut ticker = interval(Duration::from_millis(tick_ms));
    // İlk tick'i hemen tetikleme — lobby'den çıkışı serbest bırak.
    ticker.tick().await;

    loop {
        ticker.tick().await;

        let result = advance_one_tick(&state).await;
        match result {
            TickOutcome::Continued => { /* normal */ }
            TickOutcome::SeasonEnded => {
                info!("🏁 sezon bitti, game loop sonlanıyor");
                let mut s = state.lock().await;
                s.lobby.game_started = false;
                break;
            }
            TickOutcome::EngineError(msg) => {
                error!("advance_tick hatası: {msg}");
                break;
            }
            TickOutcome::Stopped => {
                info!("game loop durduruldu (game_started=false)");
                break;
            }
        }
    }
}

enum TickOutcome {
    Continued,
    SeasonEnded,
    EngineError(String),
    Stopped,
}

async fn run_manual_loop(state: Arc<Mutex<ServerState>>) {
    // Manual mode: state.advance_notify'a uyandığında tick advance et.
    // İlk tick'i de bekle — başlamak için en az bir AdvanceReady gerekiyor.
    loop {
        let notify = {
            let s = state.lock().await;
            s.advance_notify.clone()
        };
        notify.notified().await;

        let result = advance_one_tick(&state).await;
        match result {
            TickOutcome::Continued => {
                // Bir sonraki tur için tüm oyuncuların ready'sini sıfırla.
                let mut s = state.lock().await;
                s.advance_pending.clear();
            }
            TickOutcome::SeasonEnded => {
                info!("🏁 sezon bitti, manual loop sonlanıyor");
                let mut s = state.lock().await;
                s.lobby.game_started = false;
                break;
            }
            TickOutcome::EngineError(msg) => {
                error!("advance_tick hatası: {msg}");
                break;
            }
            TickOutcome::Stopped => {
                info!("manual loop durduruldu");
                break;
            }
        }
    }
}

/// Bir oyuncudan `AdvanceReady` geldi. Tüm bağlı oyuncular hazır olduğunda
/// game loop'u uyandır. Manual mode'da çağrılır; auto mode'da no-op.
pub async fn handle_advance_ready(
    state: &Arc<Mutex<ServerState>>,
    player_id: moneywar_domain::PlayerId,
) {
    let mut s = state.lock().await;
    if !s.lobby.game_started {
        return;
    }
    s.advance_pending.insert(player_id);
    let all_ready = s
        .lobby
        .slots
        .keys()
        .all(|id| s.advance_pending.contains(id));
    if all_ready && !s.lobby.slots.is_empty() {
        s.advance_notify.notify_one();
    }
}

async fn advance_one_tick(state: &Arc<Mutex<ServerState>>) -> TickOutcome {
    // Pending human commands + game state snapshot al.
    let (mut commands, snapshot, next_tick, room_id) = {
        let mut s = state.lock().await;
        if !s.lobby.game_started {
            return TickOutcome::Stopped;
        }
        let Some(game) = s.game.as_ref() else {
            return TickOutcome::EngineError("game state yok".into());
        };
        let snapshot = game.clone();
        let next_tick = snapshot.current_tick.next();
        let room_id = snapshot.room_id;
        let commands = std::mem::take(&mut s.pending_commands);
        (commands, snapshot, next_tick, room_id)
    };

    // NPC emirlerini hesapla.
    let mut npc_rng = rng_for(room_id, next_tick);
    let npc_commands = decide_all_npcs(&snapshot, &mut npc_rng, next_tick, SERVER_DIFFICULTY);
    commands.extend(npc_commands);

    // advance_tick saf fonksiyon — uzun değilse Mutex dışında çalıştır.
    let advance_result = advance_tick(&snapshot, &commands);

    let new_state = match advance_result {
        Ok((s, _report)) => s,
        Err(e) => return TickOutcome::EngineError(format!("{e}")),
    };

    let season_done = new_state.current_tick.value() >= new_state.config.season_ticks;

    // Yeni state'i state'e yaz, broadcast et.
    let broadcast_msg = {
        let mut s = state.lock().await;
        s.game = Some(new_state.clone());
        ServerMessage::TickAdvanced {
            tick: new_state.current_tick,
            state: Box::new(new_state),
            state_hash: None,
        }
    };

    // Broadcast — Mutex dışında olabilir mi? state'i tutuyoruz çünkü slots oradan.
    {
        let s = state.lock().await;
        s.lobby.broadcast(&broadcast_msg).await;
    }

    if season_done {
        TickOutcome::SeasonEnded
    } else {
        TickOutcome::Continued
    }
}

/// Connection task'larından gelen `SubmitCommand`'ı pending queue'ya ekle.
/// Game loop bir sonraki tick'te drain eder.
pub async fn enqueue_command(state: &Arc<Mutex<ServerState>>, cmd: Command) {
    let mut s = state.lock().await;
    if !s.lobby.game_started {
        warn!("oyun başlamadı, command yok sayılıyor");
        return;
    }
    s.pending_commands.push(cmd);
}

/// Aktif `GameState`'in initial snapshot'ını set et — `start_game` çağrıldığında.
pub async fn install_initial_state(state: &Arc<Mutex<ServerState>>, gs: GameState) {
    let mut s = state.lock().await;
    s.game = Some(gs);
    s.pending_commands.clear();
}
