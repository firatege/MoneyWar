//! Anlaşma Masası kontratları (game-design.md §2, §7).
//!
//! İki oyuncu arası bağlayıcı söz, kapora escrow ile motor tarafından
//! zorla uygulanır. İki format:
//! - **Kişiye özel:** Sadece seçilen oyuncu kabul edebilir.
//! - **İlan:** Panoda asılı, ilk kapan alır.
//!
//! State machine: `Proposed` → `Active` → (`Fulfilled` | `Breached`).

use serde::{Deserialize, Serialize};

use crate::{CityId, ContractId, DomainError, Money, PlayerId, ProductKind, Tick};

/// Kontrat önerisi payload'ı — oyuncudan gelen niyet. Engine bu spec'ten
/// gerçek `Contract` kurar (ID atar, `proposed_tick` geçirir, validation yapar).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractProposal {
    pub seller: PlayerId,
    pub listing: ListingKind,
    pub product: ProductKind,
    pub quantity: u32,
    pub unit_price: Money,
    pub delivery_city: CityId,
    pub delivery_tick: Tick,
    pub seller_deposit: Money,
    pub buyer_deposit: Money,
}

/// Kontratın liste formatı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListingKind {
    /// Panoda asılı, ilk kapan alır.
    Public,
    /// Sadece hedef oyuncu kabul edebilir.
    Personal { target: PlayerId },
}

impl ListingKind {
    #[must_use]
    pub const fn is_public(&self) -> bool {
        matches!(self, Self::Public)
    }

    #[must_use]
    pub const fn target(&self) -> Option<PlayerId> {
        match self {
            Self::Personal { target } => Some(*target),
            Self::Public => None,
        }
    }
}

/// Kontrat durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractState {
    /// Öneri yapıldı, karşı taraf kabul etmedi.
    Proposed,
    /// Kabul edildi, escrow kilitli, teslimat bekliyor.
    Active,
    /// Teslimat tick'inde şartlar sağlandı, escrow serbest.
    Fulfilled,
    /// Biri caydı, kapora yakıldı. `breacher` caymaci oyuncu.
    Breached { breacher: PlayerId },
}

/// Bağlayıcı kontrat (Anlaşma Masası).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    pub id: ContractId,
    pub seller: PlayerId,
    pub listing: ListingKind,
    /// Kabul eden oyuncu. `Proposed` durumunda `None`.
    pub accepted_by: Option<PlayerId>,
    pub product: ProductKind,
    pub quantity: u32,
    pub unit_price: Money,
    pub delivery_city: CityId,
    pub delivery_tick: Tick,
    /// Satıcı kaporası (escrow). Caydıysa yakılır.
    pub seller_deposit: Money,
    /// Alıcı kaporası (escrow). Caydıysa yakılır.
    pub buyer_deposit: Money,
    pub state: ContractState,
}

impl Contract {
    /// Yeni kontrat önerisi. `Proposed` durumunda başlar.
    ///
    /// Doğrular: `quantity > 0`, `unit_price > 0`, kaporalar ≥ 0,
    /// `delivery_tick > proposed_tick`. `proposed_tick` sadece validation
    /// için referans — kontrat yapısına kaydedilmez.
    #[allow(clippy::too_many_arguments)]
    pub fn propose(
        id: ContractId,
        seller: PlayerId,
        listing: ListingKind,
        product: ProductKind,
        quantity: u32,
        unit_price: Money,
        delivery_city: CityId,
        delivery_tick: Tick,
        proposed_tick: Tick,
        seller_deposit: Money,
        buyer_deposit: Money,
    ) -> Result<Self, DomainError> {
        if quantity == 0 {
            return Err(DomainError::Validation(
                "contract quantity must be > 0".into(),
            ));
        }
        if !unit_price.is_positive() {
            return Err(DomainError::Validation(format!(
                "contract unit_price must be positive, got {unit_price}"
            )));
        }
        if seller_deposit.is_negative() || buyer_deposit.is_negative() {
            return Err(DomainError::Validation(
                "contract deposits must be non-negative".into(),
            ));
        }
        if !delivery_tick.is_before(proposed_tick) && delivery_tick == proposed_tick {
            return Err(DomainError::Validation(
                "delivery_tick must be strictly after proposed_tick".into(),
            ));
        }
        if delivery_tick.is_before(proposed_tick) {
            return Err(DomainError::Validation(
                "delivery_tick cannot be in the past".into(),
            ));
        }
        if let ListingKind::Personal { target } = listing {
            if target == seller {
                return Err(DomainError::Validation(
                    "seller cannot offer a personal contract to self".into(),
                ));
            }
        }

        Ok(Self {
            id,
            seller,
            listing,
            accepted_by: None,
            product,
            quantity,
            unit_price,
            delivery_city,
            delivery_tick,
            seller_deposit,
            buyer_deposit,
            state: ContractState::Proposed,
        })
    }

    /// Kabul et — `Proposed` → `Active`.
    ///
    /// Hatalar:
    /// - Satıcı kendi kontratını kabul edemez (`Validation`)
    /// - Kişiye özel kontratta `by != target` ise (`Validation`)
    /// - Kontrat `Proposed` değilse (`InvalidTransition`)
    pub fn accept(&mut self, by: PlayerId) -> Result<(), DomainError> {
        if self.state != ContractState::Proposed {
            return Err(DomainError::InvalidTransition {
                entity: "contract",
                from: contract_state_name(self.state),
                to: "Active",
            });
        }
        if self.seller == by {
            return Err(DomainError::Validation(
                "seller cannot accept own contract".into(),
            ));
        }
        if let ListingKind::Personal { target } = self.listing {
            if target != by {
                return Err(DomainError::Validation(format!(
                    "personal contract targets {target}, not {by}"
                )));
            }
        }
        self.accepted_by = Some(by);
        self.state = ContractState::Active;
        Ok(())
    }

    /// Başarılı teslimat — `Active` → `Fulfilled`.
    pub fn fulfill(&mut self) -> Result<(), DomainError> {
        if self.state != ContractState::Active {
            return Err(DomainError::InvalidTransition {
                entity: "contract",
                from: contract_state_name(self.state),
                to: "Fulfilled",
            });
        }
        self.state = ContractState::Fulfilled;
        Ok(())
    }

    /// Cayma — `Active` → `Breached`. `breacher` caymaci tarafı.
    pub fn breach(&mut self, breacher: PlayerId) -> Result<(), DomainError> {
        if self.state != ContractState::Active {
            return Err(DomainError::InvalidTransition {
                entity: "contract",
                from: contract_state_name(self.state),
                to: "Breached",
            });
        }
        if breacher != self.seller && Some(breacher) != self.accepted_by {
            return Err(DomainError::Validation(format!(
                "breacher {breacher} is not a party to this contract"
            )));
        }
        self.state = ContractState::Breached { breacher };
        Ok(())
    }

    /// Toplam kontrat değeri (miktar × fiyat). Overflow güvenli.
    pub fn total_value(&self) -> Result<Money, DomainError> {
        self.unit_price.checked_mul_scalar(i64::from(self.quantity))
    }

    /// Toplam escrow tutarı (her iki tarafın kaporası).
    pub fn total_escrow(&self) -> Result<Money, DomainError> {
        self.seller_deposit.checked_add(self.buyer_deposit)
    }
}

const fn contract_state_name(s: ContractState) -> &'static str {
    match s {
        ContractState::Proposed => "Proposed",
        ContractState::Active => "Active",
        ContractState::Fulfilled => "Fulfilled",
        ContractState::Breached { .. } => "Breached",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposed_public() -> Contract {
        Contract::propose(
            ContractId::new(1),
            PlayerId::new(1),
            ListingKind::Public,
            ProductKind::Kumas,
            50,
            Money::from_lira(20).unwrap(),
            CityId::Istanbul,
            Tick::new(10),
            Tick::new(2),
            Money::from_lira(100).unwrap(),
            Money::from_lira(100).unwrap(),
        )
        .unwrap()
    }

    fn proposed_personal(target: PlayerId) -> Contract {
        Contract::propose(
            ContractId::new(2),
            PlayerId::new(1),
            ListingKind::Personal { target },
            ProductKind::Un,
            20,
            Money::from_lira(15).unwrap(),
            CityId::Ankara,
            Tick::new(10),
            Tick::new(2),
            Money::from_lira(50).unwrap(),
            Money::from_lira(50).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn proposed_contract_starts_with_no_buyer() {
        let c = proposed_public();
        assert_eq!(c.state, ContractState::Proposed);
        assert_eq!(c.accepted_by, None);
    }

    #[test]
    fn zero_quantity_rejected() {
        let err = Contract::propose(
            ContractId::new(1),
            PlayerId::new(1),
            ListingKind::Public,
            ProductKind::Kumas,
            0,
            Money::from_lira(10).unwrap(),
            CityId::Istanbul,
            Tick::new(10),
            Tick::new(2),
            Money::ZERO,
            Money::ZERO,
        )
        .expect_err("zero qty");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn negative_deposit_rejected() {
        let err = Contract::propose(
            ContractId::new(1),
            PlayerId::new(1),
            ListingKind::Public,
            ProductKind::Kumas,
            10,
            Money::from_lira(10).unwrap(),
            CityId::Istanbul,
            Tick::new(10),
            Tick::new(2),
            Money::from_cents(-100),
            Money::ZERO,
        )
        .expect_err("neg deposit");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn delivery_must_be_after_proposed() {
        let err = Contract::propose(
            ContractId::new(1),
            PlayerId::new(1),
            ListingKind::Public,
            ProductKind::Kumas,
            10,
            Money::from_lira(10).unwrap(),
            CityId::Istanbul,
            Tick::new(2),
            Tick::new(2),
            Money::ZERO,
            Money::ZERO,
        )
        .expect_err("same tick");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn personal_contract_to_self_rejected() {
        let err = Contract::propose(
            ContractId::new(1),
            PlayerId::new(1),
            ListingKind::Personal {
                target: PlayerId::new(1),
            },
            ProductKind::Kumas,
            10,
            Money::from_lira(10).unwrap(),
            CityId::Istanbul,
            Tick::new(10),
            Tick::new(2),
            Money::ZERO,
            Money::ZERO,
        )
        .expect_err("self personal");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn public_contract_accepted_by_any_buyer() {
        let mut c = proposed_public();
        c.accept(PlayerId::new(99)).unwrap();
        assert_eq!(c.state, ContractState::Active);
        assert_eq!(c.accepted_by, Some(PlayerId::new(99)));
    }

    #[test]
    fn personal_contract_accepted_only_by_target() {
        let target = PlayerId::new(5);
        let mut c = proposed_personal(target);
        let err = c.accept(PlayerId::new(99)).expect_err("wrong buyer");
        assert!(matches!(err, DomainError::Validation(_)));

        // right buyer
        c.accept(target).unwrap();
        assert_eq!(c.state, ContractState::Active);
    }

    #[test]
    fn seller_cannot_accept_own_contract() {
        let mut c = proposed_public();
        let err = c.accept(PlayerId::new(1)).expect_err("self accept");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn double_accept_rejected() {
        let mut c = proposed_public();
        c.accept(PlayerId::new(99)).unwrap();
        let err = c.accept(PlayerId::new(42)).expect_err("double accept");
        assert!(matches!(err, DomainError::InvalidTransition { .. }));
    }

    #[test]
    fn fulfill_requires_active() {
        let mut c = proposed_public();
        let err = c.fulfill().expect_err("not active");
        assert!(matches!(err, DomainError::InvalidTransition { .. }));

        c.accept(PlayerId::new(99)).unwrap();
        c.fulfill().unwrap();
        assert_eq!(c.state, ContractState::Fulfilled);
    }

    #[test]
    fn breach_requires_active() {
        let mut c = proposed_public();
        let err = c.breach(PlayerId::new(1)).expect_err("not active");
        assert!(matches!(err, DomainError::InvalidTransition { .. }));
    }

    #[test]
    fn breach_by_non_party_rejected() {
        let mut c = proposed_public();
        c.accept(PlayerId::new(99)).unwrap();
        let err = c.breach(PlayerId::new(42)).expect_err("stranger");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn breach_by_seller_sets_breacher() {
        let mut c = proposed_public();
        c.accept(PlayerId::new(99)).unwrap();
        c.breach(PlayerId::new(1)).unwrap();
        assert_eq!(
            c.state,
            ContractState::Breached {
                breacher: PlayerId::new(1)
            }
        );
    }

    #[test]
    fn breach_by_buyer_sets_breacher() {
        let mut c = proposed_public();
        c.accept(PlayerId::new(99)).unwrap();
        c.breach(PlayerId::new(99)).unwrap();
        assert_eq!(
            c.state,
            ContractState::Breached {
                breacher: PlayerId::new(99)
            }
        );
    }

    #[test]
    fn total_value_works() {
        let c = proposed_public();
        // 50 × 20₺ = 1000₺
        assert_eq!(c.total_value().unwrap(), Money::from_lira(1_000).unwrap());
    }

    #[test]
    fn total_escrow_sums_deposits() {
        let c = proposed_public();
        assert_eq!(c.total_escrow().unwrap(), Money::from_lira(200).unwrap());
    }

    #[test]
    fn listing_kind_helpers() {
        assert!(ListingKind::Public.is_public());
        assert_eq!(ListingKind::Public.target(), None);

        let t = PlayerId::new(5);
        let personal = ListingKind::Personal { target: t };
        assert!(!personal.is_public());
        assert_eq!(personal.target(), Some(t));
    }

    #[test]
    fn serde_roundtrip_proposed() {
        let c = proposed_public();
        let back: Contract = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn serde_roundtrip_breached() {
        let mut c = proposed_public();
        c.accept(PlayerId::new(99)).unwrap();
        c.breach(PlayerId::new(99)).unwrap();
        let back: Contract = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }
}
