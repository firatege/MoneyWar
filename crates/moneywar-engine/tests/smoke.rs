//! Integration smoke: `moneywar-engine` public API + domain entegrasyonu.

use moneywar_domain::DomainError;
use moneywar_engine::EngineError;

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
