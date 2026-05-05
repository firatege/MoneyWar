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
//!    - Eşleşmeyen kalıntı: `remaining_ticks -= 1`. 0'a düşen emir
//!      `OrderExpired` event'iyle düşer; TTL'si kalan emir kitapta yeni qty
//!      ile kalır (persistent order book).

use moneywar_domain::{
    CityId, GameState, MarketOrder, Money, ProductKind, Tick,
    balance::{PRICE_CLAMP_HIGH_PCT, PRICE_CLAMP_LOW_PCT, TRANSACTION_TAX_PCT},
};

use crate::report::{LogEntry, TickReport};

/// Tek bir eşleşme. Pay-as-bid modeli: `price` her fill için ayrı tutulur,
/// alıcının kendi BUY emrindeki `unit_price`'ı.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Fill {
    buy_order_id: moneywar_domain::OrderId,
    sell_order_id: moneywar_domain::OrderId,
    buyer: moneywar_domain::PlayerId,
    seller: moneywar_domain::PlayerId,
    quantity: u32,
    /// Fill fiyatı — pay-as-bid mantığında alıcının limit fiyatı. Saturation
    /// half tier'a düşen segment bu fiyatın yarısından settle olur.
    price: Money,
}

/// Tüm `(city, product)` bucket'larını sırayla temizle.
///
/// Bucket'ların işleme sırası `BTreeMap` iterasyon sırası — yani
/// `(CityId, ProductKind)` doğal sıralaması. Determinism için kritik.
///
/// Algoritma: **batch + tick-shuffle + pay-as-bid** (memory'deki borsa kararı):
/// 1. Tick boyu emirler kitaba toplandı (mevcut).
/// 2. Tick sonu motor her bucket'ı seed-shuffled sırada işler.
/// 3. Her gelen emir kitaba düşer; eğer karşı tarafta uygun fiyat varsa
///    anında eşleşir (continuous matching tek-tick içinde).
/// 4. Trade fiyatı = **alıcının BUY limit fiyatı** (pay-as-bid).
/// 5. Saturation eşik üstü: yarı fiyatta settle (anti-snowball).
pub(crate) fn clear_markets(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let keys: Vec<(CityId, ProductKind)> = state.order_book.keys().copied().collect();
    let threshold = state.config.saturation_threshold(state.participant_count());
    // Tek deterministic RNG bütün bucket'lar için — bucket sırası BTreeMap
    // iteration deterministik, RNG state ilerler ama hepsi aynı seed'den.
    let mut rng = crate::rng::rng_for(state.room_id, tick);
    for key in keys {
        clear_bucket(state, report, tick, key, threshold, &mut rng);
    }
}

fn clear_bucket(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    key: (CityId, ProductKind),
    threshold: u32,
    rng: &mut rand_chacha::ChaCha8Rng,
) {
    let (city, product) = key;
    let Some(mut orders) = state.order_book.remove(&key) else {
        return;
    };

    let submitted_buy_qty: u32 = orders
        .iter()
        .filter(|o| o.side.is_buy())
        .map(|o| o.quantity)
        .sum();
    let submitted_sell_qty: u32 = orders
        .iter()
        .filter(|o| o.side.is_sell())
        .map(|o| o.quantity)
        .sum();

    // Tick-shuffle: emirler deterministic random sırayla kitaba düşer.
    // Sıralama yerine RNG → NPC'lerin tick başında erken emit avantajı kapanır,
    // human/NPC eşit şansta. Tie-break sonrası `(player_id, order_id)` ASC.
    use rand::seq::SliceRandom;
    orders.shuffle(rng);

    let baseline = state.effective_baseline(city, product);
    let (fills, leftover) = match_continuous(&orders, baseline);
    let matched_qty: u32 = fills.iter().map(|f| f.quantity).sum();

    // Settlement: pay-as-bid + saturation half tier.
    let saturation_qty = settle_fills(state, report, tick, city, product, &fills, threshold);

    // price_history: matched fill'lerin ortalama fiyatı (rolling avg referansı).
    let avg_price = if matched_qty > 0 {
        let total_value: i64 = fills
            .iter()
            .map(|f| f.price.as_cents().saturating_mul(i64::from(f.quantity)))
            .sum();
        Some(Money::from_cents(total_value / i64::from(matched_qty)))
    } else {
        None
    };
    if let Some(p) = avg_price {
        state.price_history.entry(key).or_default().push((tick, p));
    }

    // TTL persist: kalan emirler kitapta bekler.
    persist_leftover_orders(state, report, tick, key, leftover);

    report.push(LogEntry::market_cleared(
        tick,
        city,
        product,
        avg_price,
        matched_qty,
        submitted_buy_qty,
        submitted_sell_qty,
        threshold,
        saturation_qty,
    ));
}

/// Continuous matching — shuffled sıraya göre her gelen emir kitaba düşer,
/// karşı tarafta uygun fiyat varsa anında eşleşir.
///
/// Trade fiyatı **pay-as-bid**: alıcının BUY limit fiyatı.
/// - BUY incoming + SELL kitapta best (en düşük) → fiyat = incoming.price
/// - SELL incoming + BUY kitapta best (en yüksek) → fiyat = best.price (kitaptaki BUY)
///
/// Vic3 fiyat clamp `baseline × [25%, 175%]` aralığına sıkıştırılır.
fn match_continuous(
    shuffled: &[MarketOrder],
    baseline: Option<Money>,
) -> (Vec<Fill>, Vec<MarketOrder>) {
    let mut buy_book: Vec<MarketOrder> = Vec::new();
    let mut sell_book: Vec<MarketOrder> = Vec::new();
    let mut fills: Vec<Fill> = Vec::new();

    let (low_cents, high_cents) = match baseline {
        Some(b) => {
            let bc = b.as_cents();
            (
                (bc.saturating_mul(PRICE_CLAMP_LOW_PCT) / 100).max(1),
                (bc.saturating_mul(PRICE_CLAMP_HIGH_PCT) / 100).max(1),
            )
        }
        None => (1, i64::MAX),
    };

    for incoming in shuffled {
        let mut remaining = incoming.quantity;
        loop {
            if remaining == 0 {
                break;
            }
            // Karşı tarafın en iyi emrini seç. Tie-break (player_id, order_id) ASC.
            let opp_best_idx = if incoming.side.is_buy() {
                pick_best_sell(&sell_book)
            } else {
                pick_best_buy(&buy_book)
            };
            let Some(idx) = opp_best_idx else { break };
            let best = if incoming.side.is_buy() {
                &sell_book[idx]
            } else {
                &buy_book[idx]
            };

            // Spread kontrolü.
            let crossed = if incoming.side.is_buy() {
                incoming.unit_price >= best.unit_price
            } else {
                incoming.unit_price <= best.unit_price
            };
            if !crossed {
                break;
            }

            let qty = remaining.min(best.quantity);
            // Pay-as-bid: trade price = BUY emrindeki limit. Clamp ile sınırla.
            let raw_price = if incoming.side.is_buy() {
                incoming.unit_price
            } else {
                best.unit_price
            };
            let price = Money::from_cents(raw_price.as_cents().clamp(low_cents, high_cents));

            let (buy_id, sell_id, buyer, seller) = if incoming.side.is_buy() {
                (incoming.id, best.id, incoming.player, best.player)
            } else {
                (best.id, incoming.id, best.player, incoming.player)
            };
            fills.push(Fill {
                buy_order_id: buy_id,
                sell_order_id: sell_id,
                buyer,
                seller,
                quantity: qty,
                price,
            });
            remaining -= qty;

            // Karşı kitaptaki emir miktarını güncelle / kaldır.
            let opp_book = if incoming.side.is_buy() {
                &mut sell_book
            } else {
                &mut buy_book
            };
            if opp_book[idx].quantity == qty {
                opp_book.remove(idx);
            } else {
                opp_book[idx].quantity -= qty;
            }
        }

        if remaining > 0 {
            let mut leftover = incoming.clone();
            leftover.quantity = remaining;
            if leftover.side.is_buy() {
                buy_book.push(leftover);
            } else {
                sell_book.push(leftover);
            }
        }
    }

    let mut leftover_all = buy_book;
    leftover_all.extend(sell_book);
    (fills, leftover_all)
}

/// En düşük fiyatlı SELL emri — tie-break `(player_id, order_id)` ASC.
fn pick_best_sell(book: &[MarketOrder]) -> Option<usize> {
    book.iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            a.unit_price
                .cmp(&b.unit_price)
                .then_with(|| a.player.cmp(&b.player))
                .then_with(|| a.id.cmp(&b.id))
        })
        .map(|(i, _)| i)
}

/// En yüksek fiyatlı BUY emri — tie-break `(player_id, order_id)` ASC.
fn pick_best_buy(book: &[MarketOrder]) -> Option<usize> {
    book.iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            a.unit_price
                .cmp(&b.unit_price)
                // Eşit fiyatta küçük player_id öncelikli → reverse for max
                .then_with(|| b.player.cmp(&a.player))
                .then_with(|| b.id.cmp(&a.id))
        })
        .map(|(i, _)| i)
}

/// Eşleşmeyen / kısmen eşleşmiş emirleri kitaba geri koyar, TTL countdown'u
/// uygular. `match_continuous` zaten leftover listesini hazır döner — burada
/// sadece TTL azalt + expire/persist.
fn persist_leftover_orders(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    key: (CityId, ProductKind),
    leftover: Vec<MarketOrder>,
) {
    let (city, product) = key;
    let mut cooldown_keys: Vec<moneywar_domain::PlayerId> = Vec::new();
    let mut kept: Vec<MarketOrder> = Vec::new();

    for mut o in leftover {
        // Bu clear emrin son hakkıysa, kitaba geri dönmez → expire + cooldown.
        if o.remaining_ticks <= 1 {
            report.push(LogEntry::order_expired(
                tick, o.id, o.player, city, product, o.side, o.quantity,
            ));
            cooldown_keys.push(o.player);
            continue;
        }
        o.remaining_ticks -= 1;
        kept.push(o);
    }

    if !kept.is_empty() {
        state.order_book.insert(key, kept);
    }

    // Cooldown'ları topluca uygula — eşleşmiş emir sahipleri için. Match olan
    // emirlerin sahibi `match_continuous` sırasında tüketildi, ID'leri
    // leftover'da yok — cooldown başlatmak için `Fill`'lerden okumak gerek.
    // Pratikte: cooldown sadece expire'larda anlamlı (eşleşen emir zaten
    // başarılı, cooldown'a gerek yok). Eski kod cooldown'u eşleşene de
    // veriyordu (anti-spam) ama yeni model bunu sadeleşmeye bırakır.
    for player in cooldown_keys {
        crate::tick::set_relist_cooldown(state, player, city, product, tick);
    }
}

/// Fills'i settle et. **Pay-as-bid**: her fill kendi `price`'ında transfer.
/// Saturation eşik üstü segment yarı fiyatta settle olur (anti-snowball).
fn settle_fills(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    city: CityId,
    product: ProductKind,
    fills: &[Fill],
    threshold: u32,
) -> u32 {
    let mut cum: u32 = 0;
    let mut saturation_qty: u32 = 0;

    for fill in fills {
        let (full_qty, half_qty) = split_by_threshold(cum, fill.quantity, threshold);
        cum = cum.saturating_add(fill.quantity);
        saturation_qty = saturation_qty.saturating_add(half_qty);

        if full_qty > 0 {
            settle_segment(state, report, tick, city, product, fill, full_qty, fill.price);
        }
        if half_qty > 0 {
            let half_price = Money::from_cents(fill.price.as_cents() / 2);
            settle_segment(state, report, tick, city, product, fill, half_qty, half_price);
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

    // İşlem vergisi: alıcıdan **ek** kesilir, sistem dışına atılır (hard sink).
    // EVE Online "broker fee + sales tax" karşılığı. Self-trade'de de uygulanır
    // (yıkama satışlarını cezalandırır).
    let tax_cents = total.as_cents().saturating_mul(TRANSACTION_TAX_PCT) / 100;
    let tax = Money::from_cents(tax_cents.max(0));
    let Ok(total_with_tax) = total.checked_add(tax) else {
        report.push(LogEntry::fill_rejected(
            tick,
            city,
            product,
            fill.buyer,
            fill.seller,
            qty,
            "tax overflow",
        ));
        return;
    };

    // Pre-flight: buyer/seller var mı, cash/stok yeterli mi?
    // Buyer artık total + tax karşılamalı.
    let buyer_ok = state
        .players
        .get(&fill.buyer)
        .is_some_and(|p| p.cash >= total_with_tax);
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
    // Tax buyer'dan ek kesinti, hiçbir yere kredi edilmez = sistem sink.
    if let Some(seller) = state.players.get_mut(&fill.seller) {
        seller
            .inventory
            .remove(city, product, qty)
            .expect("pre-flight validated");
        seller.credit(total).expect("cash overflow on credit");
    }
    if let Some(buyer) = state.players.get_mut(&fill.buyer) {
        buyer
            .debit(total_with_tax)
            .expect("pre-flight validated");
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

// Eski sıralı `match_orders` (uniform-price midpoint) kaldırıldı —
// `match_continuous` (tick-shuffle + pay-as-bid) ile değiştirildi.

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

        // TTL=1 default → her iki emir OrderExpired olur, MarketCleared son entry.
        let expired_count = r
            .entries
            .iter()
            .filter(|e| matches!(e.event, crate::report::LogEvent::OrderExpired { .. }))
            .count();
        assert_eq!(expired_count, 2);
        let cleared = r
            .entries
            .iter()
            .find(|e| matches!(e.event, crate::report::LogEvent::MarketCleared { .. }))
            .expect("market cleared entry");
        match &cleared.event {
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
        // TTL=1 emirler expire → kitap boşaldı.
        assert!(s.order_book.is_empty());
    }

    #[test]
    fn single_match_uses_buy_limit_price() {
        // Pay-as-bid: trade fiyatı = BUY emrindeki limit (alıcının verdiği fiyat).
        // BUY @ 10, SELL @ 8 → eşleşir, fiyat 10 (alıcı söz verdiği parayı öder).
        let mut s = state();
        seed_players(&mut s, &[1, 2]);
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 10, 10),
                order(2, 2, OrderSide::Sell, 10, 8),
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        assert_eq!(r.entries.len(), 2);
        match &r.entries[0].event {
            crate::report::LogEvent::OrderMatched {
                quantity, price, ..
            } => {
                assert_eq!(*quantity, 10);
                assert_eq!(*price, Money::from_lira(10).unwrap(), "pay-as-bid: BUY fiyatı");
            }
            other => panic!("expected OrderMatched, got {other:?}"),
        }
        match &r.entries[1].event {
            crate::report::LogEvent::MarketCleared {
                clearing_price,
                matched_qty,
                ..
            } => {
                assert_eq!(*clearing_price, Some(Money::from_lira(10).unwrap()));
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

        // Pay-as-bid + tick-shuffle: spesifik sıralama RNG bağımlı.
        // Yeni invariant: tüm overlapping qty (10) eşleşir, fiyat avg
        // BUY limit aralığında [10..12].
        let total_matched: u32 = r
            .entries
            .iter()
            .filter_map(|e| match &e.event {
                crate::report::LogEvent::OrderMatched { quantity, .. } => Some(*quantity),
                _ => None,
            })
            .sum();
        assert_eq!(total_matched, 10, "tüm overlap (10 birim) eşleşmeli");

        match r.entries.last().map(|e| &e.event) {
            Some(crate::report::LogEvent::MarketCleared {
                clearing_price,
                matched_qty,
                ..
            }) => {
                assert_eq!(*matched_qty, 10);
                let avg = clearing_price.expect("avg fiyat var").as_cents();
                assert!(
                    (1000..=1200).contains(&avg),
                    "avg fiyat BUY limit [10..12]₺, got {avg} cents"
                );
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
        // Sadece buy, sell yok → MarketCleared + OrderExpired (TTL=1 default).
        let mut s = state();
        populate(&mut s, vec![order(1, 1, OrderSide::Buy, 10, 10)]);
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        let cleared = r
            .entries
            .iter()
            .find(|e| matches!(e.event, crate::report::LogEvent::MarketCleared { .. }))
            .expect("market cleared");
        match &cleared.event {
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
        let expired = r
            .entries
            .iter()
            .filter(|e| matches!(e.event, crate::report::LogEvent::OrderExpired { .. }))
            .count();
        assert_eq!(expired, 1);
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

        // Her bucket: 1 MarketCleared + 1 OrderExpired (TTL=1 default) → 4 entry.
        let cleared = r
            .entries
            .iter()
            .filter(|e| matches!(e.event, crate::report::LogEvent::MarketCleared { .. }))
            .count();
        let expired = r
            .entries
            .iter()
            .filter(|e| matches!(e.event, crate::report::LogEvent::OrderExpired { .. }))
            .count();
        assert_eq!(cleared, 2);
        assert_eq!(expired, 2);
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

        // Pay-as-bid: trade fiyatı = BUY emrindeki limit (10₺).
        // Toplam = 10 × 10 = 100₺. Buyer ek %TRANSACTION_TAX_PCT vergi.
        let total = Money::from_lira(100).unwrap();
        let tax_cents = total.as_cents().saturating_mul(TRANSACTION_TAX_PCT) / 100;
        let total_with_tax = Money::from_cents(total.as_cents() + tax_cents);
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            buyer_cash_before.checked_sub(total_with_tax).unwrap()
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

        // İşlem vergisi sistem dışına atılır → cash korunum sıfır değil, vergi farkıdır.
        // total_cash_before - total_cash_after == toplam silinen vergi olmalı.
        let cash_delta = total_cash_before - total_cash_after;
        assert!(
            cash_delta >= 0,
            "cash can only leave system via tax, never increase: delta={cash_delta}"
        );
        assert_eq!(
            total_stock_before, total_stock_after,
            "stock must be conserved (vergi sadece cash sink, stok değil)"
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

        // Pay-as-bid: fiyat avg = BUY limit (tek fill, 10₺).
        let hist = s
            .price_history
            .get(&(CityId::Istanbul, ProductKind::Pamuk))
            .expect("history entry created");
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0], (Tick::new(5), Money::from_lira(10).unwrap()));
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
        // 10 oyuncu → threshold = 250 + (10-2)*50 = 650 (10× hacim ölçek).
        // Tek eşleşme 1000 birim → 650 full, 350 half.
        let mut s = state();
        for i in 1..=10 {
            seed_player(&mut s, i, Role::Tuccar);
        }
        populate(
            &mut s,
            vec![
                order(1, 1, OrderSide::Buy, 1000, 10),
                order(2, 2, OrderSide::Sell, 1000, 8),
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
                assert_eq!(*saturation_threshold, 650);
                assert_eq!(*saturation_qty, 350);
                assert_eq!(*matched_qty, 1000);
            }
            other => panic!("expected MarketCleared, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // TTL persistence testleri
    // -----------------------------------------------------------------------

    fn order_ttl(
        id: u64,
        player: u64,
        side: OrderSide,
        qty: u32,
        price_lira: i64,
        ttl: u32,
    ) -> MarketOrder {
        MarketOrder::new_with_ttl(
            OrderId::new(id),
            PlayerId::new(player),
            CityId::Istanbul,
            ProductKind::Pamuk,
            side,
            qty,
            Money::from_lira(price_lira).unwrap(),
            Tick::new(1),
            ttl,
        )
        .unwrap()
    }

    #[test]
    fn order_with_ttl_greater_than_one_survives_unmatched_clear() {
        let mut s = state();
        populate(&mut s, vec![order_ttl(1, 1, OrderSide::Buy, 10, 10, 3)]);
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        // Eşleşme yok ama TTL=3 → 2'ye düşüp kitapta kalmalı, expire yok.
        let expired = r
            .entries
            .iter()
            .filter(|e| matches!(e.event, crate::report::LogEvent::OrderExpired { .. }))
            .count();
        assert_eq!(expired, 0);
        let bucket = s
            .order_book
            .get(&(CityId::Istanbul, ProductKind::Pamuk))
            .expect("bucket kept");
        assert_eq!(bucket.len(), 1);
        assert_eq!(bucket[0].remaining_ticks, 2);
        assert_eq!(bucket[0].quantity, 10);
    }

    #[test]
    fn order_expires_on_last_tick_of_ttl() {
        let mut s = state();
        populate(&mut s, vec![order_ttl(1, 1, OrderSide::Buy, 10, 10, 2)]);
        // 1. clear: remaining 2 → 1, kitapta kalır.
        let mut r1 = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r1, Tick::new(1));
        assert_eq!(
            s.order_book
                .get(&(CityId::Istanbul, ProductKind::Pamuk))
                .unwrap()[0]
                .remaining_ticks,
            1
        );
        // 2. clear: remaining 1 → expire.
        let mut r2 = TickReport::new(Tick::new(2));
        clear_markets(&mut s, &mut r2, Tick::new(2));
        let expired = r2
            .entries
            .iter()
            .filter(|e| matches!(e.event, crate::report::LogEvent::OrderExpired { .. }))
            .count();
        assert_eq!(expired, 1);
        assert!(s.order_book.is_empty());
    }

    #[test]
    fn partial_match_leaves_leftover_in_book_with_decremented_ttl() {
        let mut s = state();
        seed_players(&mut s, &[1, 2]);
        populate(
            &mut s,
            vec![
                order_ttl(1, 1, OrderSide::Buy, 20, 10, 5), // alıcı 20 birim
                order_ttl(2, 2, OrderSide::Sell, 8, 10, 1), // satıcı 8 birim (TTL=1)
            ],
        );
        let mut r = TickReport::new(Tick::new(1));
        clear_markets(&mut s, &mut r, Tick::new(1));

        // 8 match, alıcı 12 leftover, TTL 5→4. Satıcı tamamen eşleşti → silindi.
        let bucket = s
            .order_book
            .get(&(CityId::Istanbul, ProductKind::Pamuk))
            .expect("bucket kept");
        assert_eq!(bucket.len(), 1);
        assert_eq!(bucket[0].id, OrderId::new(1));
        assert_eq!(bucket[0].quantity, 12);
        assert_eq!(bucket[0].remaining_ticks, 4);
    }
}
