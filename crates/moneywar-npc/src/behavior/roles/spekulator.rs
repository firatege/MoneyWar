//! Spekülatör rol davranışı — market maker, **odaklı** spread.
//!
//! Önceki sürüm 18 bucket için BID + ASK basıyordu, kitabı kaynatıyordu.
//! Yeni sürüm sadece **arbitraj fırsatı** olan bucket'lara odaklanır:
//! şehirler arası fiyat farkı varsa o ürün için BID + ASK koy. Yoksa pas.
//!
//! Bu mekaniği gerçek hayat market maker davranışına yaklaştırır: spread
//! kazancı volatilite ile orantılı, sakin pazarda Spekülatör emit etmiyor.
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

/// Spread daha düşük ise Spekülatör için arbitraj fırsat yok — pas.
/// Şehirler arası fiyat farkı `(max - min) / min × 100` >= bu eşik ise emit.
const SPREAD_OPPORTUNITY_PCT: i64 = 10;

#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();

    // Hangi ürünler için arbitraj fırsatı var?
    let opportunity_products: Vec<ProductKind> = ProductKind::ALL
        .iter()
        .copied()
        .filter(|product| has_arbitrage_opportunity(state, *product))
        .collect();
    if opportunity_products.is_empty() {
        return out;
    }

    // Her fırsatlı ürün için 3 şehirde de BID/ASK koy (likidite zerk).
    // Bucket sayısı: opportunity_count × 3 şehir = max 18 (6 ürünün hepsi).
    let bucket_count = (opportunity_products.len() as i64) * 3;
    let bid_cash = Money::from_cents(
        player.cash.as_cents().saturating_div(bucket_count.max(1)).max(0),
    );

    for product in &opportunity_products {
        for city in CityId::ALL {
            let baseline = state
                .effective_baseline(city, *product)
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

            if bid_price.as_cents() > 0 {
                let qty = affordable_qty(bid_cash, bid_price, 15);
                if qty > 0 {
                    out.push(ActionCandidate::SubmitOrder {
                        side: OrderSide::Buy,
                        city,
                        product: *product,
                        quantity: qty,
                        unit_price: bid_price,
                    });
                }
            }

            let stock = player.inventory.get(city, *product);
            if stock > 0 && ask_price.as_cents() > 0 {
                let qty = (stock / 2).max(1).min(15);
                out.push(ActionCandidate::SubmitOrder {
                    side: OrderSide::Sell,
                    city,
                    product: *product,
                    quantity: qty,
                    unit_price: ask_price,
                });
            }
        }
    }
    out
}

/// Bu ürün için şehirler arası max-min fiyat farkı `SPREAD_OPPORTUNITY_PCT`
/// eşiğini aşıyor mu? Aşıyorsa Spekülatör için arbitraj fırsatı var.
fn has_arbitrage_opportunity(state: &GameState, product: ProductKind) -> bool {
    let mut prices: Vec<i64> = Vec::new();
    for city in CityId::ALL {
        if let Some(p) = state.effective_baseline(city, product) {
            prices.push(p.as_cents());
        }
    }
    if prices.len() < 2 {
        return false;
    }
    let min = *prices.iter().min().unwrap_or(&0);
    let max = *prices.iter().max().unwrap_or(&0);
    if min <= 0 {
        return false;
    }
    (max - min) * 100 / min >= SPREAD_OPPORTUNITY_PCT
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

    fn fresh_with_spread(product: ProductKind, prices: [i64; 3]) -> GameState {
        let mut s = fresh();
        for (city, lira) in CityId::ALL.iter().zip(prices.iter()) {
            s.price_baseline
                .insert((*city, product), Money::from_lira(*lira).unwrap());
        }
        s
    }

    #[test]
    fn no_arbitrage_no_emit() {
        // fresh_state'te price_baseline boş → her şehir aynı default → spread 0
        // → arbitrage yok → Spekülatör pas geçer.
        let s = fresh();
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        assert!(
            cands.is_empty(),
            "spread yoksa Spekülatör emit etmemeli"
        );
    }

    #[test]
    fn arbitrage_opportunity_yields_bids() {
        // Pamuk: İst 4₺, Ank 6₺, Izm 8₺ → spread %100 → fırsat.
        let s = fresh_with_spread(ProductKind::Pamuk, [4, 6, 8]);
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        let bids = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder {
                side: OrderSide::Buy, product: ProductKind::Pamuk, ..
            }))
            .count();
        assert_eq!(bids, 3, "fırsatlı ürün için 3 şehirde BID");
    }

    #[test]
    fn no_stock_no_asks() {
        let s = fresh_with_spread(ProductKind::Pamuk, [4, 6, 8]);
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        let asks = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, .. }))
            .count();
        assert_eq!(asks, 0);
    }

    #[test]
    fn stock_in_arbitrage_product_yields_ask() {
        let s = fresh_with_spread(ProductKind::Pamuk, [4, 6, 8]);
        let mut p = spek(40_000);
        p.inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 30)
            .unwrap();
        let cands = enumerate(&s, &p);
        let has_ask = cands.iter().any(|c| matches!(c,
            ActionCandidate::SubmitOrder {
                side: OrderSide::Sell,
                city: CityId::Istanbul,
                product: ProductKind::Pamuk,
                ..
            }
        ));
        assert!(has_ask, "fırsatlı ürün stoğu varsa ASK emit");
    }

    #[test]
    fn bid_below_ask() {
        let s = fresh_with_spread(ProductKind::Pamuk, [4, 6, 8]);
        let mut p = spek(40_000);
        p.inventory.add(CityId::Istanbul, ProductKind::Pamuk, 10).unwrap();
        let cands = enumerate(&s, &p);
        let bid = cands.iter().find_map(|c| match c {
            ActionCandidate::SubmitOrder {
                side: OrderSide::Buy,
                city: CityId::Istanbul,
                product: ProductKind::Pamuk,
                unit_price,
                ..
            } => Some(*unit_price),
            _ => None,
        });
        let ask = cands.iter().find_map(|c| match c {
            ActionCandidate::SubmitOrder {
                side: OrderSide::Sell,
                city: CityId::Istanbul,
                product: ProductKind::Pamuk,
                unit_price,
                ..
            } => Some(*unit_price),
            _ => None,
        });
        assert!(bid.unwrap().as_cents() < ask.unwrap().as_cents());
    }

    #[test]
    fn small_spread_below_threshold_no_emit() {
        // %5 spread < SPREAD_OPPORTUNITY_PCT eşiği → fırsat yok
        let s = fresh_with_spread(ProductKind::Pamuk, [10, 10, 10]);
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        assert!(cands.is_empty());
    }

    #[test]
    fn deterministic_no_rng() {
        let s = fresh_with_spread(ProductKind::Pamuk, [4, 6, 8]);
        let p = spek(40_000);
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }
}
