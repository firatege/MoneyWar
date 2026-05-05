//! Yeni NPC karar motoru — utility scoring + role-shaped action enumeration.
//!
//! Eski fuzzy + DSS hibridini değiştiriyor. `Difficulty::Behavioral` flag'i ile
//! seçilir; default fuzzy. Faz D'de eski motor silinince Easy/Medium/Hard buraya
//! yönlendirilecek.
//!
//! # Akış
//!
//! ```text
//! decide_behavior(state, pid, rng, tick, difficulty)
//!   ↓
//! 1. silence check                        — tick atlama
//! 2. enumerate_candidates(state, player)  — rol-spesifik aday listesi
//! 3. score her aday:                      — utility skor
//!     score = Σ w_i × signal_i
//!     + noise = (rng - 0.5) × 2 × difficulty.noise
//! 4. min_score filter                     — eşik altı düşer
//! 5. top-K sort (skor desc)               — en iyi K seç
//! 6. ActionCandidate → Command            — emit
//! ```
//!
//! # Determinism
//!
//! `rng` sadece silence + noise için. Sıralama `BTreeMap` iterasyon (state) +
//! aday sırası (enumerate) + skor karşılaştırma. Tie-break ihtimali `(score,
//! city, product)` lex sırasıyla.

pub mod candidates;
pub mod difficulty;
pub mod personality;
pub mod roles;
pub mod scoring;
pub mod signals;

use std::cmp::Ordering;

use moneywar_domain::{Command, GameState, MarketOrder, NpcKind, OrderId, PlayerId, Tick};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

pub use difficulty::BehaviorDifficulty;

use crate::npc_order_id;
use candidates::ActionCandidate;

/// Tüm NPC'ler için entry point. `decide_all_npcs(Difficulty::Behavioral)` buradan
/// dispatch eder.
#[must_use]
pub fn decide_behavior(
    state: &GameState,
    pid: PlayerId,
    rng: &mut ChaCha8Rng,
    tick: Tick,
    difficulty: BehaviorDifficulty,
) -> Vec<Command> {
    let Some(player) = state.players.get(&pid) else {
        return Vec::new();
    };
    if !player.is_npc {
        return Vec::new();
    }

    // Silence — tick atla.
    if difficulty.silence_per_10 > 0
        && rng.random_range(0u32..10) < difficulty.silence_per_10
    {
        return Vec::new();
    }

    // Rol-spesifik aday listesi.
    let candidates = enumerate_for_kind(state, player);
    if candidates.is_empty() {
        return Vec::new();
    }

    // Skor hesapla — her aday için kendi `(city, product)` bağlamından sinyaller.
    let weights = personality::for_kind_personality(player.npc_kind, player.personality);
    let mut scored: Vec<(ActionCandidate, f64)> = candidates
        .into_iter()
        .map(|cand| {
            let base_score = if let Some((city, product)) = cand.context() {
                let inputs = signals::compute_inputs(state, pid, city, product);
                scoring::score_candidate(&inputs, &weights)
            } else {
                0.0
            };
            let noise = if difficulty.noise > 0.0 {
                (rng.random::<f64>() - 0.5) * 2.0 * difficulty.noise
            } else {
                0.0
            };
            (cand, base_score + noise)
        })
        .collect();

    // Min skor filtre.
    scored.retain(|(_, s)| *s >= difficulty.min_score);

    // Top-K (skor desc, tie deterministic via insertion order).
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    scored.truncate(difficulty.top_k as usize);

    // ActionCandidate → Command.
    let mut cmds = Vec::with_capacity(scored.len());
    for (i, (cand, _)) in scored.into_iter().enumerate() {
        if let Some(cmd) = candidate_to_command(cand, pid, tick, u32::try_from(i).unwrap_or(0)) {
            cmds.push(cmd);
        }
    }
    cmds
}

/// Player'ın `npc_kind`'ına göre aday üretici dispatch.
/// Faz B: Çiftçi pilot. Faz C+'da diğer roller eklenecek.
fn enumerate_for_kind(
    state: &GameState,
    player: &moneywar_domain::Player,
) -> Vec<ActionCandidate> {
    match player.npc_kind {
        Some(NpcKind::Ciftci) => roles::ciftci::enumerate(state, player),
        Some(NpcKind::Alici) => roles::alici::enumerate(state, player),
        Some(NpcKind::Sanayici) => roles::sanayici::enumerate(state, player),
        Some(NpcKind::Esnaf) => roles::esnaf::enumerate(state, player),
        Some(NpcKind::Spekulator) => roles::spekulator::enumerate(state, player),
        Some(NpcKind::Tuccar) => roles::tuccar::enumerate(state, player),
        // Banka behavior'da yok — özel akış (`engine::tick_banks`).
        Some(NpcKind::Banka) | None => Vec::new(),
    }
}

fn candidate_to_command(
    cand: ActionCandidate,
    pid: PlayerId,
    tick: Tick,
    seq: u32,
) -> Option<Command> {
    match cand {
        ActionCandidate::SubmitOrder {
            side,
            city,
            product,
            quantity,
            unit_price,
        } => {
            if quantity == 0 || unit_price.as_cents() <= 0 {
                return None;
            }
            let order = MarketOrder::new(
                OrderId::new(npc_order_id(pid, tick, seq)),
                pid,
                city,
                product,
                side,
                quantity,
                unit_price,
                tick,
            )
            .ok()?;
            Some(Command::SubmitOrder(order))
        }
        ActionCandidate::BuildFactory { city, product } => Some(Command::BuildFactory {
            owner: pid,
            city,
            product,
        }),
        ActionCandidate::BuyCaravan { starting_city } => Some(Command::BuyCaravan {
            owner: pid,
            starting_city,
        }),
        ActionCandidate::DispatchCaravan {
            caravan_id,
            from,
            to,
            cargo,
        } => Some(Command::DispatchCaravan {
            caravan_id,
            from,
            to,
            cargo,
        }),
        ActionCandidate::ProposeContract(p) => Some(Command::ProposeContract(p)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CityId, GameState, Money, NpcKind, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };
    use rand_chacha::rand_core::SeedableRng;

    fn fresh_state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    #[test]
    fn missing_player_returns_empty() {
        let s = fresh_state();
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_behavior(
            &s,
            PlayerId::new(999),
            &mut rng,
            Tick::new(1),
            BehaviorDifficulty::HARD,
        );
        assert!(cmds.is_empty());
    }

    #[test]
    fn human_player_returns_empty() {
        let mut s = fresh_state();
        let pid = PlayerId::new(1);
        let p = Player::new(
            pid,
            "h",
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            false,
        )
        .unwrap();
        s.players.insert(pid, p);
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_behavior(&s, pid, &mut rng, Tick::new(1), BehaviorDifficulty::HARD);
        assert!(cmds.is_empty());
    }

    #[test]
    fn ciftci_with_stock_emits_sell_order() {
        let mut s = fresh_state();
        let pid = PlayerId::new(100);
        let mut p = Player::new(
            pid,
            "ciftci",
            Role::Tuccar,
            Money::from_lira(8_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Ciftci);
        p.inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 200)
            .unwrap();
        s.players.insert(pid, p);

        let mut rng = ChaCha8Rng::from_seed([42u8; 32]);
        let cmds = decide_behavior(&s, pid, &mut rng, Tick::new(1), BehaviorDifficulty::HARD);
        assert!(!cmds.is_empty(), "Çiftçi stoğu varsa SELL emit etmeli");
        let Command::SubmitOrder(o) = &cmds[0] else {
            panic!("Çiftçi sadece SubmitOrder emit etmeli");
        };
        assert_eq!(o.side, moneywar_domain::OrderSide::Sell);
        assert!(o.product.is_raw());
    }

    #[test]
    fn unmigrated_role_returns_empty() {
        let mut s = fresh_state();
        let pid = PlayerId::new(100);
        let p = Player::new(
            pid,
            "tuccar",
            Role::Tuccar,
            Money::from_lira(15_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Tuccar);
        s.players.insert(pid, p);

        let mut rng = ChaCha8Rng::from_seed([42u8; 32]);
        let cmds = decide_behavior(&s, pid, &mut rng, Tick::new(1), BehaviorDifficulty::HARD);
        // Faz B: Tüccar henüz göç etmedi, behavior boş döner.
        assert!(cmds.is_empty());
    }

    #[test]
    fn deterministic_for_same_seed() {
        let mut s = fresh_state();
        let pid = PlayerId::new(100);
        let mut p = Player::new(
            pid,
            "c",
            Role::Tuccar,
            Money::from_lira(8_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Ciftci);
        p.inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100)
            .unwrap();
        s.players.insert(pid, p);

        let mut r1 = ChaCha8Rng::from_seed([7u8; 32]);
        let mut r2 = ChaCha8Rng::from_seed([7u8; 32]);
        let a = decide_behavior(&s, pid, &mut r1, Tick::new(5), BehaviorDifficulty::HARD);
        let b = decide_behavior(&s, pid, &mut r2, Tick::new(5), BehaviorDifficulty::HARD);
        assert_eq!(a, b);
    }
}
