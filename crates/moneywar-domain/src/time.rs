//! Oyun zaman tipleri.
//!
//! - `Tick`: oyun içi atomik zaman birimi. 0'dan başlar, motor her turda +1.
//! - `SeasonProgress`: sezonun tamamlanma yüzdesi (0-100). Olay sıklığı
//!   curve'ü ve UI göstergesi için kullanılır.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::DomainError;

/// Oyun içi atomik zaman birimi. Sistem saatinden bağımsız.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(transparent)]
pub struct Tick(u32);

impl Tick {
    /// Oyun başlangıcı (tick 0).
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// `n` tick sonrasını döndürür. Overflow = hata.
    pub fn checked_add(self, n: u32) -> Result<Self, DomainError> {
        self.0
            .checked_add(n)
            .map(Self)
            .ok_or_else(|| DomainError::Overflow(format!("tick {} + {n}", self.0)))
    }

    /// Bir sonraki tick. `u32::MAX`'te saturate eder (90 tick sezonda erişilmez).
    #[must_use]
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }

    /// `self` `other`'dan önce mi?
    #[must_use]
    pub const fn is_before(self, other: Self) -> bool {
        self.0 < other.0
    }

    /// İki tick arası fark (other - self). Negatif olursa `None`.
    #[must_use]
    pub const fn ticks_until(self, other: Self) -> Option<u32> {
        other.0.checked_sub(self.0)
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tick#{}", self.0)
    }
}

/// Sezon ilerleme yüzdesi (0-100).
///
/// Olay motoru sezon ritmini buna göre ayarlar:
/// - `is_early()` (0-49): sakin
/// - `is_mid()` (50-79): olay sıklığı artar
/// - `is_late()` (80-100): makro şoklar, comeback penceresi
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeasonProgress(u8);

impl SeasonProgress {
    pub const START: Self = Self(0);
    pub const END: Self = Self(100);

    /// 0-100 aralığını doğrular.
    pub fn new(percent: u8) -> Result<Self, DomainError> {
        if percent > 100 {
            return Err(DomainError::Validation(format!(
                "season progress must be 0-100, got {percent}"
            )));
        }
        Ok(Self(percent))
    }

    /// Mevcut tick + sezon toplamından progress hesaplar. Üstte cap 100.
    pub fn from_ticks(current: Tick, season_total: u32) -> Result<Self, DomainError> {
        if season_total == 0 {
            return Err(DomainError::Validation("season_total must be > 0".into()));
        }
        let percent = u64::from(current.0)
            .saturating_mul(100)
            .saturating_div(u64::from(season_total));
        let capped = u8::try_from(percent.min(100)).unwrap_or(100);
        Ok(Self(capped))
    }

    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }

    /// Sezonun ilk %50'sinde mi?
    #[must_use]
    pub const fn is_early(self) -> bool {
        self.0 < 50
    }

    /// Sezonun %50-80 aralığında mı?
    #[must_use]
    pub const fn is_mid(self) -> bool {
        self.0 >= 50 && self.0 < 80
    }

    /// Sezonun son %20'sinde mi? (makro şoklar burada artar)
    #[must_use]
    pub const fn is_late(self) -> bool {
        self.0 >= 80
    }
}

impl fmt::Display for SeasonProgress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_zero_is_default() {
        assert_eq!(Tick::default(), Tick::ZERO);
    }

    #[test]
    fn tick_next_increments() {
        assert_eq!(Tick::new(5).next(), Tick::new(6));
    }

    #[test]
    fn tick_next_saturates_at_max() {
        assert_eq!(Tick::new(u32::MAX).next(), Tick::new(u32::MAX));
    }

    #[test]
    fn tick_checked_add_overflows() {
        assert!(Tick::new(u32::MAX).checked_add(1).is_err());
    }

    #[test]
    fn tick_is_before() {
        assert!(Tick::new(5).is_before(Tick::new(10)));
        assert!(!Tick::new(10).is_before(Tick::new(5)));
        assert!(!Tick::new(5).is_before(Tick::new(5)));
    }

    #[test]
    fn tick_ticks_until_positive() {
        assert_eq!(Tick::new(5).ticks_until(Tick::new(10)), Some(5));
    }

    #[test]
    fn tick_ticks_until_negative_returns_none() {
        assert_eq!(Tick::new(10).ticks_until(Tick::new(5)), None);
    }

    #[test]
    fn tick_serde_roundtrip() {
        let t = Tick::new(42);
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(json, "42");
        let back: Tick = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn tick_display() {
        assert_eq!(Tick::new(3).to_string(), "tick#3");
    }

    #[test]
    fn season_progress_validates_upper_bound() {
        assert!(SeasonProgress::new(50).is_ok());
        assert!(SeasonProgress::new(100).is_ok());
        assert!(SeasonProgress::new(101).is_err());
    }

    #[test]
    fn season_progress_from_ticks_early() {
        let p = SeasonProgress::from_ticks(Tick::new(10), 100).unwrap();
        assert_eq!(p.value(), 10);
        assert!(p.is_early());
        assert!(!p.is_mid());
        assert!(!p.is_late());
    }

    #[test]
    fn season_progress_from_ticks_mid() {
        let p = SeasonProgress::from_ticks(Tick::new(60), 100).unwrap();
        assert_eq!(p.value(), 60);
        assert!(p.is_mid());
    }

    #[test]
    fn season_progress_from_ticks_late() {
        let p = SeasonProgress::from_ticks(Tick::new(85), 100).unwrap();
        assert!(p.is_late());
    }

    #[test]
    fn season_progress_caps_over_100() {
        let p = SeasonProgress::from_ticks(Tick::new(200), 100).unwrap();
        assert_eq!(p.value(), 100);
        assert!(p.is_late());
    }

    #[test]
    fn season_progress_rejects_zero_total() {
        assert!(SeasonProgress::from_ticks(Tick::new(10), 0).is_err());
    }

    #[test]
    fn season_progress_display() {
        assert_eq!(SeasonProgress::new(42).unwrap().to_string(), "42%");
    }

    #[test]
    fn season_progress_constants() {
        assert_eq!(SeasonProgress::START.value(), 0);
        assert_eq!(SeasonProgress::END.value(), 100);
    }
}
