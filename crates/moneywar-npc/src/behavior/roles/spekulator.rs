//! Spekülatör rol davranışı — market maker, sabit spread.
//!
//! Her `(şehir × ürün)` bucket için iki taraflı emir koyar:
//! - **BID** `base × 0.95` (alım talebi, %5 markdown)
//! - **ASK** `base × 1.05` (satım teklifi, %5 markup, stokta varsa)
//!
//! Toplam 18 BID + var-olan-stok kadar ASK = ~25-36 aday/tick. Likidite ve
//! spread daraltma rolü; gerçek anlamı borsa rewrite (pay-as-bid + slippage)
//! sonrası ortaya çıkacak. Şu an dekoratif ama synthetic baseline ile uyumlu
//! olması için göç ettirildi.
//!
//! # `Weights` mantığı (`personality.rs`'te)
//!
//! - `cash +0.3`: cash varsa BID koy
//! - `arbitrage +0.4`: şehirler arası fark fırsat
//! - `event +0.3`: aktif şok varsa pozisyon al
//! - `momentum +0.2`: trend yönüne pozisyon
//! - `competition -0.1`: rakip baskıda hafif geri çekil

use moneywar_domain::{
    CityId, GameState, Money, OrderSide, Player, ProductKind,
    balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;

#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    let bid_bucket_cash = bid_bucket_budget(player);

    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let baseline = state
                .effective_baseline(city, product)
                .unwrap_or_else(|| {
                    let lira = if product.is_finished() {
                        moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA
                    } else {
                        moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA
                    };
                    Money::from_lira(lira).unwrap_or(Money::ZERO)
                });
            let bid_price = scale_pct(baseline, 95);
            let ask_price = scale_pct(baseline, 105);

            // BID — cash bucket çerçevesinde tax-aware qty
            if bid_price.as_cents() > 0 {
                let qty = affordable_qty(bid_bucket_cash, bid_price, 15);
                if qty > 0 {
                    out.push(ActionCandidate::SubmitOrder {
                        side: OrderSide::Buy,
                        city,
                        product,
                        quantity: qty,
                        unit_price: bid_price,
                    });
                }
            }

            // ASK — stokta varsa, miktarın yarısı (max 15)
            let stock = player.inventory.get(city, product);
            if stock > 0 && ask_price.as_cents() > 0 {
                let qty = (stock / 2).max(1).min(15);
                out.push(ActionCandidate::SubmitOrder {
                    side: OrderSide::Sell,
                    city,
                    product,
                    quantity: qty,
                    unit_price: ask_price,
                });
            }
        }
    }
    out
}

fn bid_bucket_budget(player: &Player) -> Money {
    // 18 bucket (3 şehir × 6 ürün). Cash'in tamamı BID için (Spekülatör pasif).
    Money::from_cents((player.cash.as_cents() / 18).max(0))
}

fn scale_pct(price: Money, pct: i64) -> Money {
    Money::from_cents(price.as_cents().saturating_mul(pct) / 100)
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
    fn rich_spek_emits_eighteen_bids() {
        let s = fresh();
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        let bids = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Buy, .. }))
            .count();
        // 3 şehir × 6 ürün = 18 BID
        assert_eq!(bids, 18);
    }

    #[test]
    fn no_stock_no_asks() {
        let s = fresh();
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        let asks = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, .. }))
            .count();
        assert_eq!(asks, 0);
    }

    #[test]
    fn stock_yields_ask() {
        let s = fresh();
        let mut p = spek(40_000);
        p.inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 30)
            .unwrap();
        let cands = enumerate(&s, &p);
        let asks: Vec<_> = cands
            .iter()
            .filter_map(|c| match c {
                ActionCandidate::SubmitOrder {
                    side: OrderSide::Sell,
                    city,
                    product,
                    ..
                } => Some((*city, *product)),
                _ => None,
            })
            .collect();
        assert!(asks.contains(&(CityId::Istanbul, ProductKind::Pamuk)));
    }

    #[test]
    fn bid_below_ask() {
        let s = fresh();
        let mut p = spek(40_000);
        p.inventory.add(CityId::Istanbul, ProductKind::Kumas, 10).unwrap();
        let cands = enumerate(&s, &p);
        let bid_kumas = cands.iter().find_map(|c| match c {
            ActionCandidate::SubmitOrder {
                side: OrderSide::Buy,
                city: CityId::Istanbul,
                product: ProductKind::Kumas,
                unit_price,
                ..
            } => Some(*unit_price),
            _ => None,
        });
        let ask_kumas = cands.iter().find_map(|c| match c {
            ActionCandidate::SubmitOrder {
                side: OrderSide::Sell,
                city: CityId::Istanbul,
                product: ProductKind::Kumas,
                unit_price,
                ..
            } => Some(*unit_price),
            _ => None,
        });
        assert!(bid_kumas.unwrap().as_cents() < ask_kumas.unwrap().as_cents());
    }

    #[test]
    fn deterministic_no_rng() {
        let s = fresh();
        let p = spek(40_000);
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }
}
