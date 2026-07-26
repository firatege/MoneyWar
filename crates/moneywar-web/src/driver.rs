//! Sezon sürücüsü — sonsuz ekonomi loop'unun kalbi.
//!
//! `step()` tek tick ilerletir; sezon dolduğunda yeni seed ile yeni sezon
//! başlatır. Saf + deterministik motor (`advance_tick` + `decide_all_npcs`)
//! üstüne ince bir sarmalayıcı. Kendi state'ini ve ajan beyinlerini sahiplenir.

use serde::Serialize;

use moneywar_domain::{GameState, Tick};
use moneywar_engine::{TickReport, advance_tick, leaderboard, rng_for};
use moneywar_npc::{BrainPool, Difficulty, decide_all_npcs};

use crate::balance::{BalanceAccumulator, BalanceReport};
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

/// Kapanan bir sezonun denetim kaydı — arşive yazılacak paket.
#[derive(Debug, Clone)]
pub struct CompletedSeason {
    pub season: u64,
    pub report: BalanceReport,
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
    /// Bu sezonun denge denetimi — her tick beslenir, sezon dönüşünde sıfırlanır.
    /// `GET /api/audit` bunun anlık raporunu döndürür.
    audit: BalanceAccumulator,
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
            audit: BalanceAccumulator::default(),
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
        self.audit = BalanceAccumulator::default();
        self.audit.sample_money(&self.state);
        tracing::info!(season = self.season, "sezon manuel sıfırlandı");
    }

    fn push_history(&mut self, s: SeasonSummary) {
        self.season_history.push(s);
        if self.season_history.len() > 20 {
            self.season_history.remove(0);
        }
    }

    /// Tek tick ilerlet. Sezon dolduysa yeni seed ile yeni sezon başlat.
    ///
    /// Sezon kapandıysa **tamamlanan sezonun** denetim raporunu döndürür;
    /// çağıran onu arşivler. Sürücü diske yazmaz — IO web katmanının işi.
    pub fn step(&mut self) -> Option<CompletedSeason> {
        if self.state.current_tick.value() >= self.season_ticks {
            let summary = self.capture_summary();
            let finished = CompletedSeason {
                season: self.season,
                report: self.audit_report(),
            };
            self.push_history(summary);
            self.season += 1;
            let seed = next_seed(self.base_seed, self.season);
            self.state = new_season(seed);
            self.last_report = TickReport::new(Tick::ZERO);
            self.audit = BalanceAccumulator::default();
            self.audit.sample_money(&self.state);
            tracing::info!(season = self.season, seed, "yeni sezon başladı");
            return Some(finished);
        }

        let next_tick = Tick::new(self.state.current_tick.value() + 1);
        let mut rng = rng_for(self.state.room_id, next_tick);
        let cmds = decide_all_npcs(&self.state, &mut rng, next_tick, self.difficulty, &mut self.brains);
        match advance_tick(&self.state, &cmds) {
            Ok((next_state, report)) => {
                self.state = next_state;
                for entry in &report.entries {
                    self.audit.record(&self.state, &entry.event);
                }
                self.audit.sample_money(&self.state);
                self.last_report = report;
            }
            Err(e) => {
                tracing::error!(tick = next_tick.value(), error = %e, "advance_tick hatası");
            }
        }
        None
    }

    /// Bu sezonun **şimdiye kadarki** denge denetimi.
    ///
    /// Sim'in sezon sonu raporuyla aynı hesap; fark, sezon ortasında da
    /// çağrılabilmesi. "Şu an bir sorun var mı?" sorusunun tek adresi.
    #[must_use]
    pub fn audit_report(&self) -> BalanceReport {
        self.audit.finalize(&self.state)
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


#[cfg(test)]
mod tests {
    use super::*;

    fn stepped(ticks: u32) -> SimDriver {
        let mut d = SimDriver::new(crate::DEFAULT_SEED, 350, 3, crate::DIFFICULTY);
        for _ in 0..ticks {
            d.step();
        }
        d
    }

    #[test]
    fn audit_report_is_available_mid_season() {
        // "Şu an sorun var mı?" sorusu sezon bitmeden sorulabilmeli — sim
        // sezon sonunda rapor üretiyordu, canlı oyunda beklemek işe yaramaz.
        let d = stepped(40);
        let r = d.audit_report();
        assert!(!r.roles.is_empty(), "rol tablosu dolmalı");
        assert!(
            r.money.supply_start > 0,
            "para arzı örneklemesi sezon başından itibaren olmalı"
        );
    }

    #[test]
    fn audit_accumulates_over_ticks() {
        let early = stepped(10).audit_report();
        let late = stepped(60).audit_report();
        let sum = |r: &BalanceReport| -> u64 {
            r.roles.iter().map(|x| x.flow.fills).sum()
        };
        assert!(sum(&late) > sum(&early), "denetim tick'lerle birikmeli");
    }

    #[test]
    fn audit_resets_on_season_rollover() {
        // Sezon dönünce denetim sıfırlanmalı; yoksa yeni sezonun tablosu
        // eskisinin verisiyle kirlenir.
        let mut d = SimDriver::new(crate::DEFAULT_SEED, 5, 3, crate::DIFFICULTY);
        for _ in 0..5 {
            d.step();
        }
        let before_rollover: u64 = d.audit_report().roles.iter().map(|r| r.flow.fills).sum();
        assert!(before_rollover > 0, "sezon içinde fill birikmiş olmalı");

        d.step(); // sezon dolu → yeni sezon
        assert_eq!(d.season, 2);
        let after: u64 = d.audit_report().roles.iter().map(|r| r.flow.fills).sum();
        assert_eq!(after, 0, "yeni sezon temiz denetimle başlamalı");
    }

    #[test]
    fn audit_report_serializes_to_json() {
        // Endpoint JSON döndürüyor; alanların serileşmesi kırılırsa
        // canlıda 500 alırız.
        let r = stepped(20).audit_report();
        let json = serde_json::to_string(&r).expect("rapor serileşmeli");
        assert!(json.contains("roles"));
        assert!(json.contains("money"));
    }
}
