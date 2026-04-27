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
        .add(CityId::Istanbul, ProductKind::Pamuk, 100)
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
    // Tick 1 sonu üretim döngüsü: batch başladı, pamuk 10 tüketildi → 90 kaldı.
    assert_eq!(
        s1.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk),
        90
    );
    assert_eq!(s1.factories.values().next().unwrap().batches.len(), 1);

    // Tick 2: yeni batch başlar (üretim 3 tick, hiçbiri tamamlanmadı).
    let (s2, _r2) = advance_tick(&s1, &[]).unwrap();
    assert_eq!(s2.factories.values().next().unwrap().batches.len(), 2);
    assert_eq!(
        s2.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk),
        80
    );

    // Tick 3: yeni batch başlar, hâlâ tamamlanma yok.
    let (s3, _r3) = advance_tick(&s2, &[]).unwrap();
    assert_eq!(s3.factories.values().next().unwrap().batches.len(), 3);
    assert_eq!(
        s3.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        0
    );

    // Tick 4: ilk batch tamamlanır (completion_tick = 1+3=4). Kumas envantere.
    let (s4, _r4) = advance_tick(&s3, &[]).unwrap();
    let kumas = s4.players[&PlayerId::new(1)]
        .inventory
        .get(CityId::Istanbul, ProductKind::Kumas);
    assert_eq!(kumas, 10, "10 kumas produced after 3 tick delay");

    // Tick 5: Sanayici 5 kumas satar; Tüccar 5 kumas alır.
    let sell = Command::SubmitOrder(
        MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Kumas,
            OrderSide::Sell,
            5,
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
            5,
            Money::from_lira(20).unwrap(),
            Tick::new(5),
        )
        .unwrap(),
    );
    let (s5, _r5) = advance_tick(&s4, &[sell, buy]).unwrap();

    // Midpoint price = (20 + 18) / 2 = 19₺. 5 × 19 = 95₺.
    let expected_price = Money::from_lira(19).unwrap();
    assert_eq!(
        s5.price_history[&(CityId::Istanbul, ProductKind::Kumas)]
            .last()
            .unwrap()
            .1,
        expected_price
    );
    // Tüccar envanterinde 5 kumas.
    assert_eq!(
        s5.players[&PlayerId::new(2)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        5
    );
    // Tick 5'te ikinci batch tamamlanır (started=2, completion=5). 10 - 5 + 10 = 15.
    assert_eq!(
        s5.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        15
    );
    // Para korunumu: 200_000₺ × 100 cent.
    let s = s5.players[&PlayerId::new(1)].cash.as_cents();
    let t = s5.players[&PlayerId::new(2)].cash.as_cents();
    let total = s + t;
    assert_eq!(total, 200_000 * 100);
}
