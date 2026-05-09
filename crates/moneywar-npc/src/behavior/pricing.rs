//! NPC fiyatlandırma yardımcıları — fiyat keşfi (price discovery) için.
//!
//! Eski model: NPC'ler `state.effective_baseline()` (statik) okuyor → BUY/SELL
//! aynı fiyat → "donmuş pazar" (Ankara Pamuk hep 615₺ × 90 tick).
//!
//! Yeni model:
//! 1. `state.reference_price()` — son 5 clearing'in ortalaması (rolling avg)
//!    veya yoksa baseline → NPC son trade'lere adapte olur
//! 2. `apply_jitter()` — her (tick, city, product, side) tuple'ı için
//!    deterministik ±5% noise → NPC bid/ask'ları farklılaşır → clearing
//!    fiyatı dağılır → rolling avg drift eder → fiyat keşfi döngüsü açılır
//!
//! Determinism: jitter sadece (tick, city, product, side) hash'ten — RNG yok,
//! aynı state aynı çıktı.

use moneywar_domain::{
    CityId, GameState, MAX_NO_MATCH_STREAK, Money, OrderSide, PlayerId, ProductKind, Tick,
};

/// Bu (tick, city, product, side, player) için ±3% jitter yüzdesi (-3..=+3).
/// v0.4.1: player_id eklendi — aynı bucket'ta farklı NPC'ler farklı jitter alır,
/// senkronize emir spam'ı kırılır. Eski versiyon 4 Tüccar aynı fiyatı veriyordu
/// ("bot army" sorunu, user gözlemi: NPC'ler birlikte hareket ediyor).
#[must_use]
pub fn jitter_pct(
    tick: Tick,
    city: CityId,
    product: ProductKind,
    side: OrderSide,
    player_id: PlayerId,
) -> i64 {
    let city_idx: u64 = match city {
        CityId::Istanbul => 1,
        CityId::Ankara => 2,
        CityId::Izmir => 3,
        CityId::Bursa => 4,
        CityId::Konya => 5,
    };
    let product_idx: u64 = match product {
        ProductKind::Pamuk => 1,
        ProductKind::Bugday => 2,
        ProductKind::Zeytin => 3,
        ProductKind::Kumas => 4,
        ProductKind::Un => 5,
        ProductKind::Zeytinyagi => 6,
    };
    let side_idx: u64 = match side {
        OrderSide::Buy => 1,
        OrderSide::Sell => 2,
    };
    // FNV-ish karışım — küçük tablo, iyi dağılım için 2654435761 (Knuth).
    let mut h = u64::from(tick.value());
    h = h.wrapping_mul(2_654_435_761);
    h ^= city_idx.wrapping_mul(7);
    h ^= product_idx.wrapping_mul(13);
    h ^= side_idx.wrapping_mul(17);
    h ^= player_id.value().wrapping_mul(31);
    h = h.wrapping_mul(2_654_435_761);
    ((h % 7) as i64) - 3
}

/// Fiyata ±3% NPC-spesifik jitter uygula. Sıfır/negatif sonuç 1 cent'e clamp.
#[must_use]
pub fn apply_jitter(
    price: Money,
    tick: Tick,
    city: CityId,
    product: ProductKind,
    side: OrderSide,
    player_id: PlayerId,
) -> Money {
    let pct = jitter_pct(tick, city, product, side, player_id);
    let multiplier = 100i64 + pct;
    let cents = price
        .as_cents()
        .saturating_mul(multiplier)
        .saturating_div(100);
    Money::from_cents(cents.max(1))
}

// =====================================================================
// v8.20: Order-book aware pricing — Faz 2
// =====================================================================
//
// Tasarım: rol-temelli **asimetrik cross**.
// - Çiftçi/Alıcı: CROSS — karşı tarafa yetiş
// - Sanayici/Spek: PASSIVE — kendi limitinde bekle
// - Tüccar: arbitraj kârı varsa CROSS
//
// Anti-deadlock: iki taraf da PASSIVE olsa bile **patience erosion** ve
// **season drift** ile zaman içinde birbirine yaklaşırlar.

/// Pricing policy — NPC karşı taraf emrine yetişmek istiyor mu, yoksa
/// kendi limitinde mi bekliyor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossPolicy {
    /// Karşı taraf en iyi emrine yetiş (best_bid/best_ask). Floor/ceiling
    /// ihlal edilirse pas geçer.
    Cross,
    /// Kendi limit fiyatını koru. Patience/season drift ile yine yumuşar.
    Passive,
}

/// Patience erosion + season drift + difficulty softener'in toplam yumuşama
/// yüzdesi (0-45 arası). Hem SELL floor'ı düşürür, hem BUY ceiling'i yükseltir.
///
/// - **Patience erosion** (0-15%): art arda match olmadan geçen tick sayısı.
/// - **Season drift** (0-15%): sezon ilerledikçe pozisyon kapatma baskısı.
/// - **Difficulty softener** (0-15%): Easy mode'da NPC fiyat marjları human
///   lehine kayar (state.market_softener_pct). Hard mode'da 0.
#[must_use]
fn urgency_pct(state: &GameState, player: PlayerId, city: CityId, product: ProductKind) -> i64 {
    let streak = state.no_match_streak(player, city, product);
    let patience = i64::from(streak.min(MAX_NO_MATCH_STREAK));
    let progress = state.season_progress().value(); // 0..=100
    let drift = i64::from(progress) * 15 / 100; // 0..=15
    let softener = i64::from(state.market_softener_pct).min(15);
    patience
        .saturating_add(drift)
        .saturating_add(softener)
        .min(45)
}

/// SELL emir için marketable fiyat hesabı (Çiftçi, Spek-ASK, Tüccar SELL).
///
/// `stock_floor` = "bu fiyatın altına asla satmam" tabanı. Genelde
/// `reference × stock_discount_pct` (Çiftçi'nin stok-baskısı indirimi).
///
/// - `Cross`: best_bid varsa ve floor'un üstündeyse → bid'i hedef al.
/// - `Passive`: floor'da kal.
///
/// Sonuç urgency_pct ile yumuşatılır → patience erosion + season drift uygular.
/// Zero/negative sonuç None döner (jitter sonrası).
#[must_use]
pub fn marketable_ask(
    state: &GameState,
    player: PlayerId,
    city: CityId,
    product: ProductKind,
    stock_floor: Money,
    policy: CrossPolicy,
    tick: Tick,
) -> Option<Money> {
    let urgency = urgency_pct(state, player, city, product);
    // SELL tarafında urgency floor'ı **düşürür** — daha agresif satış.
    let softened_floor = scale_pct(stock_floor, 100 - urgency);

    let crossed = matches!(policy, CrossPolicy::Cross)
        && state
            .best_bid(city, product)
            .is_some_and(|(b, _)| b >= softened_floor);
    let target = match policy {
        CrossPolicy::Cross => match state.best_bid(city, product) {
            Some((bid, _)) if bid >= softened_floor => bid,
            _ => softened_floor,
        },
        CrossPolicy::Passive => softened_floor,
    };
    // v0.5.1 fix: Cross policy'de target = best_bid; jitter sonrası SELL > BID
    // olursa match yok (jitter ±3% bid'in altına ezebilir). SELL Cross'ta
    // jitter yok — atomik bid eşleşmesi garantili. Passive'de jitter normal.
    let final_price = if crossed {
        target
    } else {
        apply_jitter(target, tick, city, product, OrderSide::Sell, player)
    };
    if final_price.as_cents() <= 0 {
        return None;
    }
    Some(final_price)
}

/// BUY emir için marketable fiyat hesabı (Sanayici, Alıcı, Spek-BID, Tüccar BUY).
///
/// `cash_ceiling` = "bu fiyatın üstüne asla almam" tavanı. Genelde
/// `reference × premium_pct`.
///
/// - `Cross`: best_ask varsa ve ceiling'in altındaysa → ask'ı hedef al.
/// - `Passive`: ceiling'de kal.
///
/// urgency_pct ceiling'i **yükseltir** — daha agresif alım.
#[must_use]
pub fn marketable_bid(
    state: &GameState,
    player: PlayerId,
    city: CityId,
    product: ProductKind,
    cash_ceiling: Money,
    policy: CrossPolicy,
    tick: Tick,
) -> Option<Money> {
    let urgency = urgency_pct(state, player, city, product);
    // BUY tarafında urgency ceiling'i **yükseltir** — daha pahalıya razı.
    let softened_ceiling = scale_pct(cash_ceiling, 100 + urgency);

    let crossed = matches!(policy, CrossPolicy::Cross)
        && state
            .best_ask(city, product)
            .is_some_and(|(a, _)| a <= softened_ceiling);
    let target = match policy {
        CrossPolicy::Cross => match state.best_ask(city, product) {
            Some((ask, _)) if ask <= softened_ceiling => ask,
            _ => softened_ceiling,
        },
        CrossPolicy::Passive => softened_ceiling,
    };
    // v0.5.1 fix: Cross policy'de target = best_ask; jitter sonrası BUY < ASK
    // olursa match yok. BUY Cross'ta jitter yok — atomik ask eşleşmesi.
    let final_price = if crossed {
        target
    } else {
        apply_jitter(target, tick, city, product, OrderSide::Buy, player)
    };
    if final_price.as_cents() <= 0 {
        return None;
    }
    Some(final_price)
}

fn scale_pct(price: Money, pct: i64) -> Money {
    Money::from_cents(
        price
            .as_cents()
            .saturating_mul(pct.max(0))
            .saturating_div(100),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_in_bounds() {
        let p1 = PlayerId::new(100);
        for tick in 0u32..50 {
            for city in CityId::ALL {
                for product in ProductKind::ALL {
                    for side in [OrderSide::Buy, OrderSide::Sell] {
                        let p = jitter_pct(Tick::new(tick), city, product, side, p1);
                        assert!((-3..=3).contains(&p), "jitter {p} out of range");
                    }
                }
            }
        }
    }

    #[test]
    fn jitter_varies_across_buckets() {
        let tick = Tick::new(10);
        let p1 = PlayerId::new(100);
        let mut seen = std::collections::BTreeSet::new();
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                seen.insert(jitter_pct(tick, city, product, OrderSide::Buy, p1));
            }
        }
        assert!(seen.len() >= 3, "jitter çeşitlilik düşük: {seen:?}");
    }

    #[test]
    fn jitter_varies_across_players() {
        // v0.4.1: Aynı bucket'ta farklı NPC'ler farklı jitter — "bot army"
        // sorununun fix'i. 8 farklı player için en az 3 farklı jitter beklenir.
        let tick = Tick::new(10);
        let mut seen = std::collections::BTreeSet::new();
        for pid_n in 100u64..108 {
            seen.insert(jitter_pct(
                tick,
                CityId::Istanbul,
                ProductKind::Pamuk,
                OrderSide::Buy,
                PlayerId::new(pid_n),
            ));
        }
        assert!(
            seen.len() >= 3,
            "NPC-spesifik jitter çeşitliliği düşük: {seen:?}"
        );
    }

    #[test]
    fn jitter_deterministic() {
        let pl = PlayerId::new(42);
        let p1 = jitter_pct(
            Tick::new(42),
            CityId::Ankara,
            ProductKind::Pamuk,
            OrderSide::Buy,
            pl,
        );
        let p2 = jitter_pct(
            Tick::new(42),
            CityId::Ankara,
            ProductKind::Pamuk,
            OrderSide::Buy,
            pl,
        );
        assert_eq!(p1, p2);
    }

    #[test]
    fn apply_jitter_preserves_order_of_magnitude() {
        let price = Money::from_cents(1000);
        let jittered = apply_jitter(
            price,
            Tick::new(5),
            CityId::Izmir,
            ProductKind::Un,
            OrderSide::Sell,
            PlayerId::new(100),
        );
        assert!(jittered.as_cents() >= 970);
        assert!(jittered.as_cents() <= 1030);
    }

    use moneywar_domain::{MarketOrder, OrderId, RoomConfig, RoomId};

    fn fresh_state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn pid(n: u64) -> PlayerId {
        PlayerId::new(n)
    }

    fn order(side: OrderSide, price_cents: i64, qty: u32, owner: u64) -> MarketOrder {
        MarketOrder::new(
            OrderId::new(qty as u64 + owner * 1000),
            pid(owner),
            CityId::Istanbul,
            ProductKind::Pamuk,
            side,
            qty,
            Money::from_cents(price_cents),
            Tick::new(1),
        )
        .unwrap()
    }

    #[test]
    fn marketable_ask_returns_floor_when_book_empty() {
        let s = fresh_state();
        let floor = Money::from_cents(1000);
        let p = marketable_ask(
            &s,
            pid(2),
            CityId::Istanbul,
            ProductKind::Pamuk,
            floor,
            CrossPolicy::Passive,
            Tick::new(5),
        )
        .unwrap();
        // jitter ±3% → 970..=1030
        assert!((970..=1030).contains(&p.as_cents()));
    }

    #[test]
    fn marketable_ask_crosses_to_best_bid_when_above_floor() {
        let mut s = fresh_state();
        s.order_book.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![order(OrderSide::Buy, 1500, 10, 1)],
        );
        let floor = Money::from_cents(1000);
        let p = marketable_ask(
            &s,
            pid(2),
            CityId::Istanbul,
            ProductKind::Pamuk,
            floor,
            CrossPolicy::Cross,
            Tick::new(5),
        )
        .unwrap();
        // best_bid 1500 > floor 1000 → bid hedef alınır, jitter ±3% → 1455..=1545
        assert!((1455..=1545).contains(&p.as_cents()));
    }

    #[test]
    fn marketable_ask_holds_floor_when_bid_below_floor() {
        let mut s = fresh_state();
        s.order_book.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![order(OrderSide::Buy, 500, 10, 1)],
        );
        let floor = Money::from_cents(1000);
        let p = marketable_ask(
            &s,
            pid(2),
            CityId::Istanbul,
            ProductKind::Pamuk,
            floor,
            CrossPolicy::Cross,
            Tick::new(5),
        )
        .unwrap();
        // best_bid 500 < floor 1000 → floor korunur (maliyetin altına satmaz)
        assert!((970..=1030).contains(&p.as_cents()));
    }

    #[test]
    fn marketable_bid_crosses_to_best_ask_when_below_ceiling() {
        let mut s = fresh_state();
        s.order_book.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![order(OrderSide::Sell, 800, 10, 1)],
        );
        let ceiling = Money::from_cents(1000);
        let p = marketable_bid(
            &s,
            pid(2),
            CityId::Istanbul,
            ProductKind::Pamuk,
            ceiling,
            CrossPolicy::Cross,
            Tick::new(5),
        )
        .unwrap();
        // best_ask 800 < ceiling 1000 → ask hedef alınır, jitter ±3% → 776..=824
        assert!((776..=824).contains(&p.as_cents()));
    }

    #[test]
    fn patience_erosion_softens_floor() {
        let mut s = fresh_state();
        // Player 2 için bu bucket'ta 10 tick streak — %10 yumuşama
        s.no_match_streak
            .insert((pid(2), CityId::Istanbul, ProductKind::Pamuk), 10);
        let floor = Money::from_cents(1000);
        let p = marketable_ask(
            &s,
            pid(2),
            CityId::Istanbul,
            ProductKind::Pamuk,
            floor,
            CrossPolicy::Passive,
            Tick::new(5),
        )
        .unwrap();
        // %10 yumuşama → 900 hedef, jitter ±3% → 873..=927
        assert!((873..=927).contains(&p.as_cents()), "got {}", p.as_cents());
    }

    #[test]
    fn patience_streak_capped_at_max() {
        let mut s = fresh_state();
        // 100 tick streak → cap 15
        s.no_match_streak
            .insert((pid(2), CityId::Istanbul, ProductKind::Pamuk), 100);
        let streak = s.no_match_streak(pid(2), CityId::Istanbul, ProductKind::Pamuk);
        assert_eq!(streak, MAX_NO_MATCH_STREAK);
    }

    #[test]
    fn passive_npcs_converge_within_15_ticks() {
        // İki taraf da PASSIVE: floor 1000, ceiling 800.
        // İlk tick'te BID(800) < ASK(1000) → match yok
        // 15 tick streak sonrası: floor → 1000×(1-30%)=700,  ceiling → 800×(1+30%)=1040
        // → 700 ASK ≤ 1040 BID → match olur
        let mut s = fresh_state();
        let key = (CityId::Istanbul, ProductKind::Pamuk);
        s.no_match_streak.insert((pid(1), key.0, key.1), 15);
        s.no_match_streak.insert((pid(2), key.0, key.1), 15);
        // Sezon ilerlemesi 0 (current_tick=0). Sadece patience etkisi.

        let ask = marketable_ask(
            &s,
            pid(1),
            key.0,
            key.1,
            Money::from_cents(1000),
            CrossPolicy::Passive,
            Tick::new(15),
        )
        .unwrap();
        let bid = marketable_bid(
            &s,
            pid(2),
            key.0,
            key.1,
            Money::from_cents(800),
            CrossPolicy::Passive,
            Tick::new(15),
        )
        .unwrap();
        // Patience max 15 → ask floor 1000×0.85=850, bid ceiling 800×1.15=920
        // jitter ±3% → ask ~825..=875, bid ~893..=947
        // bid > ask → match alanı açıldı
        assert!(
            bid.as_cents() > ask.as_cents() - 50,
            "konverge etmedi: ask={} bid={}",
            ask.as_cents(),
            bid.as_cents()
        );
    }
}
