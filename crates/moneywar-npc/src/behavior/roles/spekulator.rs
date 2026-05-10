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
/// ham çürüdü; 0 SELL match). v8.19'da 3 bucket'a indirilmişti.
///
/// v0.6.0 Faz 2 (cliff fix): Sanayici off-fab BUY kaldırıldı, Spek likidite
/// devraldı. 5 prime_raw bucket dar — Sanayici fab şehri specialty'siyle
/// çakışmadığında Spek müşteri bulamadı, stok birikti (Faz 2'de -19K zarar).
/// Yeni: **15 raw bucket** (5 şehir × 3 raw). Her raw'ı her şehirde işler →
/// Sanayici fab şehri ne olursa olsun Spek o şehirde alternatif tedarikçi.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    // 15 raw bucket (5 şehir × 3 raw) → cash bölünür.
    let bucket_count = (CityId::ALL.len() * moneywar_domain::ProductKind::RAW_MATERIALS.len())
        .max(1) as i64;
    let bucket_cash =
        Money::from_cents(player.cash.as_cents().saturating_div(bucket_count).max(0));

    for city in CityId::ALL {
        for &product in &moneywar_domain::ProductKind::RAW_MATERIALS {
            let Some(reference) = state.reference_price(city, product) else {
                continue;
            };
            if reference.as_cents() <= 0 {
                continue;
            }

        // BID — reference × 0.99 + jitter (dar spread, v8.15 mantığı).
        let bid_base = scale_pct(reference, 99);
        let bid_price = apply_jitter(
            bid_base,
            state.current_tick,
            city,
            product,
            OrderSide::Buy,
            player.id,
        );
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

        // ASK — stok-baskılı pricing. Az stoklu → reference × 1.01 (kar
        // marjı). Birikmiş stoklu → reference × 0.97 (Çiftçi'den ucuz
        // sat, hızlı erit). v0.6.0 Faz 2 sonrası: 15-bucket Spek mal alıp
        // satamazdı (-22K), stok-baskılı ASK ile satış akışı açılır.
        let stock = player.inventory.get(city, product);
        if stock > 0 {
            let ask_pct = if stock >= 100 { 97 } else if stock >= 50 { 99 } else { 101 };
            let ask_base = scale_pct(reference, ask_pct);
            let ask_price = apply_jitter(
                ask_base,
                state.current_tick,
                city,
                product,
                OrderSide::Sell,
                player.id,
            );
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
    fn no_baseline_no_emit() {
        // baseline boşsa reference_price None → Spek hiçbir bucket'a girmez.
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let p = spek(40_000);
        let cands = enumerate(&s, &p);
        assert!(cands.is_empty(), "baseline yoksa emit yok");
    }

    #[test]
    fn emits_bid_in_all_raw_buckets() {
        // v0.6.0: 15 raw bucket (5 şehir × 3 raw). Spek her şehirde her raw
        // için BID emit eder — likidite garantisi.
        let s = fresh_with_specialty();
        let p = spek(150_000);
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
        assert_eq!(bid_buckets.len(), 15, "5 şehir × 3 raw = 15 BID");
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
    fn stock_in_any_raw_bucket_yields_ask() {
        // v0.6.0: 15-bucket Spek her şehir × her raw'da çalışıyor.
        // Izm'de Pamuk stoğu varsa Spek orada ASK emit eder.
        let s = fresh_with_specialty();
        let mut p = spek(40_000);
        p.inventory
            .add(CityId::Izmir, ProductKind::Pamuk, 30)
            .unwrap();
        let cands = enumerate(&s, &p);
        let has_ask = cands.iter().any(|c| matches!(
            c,
            ActionCandidate::SubmitOrder {
                side: OrderSide::Sell,
                city: CityId::Izmir,
                product: ProductKind::Pamuk,
                ..
            }
        ));
        assert!(has_ask, "her raw bucket'ta stoklu ASK emit edilir");
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
