//! Rol-spesifik aday üretimi.
//!
//! Her rol kendi `enumerate(state, player) -> Vec<ActionCandidate>` fonksiyonu
//! ile başlar. Orchestrator (`decide_behavior`) aday listesini alır, her birine
//! utility skor verir, top-K'yı seçer.
//!
//! Faz B: Çiftçi pilot (sell-only).
//! Faz C: Alıcı, Sanayici. Sıradaki: Esnaf → Spekülatör → Tüccar → Banka.

pub mod alici;
pub mod ciftci;
pub mod sanayici;
