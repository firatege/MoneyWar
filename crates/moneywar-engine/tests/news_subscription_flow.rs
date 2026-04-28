//! 4-tier abonelik integration smoke: tüm akışın `advance_tick` üstünden
//! geçerken çalıştığını doğrular — özellikle recurring debit, warning/downgrade
//! ve para korunumunu.

use moneywar_domain::{
    Command, GameState, Money, NewsTier, Player, PlayerId, Role, RoomConfig, RoomId,
};
use moneywar_engine::advance_tick;

fn fresh_state() -> GameState {
    GameState::new(RoomId::new(42), RoomConfig::hizli())
}

fn add_player(state: &mut GameState, id: u64, role: Role, cash_lira: i64) -> PlayerId {
    let p = Player::new(
        PlayerId::new(id),
        format!("P{id}"),
        role,
        Money::from_lira(cash_lira).unwrap(),
        false,
    )
    .unwrap();
    state.players.insert(p.id, p);
    PlayerId::new(id)
}

fn run_n_ticks(state: &mut GameState, n: u32) {
    for _ in 0..n {
        let (next, _report) = advance_tick(state, &[]).unwrap();
        *state = next;
    }
}

#[test]
fn gold_subscriber_pays_each_tick_until_broke_then_downgrades() {
    let mut s = fresh_state();
    // Sanayici 200₺ ile Gold abone — Gold = 40₺/tick.
    let pid = add_player(&mut s, 1, Role::Sanayici, 200);
    let cmd = Command::SubscribeNews {
        player: pid,
        tier: NewsTier::Gold,
    };
    let (next, _) = advance_tick(&s, &[cmd]).unwrap();
    s = next;
    // Aynı tick'in sonunda charge çalıştı: 200 - 40 = 160.
    assert_eq!(s.players[&pid].cash, Money::from_lira(160).unwrap());
    assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));

    // 4 tick daha → 160 → 120 → 80 → 40 → 0.
    run_n_ticks(&mut s, 4);
    assert_eq!(s.players[&pid].cash, Money::ZERO);
    assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));

    // Bir tick daha → cash 0 < 40, ilk uyarı (downgrade YOK).
    run_n_ticks(&mut s, 1);
    assert!(s.news_payment_warned.contains(&pid));
    assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));

    // Bir tick daha → ikinci shortfall, Free'ye düşer (silinmez, Free yazılır).
    run_n_ticks(&mut s, 1);
    assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Free));
    assert!(!s.news_payment_warned.contains(&pid));
}

#[test]
fn tuccar_bronze_default_charges_each_tick_but_modest() {
    // v3: Bronze artık ücretli (Tüccar 2₺/tick). Subscribe etmiyor → default Bronze.
    let mut s = fresh_state();
    let pid = add_player(&mut s, 1, Role::Tuccar, 100);
    run_n_ticks(&mut s, 50);
    // 50 tick × 2₺ = 100₺ → tam yetiyor → cash 0.
    assert_eq!(s.players[&pid].cash, Money::ZERO);
}

#[test]
fn explicit_free_subscriber_pays_nothing() {
    // Free'ye geçen oyuncu ücret ödemez.
    let mut s = fresh_state();
    let pid = add_player(&mut s, 1, Role::Tuccar, 100);
    s.news_subscriptions.insert(pid, NewsTier::Free);
    run_n_ticks(&mut s, 50);
    assert_eq!(s.players[&pid].cash, Money::from_lira(100).unwrap());
}

#[test]
fn money_conserved_across_subscription_lifecycle() {
    // Sanayici 1k ile Gold subscribe, recurring debit cash'i azaltıyor ama
    // hiçbir yere kaybolmuyor — debit edilen miktarın toplamı initial-final fark.
    let mut s = fresh_state();
    let pid = add_player(&mut s, 1, Role::Sanayici, 1_000);
    let initial_cash = s.players[&pid].cash;
    let cmd = Command::SubscribeNews {
        player: pid,
        tier: NewsTier::Silver,
    };
    let (next, _) = advance_tick(&s, &[cmd]).unwrap();
    s = next;
    run_n_ticks(&mut s, 10);

    // 11 tick × 15₺ = 165₺ debit edilmeliydi (subscribe tick'i + 10 sonraki).
    let final_cash = s.players[&pid].cash;
    let drained = initial_cash.checked_sub(final_cash).unwrap();
    assert_eq!(drained, Money::from_lira(11 * 15).unwrap());
}

#[test]
fn payment_resumes_clears_warning_flag() {
    let mut s = fresh_state();
    let pid = add_player(&mut s, 1, Role::Sanayici, 30);
    // Manuel olarak Gold + warning kur.
    s.news_subscriptions.insert(pid, NewsTier::Gold);
    s.news_payment_warned.insert(pid);
    // Cash'i Gold tick fee'sini karşılayacak şekilde artır.
    s.players.get_mut(&pid).unwrap().cash = Money::from_lira(500).unwrap();

    run_n_ticks(&mut s, 1);
    // Tick debit'i (40₺) çekildi, uyarı temizlendi.
    assert!(!s.news_payment_warned.contains(&pid));
    assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));
    assert_eq!(s.players[&pid].cash, Money::from_lira(460).unwrap());
}
