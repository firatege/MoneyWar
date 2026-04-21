//! Hal Pazarı limit emirleri.
//!
//! Her tick süresince oyuncular emir yazar, emirler kilitli tutulur. Tick
//! sınırında motor batch auction ile tek takas fiyatı çıkarır (Faz 3).
//! Bu modül sadece emrin VERİ modelini tutar — eşleştirme engine'de.

use serde::{Deserialize, Serialize};

use crate::{CityId, DomainError, Money, OrderId, PlayerId, ProductKind, Tick};

/// Alım mı satım mı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderSide {
    /// Alım emri — "bu fiyata kadar ödemeye hazırım".
    Buy,
    /// Satım emri — "bu fiyattan aşağıya satmam".
    Sell,
}

impl OrderSide {
    #[must_use]
    pub const fn is_buy(self) -> bool {
        matches!(self, Self::Buy)
    }

    #[must_use]
    pub const fn is_sell(self) -> bool {
        matches!(self, Self::Sell)
    }
}

/// Hal Pazarı limit emri.
///
/// Tick süresince kilitli (bluff alanı §1), tick sınırında tüm emirler
/// motor tarafından okunup tek takas fiyatıyla eşleştirilir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketOrder {
    pub id: OrderId,
    pub player: PlayerId,
    pub city: CityId,
    pub product: ProductKind,
    pub side: OrderSide,
    pub quantity: u32,
    /// Limit fiyat (birim başına). Alım için maksimum, satım için minimum.
    pub unit_price: Money,
    pub submitted_tick: Tick,
}

impl MarketOrder {
    /// Yeni emir. Doğrular:
    /// - `quantity > 0`
    /// - `unit_price > 0` (sıfır/negatif fiyatlı emir anlamsız)
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: OrderId,
        player: PlayerId,
        city: CityId,
        product: ProductKind,
        side: OrderSide,
        quantity: u32,
        unit_price: Money,
        submitted_tick: Tick,
    ) -> Result<Self, DomainError> {
        if quantity == 0 {
            return Err(DomainError::Validation("order quantity must be > 0".into()));
        }
        if !unit_price.is_positive() {
            return Err(DomainError::Validation(format!(
                "order unit_price must be positive, got {unit_price}"
            )));
        }
        Ok(Self {
            id,
            player,
            city,
            product,
            side,
            quantity,
            unit_price,
            submitted_tick,
        })
    }

    /// Emrin toplam değeri (miktar × fiyat). Overflow güvenli.
    pub fn total_value(&self) -> Result<Money, DomainError> {
        self.unit_price.checked_mul_scalar(i64::from(self.quantity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_order(side: OrderSide) -> MarketOrder {
        MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
            side,
            100,
            Money::from_lira(10).unwrap(),
            Tick::new(5),
        )
        .unwrap()
    }

    #[test]
    fn side_helpers() {
        assert!(OrderSide::Buy.is_buy());
        assert!(!OrderSide::Buy.is_sell());
        assert!(OrderSide::Sell.is_sell());
        assert!(!OrderSide::Sell.is_buy());
    }

    #[test]
    fn valid_order_passes_validation() {
        let o = valid_order(OrderSide::Buy);
        assert_eq!(o.quantity, 100);
        assert_eq!(o.unit_price, Money::from_lira(10).unwrap());
    }

    #[test]
    fn zero_quantity_rejected() {
        let err = MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
            OrderSide::Buy,
            0,
            Money::from_lira(10).unwrap(),
            Tick::ZERO,
        )
        .expect_err("zero qty");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn zero_unit_price_rejected() {
        let err = MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
            OrderSide::Buy,
            10,
            Money::ZERO,
            Tick::ZERO,
        )
        .expect_err("zero price");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn negative_unit_price_rejected() {
        let err = MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
            OrderSide::Buy,
            10,
            Money::from_cents(-100),
            Tick::ZERO,
        )
        .expect_err("negative price");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn total_value_multiplies_qty_and_price() {
        let o = valid_order(OrderSide::Buy);
        assert_eq!(o.total_value().unwrap(), Money::from_lira(1_000).unwrap());
    }

    #[test]
    fn total_value_detects_overflow() {
        let o = MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
            OrderSide::Buy,
            u32::MAX,
            Money::from_cents(i64::MAX / 2),
            Tick::ZERO,
        )
        .unwrap();
        assert!(o.total_value().is_err());
    }

    #[test]
    fn serde_roundtrip() {
        let o = valid_order(OrderSide::Sell);
        let back: MarketOrder = serde_json::from_str(&serde_json::to_string(&o).unwrap()).unwrap();
        assert_eq!(o, back);
    }
}
