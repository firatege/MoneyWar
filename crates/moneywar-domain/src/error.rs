//! Domain katmanı hataları.
//!
//! Tüm validation/parse/business-rule hatalarını tek çatı altında toplar.
//! Structured variant'lar downstream kodun `match` ile anlamlı hata ayırt
//! etmesine izin verir (ör. `InsufficientFunds` karşısında UI "yeterli
//! paran yok" gösterir).

use thiserror::Error;

use crate::{CityId, Money, ProductKind};

/// `moneywar-domain` crate'inin tüm hata tipleri.
#[derive(Debug, Error, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum DomainError {
    /// Genel doğrulama hatası (newtype parse, range check, invariant).
    #[error("validation failed: {0}")]
    Validation(String),

    /// Aritmetik taşma — `Money` / `Tick` checked işlem başarısız.
    #[error("arithmetic overflow: {0}")]
    Overflow(String),

    /// Envanterde yeterli stok yok.
    #[error("insufficient stock in {city}: {product} have={have}, want={want}")]
    InsufficientStock {
        city: CityId,
        product: ProductKind,
        have: u32,
        want: u32,
    },

    /// Nakit yeterli değil (satın alma, kontrat kaporası vb).
    #[error("insufficient funds: have={have}, want={want}")]
    InsufficientFunds { have: Money, want: Money },

    /// Kapasite aşıldı (kervan, fabrika slot, piyasa doygunluğu).
    #[error("capacity exceeded: {resource} limit={limit}, requested={requested}")]
    CapacityExceeded {
        resource: &'static str,
        limit: u32,
        requested: u32,
    },

    /// State machine geçişi geçersiz (ör. `Idle` caravan dispatch gerekli, `EnRoute`'ta yeniden dispatch).
    #[error("invalid state transition: {entity} from {from} → {to}")]
    InvalidTransition {
        entity: &'static str,
        from: &'static str,
        to: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_error_displays_message() {
        let err = DomainError::Validation("negative money".into());
        let s = err.to_string();
        assert!(s.contains("validation failed"));
        assert!(s.contains("negative money"));
    }

    #[test]
    fn overflow_error_displays_message() {
        let err = DomainError::Overflow("i64::MAX + 1".into());
        assert!(err.to_string().starts_with("arithmetic overflow"));
    }

    #[test]
    fn insufficient_stock_formats_city_and_product() {
        let err = DomainError::InsufficientStock {
            city: CityId::Istanbul,
            product: ProductKind::Pamuk,
            have: 5,
            want: 10,
        };
        let s = err.to_string();
        assert!(s.contains("İstanbul"));
        assert!(s.contains("Pamuk"));
        assert!(s.contains("have=5"));
        assert!(s.contains("want=10"));
    }

    #[test]
    fn insufficient_funds_formats_money() {
        let err = DomainError::InsufficientFunds {
            have: Money::from_cents(500),
            want: Money::from_cents(1000),
        };
        let s = err.to_string();
        assert!(s.contains("5.00₺"));
        assert!(s.contains("10.00₺"));
    }

    #[test]
    fn capacity_exceeded_formats_values() {
        let err = DomainError::CapacityExceeded {
            resource: "caravan",
            limit: 20,
            requested: 35,
        };
        let s = err.to_string();
        assert!(s.contains("caravan"));
        assert!(s.contains("limit=20"));
        assert!(s.contains("requested=35"));
    }

    #[test]
    fn invalid_transition_formats_state_names() {
        let err = DomainError::InvalidTransition {
            entity: "caravan",
            from: "EnRoute",
            to: "Dispatch",
        };
        let s = err.to_string();
        assert!(s.contains("EnRoute"));
        assert!(s.contains("Dispatch"));
    }

    #[test]
    fn errors_are_equal_when_payload_matches() {
        let a = DomainError::Validation("x".into());
        let b = DomainError::Validation("x".into());
        assert_eq!(a, b);
    }
}
