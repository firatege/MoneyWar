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
//! | **5.5** ✅ | `loans::{process_take_loan, process_repay_loan, advance_loans}` — NPC banka |
//! | **6** ✅ | `news::process_subscribe_news` + `events::advance_events` — haber + olay motoru |
//! | **7** ✅ | `scoring::{score_player, leaderboard}` — skor formülü + sıralama |
//! | **8** ✅ | `moneywar-npc::{NpcBehavior, MarketMaker, decide_all_npcs}` — NPC iskelet |

use crate::{
    contracts::{
        advance_contracts, process_accept_contract as accept_contract_impl,
        process_cancel_contract as cancel_contract_impl,
        process_propose_contract as propose_contract_impl,
    },
    error::EngineError,
    events::advance_events,
    loans::{
        advance_loans, process_repay_loan as repay_loan_impl, process_take_loan as take_loan_impl,
    },
    market::clear_markets,
    news::process_subscribe_news as subscribe_news_impl,
    production::{advance_production, process_build_factory as build_factory_impl},
    report::{LogEntry, TickReport},
    rng::rng_for,
    transport::{
        advance_caravans, process_buy_caravan as buy_caravan_impl,
        process_dispatch_caravan as dispatch_caravan_impl,
    },
};
use moneywar_domain::{
    CityId, Command, DomainError, GameState, MarketOrder, Money, OrderId, PlayerId, ProductKind,
    Tick,
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
    // Deterministik RNG — `(room_id, tick)`'ten türetilir. Faz 6 olay motoru
    // bu RNG'yi kullanır.
    let mut rng = rng_for(state.room_id, next_tick);
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

    // Tick kapanışı sırası (Faz 6):
    //   0a. Aktif piyasa şoklarından expire olanları temizle (yeni olay
    //       eklenmeden — yeni olayın kendi etkisi tabii ki kalır).
    //   0b. Olay motoru — RNG ile yeni olay tetikle, abonelere haber dağıt.
    //       Fiyat şokunu da aynı yerde kaydeder.
    //   1. Üretim
    //   2. Taşıma
    //   3. Kontratlar (fulfill/breach)
    //   4. Krediler (vadesi gelen auto-settle)
    //   5. Hal Pazarı clearing
    new_state.clear_expired_shocks(next_tick);
    advance_events(&mut new_state, &mut rng, &mut report, next_tick);
    advance_production(&mut new_state, &mut report, next_tick);
    advance_caravans(&mut new_state, &mut report, next_tick);
    advance_contracts(&mut new_state, &mut report, next_tick);
    advance_loans(&mut new_state, &mut report, next_tick);
    crate::news::charge_news_subscriptions(&mut new_state, &mut report, next_tick);
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
        } => process_cancel_order(state, report, tick, *order_id, *requester),
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
        Command::SubscribeNews { player, tier } => {
            subscribe_news_impl(state, report, tick, *player, *tier)
        }
        Command::TakeLoan {
            player,
            amount,
            duration_ticks,
        } => take_loan_impl(state, report, tick, *player, *amount, *duration_ticks),
        Command::RepayLoan { player, loan_id } => {
            repay_loan_impl(state, report, tick, *player, *loan_id)
        }
        Command::CreditNpcCash { player, amount } => {
            credit_npc_cash_impl(state, *player, *amount)
        }
    }
}

/// `CreditNpcCash` — NPC'nin nakdine periyodik enjeksiyon (talep döngüsü).
/// Sadece `is_npc=true` oyuncular için kabul, insan oyuncudan gelirse reddedilir.
fn credit_npc_cash_impl(
    state: &mut GameState,
    player: PlayerId,
    amount: moneywar_domain::Money,
) -> Result<(), EngineError> {
    let p = state.players.get_mut(&player).ok_or_else(|| {
        EngineError::Domain(moneywar_domain::DomainError::Validation(format!(
            "player {player} not found"
        )))
    })?;
    if !p.is_npc {
        return Err(EngineError::Domain(
            moneywar_domain::DomainError::Validation(
                "CreditNpcCash sadece NPC için".into(),
            ),
        ));
    }
    if amount.is_negative() || amount.is_zero() {
        return Err(EngineError::Domain(
            moneywar_domain::DomainError::Validation(
                "CreditNpcCash amount > 0 olmalı".into(),
            ),
        ));
    }
    p.credit(amount)?;
    Ok(())
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
/// - Aynı `OrderId` zaten book'ta ise reddet (idempotency).
/// - **Relist cooldown** (Faz 2): `(player, city, product)` için
///   `current_tick < earliest_allowed_tick` ise reddet.
///
/// Eşleşme bu fonksiyonda değil, tick sonunda `clear_markets` içinde (Faz 3B).
fn process_submit_order(
    state: &mut GameState,
    order: &MarketOrder,
    tick: Tick,
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

    // Relist cooldown check.
    let key = (order.player, order.city, order.product);
    if let Some(&allowed_tick) = state.relist_cooldown.get(&key) {
        if tick.is_before(allowed_tick) {
            let ticks_left = allowed_tick.value().saturating_sub(tick.value());
            return Err(EngineError::Domain(DomainError::Validation(format!(
                "relist cooldown active for {} {} at {}: {} tick kaldı",
                order.player, order.product, order.city, ticks_left
            ))));
        }
        // Cooldown dolmuş → temizle (bu emir onu tüketecek, yeni cooldown ileri
        // yazılacak emir bittikçe).
        state.relist_cooldown.remove(&key);
    }

    state
        .order_book
        .entry((order.city, order.product))
        .or_default()
        .push(order.clone());
    Ok(())
}

/// `(player, city, product)` için cooldown'u `tick + ticks`'e çeker.
/// Emir `bittiğinde` (expire / cancel / full fill) çağrılır.
pub(crate) fn set_relist_cooldown(
    state: &mut GameState,
    player: PlayerId,
    city: CityId,
    product: ProductKind,
    tick: Tick,
) {
    let ticks = state.config.balance.relist_cooldown_ticks;
    if ticks == 0 {
        return;
    }
    let until = tick.checked_add(ticks).unwrap_or(Tick::new(u32::MAX));
    state.relist_cooldown.insert((player, city, product), until);
}

/// `CancelOrder` — book'tan emri çıkarır (tick açılmadan önce geri çekme).
///
/// Sahiplik kontrolü: `requester == order.player` olmalı. Emir bulunamazsa
/// veya başkasının emri ise `Validation` hatası döner; `CommandRejected`
/// log kaydına yazılır, state bozulmaz.
///
/// **Erken çekme cezası** (Faz 2): `penalty = notional × pct / 100 × remaining/ttl`.
/// Ceza oyuncunun nakitinden düşülür; yetmezse mevcut nakit kadar (0'a indirilmez
/// — Money `ZERO` olsa da debit saturate etmeli). `OrderCancelled` event'iyle
/// kaydedilir.
fn process_cancel_order(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    order_id: OrderId,
    requester: PlayerId,
) -> Result<(), EngineError> {
    // Önce emri bul — tam kopyasını al ki ceza hesabı + event için kullanabilelim.
    let mut target: Option<((CityId, ProductKind), MarketOrder)> = None;
    for (key, orders) in &state.order_book {
        if let Some(o) = orders.iter().find(|o| o.id == order_id) {
            target = Some((*key, o.clone()));
            break;
        }
    }

    let Some((key, order)) = target else {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "order {order_id} not found"
        ))));
    };

    if order.player != requester {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "order {order_id} owned by {}, not {requester}",
            order.player
        ))));
    }

    // Ceza: notional × pct / 100 × remaining / ttl.
    let balance = state.config.balance;
    let penalty_cents: i64 = if balance.cancel_penalty_pct > 0 && order.ttl_ticks > 0 {
        let notional = order
            .unit_price
            .as_cents()
            .saturating_mul(i64::from(order.quantity));
        // (notional × pct × remaining) / (100 × ttl)
        let scaled = notional
            .saturating_mul(i64::from(balance.cancel_penalty_pct))
            .saturating_mul(i64::from(order.remaining_ticks));
        let denom = 100_i64.saturating_mul(i64::from(order.ttl_ticks));
        if denom == 0 { 0 } else { scaled / denom }
    } else {
        0
    };
    let penalty = Money::from_cents(penalty_cents);

    // Ceza kadar nakit düş (saturate — yetersizse mevcut kadar çek).
    let applied_penalty = if let Some(player) = state.players.get_mut(&requester) {
        let available = player.cash.as_cents();
        let take = available.min(penalty_cents).max(0);
        if take > 0 {
            let _ = player.debit(Money::from_cents(take));
        }
        Money::from_cents(take)
    } else {
        Money::ZERO
    };

    // Kitaptan emri çıkar.
    let orders = state
        .order_book
        .get_mut(&key)
        .expect("key came from the book itself");
    orders.retain(|o| o.id != order_id);
    if orders.is_empty() {
        state.order_book.remove(&key);
    }

    report.push(LogEntry::order_cancelled(
        tick,
        order_id,
        requester,
        key.0,
        key.1,
        order.side,
        order.quantity,
        order.remaining_ticks,
        order.ttl_ticks,
        applied_penalty,
    ));

    // Relist cooldown: aynı (player, city, product) tekrar emir veremez.
    set_relist_cooldown(state, requester, key.0, key.1, tick);

    // Ceza istenen miktardan az çekildiyse kayıt dursun — silent değil.
    if applied_penalty.as_cents() < penalty.as_cents() {
        report.push(LogEntry::command_rejected(
            tick,
            requester,
            Command::CancelOrder {
                order_id,
                requester,
            },
            format!("cancel penalty partially applied: wanted {penalty}, took {applied_penalty}"),
        ));
    }
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
        assert_eq!(report.tick, Tick::new(1));
        // Faz 6: olay motoru RNG ile event üretebilir; komut sayısı sıfır
        // olduğu için accepted/rejected sıfır olmalı — sistem event'leri olabilir.
        assert_eq!(report.accepted_count(), 0);
        assert_eq!(report.rejected_count(), 0);
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
    fn cancel_ttl1_order_at_full_ttl_charges_full_penalty() {
        // TTL=1 emir, hemen cancel → remaining=1, ttl=1, ratio=1.
        // 10 birim × 5₺ = 50₺ notional. Default penalty %2 → 1₺ = 100 cent.
        let mut s0 = state();
        // Oyuncuya nakit ver (varsayılan player yok, seed edelim).
        let mut player = moneywar_domain::Player::new(
            PlayerId::new(7),
            "P7",
            moneywar_domain::Role::Tuccar,
            moneywar_domain::Money::from_lira(1_000).unwrap(),
            false,
        )
        .unwrap();
        let _ = player
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100);
        s0.players.insert(player.id, player);

        let cmds = vec![submit_order(7, 1), cancel_order(7, 1)];
        let (s1, report) = advance_tick(&s0, &cmds).unwrap();

        // Ceza event'i var mı?
        let penalty_ev = report
            .entries
            .iter()
            .find_map(|e| match &e.event {
                crate::report::LogEvent::OrderCancelled { penalty, .. } => Some(*penalty),
                _ => None,
            })
            .expect("OrderCancelled event");
        // 10 × 500 = 5000 cent × 2% = 100 cent = 1₺.
        assert_eq!(penalty_ev, moneywar_domain::Money::from_cents(100));

        // Oyuncu cash'i: 1000₺ − 1₺ (ceza) − 2₺ (Tüccar Bronze tick fee) = 997₺.
        let final_cash = s1.players[&PlayerId::new(7)].cash;
        assert_eq!(final_cash, moneywar_domain::Money::from_lira(997).unwrap());
    }

    #[test]
    fn cancel_caps_penalty_to_available_cash() {
        // Cezayı karşılayamayacak kadar az nakit → saturate (mevcut kadar çek).
        let mut s0 = state();
        let mut player = moneywar_domain::Player::new(
            PlayerId::new(8),
            "P8",
            moneywar_domain::Role::Tuccar,
            moneywar_domain::Money::from_cents(50), // 0.50₺ — tam ceza 1₺, 50 cent keseriz
            false,
        )
        .unwrap();
        let _ = player
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100);
        s0.players.insert(player.id, player);

        let cmds = vec![submit_order(8, 1), cancel_order(8, 1)];
        let (s1, _report) = advance_tick(&s0, &cmds).unwrap();
        assert_eq!(
            s1.players[&PlayerId::new(8)].cash,
            moneywar_domain::Money::ZERO
        );
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
