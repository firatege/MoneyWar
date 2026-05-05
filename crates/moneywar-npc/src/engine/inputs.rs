//! Fuzzy/DSS karar motoru için **normalize NPC girdileri**.
//!
//! Tek giriş noktası: [`compute_inputs`]. Bir NPC'nin belirli `(city, product)`
//! için tüm sinyallerini `[0.0, 1.0]` aralığında üretir; fuzzy `Engine.evaluate()`
//! direkt bu map'i alır.
//!
//! Mevcut DSS `inputs.rs` modülünden reuse + fuzzy normalize katmanı.
//! 8 ana sinyal:
//! - `cash`: NPC nakdinin tipik başlangıç ölçeğine göre normalize
//! - `stock`: bu `(city, product)` için NPC stoğu / kapasite proxy
//! - `price_rel_avg`: son fiyat / 5-tick avg (1.0 etrafı)
//! - `momentum`: fiyat eğimi `[0,1]` (negatif yönü yarımdan az değer)
//! - `urgency`: sezon ne kadar ilerledi (0=başlangıç, 1=son)
//! - `arbitrage`: şehirler arası max fark
//! - `event`: aktif şok şiddeti
//! - `competition`: bu pazarda rakip emir baskısı

use std::collections::BTreeMap;

use moneywar_domain::{CityId, GameState, PlayerId, ProductKind};

use crate::dss::inputs::{
    arbitrage_signal, competition_signal, event_signal, price_momentum, price_ratio,
};
use crate::fuzzy::Inputs;

/// "Tipik" başlangıç nakdi — Sanayici 30K, Tüccar 15K, Esnaf 10K. Normalize
/// referansı olarak ortalama 20K kullanıyoruz. Fuzzy `cash` değişkeni tipik
/// olarak `[0,1]`: 0 = bos, 1 = 2x+ ortalama nakit.
const TYPICAL_STARTING_CASH_LIRA: f64 = 20_000.0;

/// Stok normalize edici — `100 birim` referansı. Hesap: `qty / 100` clamp 0..1.
/// Fuzzy `stock` değişkeni: 0 = bos, 1 = 100+ birim.
const TYPICAL_STOCK_CAPACITY: f64 = 100.0;

/// Bir NPC için tüm fuzzy sinyallerini hesapla. `city`/`product` perspektifli
/// sinyaller (stok, momentum, vb.) bu çift için, perspektif-bağımsız sinyaller
/// (cash, arbitrage) tüm pazarı taraf — aynı map içinde.
#[must_use]
pub fn compute_inputs(
    state: &GameState,
    npc_id: PlayerId,
    city: CityId,
    product: ProductKind,
) -> Inputs {
    let mut inputs: BTreeMap<&'static str, f64> = BTreeMap::new();

    // 1. Cash — NPC nakit normalize.
    let cash_norm = state
        .players
        .get(&npc_id)
        .map_or(0.0, |p| ((p.cash.as_cents() as f64 / 100.0) / TYPICAL_STARTING_CASH_LIRA).clamp(0.0, 1.0));
    inputs.insert("cash", cash_norm);

    // 2. Stock — bu (city, product) için NPC stoğu.
    let stock_norm = state
        .players
        .get(&npc_id)
        .map_or(0.0, |p| {
            let qty = p.inventory.get(city, product);
            (f64::from(qty) / TYPICAL_STOCK_CAPACITY).clamp(0.0, 1.0)
        });
    inputs.insert("stock", stock_norm);

    // 3. price_rel_avg — son fiyat / fair value. Centered around 1.0.
    // Fuzzy için 0..1 normalize: 0.5 = adil, < 0.5 ucuz, > 0.5 pahalı.
    let ratio = price_ratio(state, city, product);
    let price_rel = (ratio / 2.0).clamp(0.0, 1.0); // 0=ucuz, 0.5=adil, 1=pahalı
    inputs.insert("price_rel_avg", price_rel);

    // 4. Momentum — `[-1, 1]` → fuzzy `[0, 1]`: 0=düşüyor, 0.5=sabit, 1=yükseliyor.
    let mom = price_momentum(state, city, product);
    inputs.insert("momentum", f64::midpoint(mom, 1.0).clamp(0.0, 1.0));

    // 5. Urgency — sezon ilerlemesi (0 = başlangıç, 1 = son tick).
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

    // 9. Bid/supply ratio — bu (city, product) için talep/arz dengesi.
    // 0 = hiç bid yok (mal alıcısız kalır), 0.5 = denge, 1 = bid çok yüksek.
    let (bid_qty, ask_qty) = state
        .order_book
        .get(&(city, product))
        .map_or((0, 0), |orders| {
            orders.iter().fold((0u32, 0u32), |(b, a), o| match o.side {
                moneywar_domain::OrderSide::Buy => (b + o.quantity, a),
                moneywar_domain::OrderSide::Sell => (b, a + o.quantity),
            })
        });
    let bid_ratio = if bid_qty + ask_qty == 0 {
        0.5
    } else {
        f64::from(bid_qty) / f64::from(bid_qty + ask_qty)
    };
    inputs.insert("bid_supply_ratio", bid_ratio.clamp(0.0, 1.0));

    // 10. İflas riski — cash / sezon-kalan tahmini gider.
    // Basit hesap: cash < 5K → kritik (1.0), cash > 50K → güvende (0.0).
    let cash_lira = state
        .players
        .get(&npc_id)
        .map_or(0.0, |p| (p.cash.as_cents() as f64) / 100.0);
    let bankruptcy = ((50_000.0 - cash_lira) / 45_000.0).clamp(0.0, 1.0);
    inputs.insert("bankruptcy_risk", bankruptcy);

    // 11. Fabrika sayısı (Sanayici özel) — 0=yok, 1=1 fab, 1.0=3+ fab.
    let factory_count = state
        .factories
        .values()
        .filter(|f| f.owner == npc_id)
        .count();
    inputs.insert("factory_count", (factory_count as f64 / 3.0).clamp(0.0, 1.0));

    // 12. Kervan sayısı (Tüccar özel) — 0=yok, 1=1, 1.0=3+ kervan.
    let caravan_count = state
        .caravans
        .values()
        .filter(|c| c.owner == npc_id)
        .count();
    inputs.insert("caravan_count", (caravan_count as f64 / 3.0).clamp(0.0, 1.0));

    // 13. Sezon kalan oranı (urgency'nin tersi). 1=sezon başı, 0=sezon sonu.
    inputs.insert(
        "season_remaining",
        (1.0 - inputs.get("urgency").copied().unwrap_or(0.0)).clamp(0.0, 1.0),
    );

    // 14. Rival action pressure (Plan v5) — bu (city, product) için bu NPC
    // **dışındaki** kaç farklı oyuncunun açık emri var. 0 = yalnız, 1 = 5+
    // farklı rakip. NPC bandwagon ve coalition davranışını besler.
    let rival_count = state
        .order_book
        .get(&(city, product))
        .map_or(0, |orders| {
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

    // 15. Ask supply ratio — arz baskısı (1 - bid_supply_ratio benzeri ama saf
    // ask hacmi). 0 = arz yok, 1 = arz piyasayı bastırıyor (ucuz alım fırsatı).
    let ask_pressure = if bid_qty + ask_qty == 0 {
        0.0
    } else {
        f64::from(ask_qty) / f64::from(bid_qty + ask_qty)
    };
    inputs.insert("ask_supply_ratio", ask_pressure.clamp(0.0, 1.0));

    // 16. Local raw advantage — bu (city, product) yerel uzmanlığa uyuyor mu?
    //   * Raw için: bu şehir'in cheap_raw'ı bu raw mı? (Istanbul=Pamuk, ...)
    //   * Mamul için: raw_input bu şehir'in cheap_raw'ı mı? (Kumas → Pamuk → Istanbul)
    //   * Hiçbiri değilse 0.0.
    // Sanayici fabrika kurma + Esnaf ham alım kararlarını şehir-spesifik yapan
    // ana sinyal. Tie-break + uzmanlaşma için kritik.
    let local_advantage = match product.raw_input() {
        Some(raw) => {
            // Mamul: raw_input bu şehir'in specialty'si mi?
            if city.cheap_raw() == raw { 1.0 } else { 0.0 }
        }
        None => {
            // Ham: bu şehir'in cheap_raw'ı mı?
            if city.cheap_raw() == product { 1.0 } else { 0.0 }
        }
    };
    inputs.insert("local_raw_advantage", local_advantage);

    inputs
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{Money, Player, Role, RoomConfig, RoomId, Tick};

    fn fresh_state() -> (GameState, PlayerId) {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(100),
            "TestNpc",
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
        // 20K cash = exactly 1x typical → 1.0
        assert!((inputs["cash"] - 1.0).abs() < 0.01);
    }

    #[test]
    fn empty_stock_returns_zero() {
        let (s, pid) = fresh_state();
        let inputs = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        assert_eq!(inputs["stock"], 0.0);
    }

    #[test]
    fn full_stock_caps_at_one() {
        let (mut s, pid) = fresh_state();
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 250)
            .unwrap();
        let inputs = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        assert_eq!(inputs["stock"], 1.0);
    }

    #[test]
    fn missing_player_returns_zeroed_inputs() {
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let inputs = compute_inputs(&s, PlayerId::new(999), CityId::Istanbul, ProductKind::Pamuk);
        assert_eq!(inputs["cash"], 0.0);
        assert_eq!(inputs["stock"], 0.0);
    }

    #[test]
    fn momentum_neutral_when_no_history() {
        let (s, pid) = fresh_state();
        let inputs = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        // (-1..1) → /2+0.5 → no history = 0 → 0.5
        assert!((inputs["momentum"] - 0.5).abs() < 0.01);
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
    fn urgency_advances_with_tick() {
        let (mut s, pid) = fresh_state();
        s.current_tick = Tick::new(s.config.season_ticks / 2);
        let inputs = compute_inputs(&s, pid, CityId::Istanbul, ProductKind::Pamuk);
        // Yarı sezon → urgency 0.5 civarı
        assert!((inputs["urgency"] - 0.5).abs() < 0.05);
    }
}
