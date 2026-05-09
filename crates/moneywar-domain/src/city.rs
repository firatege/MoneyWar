//! v0.6.0: 5 şehir — İstanbul, Ankara, İzmir, Bursa, Konya.
//!
//! Her şehir:
//! - Bir ham maddeyi ucuza üretir (doğal uzmanlaşma)
//! - Kendi talep profiline sahiptir (luxury / staple / balanced)
//! - Diğerlerine asimetrik mesafede yer alır (tick cinsinden)
//!
//! Sprint A: Bursa (sanayi şehri) + Konya (tarım merkezi) eklendi.

use serde::{Deserialize, Serialize};

use crate::ProductKind;

/// Oyundaki 5 şehir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CityId {
    Istanbul,
    Ankara,
    Izmir,
    Bursa,
    Konya,
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
    pub const ALL: [Self; 5] = [
        Self::Istanbul,
        Self::Ankara,
        Self::Izmir,
        Self::Bursa,
        Self::Konya,
    ];

    /// Bu şehrin ucuza ürettiği ham madde.
    /// İst=Pamuk (Marmara), Ank=Buğday (İç Anadolu), İzm=Zeytin (Ege),
    /// Bursa=Pamuk (Marmara üreticisi, İst ile paylaşır → arbitraj kapanır),
    /// Konya=Buğday (Konya ovası, Ankara ile paylaşır).
    #[must_use]
    pub const fn cheap_raw(self) -> ProductKind {
        match self {
            Self::Istanbul => ProductKind::Pamuk,
            Self::Ankara => ProductKind::Bugday,
            Self::Izmir => ProductKind::Zeytin,
            Self::Bursa => ProductKind::Pamuk,
            Self::Konya => ProductKind::Bugday,
        }
    }

    /// İki şehir arası tick cinsinden mesafe (Türkiye coğrafyası).
    /// Bursa İstanbul'a yakın (1), Konya Ankara/İzmir arası (2-3).
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub const fn distance_to(self, other: Self) -> u32 {
        // Aynı şehir = 0
        if matches!(
            (self, other),
            (Self::Istanbul, Self::Istanbul)
                | (Self::Ankara, Self::Ankara)
                | (Self::Izmir, Self::Izmir)
                | (Self::Bursa, Self::Bursa)
                | (Self::Konya, Self::Konya)
        ) {
            return 0;
        }
        match (self, other) {
            // Eski 3 şehir mesafeleri
            (Self::Istanbul, Self::Ankara) | (Self::Ankara, Self::Istanbul) => {
                crate::balance::DIST_ISTANBUL_ANKARA
            }
            (Self::Ankara, Self::Izmir) | (Self::Izmir, Self::Ankara) => {
                crate::balance::DIST_ANKARA_IZMIR
            }
            (Self::Istanbul, Self::Izmir) | (Self::Izmir, Self::Istanbul) => {
                crate::balance::DIST_ISTANBUL_IZMIR
            }
            // Bursa: İstanbul'un yakın komşusu
            (Self::Istanbul, Self::Bursa) | (Self::Bursa, Self::Istanbul) => 1,
            (Self::Ankara, Self::Bursa) | (Self::Bursa, Self::Ankara) => 3,
            (Self::Izmir, Self::Bursa) | (Self::Bursa, Self::Izmir) => 3,
            // Konya: İç Anadolu, Ankara'ya yakın, İzmir'e orta
            (Self::Ankara, Self::Konya) | (Self::Konya, Self::Ankara) => 2,
            (Self::Istanbul, Self::Konya) | (Self::Konya, Self::Istanbul) => 4,
            (Self::Izmir, Self::Konya) | (Self::Konya, Self::Izmir) => 3,
            (Self::Bursa, Self::Konya) | (Self::Konya, Self::Bursa) => 3,
            // Aynı şehir kombinasyonları yukarıda erken return ile yakalandı.
            _ => 0,
        }
    }

    /// Bu şehirde ürünün talep seviyesi.
    ///
    /// - İstanbul: lüks mal (bitmiş ürünler) talebi yüksek
    /// - Ankara: temel gıda (Buğday, Un) talebi yüksek
    /// - İzmir: dengeli (hepsi Normal)
    /// - Bursa: sanayi şehri — Kumaş yüksek talep (tekstil tarihçesi)
    /// - Konya: tarım merkezi — Un + Buğday yüksek talep
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub const fn demand_for(self, product: ProductKind) -> DemandLevel {
        match (self, product) {
            (Self::Istanbul, ProductKind::Kumas | ProductKind::Zeytinyagi) => DemandLevel::High,
            (Self::Ankara, ProductKind::Bugday | ProductKind::Un) => DemandLevel::High,
            (Self::Bursa, ProductKind::Kumas) => DemandLevel::High,
            (Self::Konya, ProductKind::Un | ProductKind::Bugday) => DemandLevel::High,
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
            Self::Bursa => "Bursa",
            Self::Konya => "Konya",
        }
    }

    /// v0.5: Şehir başına işlem vergisi (yüzde). EVE Online "broker fee"
    /// karşılığı: market clearing'inde alıcıdan ek olarak kesilir, sistem
    /// dışına atılır (hard sink). Şehir karakteri:
    /// - **İstanbul** %3 — büyük metropol, premium maliyet, lüks tüketim
    /// - **Ankara** %2 — başkent, orta
    /// - **İzmir** %1 — liman, kompetitif (arbitraj fırsatı)
    ///
    /// Düşük vergi → arbitraj cazip. Yüksek vergi → likidite primi.
    #[must_use]
    pub const fn transaction_tax_pct(self) -> i64 {
        match self {
            Self::Istanbul => 3,
            Self::Ankara => 2,
            Self::Izmir => 1,
            Self::Bursa => 2, // sanayi şehri, orta
            Self::Konya => 1, // tarım merkezi, düşük (arbitraj cazip)
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
    fn all_contains_five_cities() {
        // v0.6.0 Sprint A: Bursa + Konya eklendi.
        assert_eq!(CityId::ALL.len(), 5);
    }

    #[test]
    fn transaction_tax_per_city_distinct() {
        // İstanbul yüksek → likidite primi; İzmir düşük → arbitraj cazip.
        assert!(CityId::Istanbul.transaction_tax_pct() > CityId::Ankara.transaction_tax_pct());
        assert!(CityId::Ankara.transaction_tax_pct() > CityId::Izmir.transaction_tax_pct());
        // Hiçbiri 0 değil — closed-loop sink her şehirde aktif.
        for c in CityId::ALL {
            assert!(c.transaction_tax_pct() > 0);
        }
    }

    #[test]
    fn each_city_produces_some_raw() {
        // v0.6.0: Bursa+Konya İstanbul/Ankara ile ham paylaşır
        // (specialty rekabeti — arbitraj kapanır).
        assert_eq!(CityId::Istanbul.cheap_raw(), ProductKind::Pamuk);
        assert_eq!(CityId::Ankara.cheap_raw(), ProductKind::Bugday);
        assert_eq!(CityId::Izmir.cheap_raw(), ProductKind::Zeytin);
        assert_eq!(CityId::Bursa.cheap_raw(), ProductKind::Pamuk);
        assert_eq!(CityId::Konya.cheap_raw(), ProductKind::Bugday);
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
        // v3: mesafeler yarıya indi (3,2,4 → 2,1,2). UX hızlandırma.
        assert_eq!(CityId::Istanbul.distance_to(CityId::Ankara), 2);
        assert_eq!(CityId::Ankara.distance_to(CityId::Izmir), 1);
        assert_eq!(CityId::Istanbul.distance_to(CityId::Izmir), 2);
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
