//! `advance_tick` — motorun tek ve saf giriş noktası.
//!
//! ```text
//! advance_tick(state, commands) → (new_state, report)
//! ```
//!
//! **Saf fonksiyon garantisi:**
//! - `&GameState` okunur, yeni `GameState` döner (girdi mutate edilmez).
//! - I/O yok, `tokio` yok, `std::time` yok, global state yok.
//! - Rastgelelik sadece `rng_for(room_id, next_tick)` üstünden — aynı
//!   (state, commands) → bit-perfect aynı sonuç.
//!
//! Her `Command` variant'ı için ayrı `process_*` fonksiyonu vardır. Faz 2
//! iskeletinde hepsi `Ok(())` stub'ıydı; fazlar ilerledikçe gerçek mantıkla
//! dolar:
//!
//! | Faz | Durum |
//! |---|---|
//! | **3A** ✅ | `process_submit_order`, `process_cancel_order` — order book yönetimi |
//! | **3B** ✅ | Tick sonu batch auction — `market::clear_markets` + uniform clearing |
//! | **3C** ✅ | Settlement (cash/inventory), saturation eşiği, `price_history` |
//! | **4A** ✅ | `process_build_factory` + `production::advance_production` — fabrika + üretim |
//! | **4B** ✅ | `process_buy_caravan`/`process_dispatch_caravan` + `transport::advance_caravans` |
//! | **5** ✅ | `contracts::{process_propose/accept/cancel + advance_contracts}` — Anlaşma Masası |
//! | 5.5 | `process_take_loan`, `process_repay_loan` |
//! | 6 | `process_subscribe_news` + sistem eventleri |

use crate::{
    contracts::{
        advance_contracts, process_accept_contract as accept_contract_impl,
        process_cancel_contract as cancel_contract_impl,
        process_propose_contract as propose_contract_impl,
    },
    error::EngineError,
    market::clear_markets,
    production::{advance_production, process_build_factory as build_factory_impl},
    report::{LogEntry, TickReport},
    rng::rng_for,
    transport::{
        advance_caravans, process_buy_caravan as buy_caravan_impl,
        process_dispatch_caravan as dispatch_caravan_impl,
    },
};
use moneywar_domain::{
    CityId, Command, DomainError, GameState, LoanId, MarketOrder, Money, NewsTier, OrderId,
    PlayerId, ProductKind, Tick,
};

/// Motoru bir tick ileri sarar.
///
/// # Akış
/// 1. Yeni tick = `state.current_tick + 1` (saturating; `u32::MAX`'te durur).
/// 2. Deterministik RNG `(room_id, next_tick)`'ten türetilir.
/// 3. `commands` sırayla dispatch edilir; her komut için `LogEntry` üretilir.
/// 4. Yeni state ile rapor döner.
///
/// # Errors
///
/// Sadece motor invariantı ihlal edildiğinde `EngineError::Invariant` döner.
/// Komut-düzeyi hatalar state'i bozmaz — reddedilip log'a yazılır.
pub fn advance_tick(
    state: &GameState,
    commands: &[Command],
) -> Result<(GameState, TickReport), EngineError> {
    let next_tick = state.current_tick.next();
    let mut new_state = state.clone();
    // RNG şimdilik kullanılmıyor; Faz 6 olay/haber tetikleyicileri devreye
    // alındığında burada üretilip alt geçitlere iletilecek. Determinism
    // garanti için seed'i burada tutuyoruz.
    let _rng = rng_for(state.room_id, next_tick);
    let mut report = TickReport::new(next_tick);

    for cmd in commands {
        let actor = command_actor(&new_state, cmd);
        match dispatch(&mut new_state, &mut report, cmd, next_tick) {
            Ok(()) => report.push(LogEntry::command_accepted(next_tick, actor, cmd.clone())),
            Err(err) => report.push(LogEntry::command_rejected(
                next_tick,
                actor,
                cmd.clone(),
                err.to_string(),
            )),
        }
    }

    // Tick kapanışı sırası (Faz 5):
    //   1. Üretim — biten batch'ler envantere, yeni batch'ler başlar.
    //   2. Taşıma — varış zamanı gelen kervanlar hedef envanter'e boşalır.
    //   3. Kontratlar — delivery_tick'i gelenler fulfill/breach.
    //   4. Hal Pazarı clearing — post-production/transport/contract envanteri.
    advance_production(&mut new_state, &mut report, next_tick);
    advance_caravans(&mut new_state, &mut report, next_tick);
    advance_contracts(&mut new_state, &mut report, next_tick);
    clear_markets(&mut new_state, &mut report, next_tick);

    new_state.current_tick = next_tick;
    Ok((new_state, report))
}

/// Komutun aktörünü çözer.
///
/// `Command::requester()` çoğu variant için doğru cevabı verir; `DispatchCaravan`
/// için placeholder `PlayerId(0)` döner çünkü requester caravan'a bakılarak
/// bulunur. Burada lookup yapıyoruz — state'teki caravan'ın sahibi aktördür.
fn command_actor(state: &GameState, cmd: &Command) -> PlayerId {
    match cmd {
        Command::DispatchCaravan { caravan_id, .. } => state
            .caravans
            .get(caravan_id)
            .map_or_else(|| cmd.requester(), |c| c.owner),
        _ => cmd.requester(),
    }
}

/// Command variant'ına göre doğru `process_*` fonksiyonuna delege eder.
fn dispatch(
    state: &mut GameState,
    report: &mut TickReport,
    cmd: &Command,
    tick: Tick,
) -> Result<(), EngineError> {
    match cmd {
        Command::SubmitOrder(order) => process_submit_order(state, order, tick),
        Command::CancelOrder {
            order_id,
            requester,
        } => process_cancel_order(state, *order_id, *requester),
        Command::ProposeContract(proposal) => propose_contract_impl(state, report, tick, proposal),
        Command::AcceptContract {
            contract_id,
            acceptor,
        } => accept_contract_impl(state, report, tick, *contract_id, *acceptor),
        Command::CancelContractProposal {
            contract_id,
            requester,
        } => cancel_contract_impl(state, report, tick, *contract_id, *requester),
        Command::BuildFactory {
            owner,
            city,
            product,
        } => build_factory_impl(state, report, tick, *owner, *city, *product),
        Command::BuyCaravan {
            owner,
            starting_city,
        } => buy_caravan_impl(state, report, tick, *owner, *starting_city),
        Command::DispatchCaravan {
            caravan_id,
            from,
            to,
            cargo,
        } => dispatch_caravan_impl(state, report, tick, *caravan_id, *from, *to, cargo),
        Command::SubscribeNews { player, tier } => process_subscribe_news(state, *player, *tier),
        Command::TakeLoan {
            player,
            amount,
            duration_ticks,
        } => process_take_loan(state, *player, *amount, *duration_ticks, tick),
        Command::RepayLoan { player, loan_id } => process_repay_loan(state, *player, *loan_id),
    }
}

// ---------------------------------------------------------------------------
// Command işleyicileri. SubmitOrder / CancelOrder gerçek (Faz 3A); diğerleri
// hâlâ `Ok(())` stub'ı — imzalar sabit tutuldu ki ileri fazlarda çağrı yeri
// değişmesin.
// ---------------------------------------------------------------------------

/// `SubmitOrder` — emri order book'un `(city, product)` bucket'ına koyar.
///
/// Bluff alanı (§2) gereği cash/stok kilidi **yok** — validation minimal:
/// - `MarketOrder::new` zaten `quantity > 0` ve `unit_price > 0`'ı garantiler.
/// - Burada tek ek: aynı `OrderId` zaten book'ta ise reddet (idempotency).
///
/// Eşleşme bu fonksiyonda değil, tick sonunda `clear_markets` içinde (Faz 3B).
fn process_submit_order(
    state: &mut GameState,
    order: &MarketOrder,
    _tick: Tick,
) -> Result<(), EngineError> {
    let duplicate = state
        .order_book
        .values()
        .flatten()
        .any(|existing| existing.id == order.id);
    if duplicate {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "order {} already in book",
            order.id
        ))));
    }
    state
        .order_book
        .entry((order.city, order.product))
        .or_default()
        .push(order.clone());
    Ok(())
}

/// `CancelOrder` — book'tan emri çıkarır (tick açılmadan önce geri çekme).
///
/// Sahiplik kontrolü: `requester == order.player` olmalı. Emir bulunamazsa
/// veya başkasının emri ise `Validation` hatası döner; `CommandRejected`
/// log kaydına yazılır, state bozulmaz.
fn process_cancel_order(
    state: &mut GameState,
    order_id: OrderId,
    requester: PlayerId,
) -> Result<(), EngineError> {
    let mut target: Option<((CityId, ProductKind), PlayerId)> = None;
    for (key, orders) in &state.order_book {
        if let Some(o) = orders.iter().find(|o| o.id == order_id) {
            target = Some((*key, o.player));
            break;
        }
    }

    let Some((key, owner)) = target else {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "order {order_id} not found"
        ))));
    };

    if owner != requester {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "order {order_id} owned by {owner}, not {requester}"
        ))));
    }

    let orders = state
        .order_book
        .get_mut(&key)
        .expect("key came from the book itself");
    orders.retain(|o| o.id != order_id);
    if orders.is_empty() {
        state.order_book.remove(&key);
    }
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn process_subscribe_news(
    _state: &mut GameState,
    _player: PlayerId,
    _tier: NewsTier,
) -> Result<(), EngineError> {
    // FAZ 6: abonelik tier'ını güncelle, Tüccar için Silver bedava.
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn process_take_loan(
    _state: &mut GameState,
    _player: PlayerId,
    _amount: Money,
    _duration_ticks: u32,
    _tick: Tick,
) -> Result<(), EngineError> {
    // FAZ 5.5: kredi tablosundan faiz, cash'e yatır, Loan ekle.
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn process_repay_loan(
    _state: &mut GameState,
    _player: PlayerId,
    _loan_id: LoanId,
) -> Result<(), EngineError> {
    // FAZ 5.5: principal + faiz düş, Loan sil.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{CaravanId, CityId, MarketOrder, Money, OrderSide, RoomConfig, RoomId};

    fn state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn submit_order(player: u64, order_id: u64) -> Command {
        Command::SubmitOrder(
            MarketOrder::new(
                OrderId::new(order_id),
                PlayerId::new(player),
                CityId::Istanbul,
                ProductKind::Pamuk,
                OrderSide::Buy,
                10,
                Money::from_lira(5).unwrap(),
                Tick::new(1),
            )
            .unwrap(),
        )
    }

    #[test]
    fn empty_commands_advance_tick_by_one() {
        let s0 = state();
        let (s1, report) = advance_tick(&s0, &[]).unwrap();
        assert_eq!(s1.current_tick, Tick::new(1));
        assert!(report.entries.is_empty());
        assert_eq!(report.tick, Tick::new(1));
    }

    #[test]
    fn advance_does_not_mutate_input() {
        let s0 = state();
        let _ = advance_tick(&s0, &[submit_order(1, 1)]).unwrap();
        assert_eq!(s0.current_tick, Tick::ZERO);
    }

    #[test]
    fn tick_is_monotonically_increasing() {
        let s0 = state();
        let (s1, _) = advance_tick(&s0, &[]).unwrap();
        let (s2, _) = advance_tick(&s1, &[]).unwrap();
        let (s3, _) = advance_tick(&s2, &[]).unwrap();
        assert_eq!(s1.current_tick, Tick::new(1));
        assert_eq!(s2.current_tick, Tick::new(2));
        assert_eq!(s3.current_tick, Tick::new(3));
    }

    /// `submitted_buy_qty` toplamını raporun son `MarketCleared` event'inden okur.
    fn first_market_cleared(report: &crate::report::TickReport) -> &crate::report::LogEvent {
        report
            .entries
            .iter()
            .map(|e| &e.event)
            .find(|ev| matches!(ev, crate::report::LogEvent::MarketCleared { .. }))
            .expect("a MarketCleared event is expected")
    }

    #[test]
    fn submit_order_is_accepted_and_bucket_is_cleared_same_tick() {
        let s0 = state();
        let cmd = submit_order(7, 1);
        let (s1, report) = advance_tick(&s0, std::slice::from_ref(&cmd)).unwrap();
        assert_eq!(report.accepted_count(), 1);
        assert_eq!(report.entries[0].actor, Some(PlayerId::new(7)));

        // Tick bitince book temizlenir (§2: eşleşmeyen çöpe).
        assert!(s1.order_book.is_empty());

        // MarketCleared event emit edilmiş ve submitted_buy_qty=10 görüyor.
        match first_market_cleared(&report) {
            crate::report::LogEvent::MarketCleared {
                submitted_buy_qty,
                matched_qty,
                clearing_price,
                ..
            } => {
                assert_eq!(*submitted_buy_qty, 10);
                assert_eq!(*matched_qty, 0);
                assert_eq!(*clearing_price, None);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn multiple_submits_accumulate_into_clearing_totals() {
        let s0 = state();
        let cmds = vec![submit_order(1, 1), submit_order(2, 2), submit_order(3, 3)];
        let (s1, report) = advance_tick(&s0, &cmds).unwrap();
        assert!(s1.order_book.is_empty());
        match first_market_cleared(&report) {
            crate::report::LogEvent::MarketCleared {
                submitted_buy_qty, ..
            } => {
                // Her emir 10 birim, toplam 30.
                assert_eq!(*submitted_buy_qty, 30);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn different_city_product_buckets_clear_independently() {
        let s0 = state();
        let istanbul_pamuk = Command::SubmitOrder(
            MarketOrder::new(
                OrderId::new(1),
                PlayerId::new(1),
                CityId::Istanbul,
                ProductKind::Pamuk,
                OrderSide::Buy,
                10,
                Money::from_lira(5).unwrap(),
                Tick::new(1),
            )
            .unwrap(),
        );
        let ankara_bugday = Command::SubmitOrder(
            MarketOrder::new(
                OrderId::new(2),
                PlayerId::new(1),
                CityId::Ankara,
                ProductKind::Bugday,
                OrderSide::Sell,
                20,
                Money::from_lira(7).unwrap(),
                Tick::new(1),
            )
            .unwrap(),
        );
        let (s1, report) = advance_tick(&s0, &[istanbul_pamuk, ankara_bugday]).unwrap();
        assert!(s1.order_book.is_empty());

        // İki bucket → iki MarketCleared event.
        let cleared_count = report
            .entries
            .iter()
            .filter(|e| matches!(e.event, crate::report::LogEvent::MarketCleared { .. }))
            .count();
        assert_eq!(cleared_count, 2);
    }

    #[test]
    fn duplicate_order_id_in_same_tick_is_rejected() {
        let s0 = state();
        // Aynı tick içinde aynı order_id'nin ikinci gönderimi reddedilmeli
        // (ilki book'ta iken duplicate check devreye girer).
        let cmds = vec![submit_order(1, 42), submit_order(1, 42)];
        let (s1, report) = advance_tick(&s0, &cmds).unwrap();
        assert_eq!(report.accepted_count(), 1);
        assert_eq!(report.rejected_count(), 1);
        // Book yine clearing'te temizleniyor.
        assert!(s1.order_book.is_empty());
    }

    #[test]
    fn order_book_is_empty_after_each_tick() {
        let s0 = state();
        let (s1, _) = advance_tick(&s0, &[submit_order(1, 1)]).unwrap();
        assert!(s1.order_book.is_empty());
        // Bir sonraki tick de boş başlar (state'ten gelen boş book).
        let (s2, _) = advance_tick(&s1, &[]).unwrap();
        assert!(s2.order_book.is_empty());
    }

    fn cancel_order(requester: u64, order_id: u64) -> Command {
        Command::CancelOrder {
            order_id: OrderId::new(order_id),
            requester: PlayerId::new(requester),
        }
    }

    #[test]
    fn cancel_same_tick_removes_order_before_clearing() {
        let s0 = state();
        // Aynı tick: submit sonra cancel → clearing'e emir girmez.
        let cmds = vec![submit_order(7, 1), cancel_order(7, 1)];
        let (s1, report) = advance_tick(&s0, &cmds).unwrap();
        assert_eq!(report.accepted_count(), 2);
        assert!(s1.order_book.is_empty());
        // MarketCleared yok (book boşken clearing noop).
        let cleared = report
            .entries
            .iter()
            .filter(|e| matches!(e.event, crate::report::LogEvent::MarketCleared { .. }))
            .count();
        assert_eq!(cleared, 0);
    }

    #[test]
    fn cancel_one_of_two_keeps_the_other_in_clearing() {
        let s0 = state();
        let cmds = vec![submit_order(1, 1), submit_order(2, 2), cancel_order(1, 1)];
        let (s1, report) = advance_tick(&s0, &cmds).unwrap();
        assert!(s1.order_book.is_empty());
        match first_market_cleared(&report) {
            crate::report::LogEvent::MarketCleared {
                submitted_buy_qty, ..
            } => {
                // Sadece 2. emir (10 birim) clearing'e girdi.
                assert_eq!(*submitted_buy_qty, 10);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn cancel_with_wrong_owner_is_rejected_same_tick() {
        let s0 = state();
        let cmds = vec![submit_order(7, 1), cancel_order(99, 1)];
        let (s1, report) = advance_tick(&s0, &cmds).unwrap();
        assert_eq!(report.accepted_count(), 1);
        assert_eq!(report.rejected_count(), 1);
        // Emir book'ta kaldı → clearing'te görünmeli.
        assert!(s1.order_book.is_empty());
        match first_market_cleared(&report) {
            crate::report::LogEvent::MarketCleared {
                submitted_buy_qty, ..
            } => assert_eq!(*submitted_buy_qty, 10),
            _ => unreachable!(),
        }
    }

    #[test]
    fn cancel_order_not_found_is_rejected() {
        let s0 = state();
        let (s1, report) = advance_tick(&s0, &[cancel_order(1, 999)]).unwrap();
        assert_eq!(report.rejected_count(), 1);
        assert!(s1.order_book.is_empty());
    }

    #[test]
    fn command_log_ordering_preserved_before_clearing() {
        let s0 = state();
        let cmds = vec![submit_order(1, 1), submit_order(2, 2), submit_order(3, 3)];
        let (_s1, report) = advance_tick(&s0, &cmds).unwrap();
        // İlk üç entry CommandAccepted, sıra korunmuş.
        let first_three: Vec<_> = report.entries.iter().take(3).map(|e| e.actor).collect();
        assert_eq!(
            first_three,
            vec![
                Some(PlayerId::new(1)),
                Some(PlayerId::new(2)),
                Some(PlayerId::new(3)),
            ]
        );
        // Sonuncu entry sistem event (MarketCleared) → actor None.
        assert_eq!(report.entries.last().unwrap().actor, None);
    }

    #[test]
    fn same_input_same_output_deterministic() {
        let s0 = state();
        let cmds = vec![submit_order(1, 1), submit_order(2, 2)];
        let (s1a, r1a) = advance_tick(&s0, &cmds).unwrap();
        let (s1b, r1b) = advance_tick(&s0, &cmds).unwrap();
        assert_eq!(s1a, s1b);
        assert_eq!(r1a, r1b);
    }

    #[test]
    fn dispatch_caravan_actor_resolves_from_state() {
        // Caravan state'te yoksa fallback `PlayerId(0)` kullanılır.
        let s0 = state();
        let mut cargo = moneywar_domain::CargoSpec::new();
        cargo.add(ProductKind::Pamuk, 5).unwrap();
        let cmd = Command::DispatchCaravan {
            caravan_id: CaravanId::new(99),
            from: CityId::Istanbul,
            to: CityId::Izmir,
            cargo,
        };
        let (_s1, report) = advance_tick(&s0, &[cmd]).unwrap();
        assert_eq!(report.entries[0].actor, Some(PlayerId::new(0)));
    }
}
