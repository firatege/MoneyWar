//! Karar Destek Sistemi (DSS) — NPC AI üst ve orta katmanı.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::single_match_else
)]

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

pub mod inputs;
pub mod personality;
pub mod sanayici;
pub mod tuccar;
pub mod utility;

pub use moneywar_domain::Personality;
pub use personality::{Weights, weights_for};
pub use utility::{ActionCandidate, score_action};
