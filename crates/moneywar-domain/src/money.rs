//! Para tipi — `Money(i64)` cent cinsinden.
//!
//! - 1 lira = 100 cent
//! - Float **yok** (tasarım gereği, determinism için)
//! - Negatif değer izinli (borç, yakma, refund)
//! - Tüm aritmetik `checked_*` — overflow = `DomainError::Overflow`

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::DomainError;

const CENTS_PER_LIRA: i64 = 100;

/// `MoneyWar` para tipi. 1 lira = 100 cent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Money(i64);

impl Money {
    /// Sıfır para.
    pub const ZERO: Self = Self(0);

    /// Ham cent değerinden `Money` kurar.
    #[must_use]
    pub const fn from_cents(cents: i64) -> Self {
        Self(cents)
    }

    /// Tam lira değerinden `Money` kurar. `lira * 100` overflow olursa hata.
    pub fn from_lira(lira: i64) -> Result<Self, DomainError> {
        lira.checked_mul(CENTS_PER_LIRA)
            .map(Self)
            .ok_or_else(|| DomainError::Overflow(format!("from_lira({lira})")))
    }

    /// Cent değerini döndürür.
    #[must_use]
    pub const fn as_cents(self) -> i64 {
        self.0
    }

    /// **Sadece görsel çıktı için.** Aritmetik için KULLANMAYIN — float hassasiyet sorunu.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn as_lira_for_display(self) -> f64 {
        (self.0 as f64) / (CENTS_PER_LIRA as f64)
    }

    /// Overflow-safe toplama.
    pub fn checked_add(self, other: Self) -> Result<Self, DomainError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or_else(|| DomainError::Overflow(format!("{} + {}", self.0, other.0)))
    }

    /// Overflow-safe çıkarma. Sonuç negatif olabilir (borç senaryosu).
    pub fn checked_sub(self, other: Self) -> Result<Self, DomainError> {
        self.0
            .checked_sub(other.0)
            .map(Self)
            .ok_or_else(|| DomainError::Overflow(format!("{} - {}", self.0, other.0)))
    }

    /// Overflow-safe skaler çarpma (miktar × birim fiyat senaryosu).
    pub fn checked_mul_scalar(self, scalar: i64) -> Result<Self, DomainError> {
        self.0
            .checked_mul(scalar)
            .map(Self)
            .ok_or_else(|| DomainError::Overflow(format!("{} * {scalar}", self.0)))
    }

    /// Overflow-safe negasyon (`i64::MIN` için hata).
    pub fn checked_neg(self) -> Result<Self, DomainError> {
        self.0
            .checked_neg()
            .map(Self)
            .ok_or_else(|| DomainError::Overflow(format!("-{}", self.0)))
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }
}

impl Default for Money {
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let abs = self.0.unsigned_abs();
        let lira = abs / 100;
        let cents = abs % 100;
        let sign = if self.0 < 0 { "-" } else { "" };
        write!(f, "{sign}{lira}.{cents:02}₺")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zero() {
        assert_eq!(Money::default(), Money::ZERO);
        assert_eq!(Money::ZERO.as_cents(), 0);
        assert!(Money::ZERO.is_zero());
    }

    #[test]
    fn from_lira_scales_to_cents() {
        let m = Money::from_lira(5).expect("no overflow");
        assert_eq!(m.as_cents(), 500);
    }

    #[test]
    fn from_lira_overflows_on_max() {
        let err = Money::from_lira(i64::MAX).expect_err("should overflow");
        assert!(matches!(err, DomainError::Overflow(_)));
    }

    #[test]
    fn checked_add_sums_values() {
        let a = Money::from_cents(300);
        let b = Money::from_cents(200);
        assert_eq!(a.checked_add(b).unwrap(), Money::from_cents(500));
    }

    #[test]
    fn checked_add_detects_overflow() {
        let a = Money::from_cents(i64::MAX);
        let b = Money::from_cents(1);
        assert!(a.checked_add(b).is_err());
    }

    #[test]
    fn checked_sub_allows_negative_result() {
        let a = Money::from_cents(100);
        let b = Money::from_cents(300);
        let result = a.checked_sub(b).unwrap();
        assert_eq!(result.as_cents(), -200);
        assert!(result.is_negative());
    }

    #[test]
    fn checked_mul_scalar_works() {
        let m = Money::from_cents(50);
        assert_eq!(m.checked_mul_scalar(10).unwrap(), Money::from_cents(500));
    }

    #[test]
    fn checked_mul_scalar_overflows() {
        let m = Money::from_cents(i64::MAX / 2);
        assert!(m.checked_mul_scalar(3).is_err());
    }

    #[test]
    fn checked_neg_works() {
        let m = Money::from_cents(100);
        assert_eq!(m.checked_neg().unwrap(), Money::from_cents(-100));
    }

    #[test]
    fn checked_neg_detects_i64_min() {
        let m = Money::from_cents(i64::MIN);
        assert!(m.checked_neg().is_err());
    }

    #[test]
    fn display_positive() {
        assert_eq!(Money::from_cents(12345).to_string(), "123.45₺");
    }

    #[test]
    fn display_negative() {
        assert_eq!(Money::from_cents(-525).to_string(), "-5.25₺");
    }

    #[test]
    fn display_sub_lira() {
        assert_eq!(Money::from_cents(7).to_string(), "0.07₺");
        assert_eq!(Money::from_cents(-7).to_string(), "-0.07₺");
    }

    #[test]
    fn serde_transparent_roundtrip() {
        let m = Money::from_cents(12345);
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(json, "12345");
        let back: Money = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn serde_negative_roundtrip() {
        let m = Money::from_cents(-500);
        let back: Money = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn ordering_for_btreemap_keys() {
        assert!(Money::from_cents(100) < Money::from_cents(200));
        assert!(Money::from_cents(-50) < Money::ZERO);
    }
}
