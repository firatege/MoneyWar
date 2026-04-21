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

    // Tick 2: başka komut yok, üretim pass 2. batch'i başlatır.
    let (s2, _r2) = advance_tick(&s1, &[]).unwrap();
    assert_eq!(s2.factories.values().next().unwrap().batches.len(), 2);
    assert_eq!(
        s2.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk),
        80
    );

    // Tick 3: ilk batch tamamlanır (completion_tick = 1+2=3). Kumas envantere.
    let (s3, _r3) = advance_tick(&s2, &[]).unwrap();
    let kumas = s3.players[&PlayerId::new(1)]
        .inventory
        .get(CityId::Istanbul, ProductKind::Kumas);
    assert_eq!(kumas, 10, "10 kumas produced after 2 tick delay");

    // Tick 4: Sanayici 5 kumas satar; Tüccar 5 kumas alır.
    let sell = Command::SubmitOrder(
        MarketOrder::new(
            OrderId::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Kumas,
            OrderSide::Sell,
            5,
            Money::from_lira(18).unwrap(),
            Tick::new(4),
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
            Tick::new(4),
        )
        .unwrap(),
    );
    let (s4, _r4) = advance_tick(&s3, &[sell, buy]).unwrap();

    // Midpoint price = (20 + 18) / 2 = 19₺. 5 × 19 = 95₺.
    let expected_price = Money::from_lira(19).unwrap();
    assert_eq!(
        s4.price_history[&(CityId::Istanbul, ProductKind::Kumas)]
            .last()
            .unwrap()
            .1,
        expected_price
    );
    // Tüccar envanterinde 5 kumas.
    assert_eq!(
        s4.players[&PlayerId::new(2)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        5
    );
    // Sanayici'nin kumas stoğu 10 → 5 (Tüccar'a gitti) ama + tick 4'te yeni
    // batch tamamlanmış olabilir. Tick 4'te tamamlanan batch: started_tick=2,
    // completion_tick=4 → evet, 10 daha ekledi. Toplam: 10 - 5 + 10 = 15.
    assert_eq!(
        s4.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        15
    );
    // Sanayici'nin nakti: 100k - 95 + ... kazanç. Tüccar kaybetti.
    let s = s4.players[&PlayerId::new(1)].cash.as_cents();
    let t = s4.players[&PlayerId::new(2)].cash.as_cents();
    let total = s + t;
    // Para korunumu: 200_000₺ × 100 cent.
    assert_eq!(total, 200_000 * 100);
}
