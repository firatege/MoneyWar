//! Integration smoke: `moneywar-npc` public API + engine/domain zinciri.

use moneywar_engine::EngineError;
use moneywar_npc::NpcError;

#[test]
fn npc_wraps_engine_error_transparently() {
    let engine = EngineError::Invariant("deep chain".into());
    let npc: NpcError = engine.into();
    assert!(npc.to_string().contains("deep chain"));
}

#[test]
fn npc_error_is_std_error() {
    fn assert_error<E: std::error::Error>(_: &E) {}
    let err = NpcError::InvalidDecision("x".into());
    assert_error(&err);
}
