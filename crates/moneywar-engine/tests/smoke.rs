//! Integration smoke: `moneywar-engine` public API + domain entegrasyonu.

use moneywar_domain::{DomainError, GameState, RoomConfig, RoomId, Tick};
use moneywar_engine::{EngineError, advance_tick};

#[test]
fn engine_wraps_domain_error_transparently() {
    let domain = DomainError::Validation("cross-crate".into());
    let engine: EngineError = domain.into();
    assert!(engine.to_string().contains("cross-crate"));
}

#[test]
fn engine_error_is_std_error() {
    fn assert_error<E: std::error::Error>(_: &E) {}
    let err = EngineError::Invariant("x".into());
    assert_error(&err);
}

#[test]
fn advance_tick_empty_cmds_increments_tick() {
    let s0 = GameState::new(RoomId::new(1), RoomConfig::hizli());
    let (s1, report) = advance_tick(&s0, &[]).expect("ok");
    assert_eq!(s1.current_tick, Tick::new(1));
    assert_eq!(report.tick, Tick::new(1));
    assert!(report.entries.is_empty());
}

#[test]
fn advance_tick_does_not_mutate_input_state() {
    let s0 = GameState::new(RoomId::new(1), RoomConfig::hizli());
    let _ = advance_tick(&s0, &[]).unwrap();
    // Input state unchanged — reference was by shared borrow.
    assert_eq!(s0.current_tick, Tick::ZERO);
}

#[test]
fn advance_tick_runs_ten_ticks_deterministically() {
    let mut a = GameState::new(RoomId::new(42), RoomConfig::hizli());
    let mut b = GameState::new(RoomId::new(42), RoomConfig::hizli());
    for _ in 0..10 {
        a = advance_tick(&a, &[]).unwrap().0;
        b = advance_tick(&b, &[]).unwrap().0;
    }
    // Bit-perfect aynı state: determinism invariantının en sade kanıtı.
    assert_eq!(a, b);
    assert_eq!(a.current_tick, Tick::new(10));
}
