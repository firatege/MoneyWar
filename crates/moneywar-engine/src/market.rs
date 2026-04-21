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
//! 4. **Post-clearing (3C)**
//!    - `settle_fills` her fill'i saturation eşiğine göre full/half tier'a böler.
//!      `threshold = 40 + (player_count - 2) × 10` (§10). Eşik üstü segment
//!      `clearing_price / 2`'de settle — "çok döküyorsan marjinal satış yarı
//!      fiyata" anti-snowball.
//!    - `settle_segment` buyer'dan cash düşer + stok ekler, seller'a cash verir +
//!      stok düşer. Pre-flight validation yetmezse `FillRejected` event; state
//!      dokunulmaz, para korunumu ihlal edilmez.
//!    - Event'ler: her settled segment → `OrderMatched`, başarısız → `FillRejected`,
//!      bucket özeti → `MarketCleared` (threshold + `saturation_qty` alanlarıyla).
//!    - `price_history[(city, product)]` tarihçesine `(tick, clearing_price)` eklenir.
//!    - Bucket boşaltılır (eşleşmeyenler çöpe — tasarım §2).

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
    let keys: Vec<(CityId, ProductKind)> = state.order_book.keys().copied().collect();
    // Saturation eşiği oda katılımcı sayısına bağlı (§10). Tüm bucket'lar için sabit.
    let threshold = state.config.saturation_threshold(state.participant_count());
    for key in keys {
        clear_bucket(state, report, tick, key, threshold);
    }
}

fn clear_bucket(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    key: (CityId, ProductKind),
    threshold: u32,
) {
    let (city, product) = key;
    let Some(orders) = state.order_book.remove(&key) else {
        return;
    };

    let (mut buys, mut sells): (Vec<_>, Vec<_>) = orders
        .into_iter()
        .partition(|o| matches!(o.side, OrderSide::Buy));

    buys.sort_by(|a, b| {
        b.unit_price
            .cmp(&a.unit_price)
            .then_with(|| a.player.cmp(&b.player))
            .then_with(|| a.id.cmp(&b.id))
    });
    sells.sort_by(|a, b| {
        a.unit_price
            .cmp(&b.unit_price)
            .then_with(|| a.player.cmp(&b.player))
            .then_with(|| a.id.cmp(&b.id))
    });

    let submitted_buy_qty: u32 = buys.iter().map(|o| o.quantity).sum();
    let submitted_sell_qty: u32 = sells.iter().map(|o| o.quantity).sum();

    let (fills, clearing_price, matched_qty) = match_orders(&buys, &sells);

    // Settlement: clearing_price varsa her fill'i (saturation split + cash/stock
    // transfer). Başarılı transfer sayısı raporun değil, `matched_qty` zaten
    // match aşamasında kaydedilmişti — settlement reject'leri ayrı event.
    let saturation_qty = if let Some(price) = clearing_price {
        let sat = settle_fills(state, report, tick, city, product, &fills, price, threshold);
        // Eşleşme olduysa price history'ye ekle (rolling avg için).
        state
            .price_history
            .entry(key)
            .or_default()
            .push((tick, price));
        sat
    } else {
        0
    };

    report.push(LogEntry::market_cleared(
        tick,
        city,
        product,
        clearing_price,
        matched_qty,
        submitted_buy_qty,
        submitted_sell_qty,
        threshold,
        saturation_qty,
    ));
}

/// Fills'i settle et. Saturation eşiği cumulative qty'ye göre full/half tier'a
/// böler: `cum ≤ threshold` kısmı `clearing_price`'ta, üstü `clearing_price/2`'de.
/// Fill bir tier'a düşüyorsa tek segment, eşiği ortada aşıyorsa iki segment olur.
/// Her segment için `OrderMatched` emit edilir (analitik için effective price belli).
///
/// Buyer cash yetmezse veya seller stok yetmezse segment iptal, `FillRejected`
/// event'i yazılır, state değişmez (para korunumu bozulmaz).
///
/// Dönüş: half tier'a düşen toplam qty (analitik).
#[allow(clippy::too_many_arguments)]
fn settle_fills(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    city: CityId,
    product: ProductKind,
    fills: &[Fill],
    clearing_price: Money,
    threshold: u32,
) -> u32 {
    let half_price = Money::from_cents(clearing_price.as_cents() / 2);
    let mut cum: u32 = 0;
    let mut saturation_qty: u32 = 0;

    for fill in fills {
        let (full_qty, half_qty) = split_by_threshold(cum, fill.quantity, threshold);
        cum = cum.saturating_add(fill.quantity);
        saturation_qty = saturation_qty.saturating_add(half_qty);

        if full_qty > 0 {
            settle_segment(
                state,
                report,
                tick,
                city,
                product,
                fill,
                full_qty,
                clearing_price,
            );
        }
        if half_qty > 0 {
            settle_segment(
                state, report, tick, city, product, fill, half_qty, half_price,
            );
        }
    }

    saturation_qty
}

/// Fill'in `qty`'sini saturation eşiğine göre (full, half) ikilisine böl.
///
/// - `cum + qty ≤ threshold` → hepsi full tier.
/// - `cum ≥ threshold` → hepsi half tier.
/// - Aksi halde `threshold - cum` full, kalan half.
fn split_by_threshold(cum: u32, qty: u32, threshold: u32) -> (u32, u32) {
    if cum >= threshold {
        (0, qty)
    } else if cum.saturating_add(qty) <= threshold {
        (qty, 0)
    } else {
        let full = threshold - cum;
        (full, qty - full)
    }
}

/// Tek bir (fill, qty, price) segment'i settle et.
///
/// Validation pre-flight: her iki tarafın da yeterliliği kontrol edilir.
/// Birinde eksiklik varsa segment iptal, `FillRejected` event yazılır.
/// Self-trade (buyer == seller) izinlidir — net değişim sıfır.
#[allow(clippy::too_many_arguments)]
fn settle_segment(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    city: CityId,
    product: ProductKind,
    fill: &Fill,
    qty: u32,
    price: Money,
) {
    // Toplam değer = qty × price.
    let Ok(total) = price.checked_mul_scalar(i64::from(qty)) else {
        report.push(LogEntry::fill_rejected(
            tick,
            city,
            product,
            fill.buyer,
            fill.seller,
            qty,
            "segment total overflow",
        ));
        return;
    };

    // Pre-flight: buyer/seller var mı, cash/stok yeterli mi?
    let buyer_ok = state
        .players
        .get(&fill.buyer)
        .is_some_and(|p| p.cash >= total);
    let seller_ok = state
        .players
        .get(&fill.seller)
        .is_some_and(|p| p.inventory.get(city, product) >= qty);

    if !buyer_ok {
        report.push(LogEntry::fill_rejected(
            tick,
            city,
            product,
            fill.buyer,
            fill.seller,
            qty,
            "buyer insufficient funds",
        ));
        return;
    }
    if !seller_ok {
        report.push(LogEntry::fill_rejected(
            tick,
            city,
            product,
            fill.buyer,
            fill.seller,
            qty,
            "seller insufficient stock",
        ));
        return;
    }

    // Apply: seller önce (debit stok + credit cash), sonra buyer.
    // İki ayrı mutable borrow — farklı BTreeMap anahtarları ama aynı self.
    // get_mut'u sırayla çağırmak güvenli. Self-trade (same id) edge: iki
    // sıralı mutasyon net sıfır sonuç verir.
    if let Some(seller) = state.players.get_mut(&fill.seller) {
        seller
            .inventory
            .remove(city, product, qty)
            .expect("pre-flight validated");
        seller.credit(total).expect("cash overflow on credit");
    }
    if let Some(buyer) = state.players.get_mut(&fill.buyer) {
        buyer.debit(total).expect("pre-flight validated");
        buyer
            .inventory
            .add(city, product, qty)
            .expect("inventory overflow on add");
    }

    report.push(LogEntry::order_matched(
        tick,
        city,
        product,
        fill.buy_order_id,
        fill.sell_order_id,
        fill.buyer,
        fill.seller,
        qty,
        price,
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
        CityId, GameState, MarketOrder, Money, OrderId, OrderSide, Player, PlayerId, ProductKind,
        Role, RoomConfig, RoomId, Tick,
    };

    fn state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    /// Cömert test oyuncusu: 1M₺ nakit, 10k birim stok herkes için hazır.
    /// Settlement'ın cash/stock rejection yollarını ayrı testlerle sınıyoruz.
    fn seed_player(state: &mut GameState, player_id: u64, role: Role) {
        let mut p = Player::new(
            PlayerId::new(player_id),
            format!("P{player_id}"),
            role,
            Money::from_lira(1_000_000).unwrap(),
            false,
        )
        .unwrap();
        // Her şehir/ürün için 10k birim cömert stok.
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                p.inventory.add(city, product, 10_000).unwrap();
            }
        }
        state.players.insert(p.id, p);
    }

    fn seed_players(state: &mut GameState, ids: &[u64]) {
        for &id in ids {
            seed_player(state, id, Role::Tuccar);
        }
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
        seed_players(&mut s, &[1, 2]);
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
        seed_players(&mut s, &[1, 2]);
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
        seed_players(&mut s, &[1, 2, 3, 4]);
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
        seed_players(&mut s, &[1, 2, 9]);
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
        seed_players(&mut s, &[1, 2]);
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

    // -----------------------------------------------------------------------
    // Faz 3C: Settlement + Saturation + price_history testleri
    // -----------------------------------------------------------------------

    #[test]
    fn settlement_transfers_cash_and_inventory() {
        let mut s = state();
        seed_players(&mut s, &[1, 2]);
        let seller_stock_before = s.players[&PlayerId::new(2)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk);
        let buyer_cash_before = s.players[&PlayerId::new(1)].cash;
        let seller_cash_before = s.players[&PlayerId::new(2)].cash;

        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 10),
                order(2, 2, OrderSide::Sell, 10, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        // Clearing price = 9₺ (midpoint). Toplam = 10 × 9 = 90₺.
        let total = Money::from_lira(90).unwrap();
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            buyer_cash_before.checked_sub(total).unwrap()
        );
        assert_eq!(
            s.players[&PlayerId::new(2)].cash,
            seller_cash_before.checked_add(total).unwrap()
        );
        // Buyer 10 birim kazandı, seller 10 birim kaybetti.
        assert_eq!(
            s.players[&PlayerId::new(1)]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            10_000 + 10
        );
        assert_eq!(
            s.players[&PlayerId::new(2)]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            seller_stock_before - 10
        );
    }

    #[test]
    fn money_conservation_holds_across_clearing() {
        let mut s = state();
        seed_players(&mut s, &[1, 2, 3, 4]);
        let total_cash_before: i64 = s.players.values().map(|p| p.cash.as_cents()).sum();
        let total_stock_before: u64 = s.players.values().map(|p| p.inventory.total_units()).sum();

        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 5, 12),
                order(2, 2, OrderSide::Buy, 7, 11),
                order(3, 3, OrderSide::Sell, 5, 8),
                order(4, 4, OrderSide::Sell, 5, 10),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        let total_cash_after: i64 = s.players.values().map(|p| p.cash.as_cents()).sum();
        let total_stock_after: u64 = s.players.values().map(|p| p.inventory.total_units()).sum();

        assert_eq!(
            total_cash_before, total_cash_after,
            "cash must be conserved"
        );
        assert_eq!(
            total_stock_before, total_stock_after,
            "stock must be conserved"
        );
    }

    #[test]
    fn price_history_receives_entry_on_match() {
        let mut s = state();
        seed_players(&mut s, &[1, 2]);
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 10),
                order(2, 2, OrderSide::Sell, 10, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(5));
        clear_markets(&mut s, &mut r, Tick::new(5));

        let hist = s
            .price_history
            .get(&(CityId::Istanbul, ProductKind::Pamuk))
            .expect("history entry created");
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0], (Tick::new(5), Money::from_lira(9).unwrap()));
    }

    #[test]
    fn price_history_skipped_when_no_matches() {
        let mut s = state();
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 5), // spread
                order(2, 2, OrderSide::Sell, 10, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));
        assert!(
            !s.price_history
                .contains_key(&(CityId::Istanbul, ProductKind::Pamuk))
        );
    }

    #[test]
    fn insufficient_buyer_cash_emits_fill_rejected() {
        let mut s = state();
        // Alıcı fakir (100₺), 10×10 = 100₺ gerekli ama midpoint ile 10×9 = 90₺.
        // Tamamı alınabilir 90₺'lık. Cash eksik senaryosu için alıcı cash = 50₺.
        let mut buyer = Player::new(
            PlayerId::new(1),
            "fakir",
            Role::Tuccar,
            Money::from_lira(50).unwrap(),
            false,
        )
        .unwrap();
        buyer
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100)
            .unwrap();
        s.players.insert(buyer.id, buyer);
        seed_player(&mut s, 2, Role::Tuccar);

        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 10),
                order(2, 2, OrderSide::Sell, 10, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        let rejected = r.entries.iter().any(|e| {
            matches!(
                e.event,
                crate::report::LogEvent::FillRejected {
                    reason: ref s,
                    ..
                } if s.contains("buyer insufficient funds")
            )
        });
        assert!(rejected, "expected FillRejected for poor buyer");
        // Alıcının nakti değişmemeli.
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            Money::from_lira(50).unwrap()
        );
    }

    #[test]
    fn insufficient_seller_stock_emits_fill_rejected() {
        let mut s = state();
        seed_player(&mut s, 1, Role::Tuccar);
        // Satıcı stoksuz.
        let seller = Player::new(
            PlayerId::new(2),
            "stoksuz",
            Role::Tuccar,
            Money::from_lira(1_000).unwrap(),
            false,
        )
        .unwrap();
        s.players.insert(seller.id, seller);

        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 10),
                order(2, 2, OrderSide::Sell, 10, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        let rejected = r.entries.iter().any(|e| {
            matches!(
                e.event,
                crate::report::LogEvent::FillRejected {
                    reason: ref s,
                    ..
                } if s.contains("seller insufficient stock")
            )
        });
        assert!(rejected, "expected FillRejected for stockless seller");
    }

    #[test]
    fn split_by_threshold_full_below() {
        assert_eq!(split_by_threshold(0, 10, 40), (10, 0));
        assert_eq!(split_by_threshold(30, 10, 40), (10, 0));
    }

    #[test]
    fn split_by_threshold_half_above() {
        assert_eq!(split_by_threshold(40, 10, 40), (0, 10));
        assert_eq!(split_by_threshold(100, 10, 40), (0, 10));
    }

    #[test]
    fn split_by_threshold_straddling() {
        // cum=35, qty=10, threshold=40 → full=5, half=5
        assert_eq!(split_by_threshold(35, 10, 40), (5, 5));
    }

    #[test]
    fn saturation_reports_qty_over_threshold_at_half_price() {
        // 10 oyuncu ekleyip threshold'ı 120'ye çıkar: 40 + (10-2)*10.
        // Tek eşleşme 150 birim → 120 full, 30 half.
        let mut s = state();
        for i in 1..=10 {
            seed_player(&mut s, i, Role::Tuccar);
        }
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 150, 10),
                order(2, 2, OrderSide::Sell, 150, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        match r.entries.last().map(|e| &e.event) {
            Some(crate::report::LogEvent::MarketCleared {
                saturation_threshold,
                saturation_qty,
                matched_qty,
                ..
            }) => {
                assert_eq!(*saturation_threshold, 120);
                assert_eq!(*saturation_qty, 30);
                assert_eq!(*matched_qty, 150);
            }
            other => panic!("expected MarketCleared, got {other:?}"),
        }
    }
}
