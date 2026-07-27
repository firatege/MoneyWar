//! moneywar-web kütüphane yüzeyi.
//!
//! Canlı sunucu (bin `main.rs`) ve headless sim (`moneywar-sim`) **aynı** oyun
//! kurulumunu, sezon sürücüsünü ve log formatını paylaşsın diye ortak modüller
//! burada dışa açılır. Böylece sim ile frontend birebir aynı oyunu oynar —
//! kopyalanmış dünya kurulumu kaynaklı sapma (drift) olmaz.

pub mod archive;
pub mod balance;
pub mod debuglog;
pub mod detail;
pub mod driver;
pub mod dto;
pub mod ledger;
pub mod world;

use moneywar_npc::Difficulty;

/// Sezon uzunluğu (tick). 500 tick × 3 sn ≈ 25 dk.
///
/// 350'den çıkarıldı (v0.13.5). 500 uzun süre oynanamıyordu: depo ve tohum
/// gideri sistemden para siliyordu, sızıntı stok/hasatla birlikte hızlanıyor
/// ve ~t445'te Tüccar, ~t415'te Alıcı eksiye düşüyordu (t500'de makas 38×).
/// İki gider haneye yönlendirilince para arzı +%1'e oturdu ve 500 tick'te
/// tüm roller pozitif kaldı — bkz. `economy.rs`, depo/tohum dağıtımı.
pub const SEASON_TICKS: u32 = 500;
/// Tick aralığı (saniye).
pub const TICK_SECONDS: u64 = 3;
/// Varsayılan base seed ("`MoneyWar`" ASCII) — frontend ile aynı oyun dizisi.
pub const DEFAULT_SEED: u64 = 0x4D6F_6E65_7957_6172;
/// Canlı oyunun zorluğu.
pub const DIFFICULTY: Difficulty = Difficulty::Hard;
