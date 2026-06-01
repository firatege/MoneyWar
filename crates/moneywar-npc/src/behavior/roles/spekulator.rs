//! Spekülatör rol davranışı — stok birikimci / fiyat manipülatörü.
//!
//! Eski sürüm spread market maker'dı (BID × 99%, ASK × 97-101%) — fiyat
//! girişi kalkınca anlamsızlaştı.
//!
//! Yeni rol: Vic3 tarzı spekülatör.
//! 1. Hammadde ucuzsa → toplu BUY → stok biriktir
//! 2. Stok birikince → o şehirde SELL azalır → BUY > SELL → Walras fiyatı artırır
//! 3. Fiyat yeterince arttıysa → SAT → kâr
//!
//! # `Weights` mantığı
//! - `cash +0.3`: biriktirmek için nakit lazım
//! - `arbitrage +0.4`: ucuz/pahalı şehir sinyali
//! - `stock -0.3`: stok çok artınca sat

use moneywar_domain::{CityId, GameState, OrderSide, Player};

use crate::behavior::candidates::ActionCandidate;

/// Ucuz hammadde biriktir, pahalıya sat.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    let bucket_count = (CityId::ALL.len() * moneywar_domain::ProductKind::RAW_MATERIALS.len())
        .max(1) as i64;
    let bucket_cash =
        moneywar_domain::Money::from_cents(player.cash.as_cents().saturating_div(bucket_count).max(0));

    for city in CityId::ALL {
        for &product in &moneywar_domain::ProductKind::RAW_MATERIALS {
            let Some(baseline) = state.effective_baseline(city, product) else {
                continue;
            };
            if baseline.as_cents() <= 0 {
                continue;
            }
            let market_price = state.derive_market_price(city, product, OrderSide::Buy);
            let stock = player.inventory.get(city, product);

            // Ucuzsa biriktir: piyasa fiyatı baseline'ın %85'inden düşükse BUY.
            let buy_threshold = baseline.as_cents() * 85 / 100;
            if market_price.as_cents() <= buy_threshold && bucket_cash.as_cents() > 0 {
                let qty = affordable_qty(bucket_cash, market_price, 30);
                if qty > 0 {
                    out.push(ActionCandidate::SubmitOrder {
                        side: OrderSide::Buy,
                        city,
                        product,
                        quantity: qty,
                        unit_price: market_price,
                        ttl_override: None,
                    });
                }
            }

            // Stok yüksek + fiyat yükseldiyse sat: baseline %120+ ise SELL.
            let sell_threshold = baseline.as_cents() * 120 / 100;
            if stock > 20 {
                let sell_price = state.derive_market_price(city, product, OrderSide::Sell);
                if sell_price.as_cents() >= sell_threshold {
                    let qty = (stock / 2).max(1).min(30);
                    out.push(ActionCandidate::SubmitOrder {
                        side: OrderSide::Sell,
                        city,
                        product,
                        quantity: qty,
                        unit_price: sell_price,
                        ttl_override: None,
                    });
                }
            }
        }
    }
    out
}

fn affordable_qty(cash: moneywar_domain::Money, unit_price: moneywar_domain::Money, want: u32) -> u32 {
    let unit_with_tax = unit_price
        .as_cents()
        .saturating_mul(100 + moneywar_domain::balance::TRANSACTION_TAX_PCT)
        / 100;
    if unit_with_tax <= 0 {
        return 0;
    }
    let max_qty = cash.as_cents() / unit_with_tax;
    u32::try_from(max_qty).unwrap_or(u32::MAX).min(want)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId, Money, CityId};

    fn fresh() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn spek(cash: i64) -> Player {
        Player::new(
            PlayerId::new(113),
            "spek",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Spekulator)
    }

    #[test]
    fn no_baseline_no_emit() {
        let s = fresh();
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        assert!(cands.is_empty(), "baseline yoksa emit yok");
    }

    #[test]
    fn no_stock_no_sell() {
        let mut s = fresh();
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                s.price_baseline.insert((city, product), Money::from_lira(10).unwrap());
            }
        }
        let p = spek(40_000);
        let sells = enumerate(&s, &p)
            .into_iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, .. }))
            .count();
        assert_eq!(sells, 0, "stok yoksa SELL yok");
    }
}
