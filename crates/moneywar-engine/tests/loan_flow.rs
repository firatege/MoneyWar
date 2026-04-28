//! Integration: NPC kredi — manuel geri ödeme ve otomatik default.

use moneywar_domain::{
    Command, GameState, LoanId, Money, NewsTier, Player, PlayerId, Role, RoomConfig, RoomId,
};
use moneywar_engine::advance_tick;

fn init_state() -> GameState {
    let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
    let p = Player::new(
        PlayerId::new(1),
        "Borçlu",
        Role::Tuccar,
        Money::from_lira(100).unwrap(),
        false,
    )
    .unwrap();
    s.players.insert(p.id, p);
    // Bu test kredi akışını izole eder; news fee dengesini bozmasın.
    s.news_subscriptions.insert(PlayerId::new(1), NewsTier::Free);
    s
}

#[test]
fn loan_manual_repay_with_sufficient_funds() {
    let mut s = init_state();
    // Başlangıç nakti 2000 olsun ki repay için yeter.
    s.players.get_mut(&PlayerId::new(1)).unwrap().cash = Money::from_lira(2_000).unwrap();

    let (s1, _) = advance_tick(
        &s,
        &[Command::TakeLoan {
            player: PlayerId::new(1),
            amount: Money::from_lira(1_000).unwrap(),
            duration_ticks: 10,
        }],
    )
    .unwrap();
    let lid = *s1.loans.keys().next().unwrap();

    // Tick 5'te repay.
    let mut cur = s1;
    for _ in 0..3 {
        cur = advance_tick(&cur, &[]).unwrap().0;
    }
    let (s5, r5) = advance_tick(
        &cur,
        &[Command::RepayLoan {
            player: PlayerId::new(1),
            loan_id: lid,
        }],
    )
    .unwrap();
    assert!(s5.loans.is_empty());
    // Cash: 2000 + 1000 - 1150 = 1850
    assert_eq!(
        s5.players[&PlayerId::new(1)].cash,
        Money::from_lira(1_850).unwrap()
    );
    let repaid = r5.entries.iter().any(|e| {
        matches!(
            &e.event,
            moneywar_engine::LogEvent::LoanRepaid { on_time: true, .. }
        )
    });
    assert!(repaid);
}

#[test]
fn overdue_loan_auto_settles_as_default_when_cash_insufficient() {
    let s0 = init_state();
    // Cash starter = 100₺. Loan 1000₺ al → cash 1100. Sonra total_due=1150 ama oyuncu parayı harcadı...
    // Burada basit: principal'ı al, tick geçtikçe nakit artmıyor, due_tick'te
    // 1150 gerekli ama 1100 var → default.
    let (s1, _) = advance_tick(
        &s0,
        &[Command::TakeLoan {
            player: PlayerId::new(1),
            amount: Money::from_lira(1_000).unwrap(),
            duration_ticks: 3,
        }],
    )
    .unwrap();
    let lid: LoanId = *s1.loans.keys().next().unwrap();

    // Tick 2: bekle. Tick 3: bekle. Tick 4 = due_tick (1+3=4) → auto-settle.
    let s2 = advance_tick(&s1, &[]).unwrap().0;
    let s3 = advance_tick(&s2, &[]).unwrap().0;
    let (s4, r4) = advance_tick(&s3, &[]).unwrap();

    assert!(s4.loans.is_empty(), "overdue loan removed");
    // Cash tümü çekildi → 0.
    assert_eq!(s4.players[&PlayerId::new(1)].cash, Money::ZERO);

    let defaulted = r4.entries.iter().any(|e| match &e.event {
        moneywar_engine::LogEvent::LoanDefaulted {
            seized,
            unpaid_balance,
            loan_id,
            ..
        } => {
            *loan_id == lid
                && *seized == Money::from_lira(1_100).unwrap()
                && *unpaid_balance == Money::from_lira(50).unwrap()
        }
        _ => false,
    });
    assert!(defaulted, "expected LoanDefaulted event");
}
