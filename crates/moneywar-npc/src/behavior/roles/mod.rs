//! Rol-spesifik aday üretimi.
//!
//! Her rol kendi `enumerate(state, player) -> Vec<ActionCandidate>` fonksiyonu
//! ile başlar. Orchestrator (`decide_behavior`) aday listesini alır, her birine
//! utility skor verir, top-K'yı seçer.
//!
//! Faz B: Çiftçi pilot (sell-only).
//! Faz C: Alıcı, Sanayici, Esnaf, Spekülatör, Tüccar (DONE).
//! Banka skip — özel akış (`engine::tick_banks`), behavior'da boş.

pub mod alici;
pub mod ciftci;
pub mod esnaf;
pub mod sanayici;
pub mod spekulator;
pub mod tuccar;
