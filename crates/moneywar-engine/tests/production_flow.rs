//! Integration: Sanayici fabrika kurar, ham maddeyi bitmiş ürüne çevirir,
//! bir sonraki tick pazarda Tüccar'a satar.

use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, NewsTier, OrderId, OrderSide, Player, PlayerId,
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
    // Bu integration testleri üretim akışını izole eder; news fee'yi devre dışı bırak.
    s.news_subscriptions
        .insert(PlayerId::new(1), NewsTier::Free);
    s.news_subscriptions
        .insert(PlayerId::new(2), NewsTier::Free);
    s
}

#[test]
fn sanayici_builds_factory_produces_and_sells_to_tuccar() {
    // v0.4.1: Kumaş 4 tick + %80 verim (100 Pamuk → 80 Kumaş).
    let s0 = init_state();

    // Tick 1: Sanayici fabrika kurar + üretim başlar.
    let build = Command::BuildFactory {
        owner: PlayerId::new(1),
        city: CityId::Istanbul,
        product: ProductKind::Kumas,
    };
    let (s1, _r1) = advance_tick(&s0, &[build]).unwrap();
    assert_eq!(s1.factories.len(), 1);
    // Tick 1 sonu: batch başladı, pamuk 100 tüketildi → 900 kaldı.
    assert_eq!(
        s1.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Pamuk),
        900
    );
    assert_eq!(s1.factories.values().next().unwrap().batches.len(), 1);

    // Tick 2-4: yeni batch'ler başlar, hiçbiri tamamlanmaz (Kumaş 4 tick).
    let (s2, _) = advance_tick(&s1, &[]).unwrap();
    let (s3, _) = advance_tick(&s2, &[]).unwrap();
    let (s4, _r4) = advance_tick(&s3, &[]).unwrap();
    assert_eq!(s4.factories.values().next().unwrap().batches.len(), 4);
    assert_eq!(
        s4.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        0,
        "Kumaş 4 tick sürer, t4'te hiç tamamlanmadı"
    );

    // Tick 5: ilk batch tamamlanır (started=1, completion=1+4=5). %80 verim
    // → 80 Kumaş. Yeni batch başlar. Plus 50 sat / 50 al.
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

    // Pay-as-bid: trade fiyatı = BUY emrindeki limit (20₺). 50 × 20 = 1000₺.
    let expected_price = Money::from_lira(20).unwrap();
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
    // v0.4.1: Tick 5'te 1. batch tamamlanır (started=1, completion=1+4=5).
    // 80 Kumaş üretildi, 50 satıldı → 30 kalır. 5. batch başladı.
    assert_eq!(
        s5.players[&PlayerId::new(1)]
            .inventory
            .get(CityId::Istanbul, ProductKind::Kumas),
        30
    );
    // Para korunumu: 200_000₺ × 100 cent — eksiği işlem vergisi (sistem sink).
    // İşlem yapıldığı için toplam <= başlangıç. Vergi miktarı tam değişken,
    // korunum invariant'ı yumuşatılır: aralık kontrolü.
    let s = s5.players[&PlayerId::new(1)].cash.as_cents();
    let t = s5.players[&PlayerId::new(2)].cash.as_cents();
    let total = s + t;
    let initial = 200_000_i64 * 100;
    assert!(
        total <= initial && total >= initial - 10_000,
        "para sadece vergi ile sızabilir: total={total} initial={initial}"
    );
}
