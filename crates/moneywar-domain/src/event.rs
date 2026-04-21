//! Olay motoru veri modelleri (game-design.md §6).
//!
//! Olaylar tick'lerde üretilir, piyasayı etkiler (arz/talep şoku, rota
//! gecikmesi, talep patlaması). Haber servisi (`news` modülü) bu
//! olayları oyunculara tier'lı dağıtır.

use serde::{Deserialize, Serialize};

use crate::{CityId, ProductKind};

/// Olayın etki büyüklüğü.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventSeverity {
    /// Küçük etki (fiyat %5-10 oynar).
    Minor,
    /// Orta etki (fiyat %10-25 oynar).
    Major,
    /// Makro şok — sezonun son %20'sinde olur (fiyat %25+ oynar).
    Macro,
}

impl EventSeverity {
    /// Motor bu değeri fiyat şok katsayısı olarak kullanır (Faz 6).
    #[must_use]
    pub const fn nominal_shock_percent(self) -> u32 {
        match self {
            Self::Minor => 8,
            Self::Major => 18,
            Self::Macro => 35,
        }
    }
}

/// Oyun dünyasında tetiklenen olay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GameEvent {
    /// Kuraklık — ham madde üretimi düşer, fiyat fırlar.
    Drought {
        city: CityId,
        product: ProductKind,
        severity: EventSeverity,
    },
    /// Grev — üretim durur, fiyat yükselir.
    Strike {
        city: CityId,
        product: ProductKind,
        severity: EventSeverity,
    },
    /// Yol kapandı — kervan `extra_ticks` kadar gecikir.
    RoadClosure {
        from: CityId,
        to: CityId,
        extra_ticks: u32,
        severity: EventSeverity,
    },
    /// Bereketli hasat — arz patladı, fiyat çöker (fırsat).
    BumperHarvest {
        city: CityId,
        product: ProductKind,
        severity: EventSeverity,
    },
    /// Yeni pazar açıldı — geçici talep patlaması.
    NewMarket {
        city: CityId,
        product: ProductKind,
        extra_demand: u32,
    },
}

impl GameEvent {
    /// Olayın doğrudan etkilediği şehirler (yol kapanması = 2 şehir).
    #[must_use]
    pub fn affected_cities(&self) -> Vec<CityId> {
        match *self {
            Self::Drought { city, .. }
            | Self::Strike { city, .. }
            | Self::BumperHarvest { city, .. }
            | Self::NewMarket { city, .. } => vec![city],
            Self::RoadClosure { from, to, .. } => vec![from, to],
        }
    }

    /// Olayın doğrudan etkilediği ürün. Yol kapanması için `None`.
    #[must_use]
    pub const fn affected_product(&self) -> Option<ProductKind> {
        match *self {
            Self::Drought { product, .. }
            | Self::Strike { product, .. }
            | Self::BumperHarvest { product, .. }
            | Self::NewMarket { product, .. } => Some(product),
            Self::RoadClosure { .. } => None,
        }
    }

    /// Bu olay oyuncular için kötü haber mi? (Drought/Strike/RoadClosure = true)
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        matches!(
            self,
            Self::Drought { .. } | Self::Strike { .. } | Self::RoadClosure { .. }
        )
    }

    /// Olayın `severity` değeri. `NewMarket` için `None` (severity kavramı yok).
    #[must_use]
    pub const fn severity(&self) -> Option<EventSeverity> {
        match *self {
            Self::Drought { severity, .. }
            | Self::Strike { severity, .. }
            | Self::RoadClosure { severity, .. }
            | Self::BumperHarvest { severity, .. } => Some(severity),
            Self::NewMarket { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_shock_percent_orders_correctly() {
        assert!(
            EventSeverity::Minor.nominal_shock_percent()
                < EventSeverity::Major.nominal_shock_percent()
        );
        assert!(
            EventSeverity::Major.nominal_shock_percent()
                < EventSeverity::Macro.nominal_shock_percent()
        );
    }

    #[test]
    fn drought_affects_one_city_one_product() {
        let e = GameEvent::Drought {
            city: CityId::Ankara,
            product: ProductKind::Bugday,
            severity: EventSeverity::Major,
        };
        assert_eq!(e.affected_cities(), vec![CityId::Ankara]);
        assert_eq!(e.affected_product(), Some(ProductKind::Bugday));
        assert!(e.is_negative());
        assert_eq!(e.severity(), Some(EventSeverity::Major));
    }

    #[test]
    fn road_closure_affects_two_cities_no_product() {
        let e = GameEvent::RoadClosure {
            from: CityId::Istanbul,
            to: CityId::Izmir,
            extra_ticks: 2,
            severity: EventSeverity::Minor,
        };
        assert_eq!(e.affected_cities(), vec![CityId::Istanbul, CityId::Izmir]);
        assert_eq!(e.affected_product(), None);
        assert!(e.is_negative());
    }

    #[test]
    fn bumper_harvest_is_positive() {
        let e = GameEvent::BumperHarvest {
            city: CityId::Izmir,
            product: ProductKind::Zeytin,
            severity: EventSeverity::Minor,
        };
        assert!(!e.is_negative());
        assert_eq!(e.affected_product(), Some(ProductKind::Zeytin));
    }

    #[test]
    fn new_market_has_no_severity() {
        let e = GameEvent::NewMarket {
            city: CityId::Istanbul,
            product: ProductKind::Kumas,
            extra_demand: 100,
        };
        assert_eq!(e.severity(), None);
        assert!(!e.is_negative());
    }

    #[test]
    fn serde_roundtrip_drought() {
        let e = GameEvent::Drought {
            city: CityId::Ankara,
            product: ProductKind::Bugday,
            severity: EventSeverity::Major,
        };
        let back: GameEvent = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn serde_roundtrip_road_closure() {
        let e = GameEvent::RoadClosure {
            from: CityId::Istanbul,
            to: CityId::Ankara,
            extra_ticks: 3,
            severity: EventSeverity::Minor,
        };
        let back: GameEvent = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(e, back);
    }
}
