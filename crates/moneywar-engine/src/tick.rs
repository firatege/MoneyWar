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
//! | 3B | Tick sonu batch auction + uniform clearing price + settlement |
//! | 4 | `process_build_factory`, `process_buy_caravan`, `process_dispatch_caravan` |
//! | 5 | `process_propose_contract`, `process_accept_contract`, `process_cancel_contract` |
//! | 5.5 | `process_take_loan`, `process_repay_loan` |
//! | 6 | `process_subscribe_news` + sistem eventleri |

use moneywar_domain::{
    CaravanId, CityId, Command, ContractId, ContractProposal, DomainError, GameState, LoanId,
    MarketOrder, Money, NewsTier, OrderId, PlayerId, ProductKind, Tick,
};
use rand_chacha::ChaCha8Rng;

use crate::{
    error::EngineError,
    report::{LogEntry, TickReport},
    rng::rng_for,
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
    let mut rng = rng_for(state.room_id, next_tick);
    let mut report = TickReport::new(next_tick);

    for cmd in commands {
        let actor = command_actor(&new_state, cmd);
        match dispatch(&mut new_state, cmd, &mut rng, next_tick) {
            Ok(()) => report.push(LogEntry::command_accepted(next_tick, actor, cmd.clone())),
            Err(err) => report.push(LogEntry::command_rejected(
                next_tick,
                actor,
                cmd.clone(),
                err.to_string(),
            )),
        }
    }

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

/// Command variant'ına göre doğru `process_*` stub'ını seçer.
///
/// Her variant ayrı fonksiyona gider — ileride dolacak mantık izole olsun,
/// test'te tek tek unit edilebilsin diye.
fn dispatch(
    state: &mut GameState,
    cmd: &Command,
    rng: &mut ChaCha8Rng,
    tick: Tick,
) -> Result<(), EngineError> {
    match cmd {
        Command::SubmitOrder(order) => process_submit_order(state, order, tick),
        Command::CancelOrder {
            order_id,
            requester,
        } => process_cancel_order(state, *order_id, *requester),
        Command::ProposeContract(proposal) => process_propose_contract(state, proposal, tick),
        Command::AcceptContract {
            contract_id,
            acceptor,
        } => process_accept_contract(state, *contract_id, *acceptor, tick),
        Command::CancelContractProposal {
            contract_id,
            requester,
        } => process_cancel_contract(state, *contract_id, *requester),
        Command::BuildFactory {
            owner,
            city,
            product,
        } => process_build_factory(state, *owner, *city, *product, rng, tick),
        Command::BuyCaravan {
            owner,
            starting_city,
        } => process_buy_caravan(state, *owner, *starting_city),
        Command::DispatchCaravan {
            caravan_id,
            from,
            to,
            cargo,
        } => process_dispatch_caravan(state, *caravan_id, *from, *to, cargo.clone(), tick),
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
fn process_propose_contract(
    _state: &mut GameState,
    _proposal: &ContractProposal,
    _tick: Tick,
) -> Result<(), EngineError> {
    // FAZ 5: escrow kilitle, contracts'a ekle, ID üret.
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn process_accept_contract(
    _state: &mut GameState,
    _contract_id: ContractId,
    _acceptor: PlayerId,
    _tick: Tick,
) -> Result<(), EngineError> {
    // FAZ 5: karşı taraf kaporası kilitle, state'i Active'e al.
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn process_cancel_contract(
    _state: &mut GameState,
    _contract_id: ContractId,
    _requester: PlayerId,
) -> Result<(), EngineError> {
    // FAZ 5: escrow iade, state Cancelled.
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn process_build_factory(
    _state: &mut GameState,
    _owner: PlayerId,
    _city: CityId,
    _product: ProductKind,
    _rng: &mut ChaCha8Rng,
    _tick: Tick,
) -> Result<(), EngineError> {
    // FAZ 4: maliyet düş, Factory kur, ID üret, Sanayici tekel kontrolü.
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn process_buy_caravan(
    _state: &mut GameState,
    _owner: PlayerId,
    _starting_city: CityId,
) -> Result<(), EngineError> {
    // FAZ 4: kervan satın alma, kapasite rol'e göre.
    Ok(())
}

#[allow(clippy::unnecessary_wraps)]
fn process_dispatch_caravan(
    _state: &mut GameState,
    _caravan_id: CaravanId,
    _from: CityId,
    _to: CityId,
    _cargo: moneywar_domain::CargoSpec,
    _tick: Tick,
) -> Result<(), EngineError> {
    // FAZ 4: Idle → EnRoute geçişi, yol süresi hesapla.
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
    use moneywar_domain::{CityId, MarketOrder, Money, OrderSide, RoomConfig, RoomId};

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

    #[test]
    fn submit_order_adds_to_book_and_logs_accepted() {
        let s0 = state();
        let cmd = submit_order(7, 1);
        let (s1, report) = advance_tick(&s0, std::slice::from_ref(&cmd)).unwrap();
        assert_eq!(report.accepted_count(), 1);
        assert_eq!(report.rejected_count(), 0);
        assert_eq!(report.entries[0].actor, Some(PlayerId::new(7)));

        let bucket = s1
            .order_book
            .get(&(CityId::Istanbul, ProductKind::Pamuk))
            .expect("bucket exists after submit");
        assert_eq!(bucket.len(), 1);
        assert_eq!(bucket[0].id, OrderId::new(1));
    }

    #[test]
    fn submit_orders_accumulate_in_same_bucket() {
        let s0 = state();
        let cmds = vec![submit_order(1, 1), submit_order(2, 2), submit_order(3, 3)];
        let (s1, _) = advance_tick(&s0, &cmds).unwrap();
        let bucket = s1
            .order_book
            .get(&(CityId::Istanbul, ProductKind::Pamuk))
            .unwrap();
        assert_eq!(bucket.len(), 3);
        // Insertion order preserved.
        let ids: Vec<_> = bucket.iter().map(|o| o.id.value()).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn submit_orders_to_different_pairs_split_buckets() {
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
        let (s1, _) = advance_tick(&s0, &[istanbul_pamuk, ankara_bugday]).unwrap();
        assert_eq!(s1.order_book.len(), 2);
        assert_eq!(
            s1.order_book[&(CityId::Istanbul, ProductKind::Pamuk)].len(),
            1
        );
        assert_eq!(
            s1.order_book[&(CityId::Ankara, ProductKind::Bugday)].len(),
            1
        );
    }

    #[test]
    fn duplicate_order_id_is_rejected() {
        let s0 = state();
        let (s1, _) = advance_tick(&s0, &[submit_order(1, 42)]).unwrap();
        // Tick 2'de aynı order_id'yi tekrar gönder → reddedilmeli.
        let (s2, report) = advance_tick(&s1, &[submit_order(1, 42)]).unwrap();
        assert_eq!(report.accepted_count(), 0);
        assert_eq!(report.rejected_count(), 1);
        // Book değişmemeli: hâlâ tek kayıt.
        assert_eq!(
            s2.order_book[&(CityId::Istanbul, ProductKind::Pamuk)].len(),
            1
        );
    }

    #[test]
    fn order_book_persists_across_ticks() {
        let s0 = state();
        let (s1, _) = advance_tick(&s0, &[submit_order(1, 1)]).unwrap();
        // Bir sonraki tick boş komutlarla geçer — book hâlâ dolu olmalı
        // (clearing Faz 3B'de gelecek).
        let (s2, _) = advance_tick(&s1, &[]).unwrap();
        assert_eq!(
            s2.order_book[&(CityId::Istanbul, ProductKind::Pamuk)].len(),
            1
        );
    }

    fn cancel_order(requester: u64, order_id: u64) -> Command {
        Command::CancelOrder {
            order_id: OrderId::new(order_id),
            requester: PlayerId::new(requester),
        }
    }

    #[test]
    fn cancel_order_removes_and_cleans_empty_bucket() {
        let s0 = state();
        let (s1, _) = advance_tick(&s0, &[submit_order(7, 1)]).unwrap();
        let (s2, report) = advance_tick(&s1, &[cancel_order(7, 1)]).unwrap();
        assert_eq!(report.accepted_count(), 1);
        // Bucket boş kalınca map'ten silinmeli.
        assert!(s2.order_book.is_empty());
    }

    #[test]
    fn cancel_order_preserves_other_orders_in_bucket() {
        let s0 = state();
        let (s1, _) = advance_tick(&s0, &[submit_order(1, 1), submit_order(2, 2)]).unwrap();
        let (s2, report) = advance_tick(&s1, &[cancel_order(1, 1)]).unwrap();
        assert_eq!(report.accepted_count(), 1);
        let bucket = &s2.order_book[&(CityId::Istanbul, ProductKind::Pamuk)];
        assert_eq!(bucket.len(), 1);
        assert_eq!(bucket[0].id, OrderId::new(2));
        assert_eq!(bucket[0].player, PlayerId::new(2));
    }

    #[test]
    fn cancel_order_wrong_owner_is_rejected() {
        let s0 = state();
        let (s1, _) = advance_tick(&s0, &[submit_order(7, 1)]).unwrap();
        // Başka oyuncu iptal etmeye çalışıyor.
        let (s2, report) = advance_tick(&s1, &[cancel_order(99, 1)]).unwrap();
        assert_eq!(report.rejected_count(), 1);
        // Book değişmemeli.
        assert_eq!(
            s2.order_book[&(CityId::Istanbul, ProductKind::Pamuk)].len(),
            1
        );
    }

    #[test]
    fn cancel_order_not_found_is_rejected() {
        let s0 = state();
        let (s1, report) = advance_tick(&s0, &[cancel_order(1, 999)]).unwrap();
        assert_eq!(report.rejected_count(), 1);
        assert!(s1.order_book.is_empty());
    }

    #[test]
    fn multiple_commands_preserve_order() {
        let s0 = state();
        let cmds = vec![submit_order(1, 1), submit_order(2, 2), submit_order(3, 3)];
        let (_s1, report) = advance_tick(&s0, &cmds).unwrap();
        assert_eq!(report.entries.len(), 3);
        let actors: Vec<_> = report.entries.iter().map(|e| e.actor).collect();
        assert_eq!(
            actors,
            vec![
                Some(PlayerId::new(1)),
                Some(PlayerId::new(2)),
                Some(PlayerId::new(3)),
            ]
        );
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
