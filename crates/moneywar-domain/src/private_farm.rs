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
    /// Tarlada çalışan ırgat sayısı. Fabrikayla **aynı** işgücü havuzundan
    /// gelir; kadrosuz tarla hasat vermez.
    #[serde(default)]
    pub employees: u32,
    /// Kuruluş tick'i — kurulum beklemesi buradan hesaplanır. Sahibin en
    /// son kurduğu tarlanın üzerinden [`PRIVATE_FARM_BUILD_COOLDOWN`] tick
    /// geçmeden yenisi kurulamaz.
    ///
    /// [`PRIVATE_FARM_BUILD_COOLDOWN`]: crate::balance::PRIVATE_FARM_BUILD_COOLDOWN
    #[serde(default)]
    pub built_at: u32,
}

impl PrivateFarm {
    #[must_use]
    pub fn new(id: PrivateFarmId, owner: PlayerId, city: CityId, product: ProductKind, built_at: u32) -> Self {
        Self { id, owner, city, product, level: 1, employees: 0, built_at }
    }

    pub const fn default_level() -> u8 { 1 }

    /// Seviyeye göre tick başına üretim.
    ///
    /// Taban [`crate::balance::PRIVATE_FARM_OUTPUT_PER_TICK`]'ten gelir;
    /// seviye çarpanları ×1 / ×1.75 / ×2.75. Eskiden burada 20/35/55 gömülü
    /// duruyordu ve denge sabiti hiçbir yerden okunmuyordu — sabiti
    /// değiştirmek ekonomiyi zerre etkilemiyordu (süpürmede üç farklı debi
    /// birebir aynı sonucu verdi, kablonun kopuk olduğu böyle çıktı).
    #[must_use]
    pub const fn output_per_tick(&self) -> u32 {
        let base = crate::balance::scaled_output(crate::balance::PRIVATE_FARM_OUTPUT_PER_TICK);
        let full = match self.level {
            1 => base,
            2 => base * 7 / 4,
            _ => base * 11 / 4, // lv3+
        };
        // Kadro doluluğuyla orantılı hasat. Irgatsız tarla ürün vermez —
        // tohum kendi kendine toplanmıyor.
        full * self.staffing_pct() / 100
    }

    /// Seviyeye göre gereken ırgat sayısı.
    ///
    /// Seviye atlayan tarla daha çok adam ister; yükseltme yalnız debiyi
    /// değil emek ihtiyacını da büyütür.
    #[must_use]
    pub const fn required_employees(&self) -> u32 {
        match self.level {
            1 => crate::balance::PRIVATE_FARM_EMPLOYEES_L1,
            2 => crate::balance::PRIVATE_FARM_EMPLOYEES_L2,
            _ => crate::balance::PRIVATE_FARM_EMPLOYEES_L3,
        }
    }

    /// Kadro doluluğu (0–100). Eksik kadro hasadı orantılı düşürür, fazla
    /// ırgat fayda vermez.
    #[must_use]
    pub const fn staffing_pct(&self) -> u32 {
        let need = self.required_employees();
        if need == 0 {
            return 100;
        }
        let pct = self.employees * 100 / need;
        if pct > 100 { 100 } else { pct }
    }

    /// n'inci tarlanın kurulum maliyeti (`owned` = hâlihazırda sahip olunan).
    ///
    /// Sert kotanın yerini alan iki frenden biri: her ek tarla
    /// [`PRIVATE_FARM_COST_ESCALATION_PCT`] kadar pahalanır. Büyüme serbest
    /// ama ağırlaşıyor.
    ///
    /// [`PRIVATE_FARM_COST_ESCALATION_PCT`]: crate::balance::PRIVATE_FARM_COST_ESCALATION_PCT
    #[must_use]
    pub fn build_cost(owned: usize, slot_taken: usize) -> Option<crate::Money> {
        let base = crate::balance::PRIVATE_FARM_BUILD_COST_LIRA;
        let owner_mult =
            100 + i64::try_from(owned).unwrap_or(0) * crate::balance::PRIVATE_FARM_COST_ESCALATION_PCT;
        // Aynı (şehir, ürün) slotunda tarla varsa ayrıca +%50 — tarla yeri
        // kıymetli, herkes aynı ovaya üşüşmesin.
        let slot_mult = 100 + i64::try_from(slot_taken).unwrap_or(0) * 50;
        crate::Money::from_lira(base * owner_mult / 100 * slot_mult / 100).ok()
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
