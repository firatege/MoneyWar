//! Sezon sürücüsü — sonsuz ekonomi loop'unun kalbi.
//!
//! `step()` tek tick ilerletir; sezon dolduğunda yeni seed ile yeni sezon
//! başlatır. Saf + deterministik motor (`advance_tick` + `decide_all_npcs`)
//! üstüne ince bir sarmalayıcı. Kendi state'ini ve ajan beyinlerini sahiplenir.

use serde::Serialize;

use moneywar_domain::{GameState, Tick};
use moneywar_engine::{TickReport, advance_tick, leaderboard, rng_for};
use moneywar_npc::{BrainPool, Difficulty, decide_all_npcs};

use crate::dto::{Snapshot, build_snapshot};
use crate::world::new_season;

/// Bir sezonun sonu özeti — geçmiş sezon tablosu için saklanır.
#[derive(Debug, Clone, Serialize)]
pub struct SeasonSummary {
    pub season: u64,
    pub ticks_completed: u32,
    /// (id, name, `npc_kind`, `pnl_lira`) sıralı skor tablosu.
    pub top: Vec<SeasonEntry>,
}

/// Skor tablosu satırı.
#[derive(Debug, Clone, Serialize)]
pub struct SeasonEntry {
    pub rank: u32,
    pub id: u64,
    pub name: String,
    pub npc_kind: Option<String>,
    pub pnl_lira: f64,
    pub cash_lira: f64,
}

/// `SplitMix64` — sezon başına farklı ama deterministik seed türetir.
fn next_seed(base: u64, season: u64) -> u64 {
    base.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(season.wrapping_mul(0x1234_5678_9ABC_DEF1))
}

/// Sezon döngüsü sürücüsü.
#[derive(Debug)]
pub struct SimDriver {
    pub state: GameState,
    pub last_report: TickReport,
    pub season: u64,
    pub season_ticks: u32,
    pub seconds_per_tick: u32,
    /// Ajan hafıza havuzu — tick'ler arası yaşar, sezon sıfırlasa da korunur.
    pub brains: BrainPool,
    /// Son 20 sezonun özeti.
    pub season_history: Vec<SeasonSummary>,
    base_seed: u64,
    difficulty: Difficulty,
}

impl SimDriver {
    /// Yeni sürücü — 1. sezonu `base_seed`'den kurar.
    #[must_use]
    pub fn new(
        base_seed: u64,
        season_ticks: u32,
        seconds_per_tick: u32,
        difficulty: Difficulty,
    ) -> Self {
        let seed = next_seed(base_seed, 1);
        Self {
            state: new_season(seed),
            last_report: TickReport::new(Tick::ZERO),
            season: 1,
            season_ticks,
            seconds_per_tick,
            brains: BrainPool::default(),
            season_history: Vec::new(),
            base_seed,
            difficulty,
        }
    }

    /// Mevcut sezonun özetini çıkar (skor tablosu + tick).
    fn capture_summary(&self) -> SeasonSummary {
        let lb = leaderboard(&self.state);
        let top = lb
            .into_iter()
            .enumerate()
            .take(15)
            .map(|(i, score)| {
                let pid = score.player_id;
                let p = self.state.players.get(&pid);
                SeasonEntry {
                    rank: u32::try_from(i + 1).unwrap_or(99),
                    id: pid.value(),
                    name: p.map(|pl| pl.name.clone()).unwrap_or_default(),
                    npc_kind: p.and_then(|pl| pl.npc_kind).map(|k| k.label().to_owned()),
                    pnl_lira: score.total.as_cents() as f64 / 100.0,
                    cash_lira: score.cash.as_cents() as f64 / 100.0,
                }
            })
            .collect();
        SeasonSummary {
            season: self.season,
            ticks_completed: self.state.current_tick.value(),
            top,
        }
    }

    /// Mevcut sezonu sıfırla — aynı sezon numarası, tick 0'dan başlar.
    pub fn reset_season(&mut self) {
        let summary = self.capture_summary();
        self.push_history(summary);
        let seed = next_seed(self.base_seed, self.season);
        self.state = new_season(seed);
        self.last_report = TickReport::new(Tick::ZERO);
        tracing::info!(season = self.season, "sezon manuel sıfırlandı");
    }

    fn push_history(&mut self, s: SeasonSummary) {
        self.season_history.push(s);
        if self.season_history.len() > 20 {
            self.season_history.remove(0);
        }
    }

    /// Tek tick ilerlet. Sezon dolduysa yeni seed ile yeni sezon başlat.
    pub fn step(&mut self) {
        if self.state.current_tick.value() >= self.season_ticks {
            let summary = self.capture_summary();
            self.push_history(summary);
            self.season += 1;
            let seed = next_seed(self.base_seed, self.season);
            self.state = new_season(seed);
            self.last_report = TickReport::new(Tick::ZERO);
            tracing::info!(season = self.season, seed, "yeni sezon başladı");
            return;
        }

        let next_tick = Tick::new(self.state.current_tick.value() + 1);
        let mut rng = rng_for(self.state.room_id, next_tick);
        let cmds = decide_all_npcs(&self.state, &mut rng, next_tick, self.difficulty, &mut self.brains);
        match advance_tick(&self.state, &cmds) {
            Ok((next_state, report)) => {
                self.state = next_state;
                self.last_report = report;
            }
            Err(e) => {
                tracing::error!(tick = next_tick.value(), error = %e, "advance_tick hatası");
            }
        }
    }

    /// Mevcut durumun tam snapshot'ı.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        build_snapshot(
            &self.state,
            &self.last_report,
            self.season,
            self.season_ticks,
            self.seconds_per_tick,
            &self.brains,
        )
    }
}
