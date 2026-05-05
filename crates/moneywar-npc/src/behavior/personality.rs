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
pub fn for_kind_personality(kind: Option<NpcKind>, personality: Option<Personality>) -> Weights {
    let base = match kind {
        Some(NpcKind::Ciftci) => ciftci_default(),
        Some(NpcKind::Alici) => alici_default(),
        Some(NpcKind::Sanayici) => sanayici_default(),
        Some(NpcKind::Esnaf) => esnaf_default(),
        Some(NpcKind::Spekulator) => spekulator_default(),
        Some(NpcKind::Tuccar) => tuccar_default(),
        // Banka behavior'da yok — `engine::tick_banks` özel akışı.
        Some(NpcKind::Banka) | None => return Weights::ZERO,
    };
    apply_personality(base, personality)
}

/// Kişilik etkilerini base ağırlıklara uygula.
///
/// Eski fuzzy motorda kişilik output multiplier'ıydı — yeni motorda **sinyal
/// ağırlıklarını** ölçekler. Aynı sinyal farklı kişilik için farklı önem
/// taşır:
/// - **Aggressive**: cash + arbitrage agresif (al/sat hızlı), competition zayıf
/// - **Hoarder**: stock pozitifse satışı bastır (depola), urgency düşük
/// - **Arbitrageur**: arbitrage + event ana sürücü (fırsat avcısı)
/// - **EventTrader**: event + momentum (haber-driven hızlı reaksiyon)
/// - **TrendFollower**: momentum amplify
/// - **MeanReverter**: momentum tersine (yükselen → satar)
/// - **Cartel**: competition negatif amplify (rekabet yerine plan)
fn apply_personality(mut w: Weights, personality: Option<Personality>) -> Weights {
    let Some(p) = personality else {
        return w;
    };
    match p {
        Personality::Aggressive => {
            w.cash *= 1.5;
            w.arbitrage *= 1.5;
            w.competition *= 0.5;
        }
        Personality::Hoarder => {
            // Stoğu satmak yerine biriktir → stock weight'i ters çevirilir
            // (pozitifse negatif olur), urgency'yi azalt.
            w.stock = -w.stock.abs() * 0.5;
            w.urgency *= 0.5;
        }
        Personality::Arbitrageur => {
            w.arbitrage *= 2.0;
            w.event *= 1.5;
        }
        Personality::EventTrader => {
            w.event *= 2.0;
            w.momentum *= 1.5;
        }
        Personality::TrendFollower => {
            w.momentum *= 2.0;
        }
        Personality::MeanReverter => {
            // Trend tersine — momentum sinyalini ters yönde kullan
            w.momentum *= -1.0;
        }
        Personality::Cartel => {
            // Rekabetten kaçın (negatif amplify), uzun vadeli planlama
            w.competition *= 2.0;
            w.urgency *= 0.7;
        }
    }
    w
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

/// Sanayici default ağırlıkları — multi-aksiyon (Build + BUY raw + SELL mamul).
/// Aday tipini `enumerate` filtreliyor; weights "iş yapma motivasyonu":
/// - `cash +0.4`: cash varsa hareket (build/buy)
/// - `urgency +0.3`: sezon ilerledikçe agresifleş
/// - `arbitrage +0.3`: şehirler arası fırsat
/// - `price_rel_avg +0.2`: fiyat fırsatlarını yakala
/// - `competition -0.2`: rakip baskı varsa bekle
/// - `local_raw_advantage +0.2`: uzmanlık şehrini önceliklendir
const fn sanayici_default() -> Weights {
    Weights {
        cash: 0.4,
        urgency: 0.3,
        arbitrage: 0.3,
        price_rel_avg: 0.2,
        local_raw_advantage: 0.2,
        competition: -0.2,
        ..Weights::ZERO
    }
}

/// Spekülatör default ağırlıkları — market maker, sabit spread.
/// - `arbitrage +0.4`: şehir farkı fırsat
/// - `cash +0.3`: cash varsa BID
/// - `event +0.3`: aktif şok pozisyon fırsatı
/// - `momentum +0.2`: trend yönü
/// - `competition -0.1`: rakip baskı (pasif geri çekil)
const fn spekulator_default() -> Weights {
    Weights {
        arbitrage: 0.4,
        cash: 0.3,
        event: 0.3,
        momentum: 0.2,
        competition: -0.1,
        ..Weights::ZERO
    }
}

/// Tüccar default ağırlıkları — şehirler arası arbitraj.
/// - `arbitrage +0.6`: ana sürücü
/// - `cash +0.3`: cash varsa al
/// - `urgency +0.2`: sezon basıncı
/// - `competition -0.2`: rakip baskı
/// - `momentum +0.1`: trend yönü
const fn tuccar_default() -> Weights {
    Weights {
        arbitrage: 0.6,
        cash: 0.3,
        urgency: 0.2,
        momentum: 0.1,
        competition: -0.2,
        ..Weights::ZERO
    }
}

/// Esnaf default ağırlıkları — toptancı, ham mal aracısı.
/// - `cash +0.5`: cash varsa al (BUY ana sürücü)
/// - `arbitrage +0.3`: şehirler arası fark fırsat
/// - `urgency +0.2`: sezon basıncı
/// - `competition -0.2`: rakip baskı
/// - `local_raw_advantage +0.2`: uzmanlık şehir önceliği
const fn esnaf_default() -> Weights {
    Weights {
        cash: 0.5,
        arbitrage: 0.3,
        urgency: 0.2,
        local_raw_advantage: 0.2,
        competition: -0.2,
        ..Weights::ZERO
    }
}

/// Alıcı default ağırlıkları — buy-only tüketici mantığı.
/// - `cash +1.0`: cash varsa AL (ana sürücü, tüketici)
/// - `price_rel_avg -0.5`: ucuzken al (pahalıyken sus)
/// - `stock -0.3`: kendi mamul stoğu varsa iştahı azalt
/// - `momentum +0.2`: yükseliyor → şimdi al (geç kalma)
/// - `urgency +0.2`: sezon sonu hafif basınç
/// - `competition -0.2`: rakip baskı varsa bekle
const fn alici_default() -> Weights {
    Weights {
        cash: 1.0,
        price_rel_avg: -0.5,
        stock: -0.3,
        momentum: 0.2,
        urgency: 0.2,
        competition: -0.2,
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
    fn banka_returns_zero() {
        // Banka behavior'da yok (özel akış engine::tick_banks).
        assert_eq!(
            for_kind_personality(Some(NpcKind::Banka), None),
            Weights::ZERO
        );
    }

    #[test]
    fn all_migrated_roles_have_nonzero_weights() {
        for kind in [
            NpcKind::Ciftci,
            NpcKind::Alici,
            NpcKind::Sanayici,
            NpcKind::Esnaf,
            NpcKind::Spekulator,
            NpcKind::Tuccar,
        ] {
            let w = for_kind_personality(Some(kind), None);
            assert_ne!(w, Weights::ZERO, "{kind:?} göç etti, weights tanımlı olmalı");
        }
    }

    #[test]
    fn aggressive_amplifies_cash_and_arbitrage() {
        let base = for_kind_personality(Some(NpcKind::Sanayici), None);
        let aggro = for_kind_personality(Some(NpcKind::Sanayici), Some(Personality::Aggressive));
        assert!(aggro.cash > base.cash);
        assert!(aggro.arbitrage > base.arbitrage);
        // Competition zayıflar (mutlak değer azalır)
        assert!(aggro.competition.abs() < base.competition.abs());
    }

    #[test]
    fn hoarder_inverts_stock_signal() {
        let base = for_kind_personality(Some(NpcKind::Ciftci), None);
        let hoarder = for_kind_personality(Some(NpcKind::Ciftci), Some(Personality::Hoarder));
        // Çiftçi base stock=+1.0 (sat), Hoarder negatif (sakla)
        assert!(base.stock > 0.0);
        assert!(hoarder.stock < 0.0);
    }

    #[test]
    fn mean_reverter_flips_momentum() {
        let base = Weights {
            momentum: 0.5,
            ..Weights::ZERO
        };
        let flipped = apply_personality(base, Some(Personality::MeanReverter));
        assert_eq!(flipped.momentum, -0.5);
    }

    #[test]
    fn no_personality_keeps_base_weights() {
        let base = ciftci_default();
        let kept = apply_personality(base, None);
        assert_eq!(kept, base);
    }

    #[test]
    fn sanayici_weights_emphasize_cash_and_arbitrage() {
        let w = for_kind_personality(Some(NpcKind::Sanayici), None);
        assert!(w.cash > 0.0);
        assert!(w.arbitrage > 0.0);
        assert!(w.competition < 0.0);
    }

    #[test]
    fn no_kind_returns_zero() {
        let w = for_kind_personality(None, None);
        assert_eq!(w, Weights::ZERO);
    }
}
