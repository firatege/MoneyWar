//! Faz 1 integration smoke — tüm domain tiplerinin birbiriyle nasıl
//! kompoze olduğunu end-to-end doğrular.

use std::collections::BTreeMap;

use moneywar_domain::{
    Caravan, CaravanId, CityId, Command, Contract, ContractId, ContractProposal, DomainError,
    Factory, FactoryId, GameState, Inventory, ListingKind, Loan, LoanId, MarketOrder, Money,
    NewsItem, NewsTier, OrderId, OrderSide, Player, PlayerId, ProductKind, Role, RoomConfig,
    RoomId, SeasonProgress, Tick,
};

#[test]
fn domain_error_variants_all_std_error() {
    fn assert_error<E: std::error::Error>(_: &E) {}
    let errs: Vec<DomainError> = vec![
        DomainError::Validation("x".into()),
        DomainError::Overflow("y".into()),
        DomainError::InsufficientStock {
            city: CityId::Istanbul,
            product: ProductKind::Pamuk,
            have: 1,
            want: 2,
        },
        DomainError::InsufficientFunds {
            have: Money::ZERO,
            want: Money::from_lira(1).unwrap(),
        },
    ];
    for e in &errs {
        assert_error(e);
    }
}

#[test]
fn primitives_compose_in_btreemap() {
    let mut balances: BTreeMap<PlayerId, Money> = BTreeMap::new();
    balances.insert(PlayerId::new(1), Money::from_lira(100).unwrap());
    balances.insert(PlayerId::new(2), Money::from_lira(50).unwrap());

    let total = balances
        .values()
        .copied()
        .try_fold(Money::ZERO, Money::checked_add)
        .unwrap();
    assert_eq!(total, Money::from_lira(150).unwrap());
}

#[test]
fn time_drives_season_progress_end_to_end() {
    let cfg = RoomConfig::hizli();
    let t = Tick::new(cfg.season_ticks / 2);
    let p = SeasonProgress::from_ticks(t, cfg.season_ticks).unwrap();
    assert!(p.is_mid() || p.value() == 50);
}

#[test]
fn city_distance_and_product_chain_connect() {
    // İstanbul ucuz Pamuk üretir, Kumaş = Pamuk'un bitmiş hali.
    let raw = CityId::Istanbul.cheap_raw();
    assert_eq!(raw, ProductKind::Pamuk);
    assert_eq!(raw.finished_output(), Some(ProductKind::Kumas));

    // İstanbul → Ankara 2 tick (v3: yarıya indi)
    assert_eq!(CityId::Istanbul.distance_to(CityId::Ankara), 2);
}

#[test]
fn factory_enforces_finished_product() {
    let err = Factory::new(
        FactoryId::new(1),
        PlayerId::new(1),
        CityId::Istanbul,
        ProductKind::Pamuk, // raw
    )
    .expect_err("raw not allowed");
    assert!(matches!(err, DomainError::Validation(_)));
}

#[test]
fn caravan_full_dispatch_arrive_cycle() {
    let mut c = Caravan::new(CaravanId::new(1), PlayerId::new(1), 50, CityId::Istanbul);
    let mut cargo = moneywar_domain::Cargo::new();
    cargo.add(ProductKind::Kumas, 40).unwrap();

    c.dispatch(CityId::Istanbul, CityId::Ankara, cargo, Tick::new(3))
        .unwrap();
    assert!(!c.is_idle());

    let (dest, delivered) = c.arrive().unwrap();
    assert_eq!(dest, CityId::Ankara);
    assert_eq!(delivered.get(ProductKind::Kumas), 40);
    assert!(c.is_idle());
}

#[test]
fn order_and_contract_share_money_arithmetic() {
    let o = MarketOrder::new(
        OrderId::new(1),
        PlayerId::new(1),
        CityId::Istanbul,
        ProductKind::Kumas,
        OrderSide::Buy,
        10,
        Money::from_lira(15).unwrap(),
        Tick::new(1),
    )
    .unwrap();

    let c = Contract::propose(
        ContractId::new(1),
        PlayerId::new(1),
        ListingKind::Public,
        ProductKind::Kumas,
        10,
        Money::from_lira(15).unwrap(),
        CityId::Istanbul,
        Tick::new(10),
        Tick::new(2),
        Money::ZERO,
        Money::ZERO,
    )
    .unwrap();

    // Aynı miktar + fiyat → aynı toplam değer.
    assert_eq!(o.total_value().unwrap(), c.total_value().unwrap());
    assert_eq!(o.total_value().unwrap(), Money::from_lira(150).unwrap());
}

#[test]
fn inventory_supports_cross_city_holdings() {
    let mut inv = Inventory::new();
    inv.add(CityId::Istanbul, ProductKind::Pamuk, 100).unwrap();
    inv.add(CityId::Ankara, ProductKind::Bugday, 200).unwrap();
    inv.add(CityId::Izmir, ProductKind::Zeytin, 50).unwrap();

    assert_eq!(inv.total_units(), 350);
    assert_eq!(inv.entries().count(), 3);
}

#[test]
fn news_tier_lead_time_shifts_disclosure() {
    let event = moneywar_domain::GameEvent::Drought {
        city: CityId::Ankara,
        product: ProductKind::Bugday,
        severity: moneywar_domain::EventSeverity::Major,
    };

    let bronze = NewsItem::from_event(
        moneywar_domain::NewsId::new(1),
        NewsTier::Bronze,
        Tick::new(30),
        event,
    )
    .unwrap();
    let gold = NewsItem::from_event(
        moneywar_domain::NewsId::new(2),
        NewsTier::Gold,
        Tick::new(30),
        event,
    )
    .unwrap();

    assert_eq!(bronze.disclosed_tick, Tick::new(30));
    assert_eq!(gold.disclosed_tick, Tick::new(28));
}

#[test]
fn game_state_full_population_serde_roundtrip() {
    let mut state = GameState::new(RoomId::new(1), RoomConfig::hizli());

    let sanayici = Player::new(
        PlayerId::new(1),
        "Ali",
        Role::Sanayici,
        Money::from_lira(8_000).unwrap(),
        false,
    )
    .unwrap();
    let tuccar = Player::new(
        PlayerId::new(2),
        "Ayşe",
        Role::Tuccar,
        Money::from_lira(13_000).unwrap(),
        false,
    )
    .unwrap();
    state.players.insert(PlayerId::new(1), sanayici);
    state.players.insert(PlayerId::new(2), tuccar);

    let factory = Factory::new(
        FactoryId::new(1),
        PlayerId::new(1),
        CityId::Istanbul,
        ProductKind::Kumas,
    )
    .unwrap();
    state.factories.insert(FactoryId::new(1), factory);

    let caravan = Caravan::new(CaravanId::new(1), PlayerId::new(2), 50, CityId::Istanbul);
    state.caravans.insert(CaravanId::new(1), caravan);

    state
        .news_subscriptions
        .insert(PlayerId::new(2), NewsTier::Silver);

    let loan = Loan::new(
        LoanId::new(1),
        PlayerId::new(1),
        Money::from_lira(5_000).unwrap(),
        10,
        Tick::new(1),
        Tick::new(20),
    )
    .unwrap();
    state.loans.insert(LoanId::new(1), loan);

    // Serde roundtrip tüm state'i korur.
    let json = serde_json::to_string(&state).unwrap();
    let back: GameState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, back);
}

#[test]
fn command_flow_wires_to_order_and_contract() {
    let order = MarketOrder::new(
        OrderId::new(1),
        PlayerId::new(5),
        CityId::Istanbul,
        ProductKind::Pamuk,
        OrderSide::Buy,
        10,
        Money::from_lira(5).unwrap(),
        Tick::new(1),
    )
    .unwrap();

    let submit = Command::SubmitOrder(order);
    assert_eq!(submit.requester(), PlayerId::new(5));

    let proposal = ContractProposal {
        seller: PlayerId::new(7),
        listing: ListingKind::Personal {
            target: PlayerId::new(5),
        },
        product: ProductKind::Un,
        quantity: 20,
        unit_price: Money::from_lira(10).unwrap(),
        delivery_city: CityId::Ankara,
        delivery_tick: Tick::new(15),
        seller_deposit: Money::from_lira(50).unwrap(),
        buyer_deposit: Money::from_lira(50).unwrap(),
    };
    let propose = Command::ProposeContract(proposal);
    assert_eq!(propose.requester(), PlayerId::new(7));
}

#[test]
fn saturation_threshold_scales_with_participants() {
    let cfg = RoomConfig::hizli();
    // 250 + (n-2) × 50 (hacim 10× ölçek).
    assert_eq!(cfg.saturation_threshold(2), 250);
    assert_eq!(cfg.saturation_threshold(5), 400);
}

#[test]
fn config_validate_catches_bad_ranges() {
    let bad = RoomConfig::custom(1, 5, 50, 1);
    assert!(bad.is_err());
}

#[test]
fn determinism_btreemap_iteration() {
    // Aynı sıra ile eklenen elementler aynı sırada iterasyon verir.
    let mut a: BTreeMap<PlayerId, u32> = BTreeMap::new();
    a.insert(PlayerId::new(3), 30);
    a.insert(PlayerId::new(1), 10);
    a.insert(PlayerId::new(2), 20);

    let b: BTreeMap<PlayerId, u32> = a.clone();

    let collected_a: Vec<_> = a.iter().collect();
    let collected_b: Vec<_> = b.iter().collect();
    assert_eq!(collected_a, collected_b);
    // Sıralama PlayerId::Ord'a göre: 1 < 2 < 3
    assert_eq!(collected_a[0].0, &PlayerId::new(1));
    assert_eq!(collected_a[2].0, &PlayerId::new(3));
}
