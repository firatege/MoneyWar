//! Sanayici DSS — fabrika kur, ham al, üret, sat döngüsü.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::single_match_else,
    clippy::too_many_lines
)]

//!
//! Aksiyon adayları enumerate edilir:
//! - `BuildFactory` — fabrikası yoksa ya da <3 ise (her şehir × her finished)
//! - `SubmitOrder Buy` — fabrikalarının ham maddesi için
//! - `SubmitOrder Sell` — bitmiş ürünleri sat
//!
//! Her adaya kişilik ağırlıklı utility skoru. Top-K seçilir.

use moneywar_domain::{
    CityId, Command, Factory, GameState, MarketOrder, Money, OrderId, OrderSide, PlayerId,
    ProductKind, Tick,
};
use rand_chacha::ChaCha8Rng;

use crate::dss::inputs::{
    arbitrage_signal, cluster_signal, competition_signal, event_signal, human_lead_ratio,
    money_lira, pending_event_signal, price_momentum, price_ratio,
};
use crate::dss::weights_for;
use moneywar_domain::Personality;
use crate::dss::utility::{ActionCandidate, score_action};

const TOP_K: usize = 3;

/// Sanayici DSS karar fonksiyonu — top-K aksiyonu komut olarak döner.
#[must_use]
pub fn decide_sanayici_dss(
    state: &GameState,
    pid: PlayerId,
    personality: Personality,
    _rng: &mut ChaCha8Rng,
    tick: Tick,
) -> Vec<Command> {
    let Some(player) = state.players.get(&pid) else {
        return Vec::new();
    };
    let weights = weights_for(personality);
    let ttl = state.config.balance.default_order_ttl;

    let factory_count = state
        .factories
        .values()
        .filter(|f| f.owner == pid)
        .count();

    let mut scored: Vec<(Command, f64)> = Vec::new();
    let mut seq: u32 = 0;

    // 1. BuildFactory adayları — fabrika sayısı < 3 ise
    if factory_count < 3 {
        for city in CityId::ALL {
            for product in ProductKind::FINISHED_GOODS {
                let cost = Factory::build_cost(u32::try_from(factory_count).unwrap_or(0));
                if player.cash < cost {
                    continue;
                }
                let Some(raw) = product.raw_input() else {
                    continue;
                };
                let raw_price = state
                    .effective_baseline(city, raw)
                    .unwrap_or(Money::ZERO);
                let fin_price = state
                    .effective_baseline(city, product)
                    .unwrap_or(Money::ZERO);
                let margin = money_lira(fin_price) - money_lira(raw_price);
                // Beklenen kâr — sezonun kalanında ~10 batch × margin × 10 birim
                let remaining_ticks = state
                    .config
                    .season_ticks
                    .saturating_sub(tick.value());
                let est_batches = (remaining_ticks / 4) as f64; // ~4 tick/batch
                let expected_profit = margin * est_batches * 10.0;
                let action = ActionCandidate {
                    profit_lira: expected_profit,
                    capital_lira: money_lira(cost),
                    risk: 0.4 + competition_signal(state, city, product) * 0.4,
                    urgency: 0.0, // Kuruluş aciliyet hissetmez
                    momentum: price_momentum(state, city, product),
                    arbitrage: arbitrage_signal(state, product),
                    event: (event_signal(state, city, product)
    + pending_event_signal(state, pid, city, product))
    .min(1.0),
                    hold_pressure: 0.7, // fabrika uzun-vadeli stoklar
                };
                let score = score_action(action, weights);
                scored.push((
                    Command::BuildFactory {
                        owner: pid,
                        city,
                        product,
                    },
                    score,
                ));
            }
        }
    }

    // 2. SubmitOrder Buy adayları — fabrikalarının ham maddesi
    let my_factories: Vec<&Factory> = state
        .factories
        .values()
        .filter(|f| f.owner == pid)
        .collect();
    for factory in &my_factories {
        let raw = factory.raw_input();
        let have = player.inventory.get(factory.city, raw);
        if have >= 50 {
            continue; // yeterli stok
        }
        let market = state
            .price_history
            .get(&(factory.city, raw))
            .and_then(|v| v.last())
            .map(|(_, p)| *p)
            .or_else(|| state.effective_baseline(factory.city, raw))
            .unwrap_or(Money::from_cents(600));
        let bid_cents = (market.as_cents() * 105) / 100;
        let qty: u32 = 20;
        let total = bid_cents.saturating_mul(i64::from(qty));
        if player.cash.as_cents() < total {
            continue;
        }
        // Ham alımı için utility — düşük profit (gerekli iş), düşük risk
        let action = ActionCandidate {
            profit_lira: 0.0, // direct kâr yok
            capital_lira: money_lira(Money::from_cents(total)),
            risk: 0.3,
            urgency: if have < 10 { 0.9 } else { 0.5 },
            momentum: price_momentum(state, factory.city, raw),
            arbitrage: arbitrage_signal(state, raw),
            event: (event_signal(state, factory.city, raw)
    + pending_event_signal(state, pid, factory.city, raw))
    .min(1.0),
            hold_pressure: 0.4,
        };
        let score = score_action(action, weights);
        let id = OrderId::new(npc_dss_order_id(pid, tick, seq));
        seq += 1;
        if let Ok(o) = MarketOrder::new_with_ttl(
            id,
            pid,
            factory.city,
            raw,
            OrderSide::Buy,
            qty,
            Money::from_cents(bid_cents),
            tick,
            ttl,
        ) {
            scored.push((Command::SubmitOrder(o), score));
        }
    }

    // 3. SubmitOrder Sell adayları — bitmiş ürün stoğu varsa
    let entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, p, q)| p.is_finished() && *q > 0)
        .collect();
    for (city, product, qty) in entries {
        let market = state
            .price_history
            .get(&(city, product))
            .and_then(|v| v.last())
            .map(|(_, p)| *p)
            .or_else(|| state.effective_baseline(city, product))
            .unwrap_or(Money::from_cents(1500));
        let ask_cents = (market.as_cents() * 95) / 100;
        let sell_qty = qty.min(15);
        // Ratio ile profit tahmini: sat fiyatı × qty
        let expected_revenue = (ask_cents as f64 / 100.0) * f64::from(sell_qty);
        // Fair price'tan farkla utility
        let ratio = price_ratio(state, city, product);
        let action = ActionCandidate {
            profit_lira: expected_revenue * (ratio - 1.0).max(0.0),
            capital_lira: 0.0, // satışta sermaye taahhüdü yok (stok zaten sahip)
            risk: 0.3 + competition_signal(state, city, product) * 0.4,
            urgency: 0.5,
            momentum: price_momentum(state, city, product),
            arbitrage: arbitrage_signal(state, product),
            event: (event_signal(state, city, product)
    + pending_event_signal(state, pid, city, product))
    .min(1.0),
            hold_pressure: 0.0, // satınca rahatlama
        };
        let score = score_action(action, weights);
        let id = OrderId::new(npc_dss_order_id(pid, tick, seq));
        seq += 1;
        if let Ok(o) = MarketOrder::new_with_ttl(
            id,
            pid,
            city,
            product,
            OrderSide::Sell,
            sell_qty,
            Money::from_cents(ask_cents),
            tick,
            ttl,
        ) {
            scored.push((Command::SubmitOrder(o), score));
        }
    }

    // Adaptive difficulty: insan lider ise (>1.5×) NPC'ler agresifleşir.
    let human_id = find_human(state);
    let lead_boost = human_id
        .map(|hid| (human_lead_ratio(state, hid) - 1.0).clamp(0.0, 2.0))
        .unwrap_or(0.0);

    // Bandwagon (cluster): post-process scored — aynı arketipli NPC'lerin
    // aktif emirlerine bias ekle.
    for (cmd, score) in scored.iter_mut() {
        // Adaptive boost
        *score *= 1.0 + lead_boost * 0.3;
        // Cluster signal — aksiyonun (city, product)'unu çıkar
        if let Some((city, product)) = cmd_target_bucket(cmd) {
            let cs = cluster_signal(state, pid, personality, city, product);
            *score *= 1.0 + cs * 0.15;
        }
    }

    // Top-K sırala (skor desc, deterministik tie-break Cmd debug-strı ile)
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });
    scored.into_iter().take(TOP_K).map(|(c, _)| c).collect()
}

/// State'te insan oyuncuyu bulur — `is_npc=false`. `decide_all_npcs`
/// adaptive difficulty hesabı için ihtiyaç duyar.
fn find_human(state: &GameState) -> Option<PlayerId> {
    state
        .players
        .iter()
        .find(|(_, p)| !p.is_npc)
        .map(|(id, _)| *id)
}

/// Komutun hedef (city, product)'u — cluster signal için.
fn cmd_target_bucket(cmd: &Command) -> Option<(CityId, ProductKind)> {
    match cmd {
        Command::SubmitOrder(order) => Some((order.city, order.product)),
        Command::BuildFactory { city, product, .. } => Some((*city, *product)),
        _ => None,
    }
}

/// NPC DSS order ID — `NPC_ORDER_ID_OFFSET + (tick × 100k) + (pid × 100) + seq`.
/// `decide_sanayici/tuccar`'ın ID üretimi ile uyumlu — collision yok.
fn npc_dss_order_id(player_id: PlayerId, tick: Tick, seq: u32) -> u64 {
    moneywar_domain::balance::NPC_ORDER_ID_OFFSET
        .saturating_add(u64::from(tick.value()).saturating_mul(100_000))
        .saturating_add((player_id.value() % 1_000).saturating_mul(100))
        .saturating_add(u64::from(seq).min(99))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{Player, PlayerId, Role, RoomConfig, RoomId};
    use rand_chacha::rand_core::SeedableRng;

    fn fresh_state() -> (GameState, PlayerId) {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(100),
            "TestSanayici",
            Role::Sanayici,
            Money::from_lira(50_000).unwrap(),
            true,
        )
        .unwrap();
        s.players.insert(npc.id, npc);
        (s, PlayerId::new(100))
    }

    #[test]
    fn aggressive_sanayici_with_no_factory_builds() {
        let (s, pid) = fresh_state();
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_sanayici_dss(&s, pid, Personality::Aggressive, &mut rng, Tick::new(1));
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::BuildFactory { .. })),
            "fabrika yoksa kurma adayı top-K'da bulunmalı"
        );
    }

    #[test]
    fn empty_state_returns_no_commands() {
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds =
            decide_sanayici_dss(&s, PlayerId::new(999), Personality::Aggressive, &mut rng, Tick::new(1));
        assert!(cmds.is_empty());
    }

    #[test]
    fn deterministic_for_same_seed() {
        let (s, pid) = fresh_state();
        let mut r1 = ChaCha8Rng::from_seed([42u8; 32]);
        let mut r2 = ChaCha8Rng::from_seed([42u8; 32]);
        let c1 = decide_sanayici_dss(&s, pid, Personality::Arbitrageur, &mut r1, Tick::new(1));
        let c2 = decide_sanayici_dss(&s, pid, Personality::Arbitrageur, &mut r2, Tick::new(1));
        assert_eq!(c1, c2);
    }
}
