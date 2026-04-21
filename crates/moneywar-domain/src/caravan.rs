//! Kervan — şehirler arası mal taşıma.
//!
//! Deterministik varış (kayıp riski yok, sadece süre riski). `Caravan`
//! `CaravanState` enum'ı ile `Idle` ve `EnRoute` arasında geçiş yapar.
//! Kapasite yük alırken (`dispatch`) zorlanır (game-design.md §4).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CaravanId, CityId, DomainError, Money, PlayerId, ProductKind, Role, Tick};

/// Kervan yük siparişi — kervanı gönderirken ne yükleneceğini belirler.
/// `Cargo` ile farkı: `CargoSpec` komut payload'ı (serde-friendly, hafif),
/// `Cargo` envanter-ağırlıklı aktif veri yapısı.
pub type CargoSpec = Cargo;

/// Kervana yüklenmiş mal (ürün → miktar).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cargo {
    items: BTreeMap<ProductKind, u32>,
}

impl Cargo {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Yük ekler. Overflow = hata.
    pub fn add(&mut self, product: ProductKind, qty: u32) -> Result<(), DomainError> {
        if qty == 0 {
            return Ok(());
        }
        let entry = self.items.entry(product).or_insert(0);
        *entry = entry
            .checked_add(qty)
            .ok_or_else(|| DomainError::Overflow(format!("cargo add {product} + {qty}")))?;
        Ok(())
    }

    /// Miktarı döndürür (yoksa 0).
    #[must_use]
    pub fn get(&self, product: ProductKind) -> u32 {
        self.items.get(&product).copied().unwrap_or(0)
    }

    /// Toplam yük birimi (kapasite kontrolü için).
    #[must_use]
    pub fn total_units(&self) -> u64 {
        self.items.values().map(|&v| u64::from(v)).sum()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// (product, qty) üstünde deterministik iterasyon.
    pub fn entries(&self) -> impl Iterator<Item = (ProductKind, u32)> + '_ {
        self.items.iter().map(|(&p, &q)| (p, q))
    }
}

/// Kervanın anlık durumu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaravanState {
    /// Durağan, belirli bir şehirde.
    Idle { location: CityId },
    /// Rotada. `arrival_tick`'te `to` şehrine ulaşır.
    EnRoute {
        from: CityId,
        to: CityId,
        arrival_tick: Tick,
        cargo: Cargo,
    },
}

impl CaravanState {
    #[must_use]
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle { .. })
    }

    /// Şu an bulunduğu şehir (`EnRoute` = `None`).
    #[must_use]
    pub fn current_city(&self) -> Option<CityId> {
        match self {
            Self::Idle { location } => Some(*location),
            Self::EnRoute { .. } => None,
        }
    }

    fn discriminant_name(&self) -> &'static str {
        match self {
            Self::Idle { .. } => "Idle",
            Self::EnRoute { .. } => "EnRoute",
        }
    }
}

/// Kervan — oyuncunun taşıma filosundaki bir birim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Caravan {
    pub id: CaravanId,
    pub owner: PlayerId,
    /// Taşıma kapasitesi (birim). Sanayici kervanı 20, Tüccar kervanı 50 başlar.
    pub capacity: u32,
    pub state: CaravanState,
}

impl Caravan {
    /// `§10` rol bazlı başlangıç kapasitesi.
    /// Sanayici küçük (20), Tüccar büyük (50).
    #[must_use]
    pub const fn capacity_for(role: Role) -> u32 {
        match role {
            Role::Sanayici => crate::balance::CARAVAN_CAPACITY_SANAYICI,
            Role::Tuccar => crate::balance::CARAVAN_CAPACITY_TUCCAR,
        }
    }

    /// `§10` rol bazlı satın alma maliyet tablosu.
    /// `existing_count` = sahip olunan mevcut kervan sayısı. Tablo
    /// [`crate::balance`]'ta (Sanayici ve Tüccar için ayrı dizi); index
    /// tablonun uzunluğunu geçerse son değer kullanılır (sabit tavan).
    #[must_use]
    pub fn buy_cost(role: Role, existing_count: u32) -> Money {
        let table: &[i64] = match role {
            Role::Sanayici => &crate::balance::CARAVAN_COSTS_SANAYICI_LIRA,
            Role::Tuccar => &crate::balance::CARAVAN_COSTS_TUCCAR_LIRA,
        };
        let idx = (existing_count as usize).min(table.len() - 1);
        Money::from_lira(table[idx]).expect("fixed literal fits i64")
    }

    /// Yeni kervan. Başlangıç durumu `Idle` (belirtilen şehirde).
    #[must_use]
    pub fn new(id: CaravanId, owner: PlayerId, capacity: u32, starting_city: CityId) -> Self {
        Self {
            id,
            owner,
            capacity,
            state: CaravanState::Idle {
                location: starting_city,
            },
        }
    }

    /// Kervanı yola çıkar.
    ///
    /// Hatalar:
    /// - `InvalidTransition` → kervan zaten `EnRoute` ya da farklı şehirde
    /// - `CapacityExceeded` → kargo kapasiteyi aşıyor
    /// - `Validation` → from == to (aynı şehre rota yok)
    pub fn dispatch(
        &mut self,
        from: CityId,
        to: CityId,
        cargo: Cargo,
        arrival_tick: Tick,
    ) -> Result<(), DomainError> {
        if from == to {
            return Err(DomainError::Validation(format!(
                "caravan cannot dispatch to the same city: {from}"
            )));
        }

        let Some(current) = self.state.current_city() else {
            return Err(DomainError::InvalidTransition {
                entity: "caravan",
                from: self.state.discriminant_name(),
                to: "EnRoute",
            });
        };
        if current != from {
            return Err(DomainError::Validation(format!(
                "caravan is in {current}, cannot dispatch from {from}"
            )));
        }

        let total = cargo.total_units();
        let cap = u64::from(self.capacity);
        if total > cap {
            // total fits in u32 after check, use try_from
            let requested = u32::try_from(total).unwrap_or(u32::MAX);
            return Err(DomainError::CapacityExceeded {
                resource: "caravan",
                limit: self.capacity,
                requested,
            });
        }

        self.state = CaravanState::EnRoute {
            from,
            to,
            arrival_tick,
            cargo,
        };
        Ok(())
    }

    /// Varış — rotayı tamamlar, kargoyu teslim eder. `Idle` olmaya döner.
    /// Caller `current_tick >= arrival_tick` olduğunu garanti etmeli.
    pub fn arrive(&mut self) -> Result<(CityId, Cargo), DomainError> {
        if !matches!(self.state, CaravanState::EnRoute { .. }) {
            return Err(DomainError::InvalidTransition {
                entity: "caravan",
                from: self.state.discriminant_name(),
                to: "Idle (arrived)",
            });
        }
        // Placeholder replaced below; we must own the EnRoute state to destructure.
        let old = std::mem::replace(
            &mut self.state,
            CaravanState::Idle {
                location: CityId::Istanbul,
            },
        );
        let CaravanState::EnRoute { to, cargo, .. } = old else {
            unreachable!("checked above");
        };
        self.state = CaravanState::Idle { location: to };
        Ok((to, cargo))
    }

    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.state.is_idle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled_cargo(qty: u32) -> Cargo {
        let mut c = Cargo::new();
        c.add(ProductKind::Pamuk, qty).unwrap();
        c
    }

    #[test]
    fn cargo_starts_empty() {
        let c = Cargo::new();
        assert!(c.is_empty());
        assert_eq!(c.total_units(), 0);
    }

    #[test]
    fn cargo_accumulates_same_product() {
        let mut c = Cargo::new();
        c.add(ProductKind::Pamuk, 10).unwrap();
        c.add(ProductKind::Pamuk, 15).unwrap();
        assert_eq!(c.get(ProductKind::Pamuk), 25);
    }

    #[test]
    fn cargo_total_sums_all_products() {
        let mut c = Cargo::new();
        c.add(ProductKind::Pamuk, 10).unwrap();
        c.add(ProductKind::Kumas, 5).unwrap();
        assert_eq!(c.total_units(), 15);
    }

    #[test]
    fn cargo_add_zero_is_noop() {
        let mut c = Cargo::new();
        c.add(ProductKind::Pamuk, 0).unwrap();
        assert!(c.is_empty());
    }

    #[test]
    fn cargo_overflow_errors() {
        let mut c = Cargo::new();
        c.add(ProductKind::Pamuk, u32::MAX).unwrap();
        let err = c.add(ProductKind::Pamuk, 1).expect_err("overflow");
        assert!(matches!(err, DomainError::Overflow(_)));
    }

    #[test]
    fn caravan_starts_idle_at_given_city() {
        let c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        assert!(c.is_idle());
        assert_eq!(c.state.current_city(), Some(CityId::Istanbul));
    }

    #[test]
    fn caravan_dispatch_transitions_to_enroute() {
        let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        c.dispatch(
            CityId::Istanbul,
            CityId::Ankara,
            filled_cargo(10),
            Tick::new(5),
        )
        .unwrap();
        assert!(!c.is_idle());
        assert_eq!(c.state.current_city(), None);
    }

    #[test]
    fn caravan_dispatch_same_city_errors() {
        let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        let err = c
            .dispatch(
                CityId::Istanbul,
                CityId::Istanbul,
                filled_cargo(5),
                Tick::new(1),
            )
            .expect_err("same city");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn caravan_dispatch_wrong_origin_errors() {
        let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        let err = c
            .dispatch(CityId::Ankara, CityId::Izmir, filled_cargo(5), Tick::new(1))
            .expect_err("wrong origin");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn caravan_dispatch_over_capacity_errors() {
        let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        let err = c
            .dispatch(
                CityId::Istanbul,
                CityId::Ankara,
                filled_cargo(25),
                Tick::new(5),
            )
            .expect_err("overcap");
        match err {
            DomainError::CapacityExceeded {
                limit, requested, ..
            } => {
                assert_eq!(limit, 20);
                assert_eq!(requested, 25);
            }
            _ => panic!("wrong error kind"),
        }
    }

    #[test]
    fn caravan_dispatch_while_enroute_errors() {
        let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        c.dispatch(
            CityId::Istanbul,
            CityId::Ankara,
            filled_cargo(10),
            Tick::new(5),
        )
        .unwrap();
        // Second dispatch should fail — not Idle
        let err = c
            .dispatch(
                CityId::Ankara,
                CityId::Izmir,
                filled_cargo(5),
                Tick::new(10),
            )
            .expect_err("already enroute");
        assert!(matches!(err, DomainError::InvalidTransition { .. }));
    }

    #[test]
    fn caravan_arrive_transitions_to_idle_at_destination() {
        let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        c.dispatch(
            CityId::Istanbul,
            CityId::Ankara,
            filled_cargo(10),
            Tick::new(5),
        )
        .unwrap();
        let (dest, cargo) = c.arrive().unwrap();
        assert_eq!(dest, CityId::Ankara);
        assert_eq!(cargo.get(ProductKind::Pamuk), 10);
        assert!(c.is_idle());
        assert_eq!(c.state.current_city(), Some(CityId::Ankara));
    }

    #[test]
    fn caravan_arrive_while_idle_errors() {
        let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        let err = c.arrive().expect_err("not enroute");
        assert!(matches!(err, DomainError::InvalidTransition { .. }));
        // State must be preserved after failed arrive
        assert!(c.is_idle());
    }

    #[test]
    fn capacity_matches_role() {
        assert_eq!(Caravan::capacity_for(Role::Sanayici), 20);
        assert_eq!(Caravan::capacity_for(Role::Tuccar), 50);
    }

    #[test]
    fn buy_cost_sanayici_schedule() {
        assert_eq!(Caravan::buy_cost(Role::Sanayici, 0), Money::ZERO);
        assert_eq!(
            Caravan::buy_cost(Role::Sanayici, 1),
            Money::from_lira(5_000).unwrap()
        );
        assert_eq!(
            Caravan::buy_cost(Role::Sanayici, 2),
            Money::from_lira(10_000).unwrap()
        );
        // 3+ sabit 10k
        assert_eq!(
            Caravan::buy_cost(Role::Sanayici, 5),
            Money::from_lira(10_000).unwrap()
        );
    }

    #[test]
    fn buy_cost_tuccar_schedule() {
        assert_eq!(Caravan::buy_cost(Role::Tuccar, 0), Money::ZERO);
        assert_eq!(
            Caravan::buy_cost(Role::Tuccar, 1),
            Money::from_lira(6_000).unwrap()
        );
        assert_eq!(
            Caravan::buy_cost(Role::Tuccar, 2),
            Money::from_lira(10_000).unwrap()
        );
        assert_eq!(
            Caravan::buy_cost(Role::Tuccar, 3),
            Money::from_lira(15_000).unwrap()
        );
        // 4+ sabit 15k
        assert_eq!(
            Caravan::buy_cost(Role::Tuccar, 10),
            Money::from_lira(15_000).unwrap()
        );
    }

    #[test]
    fn caravan_serde_roundtrip_idle() {
        let c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        let json = serde_json::to_string(&c).unwrap();
        let back: Caravan = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn caravan_serde_roundtrip_enroute() {
        let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 20, CityId::Istanbul);
        c.dispatch(
            CityId::Istanbul,
            CityId::Ankara,
            filled_cargo(10),
            Tick::new(5),
        )
        .unwrap();
        let json = serde_json::to_string(&c).unwrap();
        let back: Caravan = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }
}
