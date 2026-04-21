//! Integration: Tüccar İstanbul'dan pamuk alıp Ankara'ya taşıyor, varışta
//! orada satıyor. Arbitraj motoru uçtan uca çalışıyor.

use moneywar_domain::{
    CargoSpec, CityId, Command, GameState, MarketOrder, Money, OrderId, OrderSide, Player,
    PlayerId, ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::advance_tick;

#[test]
#[allow(clippy::too_many_lines)] // entegrasyon: tick tick takip edilen tam senaryo
fn tuccar_istanbul_to_ankara_arbitrage_full_cycle() {
    let mut s0 = GameState::new(RoomId::new(1), RoomConfig::hizli());

    // Tüccar — İstanbul'da pamuk stoğu ve starter kervan.
    let mut tuccar = Player::new(
        PlayerId::new(1),
        "Tüccar",
        Role::Tuccar,
        Money::from_lira(10_000).unwrap(),
        false,
    )
    .unwrap();
    tuccar
        .inventory
        .add(CityId::Istanbul, ProductKind::Pamuk, 50)
        .unwrap();
    s0.players.insert(tuccar.id, tuccar);

    // Ankara'daki alıcı — bolca nakti var.
    let alici = Player::new(
        PlayerId::new(2),
        "Ankara alıcı",
        Role::Tuccar,
        Money::from_lira(100_000).unwrap(),
        false,
    )
    .unwrap();
    s0.players.insert(alici.id, alici);

    // Tick 1: kervan satın al + İstanbul'dan Ankara'ya yola çıkar.
    let buy = Command::BuyCaravan {
        owner: PlayerId::new(1),
        starting_city: CityId::Istanbul,
    };
    let (s1, _) = advance_tick(&s0, &[buy]).unwrap();
    let cid = *s1.caravans.keys().next().unwrap();

    let mut cargo = CargoSpec::new();
    cargo.add(ProductKind::Pamuk, 40).unwrap();
    let dispatch = Command::DispatchCaravan {
        caravan_id: cid,
        from: CityId::Istanbul,
        to: CityId::Ankara,
        cargo,
    };
    let (s2, _) = advance_tick(&s1, &[dispatch]).unwrap();

    // Tick 2'de envanter 50 - 40 = 10 kaldı.
    assert_eq!(
        s2.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk),
        10
    );
    assert_eq!(
        s2.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Ankara, ProductKind::Pamuk),
        0
    );

    // Tick 3, 4: hâlâ yolda (arrival_tick = 2 + 3 = 5).
    let (s3, _) = advance_tick(&s2, &[]).unwrap();
    let (s4, _) = advance_tick(&s3, &[]).unwrap();
    assert!(!s4.caravans[&cid].is_idle());

    // Tick 5: varış → Ankara envanter'ine 40 pamuk.
    let (s5, _) = advance_tick(&s4, &[]).unwrap();
    assert!(s5.caravans[&cid].is_idle());
    assert_eq!(
        s5.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Ankara, ProductKind::Pamuk),
        40
    );

    // Tick 6: Tüccar Ankara'da 10 pamuk 8₺'den satar; Ankara alıcı 10 @ 8₺ alır.
    let sell = Command::SubmitOrder(
        MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Ankara,
            ProductKind::Pamuk,
            OrderSide::Sell,
            10,
            Money::from_lira(7).unwrap(),
            Tick::new(6),
        )
        .unwrap(),
    );
    let buy_ord = Command::SubmitOrder(
        MarketOrder::new(
            OrderId::new(2),
            PlayerId::new(2),
            CityId::Ankara,
            ProductKind::Pamuk,
            OrderSide::Buy,
            10,
            Money::from_lira(9).unwrap(),
            Tick::new(6),
        )
        .unwrap(),
    );
    let (s6, _) = advance_tick(&s5, &[sell, buy_ord]).unwrap();

    // Midpoint 8₺, 10 × 8 = 80₺.
    assert_eq!(
        s6.players[&PlayerId::new(2)]
            .inventory
            .get(CityId::Ankara, ProductKind::Pamuk),
        10
    );
    assert_eq!(
        s6.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Ankara, ProductKind::Pamuk),
        30
    );
    assert_eq!(
        s6.price_history[&(CityId::Ankara, ProductKind::Pamuk)]
            .last()
            .unwrap()
            .1,
        Money::from_lira(8).unwrap()
    );
}
