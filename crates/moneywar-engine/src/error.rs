//! Tick motoru hataları.
//!
//! Engine saf fonksiyon olduğu için hata sayısı az: domain'den gelen parse/validation
//! hataları + motorun kendi invariant ihlalleri. I/O hatası **yok** (by design).

use moneywar_domain::DomainError;
use thiserror::Error;

/// `moneywar-engine` crate'inin tüm hata tipleri.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum EngineError {
    /// Domain katmanından gelen hata (`Money` overflow, `RoomConfig` validation).
    #[error("domain: {0}")]
    Domain(#[from] DomainError),

    /// Motor invariant'ı ihlal edildi (para korunumu, stok korunumu vb).
    /// Bu hata test'te patlamalı — production'da görülmemeli.
    #[error("engine invariant violated: {0}")]
    Invariant(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invariant_error_displays_message() {
        let err = EngineError::Invariant("cash sum mismatch".into());
        assert!(err.to_string().contains("invariant"));
        assert!(err.to_string().contains("cash sum mismatch"));
    }

    #[test]
    fn domain_error_converts_via_from() {
        let domain = DomainError::Validation("bad config".into());
        let engine: EngineError = domain.into();
        assert!(matches!(engine, EngineError::Domain(_)));
    }
}
