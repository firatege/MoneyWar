//! Fabrika — Sanayici tekeli, ham → bitmiş dönüşümü.
//!
//! v1'de fabrika seviyesi YOK, çoklu fabrika VAR (game-design.md §10).
//! Her fabrika tick başına 10 birim üretir (tentatif, motor parametresi).
//! Üretim süresi 2 tick → batch kuyrukta bekler, `completion_tick`'te
//! envantere döner (Faz 4'te doldurulacak).

use serde::{Deserialize, Serialize};

use crate::{CityId, DomainError, FactoryId, PlayerId, ProductKind, Tick};

/// Üretim kuyruğundaki bir batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FactoryBatch {
    pub started_tick: Tick,
    pub completion_tick: Tick,
    pub units: u32,
}

/// Fabrika.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Factory {
    pub id: FactoryId,
    pub owner: PlayerId,
    pub city: CityId,
    /// Üretilen bitmiş ürün. Ham madde üreten fabrika YOK (ham madde NPC arzından gelir).
    pub product: ProductKind,
    /// En son üretim tamamlanan tick. `None` = hiç üretim yapmadı.
    /// Atıl fabrika (§9 skor kuralı) detection için.
    pub last_production_tick: Option<Tick>,
    /// İşlenmeyi bekleyen batch'ler.
    pub batches: Vec<FactoryBatch>,
}

impl Factory {
    /// Fabrika kurar. Ürün mutlaka bitmiş (finished) olmalı.
    pub fn new(
        id: FactoryId,
        owner: PlayerId,
        city: CityId,
        product: ProductKind,
    ) -> Result<Self, DomainError> {
        if !product.is_finished() {
            return Err(DomainError::Validation(format!(
                "factory must produce a finished good, not {product:?}"
            )));
        }
        Ok(Self {
            id,
            owner,
            city,
            product,
            last_production_tick: None,
            batches: Vec::new(),
        })
    }

    /// Bu fabrikanın ham madde girdisi (Kumaş → Pamuk vb).
    #[must_use]
    pub fn raw_input(&self) -> ProductKind {
        self.product
            .raw_input()
            .expect("finished product always has raw_input by construction")
    }

    /// Son üretimden bu yana kaç tick geçti? `None` = hiç üretim yok.
    #[must_use]
    pub fn ticks_since_last_production(&self, current: Tick) -> Option<u32> {
        self.last_production_tick
            .map(|last| current.value().saturating_sub(last.value()))
    }

    /// Atıl mı? (Son `threshold` tick'te üretim yoksa)
    ///
    /// Skor formülü §9: Son 10 tick'te üretim yapmadıysa fabrika değeri 0.
    #[must_use]
    pub fn is_atil(&self, current: Tick, threshold: u32) -> bool {
        match self.last_production_tick {
            None => current.value() >= threshold,
            Some(_) => self
                .ticks_since_last_production(current)
                .is_some_and(|ticks| ticks >= threshold),
        }
    }

    /// Toplam bekleyen batch birim sayısı.
    #[must_use]
    pub fn pending_units(&self) -> u64 {
        self.batches.iter().map(|b| u64::from(b.units)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_factory() -> Factory {
        Factory::new(
            FactoryId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap()
    }

    #[test]
    fn factory_produces_finished_only() {
        let err = Factory::new(
            FactoryId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk, // raw
        )
        .expect_err("raw not allowed");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn factory_raw_input_follows_chain() {
        let f = test_factory();
        assert_eq!(f.raw_input(), ProductKind::Pamuk);
    }

    #[test]
    fn factory_starts_with_no_batches() {
        let f = test_factory();
        assert_eq!(f.batches.len(), 0);
        assert_eq!(f.pending_units(), 0);
        assert_eq!(f.last_production_tick, None);
    }

    #[test]
    fn factory_never_produced_is_atil_after_threshold() {
        let f = test_factory();
        assert!(!f.is_atil(Tick::new(5), 10));
        assert!(f.is_atil(Tick::new(10), 10));
        assert!(f.is_atil(Tick::new(20), 10));
    }

    #[test]
    fn factory_recent_production_is_not_atil() {
        let mut f = test_factory();
        f.last_production_tick = Some(Tick::new(15));
        assert!(!f.is_atil(Tick::new(20), 10));
    }

    #[test]
    fn factory_old_production_becomes_atil() {
        let mut f = test_factory();
        f.last_production_tick = Some(Tick::new(5));
        assert!(f.is_atil(Tick::new(20), 10)); // 20 - 5 = 15 >= 10
    }

    #[test]
    fn factory_ticks_since_last_production() {
        let mut f = test_factory();
        assert_eq!(f.ticks_since_last_production(Tick::new(10)), None);

        f.last_production_tick = Some(Tick::new(5));
        assert_eq!(f.ticks_since_last_production(Tick::new(12)), Some(7));
    }

    #[test]
    fn factory_pending_units_sums_batches() {
        let mut f = test_factory();
        f.batches.push(FactoryBatch {
            started_tick: Tick::new(1),
            completion_tick: Tick::new(3),
            units: 10,
        });
        f.batches.push(FactoryBatch {
            started_tick: Tick::new(2),
            completion_tick: Tick::new(4),
            units: 10,
        });
        assert_eq!(f.pending_units(), 20);
    }

    #[test]
    fn factory_serde_roundtrip() {
        let f = test_factory();
        let json = serde_json::to_string(&f).unwrap();
        let back: Factory = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }
}
