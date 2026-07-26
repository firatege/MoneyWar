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

pub mod brain;
pub mod candidates;
pub mod difficulty;
pub mod personality;
pub mod pricing;
pub mod roles;
pub mod scoring;
pub mod signals;

use std::cmp::Ordering;

use moneywar_domain::{
    Command, GameState, MarketOrder, NpcKind, OrderId, PlayerId, Tick,
    balance::NPC_DEFAULT_ORDER_TTL,
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

pub use brain::{AgentBrain, BrainPool};
pub use difficulty::BehaviorDifficulty;

use crate::npc_order_id;
use candidates::ActionCandidate;

/// Tüm NPC'ler için entry point. `decide_all_npcs(Difficulty::Behavioral)` buradan
/// dispatch eder.
#[must_use]
/// Bu aday, oyuncunun kendi relist cooldown'undaki bir pazara emir mi?
///
/// Yalnız `SubmitOrder` adayları cooldown'a tâbidir; fabrika/kervan/kontrat
/// aksiyonları etkilenmez.
fn is_on_relist_cooldown(
    state: &GameState,
    pid: PlayerId,
    cand: &ActionCandidate,
    tick: Tick,
) -> bool {
    let ActionCandidate::SubmitOrder { city, product, .. } = cand else {
        return false;
    };
    state
        .relist_cooldown
        .get(&(pid, *city, *product))
        .is_some_and(|allowed| tick.is_before(*allowed))
}

pub fn decide_behavior(
    state: &GameState,
    pid: PlayerId,
    rng: &mut ChaCha8Rng,
    tick: Tick,
    difficulty: BehaviorDifficulty,
    brain: Option<&AgentBrain>,
) -> Vec<Command> {
    let Some(player) = state.players.get(&pid) else {
        return Vec::new();
    };
    if !player.is_npc {
        return Vec::new();
    }

    // Silence — tick atla.
    if difficulty.silence_per_10 > 0 && rng.random_range(0u32..10) < difficulty.silence_per_10 {
        return Vec::new();
    }

    // Rol-spesifik aday listesi.
    let candidates = enumerate_for_kind(state, player, brain);
    if candidates.is_empty() {
        return Vec::new();
    }

    // Relist cooldown'daki pazarların emirlerini baştan ele. Motor bunları
    // zaten reddediyordu; NPC körlemesine deneyip hem komutunu hem top-K
    // slotunu harcıyordu. Ölçüm: 10 oyunda 369K red, tamamı bu sebepten —
    // tüm reddedilen komutların %99.8'i. Kendi emrinin cooldown'da olduğunu
    // bilmek gizli bilgi değil, ajanın kendi durumu.
    let candidates: Vec<ActionCandidate> = candidates
        .into_iter()
        .filter(|cand| !is_on_relist_cooldown(state, pid, cand, tick))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    // Skor hesapla — her aday için kendi `(city, product)` bağlamından sinyaller.
    // Faz 6: rol+kişilik ağırlıkları → brain traits ile dinamik modülasyon.
    let base_weights = personality::for_kind_personality(player.npc_kind, player.personality);
    let weights = brain
        .map(|b| b.traits.modulate(base_weights))
        .unwrap_or(base_weights);
    let mut scored: Vec<(ActionCandidate, f64)> = candidates
        .into_iter()
        .map(|cand| {
            // Context'siz adaylar (BuyCaravan, DispatchCaravan, kontrat) için
            // aday türüne özel sabit baz skor — 0.0 vermek bunları SubmitOrder
            // akışıyla yarışta sürekli kaybediyordu: Tüccar 90 tick'te 1
            // dispatch atıyordu. Sabit skor onları top-K'da garantili tutar.
            let base_score = if let Some((city, product)) = cand.context() {
                let mut inputs = signals::compute_inputs(state, pid, city, product);
                if let Some(b) = brain {
                    signals::inject_brain_signals(&mut inputs, b, city, product, pid, state);
                }
                scoring::score_candidate(&inputs, &weights)
            } else {
                match &cand {
                    ActionCandidate::BuyCaravan { .. } => 0.4,
                    ActionCandidate::DispatchCaravan { .. } => 0.4,
                    ActionCandidate::BuildPrivateFarm { .. } => 0.99,
                    ActionCandidate::UpgradeFarm { .. } => 0.92, // tarla yükseltme karlı
                    ActionCandidate::DemolishFactory { .. } => 0.7,
                    // Yükseltme de filtreli — yüksek sabit skor.
                    // Yükseltme kârlıdır (ROI 4-7×) — fabrika kurmaktan öncelikli.
                    ActionCandidate::UpgradeFactory { .. } => 0.95,
                    // Kadro kararı ucuz ve doğrudan üretimi etkiler — üst sırada.
                    ActionCandidate::SetStaff { .. } => 0.93,
                    ActionCandidate::ProposeContract(_)
                    | ActionCandidate::AcceptContract { .. } => 0.2,
                    // SubmitOrder/BuildFactory context döner, buraya düşmez.
                    _ => 0.0,
                }
            };
            // Faz 3: skill noise — kazanan keskin, kaybeden panikler.
            let effective_noise = brain
                .map(|b| b.skill_noise(difficulty.noise))
                .unwrap_or(difficulty.noise);
            let noise = if effective_noise > 0.0 {
                (rng.random::<f64>() - 0.5) * 2.0 * effective_noise
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

    // Kritik aksiyonlar (tarla, fabrika kapat) top_k'ya garanti girer.
    // Bunlar context'siz sabit skor alır — BuildFactory gibi yüksek-sinyalli
    // adayların arkasında kalabilir. Top_k'ya zorla ekliyoruz.
    let priority_exist = scored.iter().take(difficulty.top_k as usize)
        .any(|(c, _)| matches!(c, ActionCandidate::BuildPrivateFarm { .. }
            | ActionCandidate::DemolishFactory { .. }));
    let priority_cands: Vec<_> = scored.iter()
        .filter(|(c, _)| matches!(c, ActionCandidate::BuildPrivateFarm { .. }
            | ActionCandidate::DemolishFactory { .. }))
        .map(|(c, s)| (c.clone(), *s))
        .collect();

    scored.truncate(difficulty.top_k as usize);

    // BuildFactory: tick başına en fazla 1 — shadow cash güncellense de
    // aynı NPC aynı tick'te birden fazla fabrika kurmasın.
    let fab_count = scored.iter().filter(|(c, _)| matches!(c, ActionCandidate::BuildFactory { .. })).count();
    if fab_count > 1 {
        let mut kept = false;
        scored.retain(|(c, _)| {
            if matches!(c, ActionCandidate::BuildFactory { .. }) {
                if kept { return false; }
                kept = true;
            }
            true
        });
    }

    // Eğer öncelikli aday top_k'ya giremediyse, sona ekle.
    if !priority_exist {
        for pc in priority_cands.into_iter().take(1) {
            scored.push(pc);
        }
    }

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
fn enumerate_for_kind(state: &GameState, player: &moneywar_domain::Player, brain: Option<&brain::AgentBrain>) -> Vec<ActionCandidate> {
    match player.npc_kind {
        Some(NpcKind::Ciftci) => roles::ciftci::enumerate(state, player),
        Some(NpcKind::Alici) => roles::alici::enumerate(state, player),
        Some(NpcKind::Sanayici) => roles::sanayici::enumerate_with_brain(state, player, brain),
        Some(NpcKind::Esnaf) => roles::esnaf::enumerate(state, player),
        Some(NpcKind::Spekulator) => roles::spekulator::enumerate(state, player),
        Some(NpcKind::Tuccar) => roles::tuccar::enumerate(state, player),
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
            ttl_override,
        } => {
            if quantity == 0 || unit_price.as_cents() <= 0 {
                return None;
            }
            let ttl = ttl_override.unwrap_or(NPC_DEFAULT_ORDER_TTL);
            let order = MarketOrder::new_with_ttl(
                OrderId::new(npc_order_id(pid, tick, seq)),
                pid,
                city,
                product,
                side,
                quantity,
                unit_price,
                tick,
                ttl,
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
        ActionCandidate::AcceptContract { contract_id } => Some(Command::AcceptContract {
            contract_id,
            acceptor: pid,
        }),
        ActionCandidate::UpgradeFarm { farm_id } => Some(Command::UpgradeFarm {
            owner: pid,
            farm_id,
        }),
        ActionCandidate::BuildPrivateFarm { city, product } => Some(Command::BuildPrivateFarm {
            owner: pid,
            city,
            product,
        }),
        ActionCandidate::DemolishFactory { factory_id } => Some(Command::DemolishFactory {
            owner: pid,
            factory_id,
        }),
        ActionCandidate::SetStaff { factory_id, employees } => Some(Command::SetFactoryStaff {
            owner: pid,
            factory_id,
            employees,
        }),
        ActionCandidate::UpgradeFactory { factory_id } => Some(Command::UpgradeFactory {
            owner: pid,
            factory_id,
        }),
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
            None,
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
        let cmds = decide_behavior(&s, pid, &mut rng, Tick::new(1), BehaviorDifficulty::HARD, None);
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
        let cmds = decide_behavior(&s, pid, &mut rng, Tick::new(1), BehaviorDifficulty::HARD, None);
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
        let cmds = decide_behavior(&s, pid, &mut rng, Tick::new(1), BehaviorDifficulty::HARD, None);
        // Faz B: Tüccar henüz göç etmedi, behavior boş döner.
        assert!(cmds.is_empty());
    }

    #[test]
    fn emitted_order_uses_npc_default_ttl() {
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
            .add(CityId::Istanbul, ProductKind::Pamuk, 200)
            .unwrap();
        s.players.insert(pid, p);
        let mut rng = ChaCha8Rng::from_seed([42u8; 32]);
        let cmds = decide_behavior(&s, pid, &mut rng, Tick::new(1), BehaviorDifficulty::HARD, None);
        let Command::SubmitOrder(o) = &cmds[0] else {
            panic!()
        };
        assert_eq!(
            o.ttl_ticks,
            moneywar_domain::balance::NPC_DEFAULT_ORDER_TTL,
            "behavior emirleri NPC_DEFAULT_ORDER_TTL ile yazılmalı"
        );
        assert_eq!(o.remaining_ticks, o.ttl_ticks);
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
        let a = decide_behavior(&s, pid, &mut r1, Tick::new(5), BehaviorDifficulty::HARD, None);
        let b = decide_behavior(&s, pid, &mut r2, Tick::new(5), BehaviorDifficulty::HARD, None);
        assert_eq!(a, b);
    }
}
