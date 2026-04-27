//! Karar Destek Sistemi (DSS) — NPC AI üst ve orta katmanı.
//!
//! Mimari (3 katman):
//!
//! ```text
//! ┌─ Katman 1: PERSONALITY (statik, sezon başı atanır) ─┐
//! │   7 strateji arketipi: Aggressive / TrendFollower / │
//! │   MeanReverter / Arbitrageur / EventTrader / Hoarder│
//! │   / Cartel — utility ağırlıklarını shape'ler.       │
//! └─────────────────────────────────────────────────────┘
//!                       ↓
//! ┌─ Katman 2: UTILITY AI (her tick, taktik) ───────────┐
//! │   Aksiyon adayları enumerate → her birine expected  │
//! │   utility skoru (kişilik weights ile). Top-K seç.   │
//! └─────────────────────────────────────────────────────┘
//!                       ↓
//! ┌─ Katman 3: FUZZY MODULATION (mikro parametre) ──────┐
//! │   Seçilen aksiyon için qty, fiyat agresifliği vb.   │
//! │   parametreleri fuzzy engine ile ayarla.            │
//! └─────────────────────────────────────────────────────┘

pub mod personality;
pub mod utility;

pub use personality::Personality;
pub use utility::{ActionCandidate, score_action};
