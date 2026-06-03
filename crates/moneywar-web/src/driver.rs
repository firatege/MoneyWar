//! Sezon sürücüsü — sonsuz ekonomi loop'unun kalbi.
//!
//! `step()` tek tick ilerletir; sezon dolduğunda yeni seed ile yeni sezon
//! başlatır. Saf + deterministik motor (`advance_tick` + `decide_all_npcs`)
//! üstüne ince bir sarmalayıcı. Kendi state'ini ve ajan beyinlerini sahiplenir.

use moneywar_domain::{GameState, Tick};
use moneywar_engine::{TickReport, advance_tick, rng_for};
use moneywar_npc::{BrainPool, Difficulty, decide_all_npcs};

use crate::dto::{Snapshot, build_snapshot};
use crate::world::new_season;

/// SplitMix64 — sezon başına farklı ama deterministik seed türetir.
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
            base_seed,
            difficulty,
        }
    }

    /// Tek tick ilerlet. Sezon dolduysa yeni seed ile yeni sezon başlat.
    pub fn step(&mut self) {
        if self.state.current_tick.value() >= self.season_ticks {
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
        )
    }
}
