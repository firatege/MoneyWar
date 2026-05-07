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

use moneywar_domain::{CityId, GameState, Money, OrderSide, Player, balance::TRANSACTION_TAX_PCT};

use crate::behavior::candidates::ActionCandidate;
use crate::behavior::pricing::apply_jitter;

// SPREAD_OPPORTUNITY_PCT v8.14'te kaldırıldı — Spek artık arbitraj filtresine
// bağlı değil, lokal market maker. Eski cheapest/richest helpers da silindi.

/// v8.19: Spek **odaklı raw spekülatör** — sadece **prime_raw** bucket'larında
/// BID + stok varsa ASK. Eski 18-bucket lokal market maker -271K zarar
/// kasası (Esnafsız ham BUY iştahı düşünce Spek alıp satamadı, depoda 10K+
/// ham çürüdü; 0 SELL match). Yeni: 3 bucket (her şehrin prime ham'ı)
/// → cash konsantre, daha rekabetçi BID; ASK aynı bucket'ta SELL pressure
/// ekler. Mamul tarafı zaten temiz, Spek karışmasın.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    // 3 prime_raw bucket → cash bölünür. Eski 18 → 3, bucket başı 6× cash.
    let bucket_cash = Money::from_cents(player.cash.as_cents().saturating_div(3).max(0));

    for city in CityId::ALL {
        // Sadece prime ham — her şehrin uzmanlık ürünü.
        let Some(&product) = state.city_specialty.get(&city) else {
            continue;
        };
        if !product.is_raw() {
            continue;
        }
        let Some(reference) = state.reference_price(city, product) else {
            continue;
        };
        if reference.as_cents() <= 0 {
            continue;
        }

        // BID — reference × 0.99 + jitter (dar spread, v8.15 mantığı).
        let bid_base = scale_pct(reference, 99);
        let bid_price = apply_jitter(bid_base, state.current_tick, city, product, OrderSide::Buy);
        if bid_price.as_cents() > 0 {
            let qty = affordable_qty(bucket_cash, bid_price, 25);
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

        // ASK — stok varsa, reference × 1.01 + jitter. Spek'in birikmiş
        // stoğunu eritmek için %1 markup yeterli (v8.18'e göre).
        let stock = player.inventory.get(city, product);
        if stock > 0 {
            let ask_base = scale_pct(reference, 101);
            let ask_price =
                apply_jitter(ask_base, state.current_tick, city, product, OrderSide::Sell);
            if ask_price.as_cents() > 0 {
                let qty = (stock / 2).max(1).min(25);
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

    /// Standart specialty atama: Ist=Pamuk, Ank=Bugday, Izm=Zeytin.
    fn fresh_with_specialty() -> GameState {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        s.city_specialty
            .insert(CityId::Istanbul, ProductKind::Pamuk);
        s.city_specialty.insert(CityId::Ankara, ProductKind::Bugday);
        s.city_specialty.insert(CityId::Izmir, ProductKind::Zeytin);
        // baseline her bucket için (reference_price fallback için)
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                s.price_baseline
                    .insert((city, product), Money::from_lira(10).unwrap());
            }
        }
        s
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
    fn no_specialty_no_emit() {
        // city_specialty boşsa hiçbir bucket prime değil → Spek pas geçer.
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        assert!(cands.is_empty(), "specialty yoksa emit yok");
    }

    #[test]
    fn emits_bid_only_in_prime_raw_buckets() {
        // 3 prime_raw bucket: Ist/Pamuk, Ank/Bugday, Izm/Zeytin. Diğer 15
        // bucket pas.
        let s = fresh_with_specialty();
        let p = spek(60_000);
        let cands = enumerate(&s, &p);
        let bid_buckets: std::collections::BTreeSet<_> = cands
            .iter()
            .filter_map(|c| match c {
                ActionCandidate::SubmitOrder {
                    side: OrderSide::Buy,
                    city,
                    product,
                    ..
                } => Some((*city, *product)),
                _ => None,
            })
            .collect();
        assert_eq!(bid_buckets.len(), 3, "tam 3 prime_raw BID");
        assert!(bid_buckets.contains(&(CityId::Istanbul, ProductKind::Pamuk)));
        assert!(bid_buckets.contains(&(CityId::Ankara, ProductKind::Bugday)));
        assert!(bid_buckets.contains(&(CityId::Izmir, ProductKind::Zeytin)));
    }

    #[test]
    fn no_stock_no_asks() {
        let s = fresh_with_specialty();
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        let asks = cands
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    ActionCandidate::SubmitOrder {
                        side: OrderSide::Sell,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(asks, 0);
    }

    #[test]
    fn stock_in_prime_city_yields_ask() {
        // Spek Izm'de Zeytin stoklu → Izm prime_raw → ASK emit.
        let s = fresh_with_specialty();
        let mut p = spek(40_000);
        p.inventory
            .add(CityId::Izmir, ProductKind::Zeytin, 30)
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
        assert_eq!(asks, vec![(CityId::Izmir, ProductKind::Zeytin)]);
    }

    #[test]
    fn stock_in_non_prime_bucket_skipped() {
        // Spek Izm'de Pamuk stoklu (Izm prime=Zeytin değil Pamuk değil) →
        // Pamuk prime sadece Istanbul → Izm/Pamuk prime değil → ASK yok.
        let s = fresh_with_specialty();
        let mut p = spek(40_000);
        p.inventory
            .add(CityId::Izmir, ProductKind::Pamuk, 30)
            .unwrap();
        let cands = enumerate(&s, &p);
        let asks = cands
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    ActionCandidate::SubmitOrder {
                        side: OrderSide::Sell,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(asks, 0, "Izm/Pamuk prime değil, ASK olmamalı");
    }

    #[test]
    fn bid_below_ask() {
        let s = fresh_with_specialty();
        let mut p = spek(40_000);
        p.inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 20)
            .unwrap();
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
    fn deterministic_no_rng() {
        let s = fresh_with_specialty();
        let p = spek(40_000);
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }
}
