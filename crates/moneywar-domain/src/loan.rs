//! NPC banka kredisi (Faz 5.5).
//!
//! Oyuncu NPC bankadan sabit faizli kredi alır, vade tick'inde
//! `principal + faiz` geri öder (game-design.md §7 — "NPC borç"
//! Faz 5.5'te implement edilir, tip şimdi hazırda).

use serde::{Deserialize, Serialize};

use crate::{DomainError, LoanId, Money, PlayerId, Tick};

/// NPC bankasından alınan sabit faizli kredi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loan {
    pub id: LoanId,
    pub borrower: PlayerId,
    pub principal: Money,
    /// Basit faiz oranı (tam sayı yüzde, ör. 15 = %15).
    pub interest_rate_percent: u32,
    pub take_tick: Tick,
    pub due_tick: Tick,
    pub repaid: bool,
    /// Borç verici Banka NPC. `None` → sistem bankası (eski mekanik, oyuncu
    /// `TakeLoan` komutuyla alır, para korunumu sistem dışı). `Some(banka_id)`
    /// → Banka NPC kasasından çıktı, geri ödemede Banka'ya geri dönecek
    /// (Plan v4 closed loop).
    #[serde(default)]
    pub lender: Option<PlayerId>,
}

impl Loan {
    /// Yeni kredi. `principal` pozitif, `due > take` olmalı.
    pub fn new(
        id: LoanId,
        borrower: PlayerId,
        principal: Money,
        interest_rate_percent: u32,
        take_tick: Tick,
        due_tick: Tick,
    ) -> Result<Self, DomainError> {
        if !principal.is_positive() {
            return Err(DomainError::Validation(format!(
                "loan principal must be positive, got {principal}"
            )));
        }
        if !take_tick.is_before(due_tick) {
            return Err(DomainError::Validation(
                "loan due_tick must be strictly after take_tick".into(),
            ));
        }
        Ok(Self {
            id,
            borrower,
            principal,
            interest_rate_percent,
            take_tick,
            due_tick,
            repaid: false,
            lender: None,
        })
    }

    /// Builder-style — borç vericiyi (Banka NPC) set eder.
    /// Plan v4 closed loop: Banka kasasından çıktı, geri ödemede Banka'ya döner.
    #[must_use]
    pub fn with_lender(mut self, lender: PlayerId) -> Self {
        self.lender = Some(lender);
        self
    }

    /// Vade sonunda ödenecek toplam (principal + faiz).
    pub fn total_due(&self) -> Result<Money, DomainError> {
        // interest = principal * rate / 100
        let interest_cents = self
            .principal
            .as_cents()
            .checked_mul(i64::from(self.interest_rate_percent))
            .and_then(|n| n.checked_div(100))
            .ok_or_else(|| {
                DomainError::Overflow(format!(
                    "loan interest: {} * {}",
                    self.principal, self.interest_rate_percent
                ))
            })?;
        self.principal
            .checked_add(Money::from_cents(interest_cents))
    }

    /// `current >= due_tick` ise true (vade geldi, ödeme zamanı).
    #[must_use]
    pub fn is_due(&self, current: Tick) -> bool {
        !self.repaid && !current.is_before(self.due_tick)
    }

    /// `current > due_tick` ise true (vade geçti, tazminat senaryosu).
    #[must_use]
    pub fn is_overdue(&self, current: Tick) -> bool {
        !self.repaid && self.due_tick.is_before(current)
    }

    pub fn mark_repaid(&mut self) -> Result<(), DomainError> {
        if self.repaid {
            return Err(DomainError::InvalidTransition {
                entity: "loan",
                from: "Repaid",
                to: "Repaid (again)",
            });
        }
        self.repaid = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_loan() -> Loan {
        Loan::new(
            LoanId::new(1),
            PlayerId::new(1),
            Money::from_lira(1_000).unwrap(),
            15, // 15%
            Tick::new(10),
            Tick::new(30),
        )
        .unwrap()
    }

    #[test]
    fn valid_loan_starts_unpaid() {
        let l = sample_loan();
        assert!(!l.repaid);
    }

    #[test]
    fn principal_must_be_positive() {
        let err = Loan::new(
            LoanId::new(1),
            PlayerId::new(1),
            Money::ZERO,
            10,
            Tick::new(1),
            Tick::new(10),
        )
        .expect_err("zero principal");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn due_tick_must_be_after_take() {
        let err = Loan::new(
            LoanId::new(1),
            PlayerId::new(1),
            Money::from_lira(100).unwrap(),
            10,
            Tick::new(10),
            Tick::new(10),
        )
        .expect_err("same tick");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn total_due_adds_interest() {
        let l = sample_loan();
        // 1000₺ + 15% = 1150₺
        assert_eq!(l.total_due().unwrap(), Money::from_lira(1_150).unwrap());
    }

    #[test]
    fn total_due_zero_interest() {
        let l = Loan::new(
            LoanId::new(1),
            PlayerId::new(1),
            Money::from_lira(500).unwrap(),
            0,
            Tick::new(1),
            Tick::new(10),
        )
        .unwrap();
        assert_eq!(l.total_due().unwrap(), Money::from_lira(500).unwrap());
    }

    #[test]
    fn is_due_after_due_tick() {
        let l = sample_loan();
        assert!(!l.is_due(Tick::new(20)));
        assert!(l.is_due(Tick::new(30)));
        assert!(l.is_due(Tick::new(35)));
    }

    #[test]
    fn repaid_loan_is_not_due() {
        let mut l = sample_loan();
        l.mark_repaid().unwrap();
        assert!(!l.is_due(Tick::new(100)));
    }

    #[test]
    fn mark_repaid_once() {
        let mut l = sample_loan();
        l.mark_repaid().unwrap();
        let err = l.mark_repaid().expect_err("double repay");
        assert!(matches!(err, DomainError::InvalidTransition { .. }));
    }

    #[test]
    fn serde_roundtrip() {
        let l = sample_loan();
        let back: Loan = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
        assert_eq!(l, back);
    }
}
