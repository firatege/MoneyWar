//! Oyuncu (veya NPC) tarafından motor'a gönderilen komutlar.
//!
//! Motor her tick bu komutları sırayla işler (Faz 2'de `advance_tick`).
//! Komutlar saf niyet bildirir — doğrulama motor tarafında yapılır.

use serde::{Deserialize, Serialize};

use crate::{
    CaravanId, CargoSpec, CityId, ContractId, ContractProposal, LoanId, MarketOrder, Money,
    NewsTier, OrderId, PlayerId, ProductKind,
};

/// Oyuncu tarafından motor'a gönderilen komut.
///
/// Her tick başı oyuncu kuyruğuna eklenen komutlar Faz 1'de (emir toplama)
/// işlenir. Komut işleme sonucu:
/// - Kabul edildi → state değişir
/// - Reddedildi → hata raporu oyuncuya döner (`DomainError`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    /// Hal Pazarı limit emri gönder.
    SubmitOrder(MarketOrder),

    /// Önceden gönderilen emri iptal et (tick açılmadan önce).
    CancelOrder {
        order_id: OrderId,
        requester: PlayerId,
    },

    /// Kontrat önerisi aç (kişiye özel veya ilan).
    ProposeContract(ContractProposal),

    /// Açık bir kontrat önerisini kabul et.
    AcceptContract {
        contract_id: ContractId,
        acceptor: PlayerId,
    },

    /// Kendi önerdiği kontratı geri çek.
    CancelContractProposal {
        contract_id: ContractId,
        requester: PlayerId,
    },

    /// Yeni fabrika kur (Sanayici tekeli).
    BuildFactory {
        owner: PlayerId,
        city: CityId,
        product: ProductKind,
    },

    /// Yeni kervan satın al (rol'e göre kapasite farklı).
    BuyCaravan {
        owner: PlayerId,
        starting_city: CityId,
    },

    /// Kervan gönder (başka şehre yol çık).
    DispatchCaravan {
        caravan_id: CaravanId,
        from: CityId,
        to: CityId,
        cargo: CargoSpec,
    },

    /// Haber servisi aboneliği değiştir.
    SubscribeNews { player: PlayerId, tier: NewsTier },

    /// NPC bankasından kredi al (Faz 5.5).
    TakeLoan {
        player: PlayerId,
        amount: Money,
        duration_ticks: u32,
    },

    /// Kredi geri öde (Faz 5.5).
    RepayLoan { player: PlayerId, loan_id: LoanId },
}

impl Command {
    /// Bu komutun sahibi oyuncu. Dispatch/routing + rate-limit için.
    #[must_use]
    pub const fn requester(&self) -> PlayerId {
        match self {
            Self::SubmitOrder(o) => o.player,
            Self::CancelOrder { requester, .. }
            | Self::CancelContractProposal { requester, .. } => *requester,
            Self::ProposeContract(p) => p.seller,
            Self::AcceptContract { acceptor, .. } => *acceptor,
            Self::BuildFactory { owner, .. } | Self::BuyCaravan { owner, .. } => *owner,
            Self::DispatchCaravan { .. } => {
                // Dispatch'in sahibi caravan'ın sahibi — motor validate eder.
                // Burada placeholder: komut engine'e gidince caravan lookup yapılır.
                // Routing için requester_hint field eklenmeli mi? V1'de keep simple:
                // NPC/player ayrımı engine'de yapılır.
                PlayerId::new(0)
            }
            Self::SubscribeNews { player, .. }
            | Self::TakeLoan { player, .. }
            | Self::RepayLoan { player, .. } => *player,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ListingKind, MarketOrder, OrderSide, ProductKind, Tick};

    fn sample_order() -> MarketOrder {
        MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(42),
            CityId::Istanbul,
            ProductKind::Pamuk,
            OrderSide::Buy,
            100,
            Money::from_lira(10).unwrap(),
            Tick::new(5),
        )
        .unwrap()
    }

    fn sample_proposal() -> ContractProposal {
        ContractProposal {
            seller: PlayerId::new(7),
            listing: ListingKind::Public,
            product: ProductKind::Kumas,
            quantity: 50,
            unit_price: Money::from_lira(20).unwrap(),
            delivery_city: CityId::Istanbul,
            delivery_tick: Tick::new(10),
            seller_deposit: Money::from_lira(100).unwrap(),
            buyer_deposit: Money::from_lira(100).unwrap(),
        }
    }

    #[test]
    fn submit_order_requester_is_order_player() {
        let cmd = Command::SubmitOrder(sample_order());
        assert_eq!(cmd.requester(), PlayerId::new(42));
    }

    #[test]
    fn propose_contract_requester_is_seller() {
        let cmd = Command::ProposeContract(sample_proposal());
        assert_eq!(cmd.requester(), PlayerId::new(7));
    }

    #[test]
    fn accept_contract_requester_is_acceptor() {
        let cmd = Command::AcceptContract {
            contract_id: ContractId::new(1),
            acceptor: PlayerId::new(99),
        };
        assert_eq!(cmd.requester(), PlayerId::new(99));
    }

    #[test]
    fn build_factory_requester_is_owner() {
        let cmd = Command::BuildFactory {
            owner: PlayerId::new(3),
            city: CityId::Istanbul,
            product: ProductKind::Kumas,
        };
        assert_eq!(cmd.requester(), PlayerId::new(3));
    }

    #[test]
    fn take_loan_requester_is_player() {
        let cmd = Command::TakeLoan {
            player: PlayerId::new(5),
            amount: Money::from_lira(1_000).unwrap(),
            duration_ticks: 20,
        };
        assert_eq!(cmd.requester(), PlayerId::new(5));
    }

    #[test]
    fn serde_roundtrip_submit_order() {
        let cmd = Command::SubmitOrder(sample_order());
        let back: Command = serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
        assert_eq!(cmd, back);
    }

    #[test]
    fn serde_roundtrip_propose_contract() {
        let cmd = Command::ProposeContract(sample_proposal());
        let back: Command = serde_json::from_str(&serde_json::to_string(&cmd).unwrap()).unwrap();
        assert_eq!(cmd, back);
    }
}
