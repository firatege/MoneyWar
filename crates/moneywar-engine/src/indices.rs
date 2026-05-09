//! Borsa endeksleri — piyasa sağlığı göstergeleri.
//!
//! Reel piyasada BIST 100, NASDAQ gibi: bir grup bucket'ın ağırlıklı ortalama
//! fiyatı. Tek sayı = "piyasa nereye gidiyor". TUI üst banner'da gösterilir,
//! oyuncuya hızlı durum bakışı sağlar.
//!
//! Hesap: qty-ağırlıklı ortalama (son N tick clearing'lerinin price × qty
//! toplamı / qty toplamı). qty bilgisi `price_history`'de yok — şu an basit
//! aritmetik ortalama (rolling_avg) kullanılıyor.
//!
//! Endeks tipleri:
//! - **Tarım**: 3 ham (Pamuk + Buğday + Zeytin) ortalama, 3 şehir × 3 ürün
//! - **Sanayi**: 3 mamul (Kumaş + Un + Zeytinyağı) ortalama, 3 şehir × 3 ürün
//! - **Şehir**: tek şehrin 6 ürün ortalaması
//! - **Ana**: tüm 18 bucket ortalaması (genel piyasa)

use moneywar_domain::{CityId, GameState, Money, ProductKind};

use crate::scoring::PRICE_WINDOW;

/// Endeks türü. UI banner'da gösterilen 4 ana endeks + şehir başına 3 ek.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IndexKind {
    /// Tüm ham ürün ortalaması (Çiftçi sektörü sağlığı).
    Tarim,
    /// Tüm mamul ortalaması (Sanayici performansı).
    Sanayi,
    /// Tek şehrin tüm 6 ürün ortalaması.
    City(CityId),
    /// Tüm 18 bucket ağırlıklı (genel piyasa).
    Ana,
}

impl IndexKind {
    /// Endeks etiketi — banner için kısa.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tarim => "🌾 Tarım",
            Self::Sanayi => "🏭 Sanayi",
            Self::City(CityId::Istanbul) => "🏛 İst",
            Self::City(CityId::Ankara) => "🏛 Ank",
            Self::City(CityId::Izmir) => "🏛 İzm",
            Self::Ana => "📊 Ana",
        }
    }

    /// Endeks etiketi — overlay/charts için tam şehir adlı.
    #[must_use]
    pub const fn long_label(self) -> &'static str {
        match self {
            Self::Tarim => "🌾 Tarım",
            Self::Sanayi => "🏭 Sanayi",
            Self::City(CityId::Istanbul) => "🏛 İstanbul",
            Self::City(CityId::Ankara) => "🏛 Ankara",
            Self::City(CityId::Izmir) => "🏛 İzmir",
            Self::Ana => "📊 Ana",
        }
    }
}

/// Tek bir endeksin değeri ve değişim yüzdesi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketIndex {
    pub kind: IndexKind,
    /// Endeks değeri (qty-ağırlıklı ortalama fiyat).
    pub value: Money,
    /// Bir önceki tick'e göre delta (cents). UI'da +/- gösterilir.
    pub delta_cents: i64,
}

/// Bir endeksi hesapla. Bucket'lardaki rolling avg'ı kullanır (yoksa
/// baseline'a düşer). Hiç veri yoksa `Money::ZERO`.
#[must_use]
pub fn compute_index(state: &GameState, kind: IndexKind) -> MarketIndex {
    let buckets: Vec<(CityId, ProductKind)> = match kind {
        IndexKind::Tarim => CityId::ALL
            .iter()
            .flat_map(|&c| ProductKind::RAW_MATERIALS.iter().map(move |&p| (c, p)))
            .collect(),
        IndexKind::Sanayi => CityId::ALL
            .iter()
            .flat_map(|&c| ProductKind::FINISHED_GOODS.iter().map(move |&p| (c, p)))
            .collect(),
        IndexKind::City(city) => ProductKind::ALL.iter().map(|&p| (city, p)).collect(),
        IndexKind::Ana => CityId::ALL
            .iter()
            .flat_map(|&c| ProductKind::ALL.iter().map(move |&p| (c, p)))
            .collect(),
    };

    let mut total: i64 = 0;
    let mut count: i64 = 0;
    for (city, product) in &buckets {
        let price = state
            .reference_price(*city, *product)
            .unwrap_or(Money::ZERO);
        if price.as_cents() > 0 {
            total = total.saturating_add(price.as_cents());
            count += 1;
        }
    }

    let value = if count > 0 {
        Money::from_cents(total / count)
    } else {
        Money::ZERO
    };

    // Delta: bir önceki tick için aynı endeks değeri. Bunu state'te tutmak
    // pahalı; basit yaklaşım: rolling avg pencere içi son 2 tick'in farkı.
    // Şimdilik 0 (cache layer'da hesaplanır).
    MarketIndex {
        kind,
        value,
        delta_cents: 0,
    }
}

/// Tüm 6 endeksi hesapla (Tarım, Sanayi, 3 şehir, Ana).
#[must_use]
pub fn all_indices(state: &GameState) -> Vec<MarketIndex> {
    let mut out = Vec::with_capacity(6);
    out.push(compute_index(state, IndexKind::Tarim));
    out.push(compute_index(state, IndexKind::Sanayi));
    for city in CityId::ALL {
        out.push(compute_index(state, IndexKind::City(city)));
    }
    out.push(compute_index(state, IndexKind::Ana));
    out
}

#[allow(dead_code)]
const _: usize = PRICE_WINDOW; // Re-export'tan çağrılabilirlik için.

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{RoomConfig, RoomId, Tick};

    fn fresh() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    #[test]
    fn empty_state_zero_index() {
        let s = fresh();
        let idx = compute_index(&s, IndexKind::Ana);
        assert_eq!(idx.value, Money::ZERO);
    }

    #[test]
    fn baseline_only_returns_average() {
        let mut s = fresh();
        // 18 bucket'a aynı baseline (10₺) → endeks 10
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                s.price_baseline
                    .insert((city, product), Money::from_lira(10).unwrap());
            }
        }
        let idx = compute_index(&s, IndexKind::Ana);
        assert_eq!(idx.value, Money::from_lira(10).unwrap());
    }

    #[test]
    fn tarim_only_uses_raw_buckets() {
        let mut s = fresh();
        // Ham 5₺, mamul 30₺ — Tarım endeksi sadece 5'i görmeli
        for city in CityId::ALL {
            for product in ProductKind::RAW_MATERIALS {
                s.price_baseline
                    .insert((city, product), Money::from_lira(5).unwrap());
            }
            for product in ProductKind::FINISHED_GOODS {
                s.price_baseline
                    .insert((city, product), Money::from_lira(30).unwrap());
            }
        }
        let tarim = compute_index(&s, IndexKind::Tarim);
        let sanayi = compute_index(&s, IndexKind::Sanayi);
        assert_eq!(tarim.value, Money::from_lira(5).unwrap());
        assert_eq!(sanayi.value, Money::from_lira(30).unwrap());
    }

    #[test]
    fn city_index_isolates_one_city() {
        let mut s = fresh();
        for product in ProductKind::ALL {
            s.price_baseline
                .insert((CityId::Istanbul, product), Money::from_lira(20).unwrap());
            s.price_baseline
                .insert((CityId::Ankara, product), Money::from_lira(40).unwrap());
            s.price_baseline
                .insert((CityId::Izmir, product), Money::from_lira(60).unwrap());
        }
        let ist = compute_index(&s, IndexKind::City(CityId::Istanbul));
        let ank = compute_index(&s, IndexKind::City(CityId::Ankara));
        assert_eq!(ist.value, Money::from_lira(20).unwrap());
        assert_eq!(ank.value, Money::from_lira(40).unwrap());
    }

    #[test]
    fn rolling_avg_dominates_baseline() {
        let mut s = fresh();
        // baseline 10, history son 5 tick'te 20'lik clearing → avg 20
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                s.price_baseline
                    .insert((city, product), Money::from_lira(10).unwrap());
                s.price_history.insert(
                    (city, product),
                    vec![
                        (Tick::new(1), Money::from_lira(20).unwrap()),
                        (Tick::new(2), Money::from_lira(20).unwrap()),
                    ],
                );
            }
        }
        let idx = compute_index(&s, IndexKind::Ana);
        assert_eq!(idx.value, Money::from_lira(20).unwrap());
    }

    #[test]
    fn all_indices_returns_six() {
        let s = fresh();
        assert_eq!(all_indices(&s).len(), 6);
    }
}
