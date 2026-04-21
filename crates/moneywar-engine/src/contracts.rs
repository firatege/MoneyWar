//! Anlaşma Masası — bağlayıcı kontrat + escrow (§2 Katman 2, §7).
//!
//! State machine: `Proposed` → `Active` → (`Fulfilled` | `Breached`).
//!
//! # Escrow modeli
//!
//! - **Propose:** satıcı `seller_deposit`'i nakitinden düşer, motor tutar
//!   (Contract içinde mahfuz — `Player.cash` azalır, `Contract.seller_deposit`
//!   alanı taşır).
//! - **Accept:** alıcı `buyer_deposit`'i kilitler aynı şekilde.
//! - **Cancel:** satıcı yalnız `Proposed`'ta geri çekebilir, kaporasını geri alır.
//! - **Fulfill / Breach (5B):** teslimat tick'inde motor kararını verir.
//!
//! Para korunumu: `sum(player.cash) + sum(active_contract_escrows)` sabit.

use moneywar_domain::{
    Contract, ContractId, ContractProposal, ContractState, DomainError, GameState, ListingKind,
    PlayerId, Tick,
};

use crate::{
    error::EngineError,
    report::{LogEntry, TickReport},
};

/// `ProposeContract` komutu. Satıcı nakti yeterli olmalı; deposit düşer,
/// kontrat `Proposed` state'iyle state'e eklenir.
pub(crate) fn process_propose_contract(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    proposal: &ContractProposal,
) -> Result<(), EngineError> {
    // Satıcı var mı?
    if !state.players.contains_key(&proposal.seller) {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "seller {} not found",
            proposal.seller
        ))));
    }
    // Personal kontratta target oyuncu da var olmalı.
    if let ListingKind::Personal { target } = proposal.listing {
        if !state.players.contains_key(&target) {
            return Err(EngineError::Domain(DomainError::Validation(format!(
                "personal target {target} not found"
            ))));
        }
    }

    // ID üret + Contract::propose ile validation.
    let contract_id = ContractId::new(state.counters.next_contract_id);
    let contract = Contract::propose(
        contract_id,
        proposal.seller,
        proposal.listing,
        proposal.product,
        proposal.quantity,
        proposal.unit_price,
        proposal.delivery_city,
        proposal.delivery_tick,
        tick,
        proposal.seller_deposit,
        proposal.buyer_deposit,
    )?;

    // Satıcı kaporasını escrow'a al.
    let seller = state
        .players
        .get_mut(&proposal.seller)
        .expect("checked above");
    if seller.cash < proposal.seller_deposit {
        return Err(EngineError::Domain(DomainError::InsufficientFunds {
            have: seller.cash,
            want: proposal.seller_deposit,
        }));
    }
    seller.debit(proposal.seller_deposit)?;

    state.counters.next_contract_id = state.counters.next_contract_id.saturating_add(1);
    state.contracts.insert(contract_id, contract);

    report.push(LogEntry::contract_proposed(
        tick,
        proposal.seller,
        contract_id,
        proposal.listing,
        proposal.product,
        proposal.quantity,
        proposal.unit_price,
        proposal.delivery_city,
        proposal.delivery_tick,
        proposal.seller_deposit,
        proposal.buyer_deposit,
    ));
    Ok(())
}

/// `AcceptContract` komutu. Alıcı kaporasını kilitler, kontrat `Active` olur.
pub(crate) fn process_accept_contract(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    contract_id: ContractId,
    acceptor: PlayerId,
) -> Result<(), EngineError> {
    if !state.players.contains_key(&acceptor) {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "acceptor {acceptor} not found"
        ))));
    }
    let contract = state.contracts.get_mut(&contract_id).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!(
            "contract {contract_id} not found"
        )))
    })?;

    // accept() state geçişi + target check'i yapar; önce cash validation.
    let buyer_deposit = contract.buyer_deposit;
    {
        let acceptor_ref = state.players.get(&acceptor).expect("checked above");
        if acceptor_ref.cash < buyer_deposit {
            return Err(EngineError::Domain(DomainError::InsufficientFunds {
                have: acceptor_ref.cash,
                want: buyer_deposit,
            }));
        }
    }

    let contract = state
        .contracts
        .get_mut(&contract_id)
        .expect("checked above");
    contract.accept(acceptor)?;

    let buyer = state.players.get_mut(&acceptor).expect("checked above");
    buyer.debit(buyer_deposit)?;

    report.push(LogEntry::contract_accepted(
        tick,
        acceptor,
        contract_id,
        buyer_deposit,
    ));
    Ok(())
}

/// `CancelContractProposal` komutu. Yalnız satıcı ve yalnız `Proposed` state.
/// Kapora iade edilir, kontrat silinir.
pub(crate) fn process_cancel_contract(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    contract_id: ContractId,
    requester: PlayerId,
) -> Result<(), EngineError> {
    let contract = state.contracts.get(&contract_id).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!(
            "contract {contract_id} not found"
        )))
    })?;

    if contract.seller != requester {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "only seller can cancel; contract {contract_id} owned by {}, not {requester}",
            contract.seller
        ))));
    }
    if contract.state != ContractState::Proposed {
        return Err(EngineError::Domain(DomainError::InvalidTransition {
            entity: "contract",
            from: "Active-or-later",
            to: "Cancelled",
        }));
    }

    let seller = contract.seller;
    let deposit = contract.seller_deposit;
    state.contracts.remove(&contract_id);

    // Kapora iade.
    if let Some(player) = state.players.get_mut(&seller) {
        player.credit(deposit)?;
    }

    report.push(LogEntry::contract_cancelled(
        tick,
        seller,
        contract_id,
        deposit,
    ));
    Ok(())
}

/// Tick sonu: `delivery_tick` geçmiş `Active` kontratları settle et.
///
/// **Fulfill şartları:** satıcının `delivery_city`'de yeterli `quantity`
/// stoğu var **ve** alıcının `total_value` kadar nakti var.
///
/// **Başarısa:** Satıcı malı teslim eder, alıcı ücreti öder, her iki taraf
/// kendi kaporasını geri alır.
///
/// **Başarısızlığa:** Breach. Breacher seçimi deterministik — önce stok
/// eksikliğine bakarız (satıcı suçlu), sonra nakit eksikliğine (alıcı suçlu).
/// Breacher kaporası karşı tarafa tazminat olarak gider; karşı taraf kendi
/// kaporasını da geri alır (§2 Katman 2).
///
/// Settled kontrat `state.contracts`'tan silinir (kayıt `TickReport`'ta duruyor).
pub(crate) fn advance_contracts(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let cids: Vec<ContractId> = state.contracts.keys().copied().collect();
    for cid in cids {
        let ready = state
            .contracts
            .get(&cid)
            .is_some_and(|c| matches!(c.state, ContractState::Active) && c.delivery_tick <= tick);
        if ready {
            settle_contract(state, report, tick, cid);
        }
    }
}

fn settle_contract(state: &mut GameState, report: &mut TickReport, tick: Tick, cid: ContractId) {
    // Contract alanlarını borrow tutmadan kopyala.
    let contract = state.contracts.get(&cid).expect("checked").clone();
    let seller = contract.seller;
    let Some(buyer) = contract.accepted_by else {
        // Active state olmalıydı; buraya gelmemeli.
        return;
    };
    let delivery_city = contract.delivery_city;
    let product = contract.product;
    let qty = contract.quantity;
    let seller_deposit = contract.seller_deposit;
    let buyer_deposit = contract.buyer_deposit;

    let Ok(total_value) = contract.total_value() else {
        // Overflow edge — alıcıyı breacher say, breach olarak kapat.
        let final_state =
            breach_contract(state, seller, buyer, buyer, seller_deposit, buyer_deposit);
        state.contracts.remove(&cid);
        report.push(LogEntry::contract_settled(tick, cid, final_state));
        return;
    };

    let seller_has_stock = state
        .players
        .get(&seller)
        .is_some_and(|p| p.inventory.get(delivery_city, product) >= qty);
    let buyer_has_cash = state
        .players
        .get(&buyer)
        .is_some_and(|p| p.cash >= total_value);

    if seller_has_stock && buyer_has_cash {
        let final_state = fulfill_contract(
            state,
            seller,
            buyer,
            delivery_city,
            product,
            qty,
            total_value,
            seller_deposit,
            buyer_deposit,
        );
        state.contracts.remove(&cid);
        report.push(LogEntry::contract_settled(tick, cid, final_state));
    } else {
        // Breacher: önce stok yokluğu (satıcı), sonra nakit yokluğu (alıcı).
        let breacher = if seller_has_stock { buyer } else { seller };
        let final_state = breach_contract(
            state,
            seller,
            buyer,
            breacher,
            seller_deposit,
            buyer_deposit,
        );
        state.contracts.remove(&cid);
        report.push(LogEntry::contract_settled(tick, cid, final_state));
    }
}

#[allow(clippy::too_many_arguments)]
fn fulfill_contract(
    state: &mut GameState,
    seller: PlayerId,
    buyer: PlayerId,
    delivery_city: moneywar_domain::CityId,
    product: moneywar_domain::ProductKind,
    qty: u32,
    total_value: moneywar_domain::Money,
    seller_deposit: moneywar_domain::Money,
    buyer_deposit: moneywar_domain::Money,
) -> ContractState {
    // Satıcı: stok düş + (satış bedeli + kendi kaporasının iadesi) kredile.
    if let Some(s) = state.players.get_mut(&seller) {
        s.inventory
            .remove(delivery_city, product, qty)
            .expect("pre-flight validated");
        let _ = s.credit(total_value);
        let _ = s.credit(seller_deposit);
    }
    // Alıcı: satış bedeli düş + kaporasının iadesi + malı al.
    if let Some(b) = state.players.get_mut(&buyer) {
        let _ = b.debit(total_value);
        let _ = b.credit(buyer_deposit);
        let _ = b.inventory.add(delivery_city, product, qty);
    }
    ContractState::Fulfilled
}

fn breach_contract(
    state: &mut GameState,
    seller: PlayerId,
    buyer: PlayerId,
    breacher: PlayerId,
    seller_deposit: moneywar_domain::Money,
    buyer_deposit: moneywar_domain::Money,
) -> ContractState {
    // Breacher: kaporası gider (kendi nakti değişmez, deposit zaten kilitliydi).
    // Karşı taraf: kendi kaporası + breacher'ın kaporası (tazminat).
    let (winner, winner_own, breacher_own) = if breacher == seller {
        (buyer, buyer_deposit, seller_deposit)
    } else {
        (seller, seller_deposit, buyer_deposit)
    };
    if let Some(w) = state.players.get_mut(&winner) {
        let _ = w.credit(winner_own);
        let _ = w.credit(breacher_own);
    }
    ContractState::Breached { breacher }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CityId, ContractProposal, ListingKind, Money, Player, PlayerId, ProductKind, Role,
        RoomConfig, RoomId,
    };

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

    fn proposal(seller: u64, listing: ListingKind, deposit_lira: i64) -> ContractProposal {
        ContractProposal {
            seller: PlayerId::new(seller),
            listing,
            product: ProductKind::Kumas,
            quantity: 10,
            unit_price: Money::from_lira(20).unwrap(),
            delivery_city: CityId::Istanbul,
            delivery_tick: Tick::new(10),
            seller_deposit: Money::from_lira(deposit_lira).unwrap(),
            buyer_deposit: Money::from_lira(deposit_lira).unwrap(),
        }
    }

    #[test]
    fn propose_creates_contract_and_locks_seller_deposit() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);

        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap();

        assert_eq!(s.contracts.len(), 1);
        let c = s.contracts.values().next().unwrap();
        assert_eq!(c.state, ContractState::Proposed);
        // 1000 - 100 = 900
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            Money::from_lira(900).unwrap()
        );
    }

    #[test]
    fn propose_insufficient_funds_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 50); // < 100 deposit

        let err = process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap_err();
        assert!(err.to_string().contains("insufficient"));
        assert!(s.contracts.is_empty());
        // Nakit dokunulmadı.
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            Money::from_lira(50).unwrap()
        );
    }

    #[test]
    fn propose_seller_not_found_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let err = process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(99, ListingKind::Public, 100),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn personal_contract_requires_target_exists() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        let err = process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(
                1,
                ListingKind::Personal {
                    target: PlayerId::new(99),
                },
                100,
            ),
        )
        .unwrap_err();
        assert!(err.to_string().contains("target"));
    }

    #[test]
    fn accept_locks_buyer_deposit_and_transitions_to_active() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 1_000);

        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap();
        let cid = *s.contracts.keys().next().unwrap();
        process_accept_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(2)).unwrap();

        assert_eq!(s.contracts[&cid].state, ContractState::Active);
        assert_eq!(s.contracts[&cid].accepted_by, Some(PlayerId::new(2)));
        // Alıcı nakti 1000 - 100 = 900
        assert_eq!(
            s.players[&PlayerId::new(2)].cash,
            Money::from_lira(900).unwrap()
        );
    }

    #[test]
    fn accept_seller_own_contract_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap();
        let cid = *s.contracts.keys().next().unwrap();

        let err = process_accept_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(1))
            .unwrap_err();
        // Domain returns Validation "seller cannot accept own contract".
        assert!(err.to_string().contains("seller"));
    }

    #[test]
    fn accept_personal_wrong_buyer_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 1_000);
        add_player(&mut s, 3, 1_000);

        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(
                1,
                ListingKind::Personal {
                    target: PlayerId::new(2),
                },
                100,
            ),
        )
        .unwrap();
        let cid = *s.contracts.keys().next().unwrap();

        // Yanlış alıcı
        let err = process_accept_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(3))
            .unwrap_err();
        assert!(err.to_string().contains("personal"));
        // Doğru alıcı
        process_accept_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(2)).unwrap();
    }

    #[test]
    fn accept_insufficient_buyer_funds_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 50); // < 100

        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap();
        let cid = *s.contracts.keys().next().unwrap();

        let err = process_accept_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(2))
            .unwrap_err();
        assert!(err.to_string().contains("insufficient"));
        // Contract hâlâ Proposed.
        assert_eq!(s.contracts[&cid].state, ContractState::Proposed);
    }

    #[test]
    fn cancel_proposed_refunds_seller_deposit() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap();
        let cid = *s.contracts.keys().next().unwrap();

        process_cancel_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(1)).unwrap();
        assert!(s.contracts.is_empty());
        // Nakit geri: 1000
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            Money::from_lira(1_000).unwrap()
        );
    }

    #[test]
    fn cancel_by_non_seller_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 1_000);
        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap();
        let cid = *s.contracts.keys().next().unwrap();

        let err = process_cancel_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(2))
            .unwrap_err();
        assert!(err.to_string().contains("only seller"));
        assert!(!s.contracts.is_empty());
    }

    #[test]
    fn cancel_active_contract_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 1_000);

        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap();
        let cid = *s.contracts.keys().next().unwrap();
        process_accept_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(2)).unwrap();

        let err = process_cancel_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(1))
            .unwrap_err();
        assert!(err.to_string().contains("transition"), "got: {err}");
        assert_eq!(s.contracts[&cid].state, ContractState::Active);
    }

    #[test]
    fn money_conservation_proposal_lock_cancel() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        let total_before: i64 = s.players.values().map(|p| p.cash.as_cents()).sum();

        process_propose_contract(
            &mut s,
            &mut r,
            Tick::new(1),
            &proposal(1, ListingKind::Public, 100),
        )
        .unwrap();
        // Escrow kilitli: cash düştü, ama contract.seller_deposit'de duruyor.
        let cash_after_propose: i64 = s.players.values().map(|p| p.cash.as_cents()).sum();
        let escrow: i64 = s
            .contracts
            .values()
            .map(|c| c.seller_deposit.as_cents() + c.buyer_deposit.as_cents())
            .sum();
        // buyer_deposit henüz oyuncudan kilitlenmedi (Proposed'ta sadece
        // kontrat alanında tanımlı). Yani toplam = cash_after + seller_deposit.
        let seller_deposit: i64 = s
            .contracts
            .values()
            .map(|c| c.seller_deposit.as_cents())
            .sum();
        assert_eq!(total_before, cash_after_propose + seller_deposit);
        let _ = escrow;

        let cid = *s.contracts.keys().next().unwrap();
        process_cancel_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(1)).unwrap();
        let total_after: i64 = s.players.values().map(|p| p.cash.as_cents()).sum();
        assert_eq!(total_before, total_after);
    }
}
