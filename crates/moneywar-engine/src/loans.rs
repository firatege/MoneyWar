//! NPC bankası kredileri (§7 — Faz 5.5).
//!
//! # Mekanik
//!
//! - **Sabit faiz:** `INTEREST_RATE_PERCENT` (v1 = %15). Oyuncu komutunda
//!   faiz belirtmez, banka dayatır.
//! - **Kaldıraç iştahı:** oyuncu krediyi ister, principal cash'ine eklenir,
//!   vade tick'inde `principal + faiz` geri öder.
//! - **Tick sınırı auto-settle:** vade geçen kredi (`is_due(tick)`) motor
//!   tarafından otomatik çekilir. Nakit yetmezse tüm nakit alınır + default.
//!
//! Para korunumu: NPC bankası kapalı sistem dışı — borç verme +para yaratır,
//! geri ödeme -para silip faizi yakar. Oyun içi transferlerde (oyuncu→oyuncu)
//! korunum hâlâ geçerli; banka açık bir kaynak/havuzdur.

use moneywar_domain::{DomainError, GameState, Loan, LoanId, Money, PlayerId, Tick};

use crate::{
    error::EngineError,
    report::{LogEntry, TickReport},
};

/// v1 sabit faiz oranı (tam sayı yüzde). Değer
/// [`moneywar_domain::balance::LOAN_INTEREST_RATE_PERCENT`]'ten gelir.
pub const INTEREST_RATE_PERCENT: u32 = moneywar_domain::balance::LOAN_INTEREST_RATE_PERCENT;

/// `TakeLoan` komutu. Principal borçlunun nakitine eklenir, `Loan` kaydı açılır.
pub(crate) fn process_take_loan(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    player: PlayerId,
    amount: Money,
    duration_ticks: u32,
) -> Result<(), EngineError> {
    if duration_ticks == 0 {
        return Err(EngineError::Domain(DomainError::Validation(
            "loan duration must be > 0 ticks".into(),
        )));
    }
    // Oyuncu var mı?
    if !state.players.contains_key(&player) {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "player {player} not found"
        ))));
    }

    let due_tick = tick.checked_add(duration_ticks)?;
    let loan_id = LoanId::new(state.counters.next_loan_id);
    let loan = Loan::new(
        loan_id,
        player,
        amount,
        INTEREST_RATE_PERCENT,
        tick,
        due_tick,
    )?;
    let total_due = loan.total_due()?;

    // Principal borçluya aktar.
    let borrower = state.players.get_mut(&player).expect("checked");
    borrower.credit(amount)?;

    state.counters.next_loan_id = state.counters.next_loan_id.saturating_add(1);
    state.loans.insert(loan_id, loan);

    report.push(LogEntry::loan_taken(
        tick,
        player,
        loan_id,
        amount,
        INTEREST_RATE_PERCENT,
        due_tick,
        total_due,
    ));
    Ok(())
}

/// `RepayLoan` komutu — oyuncu inisiyatifiyle tam geri ödeme (vade öncesi
/// ya da vade tick'inde motor auto-settle'dan önce).
pub(crate) fn process_repay_loan(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    player: PlayerId,
    loan_id: LoanId,
) -> Result<(), EngineError> {
    let loan = state.loans.get(&loan_id).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!("loan {loan_id} not found")))
    })?;
    if loan.borrower != player {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "loan {loan_id} borrower is {}, not {player}",
            loan.borrower
        ))));
    }
    let total_due = loan.total_due()?;

    let borrower = state.players.get_mut(&player).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!(
            "borrower {player} not found"
        )))
    })?;
    if borrower.cash < total_due {
        return Err(EngineError::Domain(DomainError::InsufficientFunds {
            have: borrower.cash,
            want: total_due,
        }));
    }
    borrower.debit(total_due)?;

    state.loans.remove(&loan_id);
    let on_time = !tick.is_before(Tick::ZERO); // her zaman true — komut bazında on-time sayılır.
    report.push(LogEntry::loan_repaid(
        tick, player, loan_id, total_due, on_time,
    ));
    Ok(())
}

/// Tick sonu: vadesi gelmiş krediyi motor auto-settle yapar.
///
/// - Nakit yeterliyse tam çekim → `LoanRepaid(on_time=false)`.
/// - Yetmezse mevcut nakit çekilir, kalan borç silinir → `LoanDefaulted`.
///
/// Her durumda loan `state.loans`'tan kaldırılır.
pub(crate) fn advance_loans(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let loan_ids: Vec<LoanId> = state.loans.keys().copied().collect();
    for lid in loan_ids {
        let due_now = state
            .loans
            .get(&lid)
            .is_some_and(|l| !l.repaid && l.is_due(tick));
        if due_now {
            auto_settle(state, report, tick, lid);
        }
    }
}

fn auto_settle(state: &mut GameState, report: &mut TickReport, tick: Tick, lid: LoanId) {
    let loan = *state.loans.get(&lid).expect("checked");
    let borrower = loan.borrower;
    let Ok(total_due) = loan.total_due() else {
        // Overflow edge: loan'ı yok say, default event yaz.
        state.loans.remove(&lid);
        report.push(LogEntry::loan_defaulted(
            tick,
            borrower,
            lid,
            Money::ZERO,
            Money::ZERO,
        ));
        return;
    };

    let cash = state.players.get(&borrower).map_or(Money::ZERO, |p| p.cash);

    if cash >= total_due {
        if let Some(p) = state.players.get_mut(&borrower) {
            let _ = p.debit(total_due);
        }
        state.loans.remove(&lid);
        report.push(LogEntry::loan_repaid(tick, borrower, lid, total_due, false));
    } else {
        // Kısmi çekim: mevcut tüm nakit alınır, kalan silinir.
        let seized = cash;
        let unpaid_balance = total_due.checked_sub(seized).unwrap_or(Money::ZERO);
        if let Some(p) = state.players.get_mut(&borrower) {
            let _ = p.debit(seized);
        }
        state.loans.remove(&lid);
        report.push(LogEntry::loan_defaulted(
            tick,
            borrower,
            lid,
            seized,
            unpaid_balance,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{Money, Player, PlayerId, Role, RoomConfig, RoomId};

    fn state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn add_player(state: &mut GameState, id: u64, cash_lira: i64) -> PlayerId {
        let p = Player::new(
            PlayerId::new(id),
            format!("P{id}"),
            Role::Tuccar,
            Money::from_lira(cash_lira).unwrap(),
            false,
        )
        .unwrap();
        state.players.insert(p.id, p);
        PlayerId::new(id)
    }

    #[test]
    fn take_loan_credits_principal_and_creates_record() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, 0);
        process_take_loan(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            Money::from_lira(1_000).unwrap(),
            20,
        )
        .unwrap();
        assert_eq!(s.loans.len(), 1);
        assert_eq!(s.players[&pid].cash, Money::from_lira(1_000).unwrap());
        let loan = s.loans.values().next().unwrap();
        assert_eq!(loan.borrower, pid);
        assert_eq!(loan.due_tick, Tick::new(21));
        assert_eq!(loan.interest_rate_percent, INTEREST_RATE_PERCENT);
    }

    #[test]
    fn take_loan_zero_duration_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, 0);
        let err = process_take_loan(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            Money::from_lira(1_000).unwrap(),
            0,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duration"));
        assert!(s.loans.is_empty());
    }

    #[test]
    fn repay_loan_deducts_total_due_and_removes_loan() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, 2_000);
        process_take_loan(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            Money::from_lira(1_000).unwrap(),
            20,
        )
        .unwrap();
        let lid = *s.loans.keys().next().unwrap();
        // Cash = 2000 (starter) + 1000 (loan) = 3000. Repay tick 5'te, total_due = 1150.
        process_repay_loan(&mut s, &mut r, Tick::new(5), pid, lid).unwrap();
        assert!(s.loans.is_empty());
        // 3000 - 1150 = 1850
        assert_eq!(s.players[&pid].cash, Money::from_lira(1_850).unwrap());
    }

    #[test]
    fn repay_loan_insufficient_funds_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, 0);
        process_take_loan(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            Money::from_lira(1_000).unwrap(),
            20,
        )
        .unwrap();
        let lid = *s.loans.keys().next().unwrap();
        // Oyuncu tüm nakti harcadı varsayalım → cash = 0.
        s.players.get_mut(&pid).unwrap().cash = Money::ZERO;
        let err = process_repay_loan(&mut s, &mut r, Tick::new(5), pid, lid).unwrap_err();
        assert!(err.to_string().contains("insufficient"));
        assert_eq!(s.loans.len(), 1); // hâlâ açık
    }

    #[test]
    fn repay_loan_wrong_borrower_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let p1 = add_player(&mut s, 1, 2_000);
        let _p2 = add_player(&mut s, 2, 2_000);
        process_take_loan(
            &mut s,
            &mut r,
            Tick::new(1),
            p1,
            Money::from_lira(1_000).unwrap(),
            20,
        )
        .unwrap();
        let lid = *s.loans.keys().next().unwrap();
        let err =
            process_repay_loan(&mut s, &mut r, Tick::new(5), PlayerId::new(2), lid).unwrap_err();
        assert!(err.to_string().contains("borrower"));
    }

    #[test]
    fn overdue_auto_settles_when_cash_sufficient() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, 2_000);
        process_take_loan(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            Money::from_lira(1_000).unwrap(),
            5,
        )
        .unwrap();
        // due_tick = 6. Tick 6'da auto-settle.
        let mut r6 = TickReport::new(Tick::new(6));
        advance_loans(&mut s, &mut r6, Tick::new(6));
        assert!(s.loans.is_empty());
        // Cash = 3000 - 1150 = 1850.
        assert_eq!(s.players[&pid].cash, Money::from_lira(1_850).unwrap());
        let repaid = r6.entries.iter().any(|e| {
            matches!(
                &e.event,
                crate::report::LogEvent::LoanRepaid { on_time: false, .. }
            )
        });
        assert!(repaid);
    }

    #[test]
    fn overdue_insufficient_cash_defaults_and_seizes_what_exists() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, 0);
        process_take_loan(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            Money::from_lira(1_000).unwrap(),
            5,
        )
        .unwrap();
        // Cash şu an 1000 (principal'dan). Nakit yetsin diye 100'e düşür.
        s.players.get_mut(&pid).unwrap().cash = Money::from_lira(100).unwrap();

        let mut r6 = TickReport::new(Tick::new(6));
        advance_loans(&mut s, &mut r6, Tick::new(6));
        assert!(s.loans.is_empty());
        // Tüm nakit alındı.
        assert_eq!(s.players[&pid].cash, Money::ZERO);
        let defaulted = r6.entries.iter().any(|e| match &e.event {
            crate::report::LogEvent::LoanDefaulted {
                seized,
                unpaid_balance,
                ..
            } => {
                *seized == Money::from_lira(100).unwrap()
                    && *unpaid_balance == Money::from_lira(1_050).unwrap()
            }
            _ => false,
        });
        assert!(defaulted);
    }

    #[test]
    fn not_due_loans_are_not_auto_settled() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, 2_000);
        process_take_loan(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            Money::from_lira(1_000).unwrap(),
            20,
        )
        .unwrap();
        let mut r5 = TickReport::new(Tick::new(5));
        advance_loans(&mut s, &mut r5, Tick::new(5));
        // Tick 5 < due_tick (21), auto-settle tetiklenmemeli.
        assert_eq!(s.loans.len(), 1);
        assert!(r5.entries.is_empty());
    }
}
