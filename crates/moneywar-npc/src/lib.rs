//! NPC davranış motoru.
//!
//! İki dispatcher var:
//! - [`behavior`]: utility scoring + role-shaped enumeration (default; Easy/Medium/Hard).
//! - [`synthetic`]: AI'sız sade kurallar (ekonomi testi baseline'ı).
//!
//! `decide_all_npcs` her NPC için seçilen [`Difficulty`]'ye göre uygun
//! dispatcher'ı çağırır.
//!
//! # Determinism
//!
//! `decide` RNG alır; aynı (state, rng) → aynı komut seti. Motor, NPC'leri
//! `decide_all_npcs` üzerinden sıralı işler (`BTreeMap` `player_id` ASC).

pub mod behavior;
mod error;
pub mod synthetic;

pub use error::NpcError;

use moneywar_domain::{Command, GameState, PlayerId, Tick};
use rand_chacha::ChaCha8Rng;

/// NPC zorluk seviyesi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Difficulty {
    /// Az aksiyon, sessiz, eşik yüksek — yumuşak rekabet.
    Easy,
    /// Default — dengeli rekabet.
    #[default]
    Medium,
    /// Agresif — tüm aday seti, sessizlik yok.
    Hard,
    /// AI'sız sade kurallar — ekonomi mekaniği testi baseline'ı.
    /// `crate::synthetic` dispatch eder. Tuning ve regresyon için kullanılır.
    Synthetic,
}

impl Difficulty {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Easy => "Easy (yumuşak rekabet)",
            Self::Medium => "Medium (dengeli)",
            Self::Hard => "Hard (agresif)",
            Self::Synthetic => "Synthetic (sade kurallar — ekonomi testi)",
        }
    }

    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Easy => Self::Medium,
            Self::Medium => Self::Hard,
            Self::Hard => Self::Synthetic,
            Self::Synthetic => Self::Easy,
        }
    }

    /// Bu zorluğun behavior motor parametreleri.
    #[must_use]
    pub const fn behavior(self) -> behavior::BehaviorDifficulty {
        match self {
            Self::Easy => behavior::BehaviorDifficulty::EASY,
            Self::Medium | Self::Synthetic => behavior::BehaviorDifficulty::MEDIUM,
            Self::Hard => behavior::BehaviorDifficulty::HARD,
        }
    }
}

/// NPC `OrderId` ofseti — insan oyuncu havuzu ile çakışmasın diye yüksek ofsetli.
/// `seq` aynı tick'te birden çok emir verme imkanı verir (max ~100/oyuncu).
#[must_use]
pub fn npc_order_id(player_id: PlayerId, tick: Tick, seq: u32) -> u64 {
    moneywar_domain::balance::NPC_ORDER_ID_OFFSET
        .saturating_add(u64::from(tick.value()).saturating_mul(100_000))
        .saturating_add((player_id.value() % 1_000).saturating_mul(100))
        .saturating_add(u64::from(seq).min(99))
}

/// Tüm NPC'ler için bu tick'e ait komut setini, verilen zorluğa göre üret.
#[must_use]
pub fn decide_all_npcs(
    state: &GameState,
    rng: &mut ChaCha8Rng,
    tick: Tick,
    difficulty: Difficulty,
) -> Vec<Command> {
    let npc_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter_map(|(id, p)| if p.is_npc { Some(*id) } else { None })
        .collect();
    // Shadow state: NPC'ler sıralı işlenir, her birinin kararı bir sonrakine
    // görünür. Tick içi state immutable problemi çözülür → 5 Sanayici aynı
    // anda "Ist-Kumas boş" diyemez, ikinci NPC ilkini görür.
    let mut shadow = state.clone();
    let mut cmds = Vec::new();
    for pid in npc_ids {
        let next = match difficulty {
            Difficulty::Synthetic => synthetic::decide_synthetic(&shadow, pid, tick),
            Difficulty::Easy | Difficulty::Medium | Difficulty::Hard => {
                behavior::decide_behavior(&shadow, pid, rng, tick, difficulty.behavior())
            }
        };
        // BuildFactory komutlarını shadow'a yansıt — sonraki NPC görür.
        for cmd in &next {
            if let moneywar_domain::Command::BuildFactory {
                owner, city, product,
            } = cmd
            {
                let next_id = shadow.counters.next_factory_id;
                shadow.counters.next_factory_id = shadow.counters.next_factory_id.saturating_add(1);
                let fid = moneywar_domain::FactoryId::new(next_id);
                if let Ok(f) = moneywar_domain::Factory::new(fid, *owner, *city, *product) {
                    shadow.factories.insert(fid, f);
                }
            }
        }
        cmds.extend(next);
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npc_order_ids_do_not_collide_within_tick() {
        let mut ids = std::collections::BTreeSet::new();
        for i in 0..10u64 {
            for seq in 0..5u32 {
                let id = npc_order_id(PlayerId::new(i), Tick::new(1), seq);
                assert!(ids.insert(id), "collision at player {i} seq {seq}");
            }
        }
    }

    #[test]
    fn difficulty_next_cycles_through_all_variants() {
        let mut d = Difficulty::Easy;
        for _ in 0..4 {
            d = d.next();
        }
        // 4 next çağrısı sonrası başa dönmeli.
        assert_eq!(d, Difficulty::Easy);
    }

    #[test]
    fn synthetic_uses_medium_behavior_difficulty() {
        // Synthetic kendi dispatcher'ını kullanır, ama API uyumu için mapping var.
        assert_eq!(
            Difficulty::Synthetic.behavior(),
            behavior::BehaviorDifficulty::MEDIUM
        );
    }
}
