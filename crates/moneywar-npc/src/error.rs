//! NPC davranış hataları.

use moneywar_engine::EngineError;
use thiserror::Error;

/// `moneywar-npc` crate'inin tüm hata tipleri.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NpcError {
    /// NPC karar verirken motor bir invariant ihlali raporladı.
    #[error("engine: {0}")]
    Engine(#[from] EngineError),

    /// NPC kararı domain kurallarıyla uyumsuz (örn. geçersiz `Command`).
    #[error("invalid npc decision: {0}")]
    InvalidDecision(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_decision_displays_message() {
        let err = NpcError::InvalidDecision("empty command list".into());
        assert!(err.to_string().contains("invalid npc decision"));
    }

    #[test]
    fn engine_error_converts_via_from() {
        let engine = EngineError::Invariant("oops".into());
        let npc: NpcError = engine.into();
        assert!(matches!(npc, NpcError::Engine(_)));
    }
}
