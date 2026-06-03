//! Özel çiftlik — Sanayici'nin münhasır ham madde kaynağı.
//!
//! Sanayici `BuildPrivateFarm` komutuyla bir şehirde ham madde üreten özel
//! bir çiftlik kurar. Çiftlik her tick sahibinin envanterine doğrudan ham madde
//! yükler; piyasaya satmaz, başka oyuncular bu malı alamaz.
//!
//! Dikey entegrasyon mekaniği:
//! - Ham madde maliyetini sabitler (piyasa fiyatından bağımsız)
//! - Rakipler ham madde açlığı çekerken sahip üretimine devam eder
//! - Fazla üretim piyasaya sabit düşük fiyatla akar (isteğe bağlı)

use serde::{Deserialize, Serialize};

use crate::{CityId, PlayerId, ProductKind};

/// Özel çiftlik kimliği.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PrivateFarmId(pub u64);

impl PrivateFarmId {
    #[must_use]
    pub const fn new(v: u64) -> Self {
        Self(v)
    }
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for PrivateFarmId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PFarm-{}", self.0)
    }
}

/// Sanayici'nin münhasır ham madde üreticisi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateFarm {
    pub id: PrivateFarmId,
    pub owner: PlayerId,
    pub city: CityId,
    /// Üretilen ham madde. Sadece ham ürünler geçerli.
    pub product: ProductKind,
}

impl PrivateFarm {
    #[must_use]
    pub fn new(id: PrivateFarmId, owner: PlayerId, city: CityId, product: ProductKind) -> Self {
        Self { id, owner, city, product }
    }
}
