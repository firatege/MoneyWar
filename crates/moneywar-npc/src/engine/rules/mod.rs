//! Rol başına fuzzy rule base.
//!
//! Her modül tek bir `build_engine() -> fuzzy::Engine` fn export eder. Tüm
//! engine'ler aynı `build_standard_vars()` set'ini paylaşır; sadece kurallar
//! farklı.
//!
//! Çıkış (output) anahtarları her rol için ortak (orchestrator gate'ler):
//! - `buy_score`     — bu (city, product)'u almalı mı?
//! - `sell_score`    — bu (city, product)'u satmalı mı?
//! - `bid_aggressiveness` — bid fiyatı agresifliği
//! - `ask_aggressiveness` — ask fiyatı agresifliği
//! - `contract_score`     — kontrat öneri vermeli mi?
//!
//! Sanayici ek: `build_factory_score`. Spekülatör ek: `spread_score`.

pub mod alici;
pub mod esnaf;
pub mod sanayici;
pub mod spekulator;
pub mod tuccar;

use crate::fuzzy::Engine;
use moneywar_domain::{NpcKind, Personality, Role};

/// NPC kategorisi → uygun rule base engine seç.
///
/// Sanayici / Tüccar role'üne ek olarak Esnaf/Spekülatör/Alıcı NpcKind'leri
/// kendi kural setlerini alır. Hiçbiri uymazsa tüccar default.
#[must_use]
pub fn engine_for(role: Role, npc_kind: Option<NpcKind>) -> Engine {
    match npc_kind {
        Some(NpcKind::Esnaf) => esnaf::build_engine(),
        Some(NpcKind::Alici) => alici::build_engine(),
        Some(NpcKind::Spekulator) => spekulator::build_engine(),
        _ => match role {
            Role::Sanayici => sanayici::build_engine(),
            Role::Tuccar => tuccar::build_engine(),
        },
    }
}

/// Personality referansı — ileride bias multiplier hesabı için.
#[allow(dead_code)]
fn _personality_present(_p: Personality) {}
