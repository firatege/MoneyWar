//! Integration: Anlaşma Masası — fulfill ve breach patikaları uçtan uca.

use moneywar_domain::{
    CityId, Command, ContractProposal, ContractState, GameState, ListingKind, Money, Player,
    PlayerId, ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::advance_tick;

fn init_state() -> GameState {
    let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
    let mut seller = Player::new(
        PlayerId::new(1),
        "Seller",
        Role::Sanayici,
        Money::from_lira(10_000).unwrap(),
        false,
    )
    .unwrap();
    // Satıcı teslimat anında yeterli stoğa sahip olsun.
    seller
        .inventory
        .add(CityId::Istanbul, ProductKind::Kumas, 100)
        .unwrap();
    s.players.insert(seller.id, seller);

    let buyer = Player::new(
        PlayerId::new(2),
        "Buyer",
        Role::Tuccar,
        Money::from_lira(10_000).unwrap(),
        false,
    )
    .unwrap();
    s.players.insert(buyer.id, buyer);
    s
}

fn proposal(delivery_tick: u32, qty: u32) -> ContractProposal {
    ContractProposal {
        seller: PlayerId::new(1),
        listing: ListingKind::Public,
        product: ProductKind::Kumas,
        quantity: qty,
        unit_price: Money::from_lira(20).unwrap(),
        delivery_city: CityId::Istanbul,
        delivery_tick: Tick::new(delivery_tick),
        seller_deposit: Money::from_lira(200).unwrap(),
        buyer_deposit: Money::from_lira(200).unwrap(),
    }
}

#[test]
fn contract_fulfilled_on_delivery_tick() {
    let s0 = init_state();
    let total_cash_before: i64 = s0.players.values().map(|p| p.cash.as_cents()).sum();

    // Tick 1: propose
    let (s1, _) = advance_tick(&s0, &[Command::ProposeContract(proposal(5, 10))]).unwrap();
    let cid = *s1.contracts.keys().next().unwrap();
    // Tick 2: accept
    let (s2, _) = advance_tick(
        &s1,
        &[Command::AcceptContract {
            contract_id: cid,
            acceptor: PlayerId::new(2),
        }],
    )
    .unwrap();
    assert_eq!(s2.contracts[&cid].state, ContractState::Active);

    // Tick 3, 4: bekle. Contract hâlâ Active.
    let (s3, _) = advance_tick(&s2, &[]).unwrap();
    let (s4, _) = advance_tick(&s3, &[]).unwrap();
    assert_eq!(s4.contracts[&cid].state, ContractState::Active);

    // Tick 5: delivery_tick — fulfill edilir.
    let (s5, r5) = advance_tick(&s4, &[]).unwrap();
    assert!(!s5.contracts.contains_key(&cid), "fulfilled removed");

    // Seller: 10 kumas verdi, 10 × 20 = 200₺ kazandı + seller_deposit (200₺) iade.
    // Toplam değişim: -10 kumas, +200 (sale) (+deposit geri, net sıfır vs başlangıç).
    // Başlangıç cash 10k; propose sırasında -200 (deposit), fulfill sırasında +200 (deposit iade) + 200 (sale).
    assert_eq!(
        s5.players[&PlayerId::new(1)].cash,
        Money::from_lira(10_000 + 200).unwrap()
    );
    assert_eq!(
        s5.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        90
    );

    // Buyer: 10 × 20 = 200₺ ödedi, 10 kumas aldı, kendi deposit'i iade.
    // Başlangıç 10k; accept sırasında -200, fulfill -200 (satış), +200 (deposit iade).
    assert_eq!(
        s5.players[&PlayerId::new(2)].cash,
        Money::from_lira(10_000 - 200).unwrap()
    );
    assert_eq!(
        s5.players[&PlayerId::new(2)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        10
    );

    let total_cash_after: i64 = s5.players.values().map(|p| p.cash.as_cents()).sum();
    assert_eq!(
        total_cash_before, total_cash_after,
        "fulfilled contract preserves total cash"
    );

    // ContractSettled event var mı?
    let settled = r5.entries.iter().any(|e| {
        matches!(
            &e.event,
            moneywar_engine::LogEvent::ContractSettled {
                final_state: ContractState::Fulfilled,
                ..
            }
        )
    });
    assert!(settled);
}

#[test]
fn contract_breached_when_seller_has_no_stock() {
    // Satıcı stoksuzsa seller breach, kaporası buyer'a tazminat.
    let mut s = init_state();
    // Stoku sil.
    s.players
        .get_mut(&PlayerId::new(1))
        .unwrap()
        .inventory
        .remove(CityId::Istanbul, ProductKind::Kumas, 100)
        .unwrap();
    let total_cash_before: i64 = s.players.values().map(|p| p.cash.as_cents()).sum();

    let (s1, _) = advance_tick(&s, &[Command::ProposeContract(proposal(3, 10))]).unwrap();
    let cid = *s1.contracts.keys().next().unwrap();
    let (s2, _) = advance_tick(
        &s1,
        &[Command::AcceptContract {
            contract_id: cid,
            acceptor: PlayerId::new(2),
        }],
    )
    .unwrap();
    let (s3, r3) = advance_tick(&s2, &[]).unwrap();
    assert!(!s3.contracts.contains_key(&cid));

    // Seller: kaporasını kaybetti (-200). Stok değişmedi.
    // 10k - 200 = 9800
    assert_eq!(
        s3.players[&PlayerId::new(1)].cash,
        Money::from_lira(9_800).unwrap()
    );
    // Buyer: kendi kaporası iade + seller'ın kaporası tazminat olarak geldi.
    // 10k - 200 (accept) + 200 (kendi iade) + 200 (tazminat) = 10_200
    assert_eq!(
        s3.players[&PlayerId::new(2)].cash,
        Money::from_lira(10_200).unwrap()
    );

    let total_cash_after: i64 = s3.players.values().map(|p| p.cash.as_cents()).sum();
    assert_eq!(
        total_cash_before, total_cash_after,
        "breach redistributes, preserves total"
    );

    let breached = r3.entries.iter().any(|e| {
        matches!(
            &e.event,
            moneywar_engine::LogEvent::ContractSettled {
                final_state: ContractState::Breached { .. },
                ..
            }
        )
    });
    assert!(breached);
}

#[test]
fn contract_breached_when_buyer_has_no_cash_at_delivery() {
    // Alıcı accept sonrası kasayı boşaltırsa (mesela başka yere harcarsa)
    // delivery'de buyer breach. Bu testte basit simülasyon: başta az para ver.
    let mut s = init_state();
    // Alıcı tam accept için 200₺ var, ama total_value 200₺ olduğundan delivery'de
    // cash yetmeyecek (accept 200'ünü kilitledi, fulfillment sırasında 200 daha
    // çekmek gerek, cash = 0).
    s.players.get_mut(&PlayerId::new(2)).unwrap().cash = Money::from_lira(200).unwrap();
    let total_before: i64 = s.players.values().map(|p| p.cash.as_cents()).sum();

    let (s1, _) = advance_tick(&s, &[Command::ProposeContract(proposal(3, 10))]).unwrap();
    let cid = *s1.contracts.keys().next().unwrap();
    let (s2, _) = advance_tick(
        &s1,
        &[Command::AcceptContract {
            contract_id: cid,
            acceptor: PlayerId::new(2),
        }],
    )
    .unwrap();
    // Accept sonrası alıcı cash = 0, stok yok. Delivery'de satıcı stoğu var,
    // alıcı cash yok → buyer breach.
    let (s3, r3) = advance_tick(&s2, &[]).unwrap();

    // Seller: deposit iade + alıcının deposit'ı tazminat.
    // 10k - 200 (own lock) + 200 (own refund) + 200 (tazminat) = 10_200
    assert_eq!(
        s3.players[&PlayerId::new(1)].cash,
        Money::from_lira(10_200).unwrap()
    );
    // Buyer: cash zaten 0 sonra accept -200, ama 200'ü vardı → kilit 0'a indi.
    // Breach sonrası: kendi kaporası gitti → cash = 0.
    assert_eq!(s3.players[&PlayerId::new(2)].cash, Money::ZERO);

    let total_after: i64 = s3.players.values().map(|p| p.cash.as_cents()).sum();
    assert_eq!(total_before, total_after);

    let breached_by_buyer = r3.entries.iter().any(|e| {
        matches!(
            &e.event,
            moneywar_engine::LogEvent::ContractSettled {
                final_state: ContractState::Breached { breacher },
                ..
            } if *breacher == PlayerId::new(2)
        )
    });
    assert!(breached_by_buyer);
}
