//! Property-based determinism testleri.
//!
//! Engine'in invariantı: aynı (state, commands) → aynı (`new_state`, report).
//! Bu test rastgele seed'lerle bin'lerce kombinasyon deneyip herhangi birinde
//! farklılık olmadığını ispatlar.

use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, OrderId, OrderSide, PlayerId, ProductKind,
    RoomConfig, RoomId, Tick,
};
use moneywar_engine::advance_tick;
use proptest::prelude::*;

/// `(state, cmds)` pair'inden keyfi bir örnek — şimdilik sade: boş state +
/// bir avuç `SubmitOrder`. Fazlar ilerledikçe generator'ı zenginleştireceğiz.
fn arb_cmds() -> impl Strategy<Value = Vec<Command>> {
    prop::collection::vec(
        (1u64..=10, 1u64..=100, 1u32..=500, 1i64..=10_000).prop_map(
            |(player, order_id, qty, price_cents)| {
                Command::SubmitOrder(
                    MarketOrder::new(
                        OrderId::new(order_id),
                        PlayerId::new(player),
                        CityId::Istanbul,
                        ProductKind::Pamuk,
                        OrderSide::Buy,
                        qty,
                        Money::from_cents(price_cents),
                        Tick::new(1),
                    )
                    .unwrap(),
                )
            },
        ),
        0..8,
    )
}

fn arb_room_id() -> impl Strategy<Value = RoomId> {
    (1u64..=1_000).prop_map(RoomId::new)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Aynı (state, cmds) iki kez çağrıldığında aynı sonucu üretmeli.
    #[test]
    fn same_input_produces_same_output(
        room in arb_room_id(),
        cmds in arb_cmds(),
    ) {
        let s0 = GameState::new(room, RoomConfig::hizli());
        let (s1a, r1a) = advance_tick(&s0, &cmds).unwrap();
        let (s1b, r1b) = advance_tick(&s0, &cmds).unwrap();
        prop_assert_eq!(s1a, s1b);
        prop_assert_eq!(r1a, r1b);
    }

    /// Tick her çağrıda tam olarak +1 artar, hiç atlanmaz / geri gitmez.
    #[test]
    fn tick_increments_by_exactly_one(
        room in arb_room_id(),
        cmds in arb_cmds(),
    ) {
        let s0 = GameState::new(room, RoomConfig::hizli());
        let before = s0.current_tick;
        let (s1, _) = advance_tick(&s0, &cmds).unwrap();
        prop_assert_eq!(s1.current_tick.value(), before.value() + 1);
    }

    /// Rapor'daki tick her zaman state'teki yeni tick'e eşit olmalı.
    #[test]
    fn report_tick_matches_new_state_tick(
        room in arb_room_id(),
        cmds in arb_cmds(),
    ) {
        let s0 = GameState::new(room, RoomConfig::hizli());
        let (s1, report) = advance_tick(&s0, &cmds).unwrap();
        prop_assert_eq!(s1.current_tick, report.tick);
    }

    /// Rapor'daki entry sayısı komut sayısına eşittir (Faz 2 iskelet
    /// invariantı: her komut log'a tek entry bırakır).
    #[test]
    fn entry_count_equals_command_count(
        room in arb_room_id(),
        cmds in arb_cmds(),
    ) {
        let s0 = GameState::new(room, RoomConfig::hizli());
        let cmd_count = cmds.len();
        let (_s1, report) = advance_tick(&s0, &cmds).unwrap();
        prop_assert_eq!(report.entries.len(), cmd_count);
    }
}
