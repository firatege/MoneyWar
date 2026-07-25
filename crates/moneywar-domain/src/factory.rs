//! Fabrika — Sanayici tekeli, ham → bitmiş dönüşümü.
//!
//! v1'de fabrika seviyesi YOK, çoklu fabrika VAR (game-design.md §10).
//! Her fabrika tick başına 10 birim üretir (tentatif, motor parametresi).
//! Üretim süresi 2 tick → batch kuyrukta bekler, `completion_tick`'te
//! envantere döner (Faz 4'te doldurulacak).

use serde::{Deserialize, Serialize};

use crate::{CityId, DomainError, FactoryId, Money, PlayerId, ProductKind, Tick};

/// serde default helper — geriye dönük uyum için eski json'da level yoksa 1.
fn default_factory_level() -> u8 { 1 }

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
    pub last_production_tick: Option<Tick>,
    /// İşlenmeyi bekleyen batch'ler.
    pub batches: Vec<FactoryBatch>,
    /// Fabrika seviyesi (1–3). Seviye arttıkça batch büyür ve hızlanır.
    /// Yeni fabrikalar seviye 1 başlar; `UpgradeFactory` komutuyla artar.
    #[serde(default = "default_factory_level")]
    pub level: u8,
}

impl Factory {
    /// Her fabrika tick başına bu kadar ham madde tüketir / bitmiş ürün üretir.
    /// Değer [`crate::balance::FACTORY_BATCH_SIZE`]'tan gelir.
    pub const BATCH_SIZE: u32 = crate::balance::FACTORY_BATCH_SIZE;

    /// Üretim süresi — batch başlatıldıktan kaç tick sonra biter.
    /// Değer [`crate::balance::FACTORY_PRODUCTION_TICKS`]'ten gelir.
    pub const PRODUCTION_TICKS: u32 = crate::balance::FACTORY_PRODUCTION_TICKS;

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
            level: 1,
        })
    }

    /// Upgrade maliyeti — bu seviyeden bir üst seviyeye geçiş.
    #[must_use]
    pub fn upgrade_cost(current_level: u8) -> Option<crate::Money> {
        match current_level {
            1 => crate::Money::from_lira(crate::balance::FACTORY_UPGRADE_LV2_LIRA).ok(),
            2 => crate::Money::from_lira(crate::balance::FACTORY_UPGRADE_LV3_LIRA).ok(),
            _ => None,
        }
    }

    /// Bu fabrikanın batch boyutu — seviye × ürün katmanı.
    ///
    /// Seviye çarpanı: 1=×1, 2=×1.5, 3+=×2. Katman ölçeği
    /// ([`ProductKind::batch_scale_pct`]) üst katmanları küçültür ki üretim
    /// piramidi doğru yönde dursun (bkz. o fonksiyonun dokümanı).
    #[must_use]
    pub const fn batch_size(&self) -> u32 {
        let by_level = match self.level {
            1 => Self::BATCH_SIZE,
            2 => Self::BATCH_SIZE * 3 / 2,  // +%50
            _ => Self::BATCH_SIZE * 2,       // 3+ → 2×
        };
        let scaled = by_level * self.product.batch_scale_pct() / 100;
        if scaled == 0 { 1 } else { scaled }
    }

    /// Bu seviyedeki üretim tick sayısı. Seviye 1=2, 2=2, 3=1 (daha hızlı).
    #[must_use]
    pub const fn production_ticks(&self) -> u32 {
        match self.level {
            1 | 2 => Self::PRODUCTION_TICKS,
            _ => 1, // seviye 3 → tek tick'te üretim
        }
    }

    /// `§10` kurulum maliyet tablosu. `existing_count` = sahip olunan mevcut
    /// fabrika sayısı (yeni fabrika eklenmeden önce okunur).
    ///
    /// | Sıra | Maliyet |
    /// |---|---|
    /// | 1 (starter) | 0 |
    /// | 2 | 10k |
    /// | 3 | 15k |
    /// | 4 | 22k |
    /// | 5+ | 30k |
    #[must_use]
    pub fn build_cost(existing_count: u32) -> Money {
        let table = &crate::balance::FACTORY_BUILD_COSTS_LIRA;
        let idx = (existing_count as usize).min(table.len() - 1);
        Money::from_lira(table[idx]).expect("fixed literal fits i64")
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
    fn build_cost_follows_design_schedule() {
        // Sabit maliyet: 1. bedava, 2.+ 8K.
        assert_eq!(Factory::build_cost(0), Money::ZERO);
        assert_eq!(Factory::build_cost(1), Money::from_lira(8_000).unwrap());
        assert_eq!(Factory::build_cost(2), Money::from_lira(8_000).unwrap());
        assert_eq!(Factory::build_cost(10), Money::from_lira(8_000).unwrap());
        assert_eq!(Factory::build_cost(99), Money::from_lira(8_000).unwrap());
    }

    #[test]
    fn batch_size_and_duration_constants() {
        assert_eq!(Factory::BATCH_SIZE, 65); // Faz balance: 50→65
        assert_eq!(Factory::PRODUCTION_TICKS, 2);
    }

    #[test]
    fn factory_serde_roundtrip() {
        let f = test_factory();
        let json = serde_json::to_string(&f).unwrap();
        let back: Factory = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }
}
