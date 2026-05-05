//! Kişilik + rol → ağırlık tablosu.
//!
//! Eski fuzzy motorda kişilik output multiplier'ıydı (sezgisiz). Yeni motorda
//! kişilik **ağırlık tablosu**: Aggressive Çiftçi `urgency=0.7`, Hoarder Çiftçi
//! `stock=0.5, urgency=0.2` (stok biriktir).
//!
//! Faz B: Çiftçi default ağırlıkları (kişiliksiz). Faz C+ rol başına dolar.
//! Faz E: TOML config'e taşınır + grid search tuning.

use super::scoring::Weights;
use moneywar_domain::{NpcKind, Personality};

/// Kişilik + NPC kind kombinasyonu için ağırlık seti.
///
/// Faz B: Çiftçi default tanımlı. Diğer roller `Weights::ZERO` (henüz göç olmadı).
/// `personality` `None` ise neutral default. Personality bias Faz E'de eklenecek.
#[must_use]
pub fn for_kind_personality(kind: Option<NpcKind>, _personality: Option<Personality>) -> Weights {
    match kind {
        Some(NpcKind::Ciftci) => ciftci_default(),
        // Faz C+'da doldurulacak roller:
        Some(NpcKind::Alici)
        | Some(NpcKind::Esnaf)
        | Some(NpcKind::Sanayici)
        | Some(NpcKind::Tuccar)
        | Some(NpcKind::Spekulator)
        | Some(NpcKind::Banka)
        | None => Weights::ZERO,
    }
}

/// Çiftçi default ağırlıkları — sell-only mantığı.
/// - `stock +1.0`: stok varsa SAT (ana sürücü)
/// - `urgency +0.5`: sezon sonu agresifleş (mahsul fire riski)
/// - `local_raw_advantage +0.4`: uzmanlık şehrini önceliklendir (specialty bug fix mantığı)
/// - `price_rel_avg +0.3`: pahalıyken sat (kâr maksimize)
/// - `competition -0.2`: rakip baskı varsa bekle
/// - `cash -0.3`: cash düşük → satışa motive (likidite ihtiyacı)
const fn ciftci_default() -> Weights {
    Weights {
        stock: 1.0,
        urgency: 0.5,
        local_raw_advantage: 0.4,
        price_rel_avg: 0.3,
        competition: -0.2,
        cash: -0.3,
        ..Weights::ZERO
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ciftci_weights_emphasize_stock() {
        let w = for_kind_personality(Some(NpcKind::Ciftci), None);
        assert_eq!(w.stock, 1.0);
        assert!(w.urgency > 0.0);
        assert!(w.cash < 0.0);
    }

    #[test]
    fn unmigrated_roles_return_zero() {
        let w = for_kind_personality(Some(NpcKind::Sanayici), None);
        assert_eq!(w, Weights::ZERO);
    }

    #[test]
    fn no_kind_returns_zero() {
        let w = for_kind_personality(None, None);
        assert_eq!(w, Weights::ZERO);
    }
}
