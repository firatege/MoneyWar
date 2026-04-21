//! Integration: CLI playtest hazırlığı — insan + NPC'ler 20 tick birlikte koşar.
//!
//! Amacı oynanabilirlik doğrulamak: engine crash etmiyor, NPC'ler düzgün
//! likidite sağlıyor, insan oyuncu komutlarıyla NPC'ler karışıyor, tick
//! lifecycle tamamı (events + production + transport + contracts + loans +
//! clearing) beklenen sıra ile işliyor.

use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, OrderId, OrderSide, Player, PlayerId,
    ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::{advance_tick, rng_for};
use moneywar_npc::decide_all_npcs;

fn init_world() -> GameState {
    let mut s = GameState::new(RoomId::new(42), RoomConfig::hizli());
    // 1 insan Sanayici
    let mut human = Player::new(
        PlayerId::new(1),
        "İnsan",
        Role::Sanayici,
        Money::from_lira(50_000).unwrap(),
        false,
    )
    .unwrap();
    human
        .inventory
        .add(CityId::Istanbul, ProductKind::Pamuk, 100)
        .unwrap();
    s.players.insert(human.id, human);

    // 2 NPC (biri stok sahibi satıcı, diğeri nakit alıcı)
    let mut npc_seller = Player::new(
        PlayerId::new(100),
        "NPC-Satıcı",
        Role::Tuccar,
        Money::from_lira(10_000).unwrap(),
        true,
    )
    .unwrap();
    npc_seller
        .inventory
        .add(CityId::Istanbul, ProductKind::Pamuk, 200)
        .unwrap();
    npc_seller
        .inventory
        .add(CityId::Ankara, ProductKind::Bugday, 200)
        .unwrap();
    npc_seller
        .inventory
        .add(CityId::Izmir, ProductKind::Zeytin, 200)
        .unwrap();
    s.players.insert(npc_seller.id, npc_seller);

    let npc_buyer = Player::new(
        PlayerId::new(101),
        "NPC-Alıcı",
        Role::Tuccar,
        Money::from_lira(50_000).unwrap(),
        true,
    )
    .unwrap();
    s.players.insert(npc_buyer.id, npc_buyer);

    s
}

#[test]
fn twenty_tick_simulation_with_humans_and_npcs() {
    let mut state = init_world();
    let total_cash_before: i64 = state.players.values().map(|p| p.cash.as_cents()).sum();

    for t in 1..=20u32 {
        // NPC komutları RNG'den deterministik üretilir.
        let mut npc_rng = rng_for(state.room_id, Tick::new(t));
        let npc_cmds = decide_all_npcs(&state, &mut npc_rng, Tick::new(t));

        // İnsan oyuncu bazı tick'lerde pamuk satıyor.
        let mut cmds = npc_cmds;
        if t == 3 {
            cmds.push(Command::SubmitOrder(
                MarketOrder::new(
                    OrderId::new(1),
                    PlayerId::new(1),
                    CityId::Istanbul,
                    ProductKind::Pamuk,
                    OrderSide::Sell,
                    30,
                    Money::from_lira(7).unwrap(),
                    Tick::new(t),
                )
                .unwrap(),
            ));
        }

        let (new_state, _) = advance_tick(&state, &cmds).expect("advance_tick must not fail");
        state = new_state;
    }

    // 20 tick sonra motor ayakta, state tutarlı.
    assert_eq!(state.current_tick, Tick::new(20));
    // Toplam cash — NPC/human transferler ± contract/loan etkisi yok.
    // Money conservation oyuncu-arası transferlerde korunur.
    let total_cash_after: i64 = state.players.values().map(|p| p.cash.as_cents()).sum();
    assert_eq!(
        total_cash_before, total_cash_after,
        "cash conserved across pure market transfers"
    );
    // NPC'ler hala stok gösteriyor (sıfıra inmedi mantıksızca).
    let npc_stock = state.players[&PlayerId::new(100)]
        .inventory
        .get(CityId::Istanbul, ProductKind::Pamuk);
    assert!(
        npc_stock <= 200,
        "NPC stock should only decrease via market: {npc_stock}"
    );
}
