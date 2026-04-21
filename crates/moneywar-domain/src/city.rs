//! 3 şehir (game-design.md §3): İstanbul, Ankara, İzmir.
//!
//! Her şehir:
//! - Bir ham maddeyi ucuza üretir (doğal uzmanlaşma)
//! - Kendi talep profiline sahiptir (luxury / staple / balanced)
//! - Diğerlerine asimetrik mesafede yer alır (tick cinsinden)

use serde::{Deserialize, Serialize};

use crate::ProductKind;

/// Oyundaki 3 şehir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CityId {
    Istanbul,
    Ankara,
    Izmir,
}

/// Bir şehrin bir ürüne olan talep seviyesi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemandLevel {
    Low,
    Normal,
    High,
}

impl CityId {
    /// Tüm şehirler (deterministik sıra).
    pub const ALL: [Self; 3] = [Self::Istanbul, Self::Ankara, Self::Izmir];

    /// Bu şehrin ucuza ürettiği ham madde.
    #[must_use]
    pub const fn cheap_raw(self) -> ProductKind {
        match self {
            Self::Istanbul => ProductKind::Pamuk,
            Self::Ankara => ProductKind::Bugday,
            Self::Izmir => ProductKind::Zeytin,
        }
    }

    /// İki şehir arası tick cinsinden mesafe. Aynı şehir = 0.
    ///
    /// | Rota | Tick |
    /// |------|------|
    /// | İstanbul ↔ Ankara | 3 |
    /// | Ankara ↔ İzmir | 2 |
    /// | İstanbul ↔ İzmir | 4 |
    #[must_use]
    pub const fn distance_to(self, other: Self) -> u32 {
        match (self, other) {
            (Self::Istanbul, Self::Istanbul)
            | (Self::Ankara, Self::Ankara)
            | (Self::Izmir, Self::Izmir) => 0,
            (Self::Istanbul, Self::Ankara) | (Self::Ankara, Self::Istanbul) => 3,
            (Self::Ankara, Self::Izmir) | (Self::Izmir, Self::Ankara) => 2,
            (Self::Istanbul, Self::Izmir) | (Self::Izmir, Self::Istanbul) => 4,
        }
    }

    /// Bu şehirde ürünün talep seviyesi.
    ///
    /// - İstanbul: lüks mal (bitmiş ürünler) talebi yüksek
    /// - Ankara: temel gıda (Buğday, Un) talebi yüksek
    /// - İzmir: dengeli (hepsi Normal)
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub const fn demand_for(self, product: ProductKind) -> DemandLevel {
        match (self, product) {
            // İstanbul: luxury finished goods
            (Self::Istanbul, ProductKind::Kumas | ProductKind::Zeytinyagi) => DemandLevel::High,
            // Ankara: staple foods (ham + işlenmiş)
            (Self::Ankara, ProductKind::Bugday | ProductKind::Un) => DemandLevel::High,
            // İzmir + diğerleri
            _ => DemandLevel::Normal,
        }
    }

    /// UI + log için şehir adı.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Istanbul => "İstanbul",
            Self::Ankara => "Ankara",
            Self::Izmir => "İzmir",
        }
    }
}

impl std::fmt::Display for CityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_contains_three_cities() {
        assert_eq!(CityId::ALL.len(), 3);
    }

    #[test]
    fn each_city_produces_distinct_raw() {
        assert_eq!(CityId::Istanbul.cheap_raw(), ProductKind::Pamuk);
        assert_eq!(CityId::Ankara.cheap_raw(), ProductKind::Bugday);
        assert_eq!(CityId::Izmir.cheap_raw(), ProductKind::Zeytin);

        let raws: Vec<ProductKind> = CityId::ALL.iter().map(|c| c.cheap_raw()).collect();
        assert_eq!(raws.len(), 3);
        assert_ne!(raws[0], raws[1]);
        assert_ne!(raws[1], raws[2]);
    }

    #[test]
    fn distance_is_symmetric() {
        for a in CityId::ALL {
            for b in CityId::ALL {
                assert_eq!(
                    a.distance_to(b),
                    b.distance_to(a),
                    "distance {a:?}→{b:?} must equal {b:?}→{a:?}"
                );
            }
        }
    }

    #[test]
    fn same_city_distance_is_zero() {
        for c in CityId::ALL {
            assert_eq!(c.distance_to(c), 0);
        }
    }

    #[test]
    fn distance_matches_design() {
        assert_eq!(CityId::Istanbul.distance_to(CityId::Ankara), 3);
        assert_eq!(CityId::Ankara.distance_to(CityId::Izmir), 2);
        assert_eq!(CityId::Istanbul.distance_to(CityId::Izmir), 4);
    }

    #[test]
    fn istanbul_demands_luxury_high() {
        assert_eq!(
            CityId::Istanbul.demand_for(ProductKind::Kumas),
            DemandLevel::High
        );
        assert_eq!(
            CityId::Istanbul.demand_for(ProductKind::Zeytinyagi),
            DemandLevel::High
        );
    }

    #[test]
    fn ankara_demands_staples_high() {
        assert_eq!(
            CityId::Ankara.demand_for(ProductKind::Bugday),
            DemandLevel::High
        );
        assert_eq!(
            CityId::Ankara.demand_for(ProductKind::Un),
            DemandLevel::High
        );
    }

    #[test]
    fn izmir_is_balanced_normal() {
        for p in ProductKind::ALL {
            assert_eq!(
                CityId::Izmir.demand_for(p),
                DemandLevel::Normal,
                "{p:?} should be normal demand in Izmir"
            );
        }
    }

    #[test]
    fn display_uses_turkish_characters() {
        assert_eq!(CityId::Istanbul.to_string(), "İstanbul");
        assert_eq!(CityId::Izmir.to_string(), "İzmir");
    }

    #[test]
    fn serde_roundtrip_via_variant_name() {
        let c = CityId::Istanbul;
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, "\"Istanbul\"");
        let back: CityId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn ordering_is_stable_for_btreemap_keys() {
        let mut v = vec![CityId::Izmir, CityId::Istanbul, CityId::Ankara];
        v.sort();
        assert_eq!(v, vec![CityId::Istanbul, CityId::Ankara, CityId::Izmir]);
    }
}
