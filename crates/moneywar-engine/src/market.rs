//! Hal Pazarı — tick sonu batch auction.
//!
//! `advance_tick` her tick bitişinde `clear_markets` çağırır: her
//! `(city, product)` bucket'ı için uniform price temizleme yapar.
//!
//! # Algoritma (uniform price batch auction)
//!
//! 1. **Sort**
//!    - Buy emirleri: `price DESC`, tie-break `(player_id, order_id)` ASC.
//!    - Sell emirleri: `price ASC`, tie-break `(player_id, order_id)` ASC.
//! 2. **Greedy cumulative match**
//!    - Walk buy listesi başından, sell listesi başından.
//!    - `buy.price < sell.price` olursa dur (spread, kimse eşleşmez).
//!    - Eşleşen qty = `min(buy.remaining, sell.remaining)`, her ikisini azalt.
//!    - Sonuç: `fills: Vec<Fill>`.
//! 3. **Clearing price**
//!    - `clearing_price = (son_match_buy + son_match_sell) / 2` (midpoint).
//!    - Tüm fill'ler bu uniform fiyatta settle olur.
//! 4. **Post-clearing**
//!    - Event'ler emit edilir: her fill → `OrderMatched`, özet → `MarketCleared`.
//!    - Bucket boşaltılır (eşleşmeyenler çöpe — tasarım §2).
//!
//! # 3C kapsamında gelecek
//!
//! - Cash/inventory settlement (bu dosyada değil, fills'i okuyan settle fn).
//! - Saturation eşiği: eşik üstü qty `clearing_price / 2`'de settle.
//! - `price_history` güncellemesi.

use moneywar_domain::{CityId, GameState, MarketOrder, Money, OrderSide, ProductKind, Tick};

use crate::report::{LogEntry, TickReport};

/// Tek bir eşleşme. Şimdilik `market.rs` içinde private — Faz 3C settle
/// fonksiyonu buraya taşınınca export edilebilir.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fill {
    buy_order_id: moneywar_domain::OrderId,
    sell_order_id: moneywar_domain::OrderId,
    buyer: moneywar_domain::PlayerId,
    seller: moneywar_domain::PlayerId,
    quantity: u32,
}

/// Tüm `(city, product)` bucket'larını sırayla temizle.
///
/// Bucket'ların işleme sırası `BTreeMap` iterasyon sırası — yani
/// `(CityId, ProductKind)` doğal sıralaması. Determinism için kritik.
pub(crate) fn clear_markets(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    // Tüm anahtarları topla — iterasyon sırasında book'u mutate edebilmek için.
    let keys: Vec<(CityId, ProductKind)> = state.order_book.keys().copied().collect();
    for key in keys {
        clear_bucket(state, report, tick, key);
    }
}

fn clear_bucket(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    key: (CityId, ProductKind),
) {
    let (city, product) = key;
    // Bucket'ı çıkar — temizleme sonrası zaten boşaltılıyor.
    let Some(orders) = state.order_book.remove(&key) else {
        return;
    };

    let (mut buys, mut sells): (Vec<_>, Vec<_>) = orders
        .into_iter()
        .partition(|o| matches!(o.side, OrderSide::Buy));

    // Buy: price DESC → eğer aynı fiyat varsa (player_id, order_id) ASC.
    buys.sort_by(|a, b| {
        b.unit_price
            .cmp(&a.unit_price)
            .then_with(|| a.player.cmp(&b.player))
            .then_with(|| a.id.cmp(&b.id))
    });
    // Sell: price ASC, tie-break (player_id, order_id) ASC.
    sells.sort_by(|a, b| {
        a.unit_price
            .cmp(&b.unit_price)
            .then_with(|| a.player.cmp(&b.player))
            .then_with(|| a.id.cmp(&b.id))
    });

    let submitted_buy_qty: u32 = buys.iter().map(|o| o.quantity).sum();
    let submitted_sell_qty: u32 = sells.iter().map(|o| o.quantity).sum();

    let (fills, clearing_price, matched_qty) = match_orders(&buys, &sells);

    for fill in &fills {
        report.push(LogEntry::order_matched(
            tick,
            city,
            product,
            fill.buy_order_id,
            fill.sell_order_id,
            fill.buyer,
            fill.seller,
            fill.quantity,
            clearing_price.expect("matches exist → price exists"),
        ));
    }

    report.push(LogEntry::market_cleared(
        tick,
        city,
        product,
        clearing_price,
        matched_qty,
        submitted_buy_qty,
        submitted_sell_qty,
    ));
}

/// Sortlanmış buy/sell listeleri üstünde greedy cumulative matching.
///
/// Dönüş: `(fills, clearing_price, matched_qty)`. `clearing_price` `None`
/// iff `matched_qty == 0` (hiç eşleşme, spread veya bir taraf boş).
fn match_orders(buys: &[MarketOrder], sells: &[MarketOrder]) -> (Vec<Fill>, Option<Money>, u32) {
    let mut fills: Vec<Fill> = Vec::new();
    let mut matched_qty: u32 = 0;
    let mut last_buy_price: Option<Money> = None;
    let mut last_sell_price: Option<Money> = None;

    let mut i = 0usize;
    let mut j = 0usize;
    let mut buy_rem: u32 = buys.first().map_or(0, |o| o.quantity);
    let mut sell_rem: u32 = sells.first().map_or(0, |o| o.quantity);

    while i < buys.len() && j < sells.len() {
        let buy = &buys[i];
        let sell = &sells[j];

        // Spread: en iyi alıcı, en iyi satıcının altında → kimse eşleşemez.
        if buy.unit_price < sell.unit_price {
            break;
        }

        let qty = buy_rem.min(sell_rem);
        fills.push(Fill {
            buy_order_id: buy.id,
            sell_order_id: sell.id,
            buyer: buy.player,
            seller: sell.player,
            quantity: qty,
        });
        matched_qty = matched_qty.saturating_add(qty);
        last_buy_price = Some(buy.unit_price);
        last_sell_price = Some(sell.unit_price);

        buy_rem -= qty;
        sell_rem -= qty;

        if buy_rem == 0 {
            i += 1;
            if let Some(next) = buys.get(i) {
                buy_rem = next.quantity;
            }
        }
        if sell_rem == 0 {
            j += 1;
            if let Some(next) = sells.get(j) {
                sell_rem = next.quantity;
            }
        }
    }

    let clearing_price = match (last_buy_price, last_sell_price) {
        (Some(b), Some(s)) => {
            // Midpoint: (b + s) / 2 — uniform clearing.
            let sum_cents = b.as_cents().saturating_add(s.as_cents());
            Some(Money::from_cents(sum_cents / 2))
        }
        _ => None,
    };

    (fills, clearing_price, matched_qty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CityId, GameState, MarketOrder, Money, OrderId, OrderSide, PlayerId, ProductKind,
        RoomConfig, RoomId, Tick,
    };

    fn state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn order(id: u64, player: u64, side: OrderSide, qty: u32, price_lira: i64) -> MarketOrder {
        MarketOrder::new(
            OrderId::new(id),
            PlayerId::new(player),
            CityId::Istanbul,
            ProductKind::Pamuk,
            side,
            qty,
            Money::from_lira(price_lira).unwrap(),
            Tick::new(1),
        )
        .unwrap()
    }

    fn populate(state: &mut GameState, orders: Vec<MarketOrder>) {
        for o in orders {
            state
                .order_book
                .entry((o.city, o.product))
                .or_default()
                .push(o);
        }
    }

    #[test]
    fn empty_book_clears_without_events() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));
        assert!(r.entries.is_empty());
        assert!(s.order_book.is_empty());
    }

    #[test]
    fn spread_only_yields_no_matches_but_emits_cleared_event() {
        let mut s = state();
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 5),  // alıcı 5₺
                order(2, 2, OrderSide::Sell, 10, 8), // satıcı 8₺ → spread
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        // Tek entry: MarketCleared (clearing_price = None).
        assert_eq!(r.entries.len(), 1);
        match &r.entries[0].event {
            crate::report::LogEvent::MarketCleared {
                clearing_price,
                matched_qty,
                submitted_buy_qty,
                submitted_sell_qty,
                ..
            } => {
                assert_eq!(*clearing_price, None);
                assert_eq!(*matched_qty, 0);
                assert_eq!(*submitted_buy_qty, 10);
                assert_eq!(*submitted_sell_qty, 10);
            }
            other => panic!("expected MarketCleared, got {other:?}"),
        }
        // Book boşaltılmalı (eşleşmeyenler çöpe).
        assert!(s.order_book.is_empty());
    }

    #[test]
    fn single_match_uses_midpoint_price() {
        let mut s = state();
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 10), // alıcı 10₺
                order(2, 2, OrderSide::Sell, 10, 8), // satıcı 8₺
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        // OrderMatched + MarketCleared = 2 entry.
        assert_eq!(r.entries.len(), 2);
        match &r.entries[0].event {
            crate::report::LogEvent::OrderMatched {
                quantity, price, ..
            } => {
                assert_eq!(*quantity, 10);
                // Midpoint (10 + 8) / 2 = 9₺
                assert_eq!(*price, Money::from_lira(9).unwrap());
            }
            other => panic!("expected OrderMatched, got {other:?}"),
        }
        match &r.entries[1].event {
            crate::report::LogEvent::MarketCleared {
                clearing_price,
                matched_qty,
                ..
            } => {
                assert_eq!(*clearing_price, Some(Money::from_lira(9).unwrap()));
                assert_eq!(*matched_qty, 10);
            }
            other => panic!("expected MarketCleared, got {other:?}"),
        }
    }

    #[test]
    fn partial_fill_matches_only_smaller_side() {
        let mut s = state();
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 15, 10),
                order(2, 2, OrderSide::Sell, 10, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        // 10 unit match (sell limit), 5 buy unmatched → çöpe.
        let matched: u32 = r
            .entries
            .iter()
            .filter_map(|e| match &e.event {
                crate::report::LogEvent::OrderMatched { quantity, .. } => Some(*quantity),
                _ => None,
            })
            .sum();
        assert_eq!(matched, 10);
        assert!(s.order_book.is_empty());
    }

    #[test]
    fn multi_level_matching_walks_book_deterministically() {
        // buy:  b1(p1) 3@12,  b2(p2) 7@11
        // sell: s3(p3) 5@8,   s4(p4) 5@10
        //
        // Sort: buys DESC → [b1@12, b2@11]; sells ASC → [s3@8, s4@10].
        // Match adımları:
        //  (b1,s3) 3 → b1 biter, s3 kalan 2
        //  (b2,s3) 2 → s3 biter, b2 kalan 5
        //  (b2,s4) 5 → her ikisi biter
        // Toplam matched = 10. Son match: buy=11, sell=10 → midpoint 10.5 = 1050 cents.
        let mut s = state();
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 3, 12),
                order(2, 2, OrderSide::Buy, 7, 11),
                order(3, 3, OrderSide::Sell, 5, 8),
                order(4, 4, OrderSide::Sell, 5, 10),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        let matches: Vec<_> = r
            .entries
            .iter()
            .filter_map(|e| match &e.event {
                crate::report::LogEvent::OrderMatched {
                    buy_order_id,
                    sell_order_id,
                    quantity,
                    ..
                } => Some((buy_order_id.value(), sell_order_id.value(), *quantity)),
                _ => None,
            })
            .collect();
        assert_eq!(matches, vec![(1, 3, 3), (2, 3, 2), (2, 4, 5)]);

        match r.entries.last().map(|e| &e.event) {
            Some(crate::report::LogEvent::MarketCleared {
                clearing_price,
                matched_qty,
                ..
            }) => {
                assert_eq!(*matched_qty, 10);
                assert_eq!(*clearing_price, Some(Money::from_cents(1050)));
            }
            other => panic!("expected MarketCleared, got {other:?}"),
        }
    }

    #[test]
    fn same_price_tie_breaks_by_player_then_order_id() {
        // İki buy aynı fiyatta: p2/o2 önce sıralanmalı (küçük player_id).
        let mut s = state();
        populate(
            &mut s,
            vec![
                order(1, 9, OrderSide::Buy, 5, 10),
                order(2, 2, OrderSide::Buy, 5, 10),
                order(3, 1, OrderSide::Sell, 10, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        // İlk eşleşen buy player 2 olmalı (tie-break).
        match &r.entries[0].event {
            crate::report::LogEvent::OrderMatched {
                buy_order_id,
                buyer,
                ..
            } => {
                assert_eq!(*buy_order_id, OrderId::new(2));
                assert_eq!(*buyer, PlayerId::new(2));
            }
            other => panic!("expected OrderMatched, got {other:?}"),
        }
    }

    #[test]
    fn book_is_emptied_after_clearing() {
        let mut s = state();
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 10),
                order(2, 2, OrderSide::Sell, 5, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));
        assert!(s.order_book.is_empty());
    }

    #[test]
    fn only_one_side_present_clears_with_no_matches() {
        // Sadece buy emirleri, sell yok → hiç match, MarketCleared yalnız.
        let mut s = state();
        populate(&mut s, vec![order(1, 1, OrderSide::Buy, 10, 10)]);
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        assert_eq!(r.entries.len(), 1);
        match &r.entries[0].event {
            crate::report::LogEvent::MarketCleared {
                matched_qty,
                submitted_buy_qty,
                submitted_sell_qty,
                clearing_price,
                ..
            } => {
                assert_eq!(*matched_qty, 0);
                assert_eq!(*submitted_buy_qty, 10);
                assert_eq!(*submitted_sell_qty, 0);
                assert_eq!(*clearing_price, None);
            }
            other => panic!("expected MarketCleared, got {other:?}"),
        }
        assert!(s.order_book.is_empty());
    }

    #[test]
    fn multiple_buckets_clear_in_btreemap_order() {
        let mut s = state();
        // İstanbul/Pamuk ve Ankara/Bugday — BTreeMap sırası: Istanbul < Ankara? Hayır.
        // CityId enum derivation ile Istanbul=0, Ankara=1, Izmir=2 sıralaması.
        let istanbul_pamuk = order(1, 1, OrderSide::Buy, 5, 10);
        let mut ankara = MarketOrder::new(
            OrderId::new(2),
            PlayerId::new(1),
            CityId::Ankara,
            ProductKind::Bugday,
            OrderSide::Sell,
            5,
            Money::from_lira(7).unwrap(),
            Tick::new(1),
        )
        .unwrap();
        // Ankara için karşı taraf yok → kendisi MarketCleared olur.
        populate(&mut s, vec![istanbul_pamuk]);
        s.order_book
            .entry((ankara.city, ankara.product))
            .or_default()
            .push({
                ankara.id = OrderId::new(2);
                ankara.clone()
            });

        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        // Her bucket için tek MarketCleared → 2 entry.
        assert_eq!(r.entries.len(), 2);
        assert!(s.order_book.is_empty());
    }
}
