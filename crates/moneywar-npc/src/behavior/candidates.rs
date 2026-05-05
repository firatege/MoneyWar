//! Aksiyon adayı enum'u — rol-spesifik `enumerate` fonksiyonları bunu döner.
//!
//! Orchestrator (Faz B+'da `decide_behavior`) adayları skor sıralayıp top-K
//! seçer, sonra `Command`'a dönüştürür. Bu ayrım sayesinde:
//! - Skor hesabı domain'den bağımsız kalır (sadece sinyal × ağırlık).
//! - Aynı aday farklı difficulty'de farklı işlenebilir (skip/throttle).
//! - Test yazılırken aday listesi assertion edilebilir.

use moneywar_domain::{
    CargoSpec, CaravanId, CityId, ContractProposal, Money, OrderSide, ProductKind,
};

/// Bir NPC'nin yapabileceği bir aksiyon önerisi. `Command`'a henüz çevrilmedi.
///
/// Variant'lar `Command` ile 1-1 haritalanır ama orchestrator'a fiyat/qty
/// hesaplama esnekliği bırakır (örn. aggressiveness multiplier sonradan
/// uygulanır).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionCandidate {
    /// Pazar emri (al/sat).
    SubmitOrder {
        side: OrderSide,
        city: CityId,
        product: ProductKind,
        quantity: u32,
        unit_price: Money,
    },
    /// Fabrika kur (Sanayici).
    BuildFactory { city: CityId, product: ProductKind },
    /// Kervan satın al.
    BuyCaravan { starting_city: CityId },
    /// Kervan dispatch (Tüccar arbitraj).
    DispatchCaravan {
        caravan_id: CaravanId,
        from: CityId,
        to: CityId,
        cargo: CargoSpec,
    },
    /// Kontrat öner (uzun vadeli anlaşma).
    ProposeContract(ContractProposal),
}

impl ActionCandidate {
    /// Adayın aday-bağlamı (city, product). Skor hesabında `compute_inputs`
    /// bu çift için sinyalleri çeker. Bağlamsız adaylar (fab kuruluşu vs.)
    /// için `None` — orchestrator özel ele alır.
    #[must_use]
    pub const fn context(&self) -> Option<(CityId, ProductKind)> {
        match self {
            Self::SubmitOrder { city, product, .. } => Some((*city, *product)),
            Self::BuildFactory { city, product } => Some((*city, *product)),
            Self::BuyCaravan { .. }
            | Self::DispatchCaravan { .. }
            | Self::ProposeContract(_) => None,
        }
    }
}
