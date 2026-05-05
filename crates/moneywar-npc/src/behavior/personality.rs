//! Kişilik + rol → ağırlık tablosu.
//!
//! Eski fuzzy motor `PersonalityBias` ile output multiplier kullanıyordu.
//! Yeni motorda kişilik **ağırlık tablosu**: Aggressive `(w_cash=0.2, w_arb=0.9)`,
//! Hoarder `(w_stock=-0.3, w_price=-0.5)` gibi.
//!
//! Faz A: tüm kombinasyonlar `Weights::ZERO` (placeholder). Faz E'de TOML
//! dosyasından yüklenebilen 7 personality × 7 role tablosuna evrilecek.

use super::scoring::Weights;
use moneywar_domain::{NpcKind, Personality};

/// Kişilik + NPC kind kombinasyonu için ağırlık seti.
///
/// `personality` `None` ise neutral default — kişiliksiz NPC'ler tabloya
/// eşit eğilim gösterir.
#[must_use]
pub fn for_kind_personality(_kind: Option<NpcKind>, _personality: Option<Personality>) -> Weights {
    // Faz A: placeholder. Faz B+'da rol-spesifik ağırlıklar buraya yazılacak.
    Weights::ZERO
}
