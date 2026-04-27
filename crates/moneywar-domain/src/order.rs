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
///
/// **TTL modeli**: Her clear pass sonrası eşleşmeyen kalıntı kitapta kalır
/// ve `remaining_ticks` -1'e iner. 0'a düşünce `OrderExpired` ile düşer.
/// `ttl_ticks` emrin ilk verildiği zamanki taahhüt (erken çekme cezası için
/// referans). `new()` TTL=1 default ile eski davranışı korur (tek tick).
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
    /// Emrin ilk verildiği zamanki taahhüt edilen TTL. Cezanın pro-rata
    /// payda'sı — submit sonrası değişmez.
    pub ttl_ticks: u32,
    /// Kalan yaşam süresi. Her clear'de -1, 0'da `OrderExpired`.
    pub remaining_ticks: u32,
}

impl MarketOrder {
    /// TTL=1 varsayılanıyla yeni emir (tek clear, kalan düşer). Test + eski
    /// kod yolu için geriye uyumluluk köprüsü. Prod yollar `new_with_ttl` kullanmalı.
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
        Self::new_with_ttl(
            id,
            player,
            city,
            product,
            side,
            quantity,
            unit_price,
            submitted_tick,
            1,
        )
    }

    /// Yeni emir + TTL. Doğrular:
    /// - `quantity > 0`
    /// - `unit_price > 0`
    /// - `ttl_ticks >= 1`
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_ttl(
        id: OrderId,
        player: PlayerId,
        city: CityId,
        product: ProductKind,
        side: OrderSide,
        quantity: u32,
        unit_price: Money,
        submitted_tick: Tick,
        ttl_ticks: u32,
    ) -> Result<Self, DomainError> {
        if quantity == 0 {
            return Err(DomainError::Validation("order quantity must be > 0".into()));
        }
        if !unit_price.is_positive() {
            return Err(DomainError::Validation(format!(
                "order unit_price must be positive, got {unit_price}"
            )));
        }
        if ttl_ticks == 0 {
            return Err(DomainError::Validation(
                "order ttl_ticks must be ≥ 1".into(),
            ));
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
            ttl_ticks,
            remaining_ticks: ttl_ticks,
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

    #[test]
    fn new_defaults_to_single_tick_ttl() {
        let o = valid_order(OrderSide::Buy);
        assert_eq!(o.ttl_ticks, 1);
        assert_eq!(o.remaining_ticks, 1);
    }

    #[test]
    fn new_with_ttl_initializes_countdown() {
        let o = MarketOrder::new_with_ttl(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
            OrderSide::Buy,
            10,
            Money::from_lira(5).unwrap(),
            Tick::new(1),
            5,
        )
        .unwrap();
        assert_eq!(o.ttl_ticks, 5);
        assert_eq!(o.remaining_ticks, 5);
    }

    #[test]
    fn zero_ttl_rejected() {
        let err = MarketOrder::new_with_ttl(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
            OrderSide::Buy,
            10,
            Money::from_lira(5).unwrap(),
            Tick::ZERO,
            0,
        )
        .expect_err("ttl=0");
        assert!(matches!(err, DomainError::Validation(_)));
    }
}
