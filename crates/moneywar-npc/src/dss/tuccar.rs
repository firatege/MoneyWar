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

use crate::dss::inputs::{
    arbitrage_signal, competition_signal, event_signal, money_lira, price_momentum,
};
use crate::dss::personality::Personality;
use crate::dss::utility::{ActionCandidate, score_action};

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
    let weights = personality.weights();
    let ttl = state.config.balance.default_order_ttl;

    let mut scored: Vec<(Command, f64)> = Vec::new();
    let mut seq: u32 = 0;

    let caravan_count = state.caravans.values().filter(|c| c.owner == pid).count();
    let max_caravans = match personality {
        Personality::Arbitrageur => 4,
        Personality::Aggressive | Personality::Cartel => 3,
        _ => 2,
    };

    // 1. BuyCaravan adayları
    if caravan_count < max_caravans {
        let cost = Caravan::buy_cost(Role::Tuccar, u32::try_from(caravan_count).unwrap_or(0));
        if player.cash >= cost {
            for city in CityId::ALL {
                let action = ActionCandidate {
                    profit_lira: 0.0, // direkt kâr yok
                    capital_lira: money_lira(cost),
                    risk: 0.3,
                    urgency: if caravan_count == 0 { 1.0 } else { 0.5 },
                    momentum: 0.0,
                    arbitrage: 0.7, // kervan = arbitraj fırsatı için
                    event: 0.0,
                    hold_pressure: 0.6,
                };
                let score = score_action(action, weights);
                scored.push((
                    Command::BuyCaravan {
                        owner: pid,
                        starting_city: city,
                    },
                    score,
                ));
            }
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
                let action = ActionCandidate {
                    profit_lira: unit_profit * f64::from(qty),
                    capital_lira: here_price * f64::from(qty), // bağlı sermaye
                    risk: 0.4,
                    urgency: 0.6,
                    momentum: price_momentum(state, to, product),
                    arbitrage: arbitrage_signal(state, product),
                    event: event_signal(state, to, product),
                    hold_pressure: f64::from(distance) / 5.0, // uzun rota = uzun bekleme
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
            event: event_signal(state, *city, *product),
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

    // Top-K sırala — deterministic tie-break Debug str ile
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });
    scored.into_iter().take(TOP_K).map(|(c, _)| c).collect()
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
