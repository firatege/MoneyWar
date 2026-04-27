//! Tüccar DSS — kervan + arbitraj döngüsü.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::single_match_else,
    clippy::too_many_lines
)]

//!
//! Aksiyon adayları:
//! - `BuyCaravan` — kervansı yoksa (ya da kişilik agresifse < limit)
//! - `DispatchCaravan` — idle kervan × hedef şehir × en iyi cargo
//! - `SubmitOrder` — yerel BUY (taşımak için stoklamak) ya da SELL (varış sonrası)

use moneywar_domain::{
    Caravan, CargoSpec, CityId, Command, GameState, MarketOrder, Money, OrderId, OrderSide,
    PlayerId, ProductKind, Role, Tick,
};
use rand_chacha::ChaCha8Rng;

use crate::dss::contract::{accept_contract_candidates, propose_contract_candidates};
use crate::dss::inputs::{
    arbitrage_signal, cluster_signal, competition_signal, event_signal, human_lead_ratio,
    money_lira, pending_event_signal, price_momentum,
};
use crate::dss::utility::{ActionCandidate, score_action};
use crate::dss::weights_for;
use moneywar_domain::Personality;

const TOP_K: usize = 3;

#[must_use]
pub fn decide_tuccar_dss(
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

    let mut scored: Vec<(Command, f64)> = Vec::new();
    let mut seq: u32 = 0;

    let caravan_count = state.caravans.values().filter(|c| c.owner == pid).count();
    let max_caravans = match personality {
        Personality::Arbitrageur => 4,
        Personality::Aggressive | Personality::Cartel => 3,
        _ => 2,
    };

    // 1. BuyCaravan adayları — kervan ihtiyacı varsa
    // İlk kervan agresif (BEDAVA + urgency=1), sonrakiler düşük öncelik.
    // Kritik tuning: BuyCaravan utility her tick top-3'ü dolduruyor ve
    // DispatchCaravan'a yer kalmıyordu. Urgency aşamalı azaltma.
    if caravan_count < max_caravans {
        let cost = Caravan::buy_cost(Role::Tuccar, u32::try_from(caravan_count).unwrap_or(0));
        if player.cash >= cost {
            // Şehir seçimi tek bucket — 3 ayrı aday yerine 1 (Istanbul default).
            let starting_city = CityId::Istanbul;
            let urgency = match caravan_count {
                0 => 1.0,
                1 => 0.3,
                _ => 0.1,
            };
            let action = ActionCandidate {
                profit_lira: 0.0,
                capital_lira: money_lira(cost),
                risk: 0.3,
                urgency,
                momentum: 0.0,
                arbitrage: 0.5,
                event: 0.0,
                hold_pressure: 0.6,
            };
            let score = score_action(action, weights);
            scored.push((
                Command::BuyCaravan {
                    owner: pid,
                    starting_city,
                },
                score,
            ));
        }
    }

    // 2. DispatchCaravan adayları
    let idle_caravans: Vec<&Caravan> = state
        .caravans
        .values()
        .filter(|c| c.owner == pid && c.is_idle())
        .collect();

    for caravan in &idle_caravans {
        let Some(here) = caravan.state.current_city() else {
            continue;
        };
        // Her hedef şehir × her ürün için en kârlı combo bul
        for to in CityId::ALL {
            if to == here {
                continue;
            }
            for product in ProductKind::ALL {
                let here_price = state
                    .effective_baseline(here, product)
                    .map_or(0.0, money_lira);
                let there_price = state
                    .effective_baseline(to, product)
                    .map_or(0.0, money_lira);
                let unit_profit = there_price - here_price;
                if unit_profit <= 0.0 {
                    continue;
                }
                let have = player.inventory.get(here, product);
                let cap = caravan.capacity;
                let qty = have.min(cap);
                if qty == 0 {
                    continue;
                }
                let mut cargo = CargoSpec::new();
                if cargo.add(product, qty).is_err() {
                    continue;
                }
                let distance = here.distance_to(to);
                // DispatchCaravan utility — eski 0.6 urgency BuyCaravan'a
                // ezildi. Yeni: profit + arbitrage etkili olsun, urgency
                // 1.0 (kervan varsa hareket etsin).
                let action = ActionCandidate {
                    profit_lira: unit_profit * f64::from(qty) * 3.0, // 3× amplifier
                    capital_lira: here_price * f64::from(qty),
                    risk: 0.3,
                    urgency: 1.0,
                    momentum: price_momentum(state, to, product),
                    arbitrage: arbitrage_signal(state, product),
                    event: (event_signal(state, to, product)
                        + pending_event_signal(state, pid, to, product))
                    .min(1.0),
                    hold_pressure: f64::from(distance) / 5.0,
                };
                let score = score_action(action, weights);
                scored.push((
                    Command::DispatchCaravan {
                        caravan_id: caravan.id,
                        from: here,
                        to,
                        cargo,
                    },
                    score,
                ));
            }
        }
    }

    // 3. SubmitOrder Sell adayları — stoğu olan ürünler
    let entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, _, q)| *q > 0)
        .collect();
    for (city, product, qty) in entries.iter().take(6) {
        let market = state
            .price_history
            .get(&(*city, *product))
            .and_then(|v| v.last())
            .map(|(_, p)| *p)
            .or_else(|| state.effective_baseline(*city, *product))
            .unwrap_or(Money::from_cents(800));
        let ask_cents = (market.as_cents() * 105) / 100;
        let sell_qty = (*qty).min(15);
        let expected_revenue = (ask_cents as f64 / 100.0) * f64::from(sell_qty);
        let action = ActionCandidate {
            profit_lira: expected_revenue * 0.05, // küçük marj
            capital_lira: 0.0,
            risk: 0.3 + competition_signal(state, *city, *product) * 0.3,
            urgency: 0.5,
            momentum: price_momentum(state, *city, *product),
            arbitrage: arbitrage_signal(state, *product),
            event: (event_signal(state, *city, *product)
                + pending_event_signal(state, pid, *city, *product))
            .min(1.0),
            hold_pressure: 0.0,
        };
        let score = score_action(action, weights);
        let id = OrderId::new(npc_dss_order_id(pid, tick, seq));
        seq += 1;
        if let Ok(o) = MarketOrder::new_with_ttl(
            id,
            pid,
            *city,
            *product,
            OrderSide::Sell,
            sell_qty,
            Money::from_cents(ask_cents),
            tick,
            ttl,
        ) {
            scored.push((Command::SubmitOrder(o), score));
        }
    }

    // 4. Kontrat adayları — öner + kabul
    for (cmd, action) in propose_contract_candidates(state, pid, personality, tick) {
        let score = score_action(action, weights);
        scored.push((cmd, score));
    }
    for (cmd, action) in accept_contract_candidates(state, pid, personality, tick) {
        let score = score_action(action, weights);
        scored.push((cmd, score));
    }

    // Adaptive difficulty + cluster post-process
    let human_id = find_human(state);
    let lead_boost = human_id.map_or(0.0, |hid| {
        (human_lead_ratio(state, hid) - 1.0).clamp(0.0, 2.0)
    });

    for (cmd, score) in &mut scored {
        *score *= 1.0 + lead_boost * 0.3;
        if let Some((city, product)) = cmd_target_bucket(cmd) {
            let cs = cluster_signal(state, pid, personality, city, product);
            *score *= 1.0 + cs * 0.15;
        }
    }

    // Top-K sırala — deterministic tie-break Debug str ile
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });
    scored.into_iter().take(TOP_K).map(|(c, _)| c).collect()
}

fn find_human(state: &GameState) -> Option<PlayerId> {
    state
        .players
        .iter()
        .find(|(_, p)| !p.is_npc)
        .map(|(id, _)| *id)
}

fn cmd_target_bucket(cmd: &Command) -> Option<(CityId, ProductKind)> {
    match cmd {
        Command::SubmitOrder(order) => Some((order.city, order.product)),
        Command::DispatchCaravan { to, cargo, .. } => cargo.entries().next().map(|(p, _)| (*to, p)),
        _ => None,
    }
}

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

    #[test]
    fn tuccar_with_no_caravan_buys_one() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(100),
            "TestTuccar",
            Role::Tuccar,
            Money::from_lira(50_000).unwrap(),
            true,
        )
        .unwrap();
        s.players.insert(npc.id, npc);
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_tuccar_dss(
            &s,
            PlayerId::new(100),
            Personality::Arbitrageur,
            &mut rng,
            Tick::new(1),
        );
        assert!(
            cmds.iter().any(|c| matches!(c, Command::BuyCaravan { .. })),
            "kervan yoksa BuyCaravan adayı top-K'da olmalı"
        );
    }

    #[test]
    fn deterministic_for_same_seed() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(100),
            "TestTuccar",
            Role::Tuccar,
            Money::from_lira(20_000).unwrap(),
            true,
        )
        .unwrap();
        s.players.insert(npc.id, npc);
        let mut r1 = ChaCha8Rng::from_seed([42u8; 32]);
        let mut r2 = ChaCha8Rng::from_seed([42u8; 32]);
        let c1 = decide_tuccar_dss(
            &s,
            PlayerId::new(100),
            Personality::TrendFollower,
            &mut r1,
            Tick::new(1),
        );
        let c2 = decide_tuccar_dss(
            &s,
            PlayerId::new(100),
            Personality::TrendFollower,
            &mut r2,
            Tick::new(1),
        );
        assert_eq!(c1, c2);
    }
}
