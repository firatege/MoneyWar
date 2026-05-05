//! Tüccar rol davranışı — şehirler arası arbitraj.
//!
//! Her ürün için ucuz ve pahalı şehri bul. Spread > %20 ise ucuz şehirde AL,
//! pahalı şehirde SAT (stok varsa). Synthetic ile aynı kural — şu an
//! `DispatchCaravan` çağrısı yok (basit pazar arbitrajı), ileride mal taşıma
//! eklenince kervan komutları da aday üreticisinden çıkacak.
//!
//! # `Weights` mantığı (`personality.rs`'te)
//!
//! - `arbitrage +0.6`: ana sürücü
//! - `cash +0.3`: cash varsa al
//! - `urgency +0.2`: sezon basıncı
//! - `competition -0.2`: rakip baskı
//! - `momentum +0.1`: trend yönü

use moneywar_domain::{
    CityId, GameState, Money, OrderSide, Player, ProductKind,
    balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;

/// Arbitraj eşiği — bu yüzdeden az spread varsa arbitraj kârsız.
/// Faz F tuning: 20 → 15. Demand_for matrisi mamul baseline farkını
/// %25-28 yaratıyor; ham specialty farkı %14-75 (çoğunlukla yeterli).
const ARBITRAGE_SPREAD_PCT: i64 = 15;

#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    let bucket_cash = bucket_buy_budget(player);

    for product in ProductKind::ALL {
        let Some((cheap_city, cheap_price)) = cheapest_city(state, product) else {
            continue;
        };
        let Some((rich_city, rich_price)) = richest_city(state, product) else {
            continue;
        };
        if cheap_city == rich_city || cheap_price.as_cents() <= 0 {
            continue;
        }
        let spread_pct =
            (rich_price.as_cents() - cheap_price.as_cents()) * 100 / cheap_price.as_cents();
        if spread_pct < ARBITRAGE_SPREAD_PCT {
            continue;
        }

        // AL ucuzda
        let buy_qty = affordable_qty(bucket_cash, cheap_price, 25);
        if buy_qty > 0 {
            out.push(ActionCandidate::SubmitOrder {
                side: OrderSide::Buy,
                city: cheap_city,
                product,
                quantity: buy_qty,
                unit_price: cheap_price,
            });
        }

        // SAT pahalıda (stoğu varsa)
        let stock = player.inventory.get(rich_city, product);
        if stock > 0 {
            let sell_qty = stock.min(25);
            out.push(ActionCandidate::SubmitOrder {
                side: OrderSide::Sell,
                city: rich_city,
                product,
                quantity: sell_qty,
                unit_price: rich_price,
            });
        }
    }
    out
}

fn cheapest_city(state: &GameState, product: ProductKind) -> Option<(CityId, Money)> {
    CityId::ALL
        .iter()
        .copied()
        .map(|city| (city, baseline_or_default(state, city, product)))
        .filter(|(_, p)| p.as_cents() > 0)
        .min_by_key(|(_, p)| p.as_cents())
}

fn richest_city(state: &GameState, product: ProductKind) -> Option<(CityId, Money)> {
    CityId::ALL
        .iter()
        .copied()
        .map(|city| (city, baseline_or_default(state, city, product)))
        .filter(|(_, p)| p.as_cents() > 0)
        .max_by_key(|(_, p)| p.as_cents())
}

fn baseline_or_default(state: &GameState, city: CityId, product: ProductKind) -> Money {
    state.effective_baseline(city, product).unwrap_or_else(|| {
        let lira = if product.is_finished() {
            moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA
        } else {
            moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA
        };
        Money::from_lira(lira).unwrap_or(Money::ZERO)
    })
}

fn bucket_buy_budget(player: &Player) -> Money {
    // 6 ürün × 1 al-sat çifti = 6 bucket.
    Money::from_cents((player.cash.as_cents() / 6).max(0))
}

fn affordable_qty(cash: Money, unit_price: Money, want: u32) -> u32 {
    let unit_with_tax = unit_price
        .as_cents()
        .saturating_mul(100 + TRANSACTION_TAX_PCT)
        / 100;
    if unit_with_tax <= 0 {
        return 0;
    }
    let max_qty_i64 = cash.as_cents() / unit_with_tax;
    let max_qty = u32::try_from(max_qty_i64).unwrap_or(u32::MAX);
    max_qty.min(want)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId};

    fn fresh() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn tuccar(cash: i64) -> Player {
        Player::new(
            PlayerId::new(100),
            "tuc",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Tuccar)
    }

    #[test]
    fn no_baseline_no_candidates() {
        // fresh_state'te baseline yok → her şehirde NPC default fiyat → spread = 0
        // → arbitraj eşiği aşılmıyor → boş döner.
        let s = fresh();
        let p = tuccar(15_000);
        let cands = enumerate(&s, &p);
        assert!(cands.is_empty(), "spread sıfırsa arbitraj yok");
    }

    #[test]
    fn synthetic_spread_yields_arbitrage_candidates() {
        // Sun'i baseline'lar: İst Pamuk = 4, Ank = 6, Izm = 8 → spread 100%
        let mut s = fresh();
        s.price_baseline.insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline.insert((CityId::Ankara, ProductKind::Pamuk), Money::from_lira(6).unwrap());
        s.price_baseline.insert((CityId::Izmir, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        let p = tuccar(15_000);
        let cands = enumerate(&s, &p);
        let buy_in_istanbul = cands.iter().any(|c| matches!(c,
            ActionCandidate::SubmitOrder { side: OrderSide::Buy, city: CityId::Istanbul, product: ProductKind::Pamuk, .. }
        ));
        assert!(buy_in_istanbul, "ucuz şehirde AL emit etmeli");
    }

    #[test]
    fn stock_in_rich_city_yields_sell() {
        let mut s = fresh();
        s.price_baseline.insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline.insert((CityId::Izmir, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        let mut p = tuccar(15_000);
        p.inventory.add(CityId::Izmir, ProductKind::Pamuk, 30).unwrap();
        let cands = enumerate(&s, &p);
        let sell_in_izmir = cands.iter().any(|c| matches!(c,
            ActionCandidate::SubmitOrder { side: OrderSide::Sell, city: CityId::Izmir, product: ProductKind::Pamuk, .. }
        ));
        assert!(sell_in_izmir, "pahalı şehirde stok varsa SAT");
    }

    #[test]
    fn deterministic_no_rng() {
        let mut s = fresh();
        s.price_baseline.insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline.insert((CityId::Ankara, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        let p = tuccar(15_000);
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }
}
