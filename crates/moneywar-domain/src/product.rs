//! Ürün kataloğu ve üretim zinciri.
//!
//! 3 üretim zinciri, 6 ürün (game-design.md §4):
//!
//! | Ham madde | Bitmiş ürün | Ucuz üretim yeri |
//! |-----------|-------------|------------------|
//! | Pamuk     | Kumaş       | İstanbul         |
//! | Buğday    | Un          | Ankara           |
//! | Zeytin    | Zeytinyağı  | İzmir            |
//!
//! Bozulma (§4):
//! - Un: 3 tick sonra %100 kayıp (tamamen bozulur)
//! - Zeytinyağı: 5 tick sonra %10 fire
//! - Diğerleri (Pamuk, Kumaş, Buğday, Zeytin): dayanıklı

use serde::{Deserialize, Serialize};

/// 6 ürün çeşidi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProductKind {
    // Ham maddeler
    Pamuk,
    Bugday,
    Zeytin,
    // Bitmiş ürünler
    Kumas,
    Un,
    Zeytinyagi,
}

/// Bir ürünün sınıfı: ham ya da bitmiş.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProductClass {
    Raw,
    Finished,
}

/// Bozulma kuralı. `loss_percent == 100` = ürün tamamen yok olur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Perishability {
    /// Kaç tick depoda beklerse bozulma tetiklenir.
    pub after_ticks: u32,
    /// Kayıp yüzdesi (0-100). 100 = tamamen yok olur.
    pub loss_percent: u32,
}

impl ProductKind {
    /// Tüm ürünler (deterministik sıra: ham → bitmiş).
    pub const ALL: [Self; 6] = [
        Self::Pamuk,
        Self::Bugday,
        Self::Zeytin,
        Self::Kumas,
        Self::Un,
        Self::Zeytinyagi,
    ];

    /// Ham madde listesi.
    pub const RAW_MATERIALS: [Self; 3] = [Self::Pamuk, Self::Bugday, Self::Zeytin];

    /// Bitmiş ürün listesi.
    pub const FINISHED_GOODS: [Self; 3] = [Self::Kumas, Self::Un, Self::Zeytinyagi];

    /// Ürünün sınıfı.
    #[must_use]
    pub const fn class(self) -> ProductClass {
        match self {
            Self::Pamuk | Self::Bugday | Self::Zeytin => ProductClass::Raw,
            Self::Kumas | Self::Un | Self::Zeytinyagi => ProductClass::Finished,
        }
    }

    #[must_use]
    pub const fn is_raw(self) -> bool {
        matches!(self.class(), ProductClass::Raw)
    }

    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(self.class(), ProductClass::Finished)
    }

    /// Bu ham maddenin ürettiği bitmiş ürün. Bitmiş için `None`.
    #[must_use]
    pub const fn finished_output(self) -> Option<Self> {
        match self {
            Self::Pamuk => Some(Self::Kumas),
            Self::Bugday => Some(Self::Un),
            Self::Zeytin => Some(Self::Zeytinyagi),
            _ => None,
        }
    }

    /// Bu bitmiş ürün için gereken ham madde. Ham için `None`.
    #[must_use]
    pub const fn raw_input(self) -> Option<Self> {
        match self {
            Self::Kumas => Some(Self::Pamuk),
            Self::Un => Some(Self::Bugday),
            Self::Zeytinyagi => Some(Self::Zeytin),
            _ => None,
        }
    }

    /// Bozulma kuralı. Dayanıklı ürünler için `None`.
    #[must_use]
    pub const fn perishability(self) -> Option<Perishability> {
        match self {
            Self::Un => Some(Perishability {
                after_ticks: 3,
                loss_percent: 100,
            }),
            Self::Zeytinyagi => Some(Perishability {
                after_ticks: 5,
                loss_percent: 10,
            }),
            _ => None,
        }
    }

    /// Ürün kısa adı (UI + log).
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Pamuk => "Pamuk",
            Self::Bugday => "Buğday",
            Self::Zeytin => "Zeytin",
            Self::Kumas => "Kumaş",
            Self::Un => "Un",
            Self::Zeytinyagi => "Zeytinyağı",
        }
    }
}

impl std::fmt::Display for ProductKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_contains_six_products() {
        assert_eq!(ProductKind::ALL.len(), 6);
    }

    #[test]
    fn raw_and_finished_partition_correctly() {
        assert_eq!(ProductKind::RAW_MATERIALS.len(), 3);
        assert_eq!(ProductKind::FINISHED_GOODS.len(), 3);
        for raw in ProductKind::RAW_MATERIALS {
            assert!(raw.is_raw(), "{raw:?} should be raw");
            assert!(!raw.is_finished());
        }
        for finished in ProductKind::FINISHED_GOODS {
            assert!(finished.is_finished(), "{finished:?} should be finished");
            assert!(!finished.is_raw());
        }
    }

    #[test]
    fn production_chains_are_bijective() {
        assert_eq!(
            ProductKind::Pamuk.finished_output(),
            Some(ProductKind::Kumas)
        );
        assert_eq!(ProductKind::Bugday.finished_output(), Some(ProductKind::Un));
        assert_eq!(
            ProductKind::Zeytin.finished_output(),
            Some(ProductKind::Zeytinyagi)
        );

        assert_eq!(ProductKind::Kumas.raw_input(), Some(ProductKind::Pamuk));
        assert_eq!(ProductKind::Un.raw_input(), Some(ProductKind::Bugday));
        assert_eq!(
            ProductKind::Zeytinyagi.raw_input(),
            Some(ProductKind::Zeytin)
        );
    }

    #[test]
    fn raw_has_no_raw_input() {
        assert!(ProductKind::Pamuk.raw_input().is_none());
        assert!(ProductKind::Bugday.raw_input().is_none());
        assert!(ProductKind::Zeytin.raw_input().is_none());
    }

    #[test]
    fn finished_has_no_finished_output() {
        assert!(ProductKind::Kumas.finished_output().is_none());
        assert!(ProductKind::Un.finished_output().is_none());
        assert!(ProductKind::Zeytinyagi.finished_output().is_none());
    }

    #[test]
    fn un_fully_perishes_after_3_ticks() {
        let p = ProductKind::Un.perishability().unwrap();
        assert_eq!(p.after_ticks, 3);
        assert_eq!(p.loss_percent, 100);
    }

    #[test]
    fn zeytinyagi_partially_perishes_after_5_ticks() {
        let p = ProductKind::Zeytinyagi.perishability().unwrap();
        assert_eq!(p.after_ticks, 5);
        assert_eq!(p.loss_percent, 10);
    }

    #[test]
    fn durable_products_have_no_perishability() {
        assert!(ProductKind::Pamuk.perishability().is_none());
        assert!(ProductKind::Bugday.perishability().is_none());
        assert!(ProductKind::Zeytin.perishability().is_none());
        assert!(ProductKind::Kumas.perishability().is_none());
    }

    #[test]
    fn display_name_uses_turkish_characters() {
        assert_eq!(ProductKind::Kumas.to_string(), "Kumaş");
        assert_eq!(ProductKind::Bugday.to_string(), "Buğday");
        assert_eq!(ProductKind::Zeytinyagi.to_string(), "Zeytinyağı");
    }

    #[test]
    fn serde_roundtrip_via_variant_name() {
        let p = ProductKind::Pamuk;
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"Pamuk\"");
        let back: ProductKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn ordering_is_stable_for_btreemap_keys() {
        let mut v = vec![
            ProductKind::Un,
            ProductKind::Pamuk,
            ProductKind::Zeytinyagi,
            ProductKind::Bugday,
        ];
        v.sort();
        // Enum definition order dictates Ord: Pamuk < Bugday < Zeytin < Kumas < Un < Zeytinyagi
        assert_eq!(
            v,
            vec![
                ProductKind::Pamuk,
                ProductKind::Bugday,
                ProductKind::Un,
                ProductKind::Zeytinyagi,
            ]
        );
    }
}
