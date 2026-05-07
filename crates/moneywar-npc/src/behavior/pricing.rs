//! NPC fiyatlandırma yardımcıları — fiyat keşfi (price discovery) için.
//!
//! Eski model: NPC'ler `state.effective_baseline()` (statik) okuyor → BUY/SELL
//! aynı fiyat → "donmuş pazar" (Ankara Pamuk hep 615₺ × 90 tick).
//!
//! Yeni model:
//! 1. `state.reference_price()` — son 5 clearing'in ortalaması (rolling avg)
//!    veya yoksa baseline → NPC son trade'lere adapte olur
//! 2. `apply_jitter()` — her (tick, city, product, side) tuple'ı için
//!    deterministik ±5% noise → NPC bid/ask'ları farklılaşır → clearing
//!    fiyatı dağılır → rolling avg drift eder → fiyat keşfi döngüsü açılır
//!
//! Determinism: jitter sadece (tick, city, product, side) hash'ten — RNG yok,
//! aynı state aynı çıktı.

use moneywar_domain::{CityId, Money, OrderSide, ProductKind, Tick};

/// Bu (tick, city, product, side) için ±3% jitter yüzdesi (-3..=+3).
/// Deterministik hash — replay safe. ±3% range'i ilk denenen ±5%'in match
/// sayısını -36% düşürmesinden sonra daraltıldı; fiyat hareketi yeterli,
/// BUY/SELL kesişme alanı korunur.
#[must_use]
pub fn jitter_pct(tick: Tick, city: CityId, product: ProductKind, side: OrderSide) -> i64 {
    let city_idx: u64 = match city {
        CityId::Istanbul => 1,
        CityId::Ankara => 2,
        CityId::Izmir => 3,
    };
    let product_idx: u64 = match product {
        ProductKind::Pamuk => 1,
        ProductKind::Bugday => 2,
        ProductKind::Zeytin => 3,
        ProductKind::Kumas => 4,
        ProductKind::Un => 5,
        ProductKind::Zeytinyagi => 6,
    };
    let side_idx: u64 = match side {
        OrderSide::Buy => 1,
        OrderSide::Sell => 2,
    };
    // FNV-ish karışım — küçük tablo, iyi dağılım için 2654435761 (Knuth).
    let mut h = u64::from(tick.value());
    h = h.wrapping_mul(2_654_435_761);
    h ^= city_idx.wrapping_mul(7);
    h ^= product_idx.wrapping_mul(13);
    h ^= side_idx.wrapping_mul(17);
    h = h.wrapping_mul(2_654_435_761);
    ((h % 7) as i64) - 3
}

/// Fiyata ±3% jitter uygula. Sıfır veya negatif sonucu 1 cent'e clamp eder.
#[must_use]
pub fn apply_jitter(
    price: Money,
    tick: Tick,
    city: CityId,
    product: ProductKind,
    side: OrderSide,
) -> Money {
    let pct = jitter_pct(tick, city, product, side);
    let multiplier = 100i64 + pct;
    let cents = price
        .as_cents()
        .saturating_mul(multiplier)
        .saturating_div(100);
    Money::from_cents(cents.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_in_bounds() {
        for tick in 0u32..50 {
            for city in CityId::ALL {
                for product in ProductKind::ALL {
                    for side in [OrderSide::Buy, OrderSide::Sell] {
                        let p = jitter_pct(Tick::new(tick), city, product, side);
                        assert!((-3..=3).contains(&p), "jitter {p} out of range");
                    }
                }
            }
        }
    }

    #[test]
    fn jitter_varies_across_buckets() {
        // Aynı tick, farklı bucket'lar farklı jitter üretmeli (genelde).
        // Bütün bucket'larda toplama bakıp en az 3 farklı değer var mı kontrol.
        let tick = Tick::new(10);
        let mut seen = std::collections::BTreeSet::new();
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                seen.insert(jitter_pct(tick, city, product, OrderSide::Buy));
            }
        }
        assert!(seen.len() >= 3, "jitter çeşitlilik düşük: {seen:?}");
    }

    #[test]
    fn jitter_deterministic() {
        let p1 = jitter_pct(Tick::new(42), CityId::Ankara, ProductKind::Pamuk, OrderSide::Buy);
        let p2 = jitter_pct(Tick::new(42), CityId::Ankara, ProductKind::Pamuk, OrderSide::Buy);
        assert_eq!(p1, p2);
    }

    #[test]
    fn apply_jitter_preserves_order_of_magnitude() {
        let price = Money::from_cents(1000);
        let jittered = apply_jitter(price, Tick::new(5), CityId::Izmir, ProductKind::Un, OrderSide::Sell);
        // ±3% → 970..=1030
        assert!(jittered.as_cents() >= 970);
        assert!(jittered.as_cents() <= 1030);
    }
}
