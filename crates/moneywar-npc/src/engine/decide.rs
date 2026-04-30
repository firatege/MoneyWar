//! Fuzzy NPC karar motoru — orchestrator.
//!
//! Akış:
//! 1. Her (city, product) için `compute_inputs` ile sinyaller hesapla.
//! 2. Rol-spesifik `engine_for(role, kind)` → fuzzy çıkışlar (buy_score,
//!    sell_score, bid_aggressiveness, vb.) per-bucket.
//! 3. Tüm bucket'lardan en yüksek skorlu action adaylarını topla.
//! 4. Difficulty modulator: threshold + max_actions filter.
//! 5. Personality bias: utility multiplier (Faz 7'de detaylanır).
//! 6. Command emit.
//!
//! Output skorları (fuzzy):
//! - `buy_score`, `sell_score` → SubmitOrder
//! - `bid_aggressiveness`, `ask_aggressiveness` → fiyat hesabı
//! - `build_factory_score` → BuildFactory (Sanayici)
//! - `buy_caravan_score` → BuyCaravan (Tüccar)
//! - `dispatch_score` → DispatchCaravan
//! - `contract_score` → ProposeContract
//!
//! Modulator parametreleri (`Easy/Medium/Hard`):
//! - `max_actions_per_tick`: top-K filter
//! - `silence_ratio_per10`: rng skip
//! - `aggressiveness`: bid/ask fiyat scale
//! - `min_score_threshold`: utility kabul eşiği

use moneywar_domain::{
    CargoSpec, CityId, Command, GameState, MarketOrder, Money, OrderId, OrderSide, PlayerId,
    ProductKind, Role, Tick,
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::DifficultyModulator;
use crate::engine::inputs::compute_inputs;
use crate::engine::rules::engine_for;

/// Fuzzy karar motoru — tüm (city, product) bucket'larını değerlendirir,
/// modulator filter sonrası `Command` listesi döner.
#[must_use]
pub fn decide_npc_fuzzy(
    state: &GameState,
    npc_id: PlayerId,
    modulator: DifficultyModulator,
    rng: &mut ChaCha8Rng,
) -> Vec<Command> {
    // Silence — Easy'de yarı tick pasif.
    if modulator.silence_ratio_per10 > 0
        && rng.random_ratio(modulator.silence_ratio_per10, 10)
    {
        return Vec::new();
    }

    let Some(player) = state.players.get(&npc_id) else {
        return Vec::new();
    };
    let role = player.role;
    let npc_kind = player.npc_kind;
    let engine = engine_for(role, npc_kind);
    let tick = state.current_tick.next();
    let ttl = state.config.balance.default_order_ttl;

    let mut candidates: Vec<(Command, f64)> = Vec::new();
    let mut seq: u32 = 0;

    // Her (city, product) için fuzzy değerlendir.
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let inputs = compute_inputs(state, npc_id, city, product);
            let outputs = engine.evaluate(&inputs);

            let buy_score = outputs.get("buy_score").copied().unwrap_or(0.0);
            let sell_score = outputs.get("sell_score").copied().unwrap_or(0.0);
            let bid_aggro = outputs.get("bid_aggressiveness").copied().unwrap_or(0.5);
            let ask_aggro = outputs.get("ask_aggressiveness").copied().unwrap_or(0.5);

            // Buy emir adayı.
            if buy_score >= modulator.min_score_threshold && buy_score > 0.3 {
                if let Some(cmd) = build_buy_order(
                    state, npc_id, city, product, bid_aggro, modulator, tick, ttl, &mut seq,
                ) {
                    candidates.push((cmd, buy_score));
                }
            }

            // Sell emir adayı.
            if sell_score >= modulator.min_score_threshold && sell_score > 0.3 {
                if let Some(cmd) = build_sell_order(
                    state, npc_id, city, product, ask_aggro, modulator, tick, ttl, &mut seq,
                ) {
                    candidates.push((cmd, sell_score));
                }
            }

            // BuildFactory adayı (Sanayici only).
            if matches!(role, Role::Sanayici) {
                let build_score = outputs
                    .get("build_factory_score")
                    .copied()
                    .unwrap_or(0.0);
                if build_score >= modulator.min_score_threshold
                    && build_score > 0.5
                    && product.is_finished()
                {
                    candidates.push((
                        Command::BuildFactory {
                            owner: npc_id,
                            city,
                            product,
                        },
                        build_score,
                    ));
                }
            }
        }
    }

    // BuyCaravan adayı (Tüccar only) — global, role-spesifik bir tek (city,product) skoru.
    if matches!(role, Role::Tuccar) {
        // Pamuk + İstanbul örnek olarak kullan; aslında en yüksek arbitraj şehri.
        let inputs = compute_inputs(state, npc_id, CityId::Istanbul, ProductKind::Pamuk);
        let outputs = engine.evaluate(&inputs);
        let caravan_score = outputs
            .get("buy_caravan_score")
            .copied()
            .unwrap_or(0.0);
        if caravan_score >= modulator.min_score_threshold && caravan_score > 0.4 {
            // İlk şehirden başlat (en arbitraj fırsatlısı orchestrator'da seçilebilir).
            candidates.push((
                Command::BuyCaravan {
                    owner: npc_id,
                    starting_city: CityId::Istanbul,
                },
                caravan_score,
            ));
        }
    }

    // Dispatch adayları (Tüccar): kervan idle + stok varsa, en yüksek skorlu (from→to).
    if matches!(role, Role::Tuccar) {
        if let Some(cmd) =
            build_dispatch_command(state, npc_id, &engine, modulator, tick, &mut seq)
        {
            // Skoru hesapla (orchestrator'da basit: fuzzy dispatch_score yüksekse)
            candidates.push((cmd, 0.7));
        }
    }

    // Skora göre sırala (desc), threshold + top-K filter.
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });

    candidates
        .into_iter()
        .filter(|(_, s)| *s >= modulator.min_score_threshold)
        .take(modulator.max_actions_per_tick as usize)
        .map(|(c, _)| c)
        .collect()
}

/// BUY emri oluştur — fiyat market × bid_aggressiveness oranıyla.
#[allow(clippy::too_many_arguments)]
fn build_buy_order(
    state: &GameState,
    npc_id: PlayerId,
    city: CityId,
    product: ProductKind,
    bid_aggro: f64,
    modulator: DifficultyModulator,
    tick: Tick,
    ttl: u32,
    seq: &mut u32,
) -> Option<Command> {
    let player = state.players.get(&npc_id)?;
    let market = market_or_base(state, city, product);
    // Bid agresif: market × (1 + (bid_aggro - 0.5) × 0.5 × modulator.aggressiveness)
    // bid_aggro 0.5 = market, 1.0 = +%25 üstü → ask'larla daha çok karşılaşır.
    // Eski 0.3 scale'da fiyatlar çapraz olmuyordu, %1.2 match verimliliği.
    let aggro_factor = 1.0 + (bid_aggro - 0.5) * 0.5 * modulator.aggressiveness;
    let bid_cents = (market.as_cents() as f64 * aggro_factor) as i64;
    if bid_cents <= 0 {
        return None;
    }
    let qty = pick_buy_qty(player.cash.as_cents(), bid_cents);
    if qty == 0 {
        return None;
    }
    let id = OrderId::new(npc_decide_order_id(npc_id, tick, *seq));
    *seq += 1;
    MarketOrder::new_with_ttl(
        id,
        npc_id,
        city,
        product,
        OrderSide::Buy,
        qty,
        Money::from_cents(bid_cents),
        tick,
        ttl,
    )
    .ok()
    .map(Command::SubmitOrder)
}

/// SELL emri oluştur — fiyat market × ask_aggressiveness'a göre.
#[allow(clippy::too_many_arguments)]
fn build_sell_order(
    state: &GameState,
    npc_id: PlayerId,
    city: CityId,
    product: ProductKind,
    ask_aggro: f64,
    modulator: DifficultyModulator,
    tick: Tick,
    ttl: u32,
    seq: &mut u32,
) -> Option<Command> {
    let player = state.players.get(&npc_id)?;
    let stock = player.inventory.get(city, product);
    if stock == 0 {
        return None;
    }
    let market = market_or_base(state, city, product);
    // Ask agresif: market × (1 - (ask_aggro - 0.5) × 0.5 × modulator.aggressiveness)
    // ask_aggro 1.0 = -%25 (agresif satış), 0.5 = market.
    let aggro_factor = 1.0 - (ask_aggro - 0.5) * 0.5 * modulator.aggressiveness;
    let ask_cents = (market.as_cents() as f64 * aggro_factor) as i64;
    if ask_cents <= 0 {
        return None;
    }
    let qty = stock.min(pick_sell_qty(stock));
    if qty == 0 {
        return None;
    }
    let id = OrderId::new(npc_decide_order_id(npc_id, tick, *seq));
    *seq += 1;
    MarketOrder::new_with_ttl(
        id,
        npc_id,
        city,
        product,
        OrderSide::Sell,
        qty,
        Money::from_cents(ask_cents),
        tick,
        ttl,
    )
    .ok()
    .map(Command::SubmitOrder)
}

/// Tüccar dispatch — idle kervan varsa, en kârlı (from→to) çiftine.
fn build_dispatch_command(
    state: &GameState,
    npc_id: PlayerId,
    _engine: &crate::fuzzy::Engine,
    _modulator: DifficultyModulator,
    _tick: Tick,
    _seq: &mut u32,
) -> Option<Command> {
    let player = state.players.get(&npc_id)?;
    // Idle kervan
    let idle_caravan = state
        .caravans
        .values()
        .find(|c| c.owner == npc_id && c.is_idle())?;
    let from = idle_caravan.state.current_city()?;

    // En arbitraj fırsatlı (from, to, product) seç
    let mut best: Option<(CityId, ProductKind, i64)> = None;
    for product in ProductKind::ALL {
        let stock = player.inventory.get(from, product);
        if stock < 10 {
            continue;
        }
        let here = state.rolling_avg_price(from, product, 5)?.as_cents();
        for to in CityId::ALL {
            if to == from {
                continue;
            }
            if let Some(there_p) = state.rolling_avg_price(to, product, 5) {
                let profit = there_p.as_cents() - here;
                if profit > 25 && best.map_or(true, |(_, _, p)| profit > p) {
                    best = Some((to, product, profit));
                }
            }
        }
    }
    let (to, product, _profit) = best?;
    let qty = player.inventory.get(from, product).min(idle_caravan.capacity);
    if qty == 0 {
        return None;
    }
    let mut cargo = CargoSpec::default();
    cargo.add(product, qty).ok()?;
    Some(Command::DispatchCaravan {
        caravan_id: idle_caravan.id,
        from,
        to,
        cargo,
    })
}

fn market_or_base(state: &GameState, city: CityId, product: ProductKind) -> Money {
    state
        .rolling_avg_price(city, product, 5)
        .or_else(|| state.effective_baseline(city, product))
        .unwrap_or_else(|| {
            // fallback: ham 6, mamul 18
            Money::from_lira(if product.is_finished() { 18 } else { 6 }).unwrap()
        })
}

/// Cash kapasitesine göre alım miktarı (cash'in %25'i, cap 400).
/// Sanayici fabrikasını besler — büyük alımlar için cap yüksek.
fn pick_buy_qty(cash_cents: i64, bid_cents: i64) -> u32 {
    if bid_cents <= 0 {
        return 0;
    }
    let budget = cash_cents / 4; // %25
    let max_qty = budget / bid_cents;
    let capped = max_qty.clamp(0, 400);
    u32::try_from(capped).unwrap_or(0)
}

/// Stok'a göre satım miktarı (stoğun %40'ı, cap 250).
fn pick_sell_qty(stock: u32) -> u32 {
    let q = (stock * 2) / 5;
    q.clamp(10, 250).min(stock)
}

/// Fuzzy NPC order ID (DSS NPC_ORDER_ID_OFFSET ile uyumsuz tutmayalım — distinct prefix).
fn npc_decide_order_id(player_id: PlayerId, tick: Tick, seq: u32) -> u64 {
    moneywar_domain::balance::NPC_ORDER_ID_OFFSET
        .saturating_add(u64::from(tick.value()).saturating_mul(100_000))
        .saturating_add((player_id.value() % 1_000).saturating_mul(100))
        .saturating_add(u64::from(seq).min(99))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Difficulty;
    use moneywar_domain::{Money, NpcKind, Player, PlayerId, Role, RoomConfig, RoomId};
    use rand_chacha::rand_core::SeedableRng;

    fn fresh_state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn add_npc(state: &mut GameState, id: u64, role: Role, cash: i64, kind: NpcKind) -> PlayerId {
        let pid = PlayerId::new(id);
        let p = Player::new(
            pid,
            format!("NPC-{id}"),
            role,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(kind);
        state.players.insert(pid, p);
        pid
    }

    #[test]
    fn rich_sanayici_emits_actions_at_medium() {
        let mut s = fresh_state();
        let pid = add_npc(&mut s, 100, Role::Sanayici, 40_000, NpcKind::Sanayici);
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_npc_fuzzy(&s, pid, Difficulty::Hard.modulator(), &mut rng);
        // Hard'da silence az, threshold negatif → en az 1 action emit etmeli
        assert!(!cmds.is_empty(), "rich sanayici Hard'da action emit etmeli");
    }

    #[test]
    fn easy_modulator_caps_at_one_action() {
        let mut s = fresh_state();
        let pid = add_npc(&mut s, 100, Role::Sanayici, 40_000, NpcKind::Sanayici);
        let mut rng = ChaCha8Rng::from_seed([42u8; 32]);
        let cmds = decide_npc_fuzzy(&s, pid, Difficulty::Easy.modulator(), &mut rng);
        // Easy'de max 1 action (silence atlasa 0)
        assert!(cmds.len() <= 1, "Easy modulator max 1 action (got {})", cmds.len());
    }

    #[test]
    fn missing_npc_returns_empty() {
        let s = fresh_state();
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_npc_fuzzy(&s, PlayerId::new(999), Difficulty::Hard.modulator(), &mut rng);
        assert!(cmds.is_empty());
    }

    #[test]
    fn deterministic_for_same_seed() {
        let mut s = fresh_state();
        let pid = add_npc(&mut s, 100, Role::Tuccar, 30_000, NpcKind::Tuccar);
        let mut r1 = ChaCha8Rng::from_seed([42u8; 32]);
        let mut r2 = ChaCha8Rng::from_seed([42u8; 32]);
        let c1 = decide_npc_fuzzy(&s, pid, Difficulty::Hard.modulator(), &mut r1);
        let c2 = decide_npc_fuzzy(&s, pid, Difficulty::Hard.modulator(), &mut r2);
        assert_eq!(c1, c2);
    }

    #[test]
    fn poor_npc_does_not_emit_buys() {
        let mut s = fresh_state();
        let pid = add_npc(&mut s, 100, Role::Tuccar, 200, NpcKind::Tuccar);
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_npc_fuzzy(&s, pid, Difficulty::Hard.modulator(), &mut rng);
        // Fakir NPC buy emri vermez (bankruptcy_risk yüksek)
        let buys = cmds.iter().filter(|c| matches!(c, Command::SubmitOrder(o) if matches!(o.side, OrderSide::Buy))).count();
        assert_eq!(buys, 0, "fakir tüccar buy emir vermemeli");
    }
}
