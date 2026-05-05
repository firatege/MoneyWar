//! Sinyal hesaplama — `[0,1]` normalize NPC girdileri.
//!
//! Bir NPC'nin belirli `(city, product)` için tüm sinyallerini tek `BTreeMap`
//! içinde döner. `score_candidate` bu mapten ilgili anahtarları okur.
//!
//! # 16 sinyal
//!
//! - `cash`: NPC nakdi / 20K (typical starting cash)
//! - `stock`: bu (city,product) NPC stoğu / 100 birim
//! - `price_rel_avg`: son fiyat / fair value, 0=ucuz, 0.5=adil, 1=pahalı
//! - `momentum`: 5-tick fiyat trend, 0=düşüyor, 0.5=sabit, 1=yükseliyor
//! - `urgency`: sezon ilerleme (0=başlangıç, 1=son)
//! - `arbitrage`: şehirler arası max fiyat farkı
//! - `event`: aktif şok şiddeti (örn macro %35 → 0.35)
//! - `competition`: bu bucket'taki rakip emir hacmi / 200
//! - `bid_supply_ratio`: BID hacmi / (BID+ASK) — talep/arz dengesi
//! - `bankruptcy_risk`: cash 5K altında 1.0, 50K üstünde 0.0
//! - `factory_count`: NPC'nin fab sayısı / 3
//! - `caravan_count`: NPC'nin kervan sayısı / 3
//! - `season_remaining`: 1 - urgency
//! - `rival_action_pressure`: bu bucket'ta farklı oyuncu sayısı / 5
//! - `ask_supply_ratio`: ASK hacmi / (BID+ASK)
//! - `local_raw_advantage`: 1.0 eğer (city, product) yerel uzmanlık eşleşiyor
//!
//! # Determinism
//!
//! Tüm hesaplar `state` üzerinden, RNG yok. `BTreeMap` iterasyon deterministik.

#![allow(clippy::cast_precision_loss, clippy::cast_lossless)]

use std::collections::BTreeMap;

use moneywar_domain::{CityId, GameState, OrderSide, PlayerId, ProductKind};

/// NPC sinyalleri map'i. Anahtar `&'static str` — ağırlık tablolarıyla eşleşir.
pub type Inputs = BTreeMap<&'static str, f64>;

/// Tipik başlangıç nakdi — Sanayici 30K, Tüccar 15K, Esnaf 10K, ortalama 20K.
const TYPICAL_STARTING_CASH_LIRA: f64 = 20_000.0;

/// Stok normalize referansı — 100 birim üstü maksimum (1.0).
const TYPICAL_STOCK_CAPACITY: f64 = 100.0;

/// NPC için tüm sinyalleri hesapla. Perspektifli sinyaller (stock, momentum,
/// vs.) `(city, product)` çiftine göre, perspektif-bağımsız sinyaller (cash,
/// arbitrage, urgency) tüm pazara göre.
#[must_use]
pub fn compute_inputs(
    state: &GameState,
    npc_id: PlayerId,
    city: CityId,
    product: ProductKind,
) -> Inputs {
    let mut inputs: Inputs = BTreeMap::new();

    // 1. Cash — NPC nakit normalize.
    let cash_norm = state.players.get(&npc_id).map_or(0.0, |p| {
        ((p.cash.as_cents() as f64 / 100.0) / TYPICAL_STARTING_CASH_LIRA).clamp(0.0, 1.0)
    });
    inputs.insert("cash", cash_norm);

    // 2. Stock — bu (city, product) için NPC stoğu.
    let stock_norm = state.players.get(&npc_id).map_or(0.0, |p| {
        let qty = p.inventory.get(city, product);
        (f64::from(qty) / TYPICAL_STOCK_CAPACITY).clamp(0.0, 1.0)
    });
    inputs.insert("stock", stock_norm);

    // 3. price_rel_avg — son fiyat / fair value, normalize.
    let ratio = price_ratio(state, city, product);
    let price_rel = (ratio / 2.0).clamp(0.0, 1.0);
    inputs.insert("price_rel_avg", price_rel);

    // 4. Momentum — `[-1, 1]` → `[0, 1]`.
    let mom = price_momentum(state, city, product);
    inputs.insert("momentum", f64::midpoint(mom, 1.0).clamp(0.0, 1.0));

    // 5. Urgency — sezon ilerlemesi.
    let total = state.config.season_ticks.max(1);
    let done = state.current_tick.value().min(total);
    let progress = (f64::from(done) / f64::from(total)).clamp(0.0, 1.0);
    inputs.insert("urgency", progress);

    // 6. Arbitrage — şehirler arası max fark.
    inputs.insert("arbitrage", arbitrage_signal(state, product).clamp(0.0, 1.0));

    // 7. Event — aktif şok.
    inputs.insert("event", event_signal(state, city, product));

    // 8. Competition — bu pazarda rakip emir baskısı.
    inputs.insert("competition", competition_signal(state, city, product));

    // 9-15. BID/ASK ratio + bankruptcy + counts + season_remaining + rival.
    let (bid_qty, ask_qty) = state
        .order_book
        .get(&(city, product))
        .map_or((0u32, 0u32), |orders| {
            orders.iter().fold((0, 0), |(b, a), o| match o.side {
                OrderSide::Buy => (b + o.quantity, a),
                OrderSide::Sell => (b, a + o.quantity),
            })
        });
    let total_qty = bid_qty + ask_qty;
    let bid_ratio = if total_qty == 0 {
        0.5
    } else {
        f64::from(bid_qty) / f64::from(total_qty)
    };
    inputs.insert("bid_supply_ratio", bid_ratio.clamp(0.0, 1.0));

    let cash_lira = state
        .players
        .get(&npc_id)
        .map_or(0.0, |p| (p.cash.as_cents() as f64) / 100.0);
    let bankruptcy = ((50_000.0 - cash_lira) / 45_000.0).clamp(0.0, 1.0);
    inputs.insert("bankruptcy_risk", bankruptcy);

    let factory_count = state
        .factories
        .values()
        .filter(|f| f.owner == npc_id)
        .count();
    inputs.insert(
        "factory_count",
        (factory_count as f64 / 3.0).clamp(0.0, 1.0),
    );

    let caravan_count = state
        .caravans
        .values()
        .filter(|c| c.owner == npc_id)
        .count();
    inputs.insert(
        "caravan_count",
        (caravan_count as f64 / 3.0).clamp(0.0, 1.0),
    );

    inputs.insert(
        "season_remaining",
        (1.0 - inputs.get("urgency").copied().unwrap_or(0.0)).clamp(0.0, 1.0),
    );

    let rival_count = state.order_book.get(&(city, product)).map_or(0, |orders| {
        let mut rivals: std::collections::BTreeSet<PlayerId> = std::collections::BTreeSet::new();
        for o in orders {
            if o.player != npc_id {
                rivals.insert(o.player);
            }
        }
        rivals.len()
    });
    inputs.insert(
        "rival_action_pressure",
        (rival_count as f64 / 5.0).clamp(0.0, 1.0),
    );

    let ask_pressure = if total_qty == 0 {
        0.0
    } else {
        f64::from(ask_qty) / f64::from(total_qty)
    };
    inputs.insert("ask_supply_ratio", ask_pressure.clamp(0.0, 1.0));

    // 16. Local raw advantage — bu (city, product) yerel uzmanlığa uyuyor mu?
    let local_advantage = match product.raw_input() {
        Some(raw) => f64::from(u8::from(city.cheap_raw() == raw)),
        None => f64::from(u8::from(city.cheap_raw() == product)),
    };
    inputs.insert("local_raw_advantage", local_advantage);

    inputs
}

// ============================================================================
// Helper sinyaller
// ============================================================================

/// Bir (şehir, ürün) için fiyat momentum'u — son N tick fiyat trendi.
/// `[-1.0, 1.0]`: pozitif = yükseliyor, negatif = düşüyor.
fn price_momentum(state: &GameState, city: CityId, product: ProductKind) -> f64 {
    let Some(history) = state.price_history.get(&(city, product)) else {
        return 0.0;
    };
    if history.len() < 2 {
        return 0.0;
    }
    let last_n: Vec<f64> = history
        .iter()
        .rev()
        .take(5)
        .map(|(_, p)| p.as_cents() as f64)
        .collect();
    if last_n.len() < 2 {
        return 0.0;
    }
    let first = last_n.last().copied().unwrap_or(0.0);
    let last = last_n.first().copied().unwrap_or(0.0);
    if first.abs() < f64::EPSILON {
        return 0.0;
    }
    ((last - first) / first).clamp(-1.0, 1.0)
}

/// Bir ürün için max-min şehir fiyat farkı `[0.0, 1.0]` normalize.
fn arbitrage_signal(state: &GameState, product: ProductKind) -> f64 {
    let mut prices: Vec<i64> = Vec::new();
    for city in CityId::ALL {
        if let Some(p) = state
            .rolling_avg_price(city, product, 5)
            .or_else(|| state.effective_baseline(city, product))
        {
            prices.push(p.as_cents());
        }
    }
    if prices.len() < 2 {
        return 0.0;
    }
    let min = *prices.iter().min().unwrap_or(&0);
    let max = *prices.iter().max().unwrap_or(&0);
    if min <= 0 {
        return 0.0;
    }
    let spread = ((max - min) as f64) / (min as f64);
    spread.clamp(0.0, 1.0)
}

/// `(city, product)` aktif şokunun mutlak yüzdesi.
fn event_signal(state: &GameState, city: CityId, product: ProductKind) -> f64 {
    state
        .active_shocks
        .get(&(city, product))
        .map_or(0.0, |s| f64::from(s.multiplier_pct.unsigned_abs()) / 100.0)
        .clamp(0.0, 1.0)
}

/// `current_price / fair_value` oranı. Centered around 1.0.
fn price_ratio(state: &GameState, city: CityId, product: ProductKind) -> f64 {
    let current = state
        .price_history
        .get(&(city, product))
        .and_then(|v| v.last())
        .map(|(_, p)| *p);
    let baseline = state.effective_baseline(city, product);
    match (current, baseline) {
        (Some(cur), Some(base)) if base.as_cents() > 0 => {
            (cur.as_cents() as f64) / (base.as_cents() as f64)
        }
        _ => 1.0,
    }
}

/// Bir şehirde aynı (city, product) bucket'ında SELL ya da BUY emir baskısı.
fn competition_signal(state: &GameState, city: CityId, product: ProductKind) -> f64 {
    let total: u32 = state
        .order_book
        .get(&(city, product))
        .map_or(0, |orders| orders.iter().map(|o| o.quantity).sum());
    (f64::from(total) / 200.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{Money, Player, PlayerId, Role, RoomConfig, RoomId};

    fn fresh_state() -> (GameState, PlayerId) {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(100),
            "Test",
            Role::Tuccar,
            Money::from_lira(20_000).unwrap(),
            true,
        )
        .unwrap();
        s.players.insert(npc.id, npc);
        (s, PlayerId::new(100))
    }

    #[test]
    fn cash_normalized_to_typical() {
        let (s, pid) = fresh_state();
        let inputs = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        assert!((inputs["cash"] - 1.0).abs() < 0.01);
    }

    #[test]
    fn empty_stock_returns_zero() {
        let (s, pid) = fresh_state();
        let inputs = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        assert_eq!(inputs["stock"], 0.0);
    }

    #[test]
    fn missing_player_returns_zeroed_inputs() {
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let inputs = compute_inputs(&s, PlayerId::new(999), CityId::Istanbul, ProductKind::Pamuk);
        assert_eq!(inputs["cash"], 0.0);
        assert_eq!(inputs["stock"], 0.0);
    }

    #[test]
    fn deterministic_for_same_state() {
        let (s, pid) = fresh_state();
        let i1 = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        let i2 = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        assert_eq!(i1, i2);
    }

    #[test]
    fn all_inputs_in_unit_range() {
        let (s, pid) = fresh_state();
        let inputs = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        for (name, value) in &inputs {
            assert!(
                (0.0..=1.0).contains(value),
                "input {name}={value} not in [0,1]"
            );
        }
    }

    #[test]
    fn local_raw_advantage_for_specialty_pair() {
        let (s, pid) = fresh_state();
        // İstanbul-Pamuk specialty
        let inputs_ist_pamuk = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        assert_eq!(inputs_ist_pamuk["local_raw_advantage"], 1.0);
        // İstanbul-Buğday non-specialty
        let inputs_ist_bugday = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Bugday);
        assert_eq!(inputs_ist_bugday["local_raw_advantage"], 0.0);
    }
}
