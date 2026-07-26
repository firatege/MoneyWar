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

    // Ön kontroller — hiçbir mutasyondan önce, kısmi uygulama olmasın.
    let seller_ref = state
        .players
        .get(&proposal.seller)
        .expect("checked above");
    if seller_ref.cash < proposal.seller_deposit {
        return Err(EngineError::Domain(DomainError::InsufficientFunds {
            have: seller_ref.cash,
            want: proposal.seller_deposit,
        }));
    }
    // Stok escrow'u: satılan mal öneri anında kilitlenir.
    //
    // Eskiden stok yalnız **teslim** anında kontrol ediliyordu; satıcı arada
    // malı pazarda satabildiği için teslimde eli boş kalıyordu. NPC
    // kontratlarının %78'i böyle breach ile kapanıyor ve bu yüzden NPC
    // propose tamamen kapatılmıştı. Mal baştan kilitlenince satıcı stok
    // yüzünden breach *edemez* — kontrat gerçek bir taahhüt olur.
    let free_stock = seller_ref
        .inventory
        .get(proposal.delivery_city, proposal.product);
    if free_stock < proposal.quantity {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "seller {} has {free_stock} {} at {}, contract needs {}",
            proposal.seller, proposal.product, proposal.delivery_city, proposal.quantity
        ))));
    }

    let seller = state
        .players
        .get_mut(&proposal.seller)
        .expect("checked above");
    seller.debit(proposal.seller_deposit)?;
    seller
        .inventory
        .remove(proposal.delivery_city, proposal.product, proposal.quantity)
        .expect("stok yeterliliği yukarıda doğrulandı");

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
    let (city, product, qty) = (
        contract.delivery_city,
        contract.product,
        contract.quantity,
    );
    state.contracts.remove(&contract_id);

    // Kapora + escrow'daki mal iade.
    if let Some(player) = state.players.get_mut(&seller) {
        player.credit(deposit)?;
        let _ = player.inventory.add(city, product, qty);
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

    // Mal öneri anında escrow'a alındığı için satıcı tarafı garanti; geriye
    // tek breach sebebi kalır: alıcının nakdi yetmemesi.
    let buyer_has_cash = state
        .players
        .get(&buyer)
        .is_some_and(|p| p.cash >= total_value);

    if buyer_has_cash {
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
        // Tek kalan breach sebebi alıcının nakitsizliği. Escrow'daki mal
        // satıcıya geri döner.
        if let Some(s) = state.players.get_mut(&seller) {
            let _ = s.inventory.add(delivery_city, product, qty);
        }
        let final_state = breach_contract(
            state,
            seller,
            buyer,
            buyer,
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
    // Satıcı: satış bedeli + kendi kaporasının iadesi. Mal zaten öneri
    // anında escrow'a alınmıştı, burada tekrar düşülmez.
    if let Some(s) = state.players.get_mut(&seller) {
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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı

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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı
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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı
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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı
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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı
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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı
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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı
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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı
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
        give_stock(&mut s, 1, 10); // escrow: satıcının kontrat malı olmalı
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

    // ── Stok escrow'u ────────────────────────────────────────────────────────
    //
    // Kontrat teslim anında stoğa bakıyor ama öneri anında kilitlemiyordu:
    // satıcı arada malı pazarda satabiliyor, teslimde eli boş kalıyordu.
    // Ölçüm: NPC kontratlarının %78'i breach ile kapanıyordu ve bu yüzden
    // NPC propose tamamen kapatılmıştı. Escrow bunu yapısal olarak çözer —
    // satıcı stok yüzünden breach *edemez*, mal zaten kilitlidir.

    /// Satıcıya kontrat miktarı kadar stok ver.
    fn give_stock(s: &mut GameState, id: u64, qty: u32) {
        s.players
            .get_mut(&PlayerId::new(id))
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Kumas, qty)
            .unwrap();
    }

    fn stock_of(s: &GameState, id: u64) -> u32 {
        s.players[&PlayerId::new(id)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas)
    }

    #[test]
    fn propose_escrows_seller_stock() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        give_stock(&mut s, 1, 30);

        process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100))
            .unwrap();

        // 30 - 10 (kontrat miktarı) = 20 serbest kaldı.
        assert_eq!(stock_of(&s, 1), 20, "kontrat malı kilitlenmeli");
    }

    #[test]
    fn propose_rejected_without_enough_stock() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        give_stock(&mut s, 1, 5); // kontrat 10 istiyor

        let res =
            process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100));
        assert!(res.is_err(), "stoğu olmayan satıcı kontrat açamamalı");
        assert!(s.contracts.is_empty());
        // Kapora da alınmamalı.
        assert_eq!(s.players[&PlayerId::new(1)].cash, Money::from_lira(1_000).unwrap());
    }

    #[test]
    fn escrowed_stock_cannot_be_sold_twice() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        give_stock(&mut s, 1, 15);

        // İlk kontrat 10 birim kilitler → 5 kalır.
        process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100))
            .unwrap();
        // İkinci kontrat yine 10 istiyor ama serbest stok 5.
        let res =
            process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100));
        assert!(res.is_err(), "kilitli mal ikinci kez satılamaz");
        assert_eq!(s.contracts.len(), 1);
    }

    #[test]
    fn cancel_returns_escrowed_stock() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        give_stock(&mut s, 1, 30);

        process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100))
            .unwrap();
        let cid = *s.contracts.keys().next().unwrap();
        process_cancel_contract(&mut s, &mut r, Tick::new(1), cid, PlayerId::new(1)).unwrap();

        assert_eq!(stock_of(&s, 1), 30, "iptal malı geri vermeli");
    }

    #[test]
    fn fulfilled_contract_moves_escrowed_stock_to_buyer() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 1_000);
        give_stock(&mut s, 1, 10);

        process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100))
            .unwrap();
        let cid = *s.contracts.keys().next().unwrap();
        process_accept_contract(&mut s, &mut r, Tick::new(2), cid, PlayerId::new(2)).unwrap();

        // Teslim tick'ine gel.
        advance_contracts(&mut s, &mut r, Tick::new(10));

        assert_eq!(stock_of(&s, 1), 0, "satıcının malı gitti");
        assert_eq!(stock_of(&s, 2), 10, "alıcı malı aldı");
        assert!(s.contracts.is_empty());
    }

    #[test]
    fn seller_cannot_breach_on_stock_after_escrow() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 1_000);
        give_stock(&mut s, 1, 10);

        process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100))
            .unwrap();
        let cid = *s.contracts.keys().next().unwrap();
        process_accept_contract(&mut s, &mut r, Tick::new(2), cid, PlayerId::new(2)).unwrap();

        // Satıcı arada tüm serbest stoğunu boşaltsa bile kontrat malı kilitli:
        // serbest stok zaten 0, yapacak bir şey yok. Teslim yine de gerçekleşmeli.
        advance_contracts(&mut s, &mut r, Tick::new(10));

        let settled = r.entries.iter().rev().find_map(|e| match &e.event {
            crate::LogEvent::ContractSettled { final_state, .. } => Some(*final_state),
            _ => None,
        });
        assert_eq!(
            settled,
            Some(ContractState::Fulfilled),
            "escrow sonrası satıcı stok yüzünden breach edemez"
        );
    }

    #[test]
    fn buyer_breach_returns_escrowed_stock_to_seller() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 1_000);
        give_stock(&mut s, 1, 10);

        process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100))
            .unwrap();
        let cid = *s.contracts.keys().next().unwrap();
        process_accept_contract(&mut s, &mut r, Tick::new(2), cid, PlayerId::new(2)).unwrap();

        // Alıcının nakdini sıfırla → teslimde ödeyemez, breach eder.
        {
            let b = s.players.get_mut(&PlayerId::new(2)).unwrap();
            let all = b.cash;
            b.debit(all).unwrap();
        }
        advance_contracts(&mut s, &mut r, Tick::new(10));

        assert_eq!(stock_of(&s, 1), 10, "breach'te mal satıcıya dönmeli");
        assert_eq!(stock_of(&s, 2), 0);
    }

    #[test]
    fn total_stock_is_conserved_across_contract_lifecycle() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, 1_000);
        add_player(&mut s, 2, 1_000);
        give_stock(&mut s, 1, 25);

        let escrowed = |s: &GameState| -> u64 {
            s.contracts.values().map(|c| u64::from(c.quantity)).sum()
        };
        let visible = |s: &GameState| -> u64 {
            s.players.values().map(|p| p.inventory.total_units()).sum()
        };
        let total = |s: &GameState| visible(s) + escrowed(s);
        let before = total(&s);

        process_propose_contract(&mut s, &mut r, Tick::new(1), &proposal(1, ListingKind::Public, 100))
            .unwrap();
        assert_eq!(total(&s), before, "öneri malı yok etmemeli");

        let cid = *s.contracts.keys().next().unwrap();
        process_accept_contract(&mut s, &mut r, Tick::new(2), cid, PlayerId::new(2)).unwrap();
        assert_eq!(total(&s), before, "kabul malı yok etmemeli");

        advance_contracts(&mut s, &mut r, Tick::new(10));
        assert_eq!(total(&s), before, "teslim malı yok etmemeli");
    }
}
