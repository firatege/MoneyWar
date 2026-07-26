//! moneywar-web kütüphane yüzeyi.
//!
//! Canlı sunucu (bin `main.rs`) ve headless sim (`moneywar-sim`) **aynı** oyun
//! kurulumunu, sezon sürücüsünü ve log formatını paylaşsın diye ortak modüller
//! burada dışa açılır. Böylece sim ile frontend birebir aynı oyunu oynar —
//! kopyalanmış dünya kurulumu kaynaklı sapma (drift) olmaz.

pub mod archive;
pub mod balance;
pub mod debuglog;
pub mod driver;
pub mod dto;
pub mod world;

use moneywar_npc::Difficulty;

/// Sezon uzunluğu (tick). 250 tick × 5 sn ≈ 20 dk.
pub const SEASON_TICKS: u32 = 350;
/// Tick aralığı (saniye).
pub const TICK_SECONDS: u64 = 3;
/// Varsayılan base seed ("`MoneyWar`" ASCII) — frontend ile aynı oyun dizisi.
pub const DEFAULT_SEED: u64 = 0x4D6F_6E65_7957_6172;
/// Canlı oyunun zorluğu.
pub const DIFFICULTY: Difficulty = Difficulty::Hard;
