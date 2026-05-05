//! Rol-spesifik aday üretimi.
//!
//! Her rol kendi `enumerate(state, player) -> Vec<ActionCandidate>` fonksiyonu
//! ile başlar. Orchestrator (`decide_behavior`) aday listesini alır, her birine
//! utility skor verir, top-K'yı seçer.
//!
//! Faz B: Çiftçi pilot (sell-only).
//! Faz C: Alıcı (buy-only mamul). Sıradaki: Esnaf → Spekülatör → Tüccar → Sanayici → Banka.

pub mod alici;
pub mod ciftci;
