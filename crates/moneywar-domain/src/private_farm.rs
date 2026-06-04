//! Özel çiftlik — Sanayici'nin münhasır ham madde kaynağı.

use serde::{Deserialize, Serialize};

use crate::{CityId, PlayerId, ProductKind};

/// Özel çiftlik kimliği.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PrivateFarmId(pub u64);

impl PrivateFarmId {
    #[must_use] pub const fn new(v: u64) -> Self { Self(v) }
    #[must_use] pub const fn value(self) -> u64 { self.0 }
}

impl std::fmt::Display for PrivateFarmId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PFarm-{}", self.0)
    }
}

/// Sanayici'nin münhasır ham madde üreticisi — seviyeli.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateFarm {
    pub id: PrivateFarmId,
    pub owner: PlayerId,
    pub city: CityId,
    pub product: ProductKind,
    /// Çiftlik seviyesi (1-3). Seviye arttıkça daha fazla üretim.
    #[serde(default = "PrivateFarm::default_level")]
    pub level: u8,
}

impl PrivateFarm {
    #[must_use]
    pub fn new(id: PrivateFarmId, owner: PlayerId, city: CityId, product: ProductKind) -> Self {
        Self { id, owner, city, product, level: 1 }
    }

    pub const fn default_level() -> u8 { 1 }

    /// Seviyeye göre tick başına üretim.
    /// lv1=20, lv2=35, lv3=55 birim/tick
    #[must_use]
    pub const fn output_per_tick(&self) -> u32 {
        match self.level {
            1 => 20,
            2 => 35,
            _ => 55, // lv3+
        }
    }

    /// Bir üst seviyeye yükseltme maliyeti.
    #[must_use]
    pub fn upgrade_cost(current_level: u8) -> Option<crate::Money> {
        match current_level {
            1 => crate::Money::from_lira(10_000).ok(),
            2 => crate::Money::from_lira(20_000).ok(),
            _ => None, // max seviye
        }
    }

    pub const FARM_MAX_LEVEL: u8 = 3;
}
