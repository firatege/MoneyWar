//! Integration: Sanayici fabrika kurar, ham maddeyi bitmiş ürüne çevirir,
//! bir sonraki tick pazarda Tüccar'a satar.

use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, OrderId, OrderSide, Player, PlayerId,
    ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::advance_tick;

fn init_state() -> GameState {
    let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
    // Sanayici — starter fabrika bedava, pamuk stoğu ile başlasın.
    let mut sanayici = Player::new(
        PlayerId::new(1),
        "Sanayici Ali",
        Role::Sanayici,
        Money::from_lira(100_000).unwrap(),
        false,
    )
    .unwrap();
    sanayici
        .inventory
        .add(CityId::Istanbul, ProductKind::Pamuk, 1000)
        .unwrap();
    s.players.insert(sanayici.id, sanayici);

    // Tüccar — alıcı.
    let tuccar = Player::new(
        PlayerId::new(2),
        "Tüccar Ayşe",
        Role::Tuccar,
        Money::from_lira(100_000).unwrap(),
        false,
    )
    .unwrap();
    s.players.insert(tuccar.id, tuccar);
    s
}

#[test]
fn sanayici_builds_factory_produces_and_sells_to_tuccar() {
    let s0 = init_state();

    // Tick 1: Sanayici fabrika kurar.
    let build = Command::BuildFactory {
        owner: PlayerId::new(1),
        city: CityId::Istanbul,
        product: ProductKind::Kumas,
    };
    let (s1, _r1) = advance_tick(&s0, &[build]).unwrap();
    assert_eq!(s1.factories.len(), 1);
    // Tick 1 sonu üretim döngüsü: batch başladı, pamuk 100 tüketildi → 900 kaldı (10× ölçek).
    assert_eq!(
        s1.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk),
        900
    );
    assert_eq!(s1.factories.values().next().unwrap().batches.len(), 1);

    // Tick 2: yeni batch başlar (üretim 2 tick, hiçbiri tamamlanmadı).
    let (s2, _r2) = advance_tick(&s1, &[]).unwrap();
    assert_eq!(s2.factories.values().next().unwrap().batches.len(), 2);
    assert_eq!(
        s2.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk),
        800
    );

    // Tick 3: ilk batch tamamlanır (completion_tick = 1+2=3). Kumas envantere,
    // yeni batch başlar.
    let (s3, _r3) = advance_tick(&s2, &[]).unwrap();
    let kumas3 = s3.players[&PlayerId::new(1)]
        .inventory
        .get(CityId::Istanbul, ProductKind::Kumas);
    assert_eq!(kumas3, 100, "100 kumas after 2 tick delay (10× ölçek)");
    assert_eq!(
        s3.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk),
        700
    );

    // Tick 4: ikinci batch tamamlanır → kumaş 200.
    let (s4, _r4) = advance_tick(&s3, &[]).unwrap();
    let kumas = s4.players[&PlayerId::new(1)]
        .inventory
        .get(CityId::Istanbul, ProductKind::Kumas);
    assert_eq!(kumas, 200, "200 kumas after second batch completed");

    // Tick 5: Sanayici 50 kumas satar; Tüccar 50 kumas alır (10× ölçek).
    let sell = Command::SubmitOrder(
        MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Kumas,
            OrderSide::Sell,
            50,
            Money::from_lira(18).unwrap(),
            Tick::new(5),
        )
        .unwrap(),
    );
    let buy = Command::SubmitOrder(
        MarketOrder::new(
            OrderId::new(2),
            PlayerId::new(2),
            CityId::Istanbul,
            ProductKind::Kumas,
            OrderSide::Buy,
            50,
            Money::from_lira(20).unwrap(),
            Tick::new(5),
        )
        .unwrap(),
    );
    let (s5, _r5) = advance_tick(&s4, &[sell, buy]).unwrap();

    // Midpoint price = (20 + 18) / 2 = 19₺. 50 × 19 = 950₺.
    let expected_price = Money::from_lira(19).unwrap();
    assert_eq!(
        s5.price_history[&(CityId::Istanbul, ProductKind::Kumas)]
            .last()
            .unwrap()
            .1,
        expected_price
    );
    // Tüccar envanterinde 50 kumas.
    assert_eq!(
        s5.players[&PlayerId::new(2)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        50
    );
    // Tick 5'te 3. batch tamamlanır (started=3, completion=5).
    // 200 - 50 (sat) + 100 (yeni batch) = 250.
    assert_eq!(
        s5.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        250
    );
    // Para korunumu: 200_000₺ × 100 cent.
    let s = s5.players[&PlayerId::new(1)].cash.as_cents();
    let t = s5.players[&PlayerId::new(2)].cash.as_cents();
    let total = s + t;
    assert_eq!(total, 200_000 * 100);
}
