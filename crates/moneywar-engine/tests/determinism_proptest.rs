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

    /// Komut sayısı = kabul + ret. Sistem event'leri (MarketCleared, OrderMatched)
    /// ek entry oluşturur ama bu sayım dışıdır.
    #[test]
    fn accepted_plus_rejected_equals_command_count(
        room in arb_room_id(),
        cmds in arb_cmds(),
    ) {
        let s0 = GameState::new(room, RoomConfig::hizli());
        let cmd_count = cmds.len();
        let (_s1, report) = advance_tick(&s0, &cmds).unwrap();
        prop_assert_eq!(report.accepted_count() + report.rejected_count(), cmd_count);
    }

    /// Money conservation: tick başındaki toplam cash = tick sonundaki toplam
    /// cash (clearing sadece oyuncular arası transfer yapar, para yaratmaz).
    #[test]
    fn total_cash_is_conserved_across_clearing(
        room in arb_room_id(),
        cmds in arb_cmds(),
    ) {
        use moneywar_domain::{Money, Player, PlayerId, Role};
        let mut s0 = GameState::new(room, RoomConfig::hizli());
        // Cömert oyuncular ki settlement reject olmasın.
        for id in 1u64..=10 {
            let mut p = Player::new(
                PlayerId::new(id),
                format!("p{id}"),
                Role::Tuccar,
                Money::from_lira(1_000_000).unwrap(),
                false,
            ).unwrap();
            for city in moneywar_domain::CityId::ALL {
                for product in moneywar_domain::ProductKind::ALL {
                    p.inventory.add(city, product, 10_000).unwrap();
                }
            }
            s0.players.insert(p.id, p);
        }
        let total_before: i64 = s0.players.values().map(|p| p.cash.as_cents()).sum();
        let total_stock_before: u64 = s0.players.values().map(|p| p.inventory.total_units()).sum();

        let (s1, _report) = advance_tick(&s0, &cmds).unwrap();

        let total_after: i64 = s1.players.values().map(|p| p.cash.as_cents()).sum();
        let total_stock_after: u64 = s1.players.values().map(|p| p.inventory.total_units()).sum();
        prop_assert_eq!(total_before, total_after);
        prop_assert_eq!(total_stock_before, total_stock_after);
    }
}
