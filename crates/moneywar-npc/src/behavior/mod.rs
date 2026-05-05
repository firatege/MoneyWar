//! Yeni NPC karar motoru — utility scoring + role-shaped action enumeration.
//!
//! Eski fuzzy + DSS hibridini değiştiriyor. Faz A'da iskelet, Faz B'den
//! itibaren rol göçü. Eski motor (`crate::engine`, `crate::dss`, `crate::fuzzy`)
//! Faz D'ye kadar yan yana çalışır — `Difficulty::Behavioral` flag'i ile
//! seçilir, default fuzzy.
//!
//! # Mimari
//!
//! ```text
//! decide_behavior(state, pid, rng, tick, difficulty)
//!   ↓
//! 1. enumerate_candidates(state, pid)        — rol-spesifik aday listesi
//! 2. for each candidate:                     — utility skor hesapla
//!     score = Σ w_i × signal_i               — ağırlık × sinyal
//! 3. apply difficulty + personality          — top-K + min_score + noise
//! 4. convert to Command[]                    — emit
//! ```
//!
//! # Faz planı
//!
//! - **A** (BU): iskelet — `signals`, `scoring`, `candidates`, `personality`,
//!   `difficulty` modülleri. `decide_behavior` boş döner. Eski motor canlı.
//! - **B**: Çiftçi pilot — sell-only, en basit rol. Audit ile karşılaştır.
//! - **C**: 6 rol sırayla göç (Alıcı → Esnaf → Spekülatör → Tüccar →
//!   Sanayici → Banka).
//! - **D**: eski `fuzzy/`, `engine/`, `dss/` silinir.
//! - **E**: difficulty + personality TOML config + grid search tuning.

pub mod candidates;
pub mod difficulty;
pub mod personality;
pub mod scoring;
pub mod signals;

use moneywar_domain::{Command, GameState, PlayerId, Tick};
use rand_chacha::ChaCha8Rng;

pub use difficulty::BehaviorDifficulty;

/// Tüm NPC'ler için Faz B+'da çağrılacak entry point. Şu an iskelet —
/// boş komut listesi döner. `decide_all_npcs` dispatch'i `Difficulty::Behavioral`
/// kolu ile bunu çağırır; rol implementasyonları Faz B'den itibaren eklenir.
#[must_use]
pub fn decide_behavior(
    state: &GameState,
    pid: PlayerId,
    _rng: &mut ChaCha8Rng,
    _tick: Tick,
    _difficulty: BehaviorDifficulty,
) -> Vec<Command> {
    // Faz A: iskelet. Player resolution + boş dönüş.
    // Faz B'de match player.npc_kind → role-spesifik enumerate + scoring.
    let Some(player) = state.players.get(&pid) else {
        return Vec::new();
    };
    if !player.is_npc {
        return Vec::new();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        GameState, Money, NpcKind, Player, PlayerId, Role, RoomConfig, RoomId,
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
    fn faz_a_npc_returns_empty_skeleton() {
        let mut s = fresh_state();
        let pid = PlayerId::new(100);
        let p = Player::new(
            pid,
            "n",
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Tuccar);
        s.players.insert(pid, p);
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_behavior(&s, pid, &mut rng, Tick::new(1), BehaviorDifficulty::HARD);
        // Faz A iskelet — henüz rol implementasyonu yok.
        assert!(cmds.is_empty());
    }
}
