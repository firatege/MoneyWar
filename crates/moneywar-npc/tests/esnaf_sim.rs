//! Sim testi: Esnaf bid'inin etkisini ölç. 3 seed × 90 tick.
//!
//! Senaryo:
//! - 1 insan Sanayici (50k cash, 500 Pamuk + 200 Kumaş başlangıç, fabrika kuracak)
//! - 2 Esnaf (yalnız hammadde stoklu — mamul stoğu 0 → yeni bid mantığı tetiklenir)
//! - 1 Spekülatör NPC (market maker)
//! - 1 Tüccar NPC (arbitraj)
//!
//! Ölçüm: 90 tick boyunca toplam match, oyuncunun mamul (Kumaş) satış sayısı,
//! Esnaf'ların aldığı kumaş, son cash. Esnaf bid'inin oyuncunun mamul satışını
//! ne kadar kolaylaştırdığını gösterir.

use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, NewsTier, NpcKind, OrderId, OrderSide,
    Personality, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::{advance_tick, rng_for};
use moneywar_npc::{Difficulty, decide_all_npcs};

fn run_seed(seed: u64) -> SimResult {
    let mut s = GameState::new(RoomId::new(seed), RoomConfig::hizli());

    let mut human = Player::new(
        PlayerId::new(1),
        "Insan",
        Role::Sanayici,
        Money::from_lira(50_000).unwrap(),
        false,
    )
    .unwrap();
    human
        .inventory
        .add(CityId::Istanbul, ProductKind::Pamuk, 500)
        .unwrap();
    human
        .inventory
        .add(CityId::Istanbul, ProductKind::Kumas, 300)
        .unwrap();
    s.players.insert(human.id, human);
    s.news_subscriptions.insert(PlayerId::new(1), NewsTier::Free);

    // 2 Esnaf — sadece hammadde stoklu. Mamul stoğu 0 → bid mantığı tetiklenir.
    for (idx, id) in [100u64, 101].iter().enumerate() {
        let mut npc = Player::new(
            PlayerId::new(*id),
            format!("NPC-Esnaf-{idx}"),
            Role::Tuccar,
            Money::from_lira(20_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Esnaf)
        .with_personality(Personality::MeanReverter);
        for c in CityId::ALL {
            for p in ProductKind::RAW_MATERIALS {
                let _ = npc.inventory.add(c, p, 5_000);
            }
        }
        s.players.insert(npc.id, npc);
        s.news_subscriptions.insert(PlayerId::new(*id), NewsTier::Free);
    }

    // 1 Spekülatör — market maker.
    let mut spek = Player::new(
        PlayerId::new(102),
        "NPC-Spek",
        Role::Tuccar,
        Money::from_lira(40_000).unwrap(),
        true,
    )
    .unwrap()
    .with_kind(NpcKind::Spekulator)
    .with_personality(Personality::EventTrader);
    for c in CityId::ALL {
        for p in ProductKind::ALL {
            let _ = spek.inventory.add(c, p, 500);
        }
    }
    s.players.insert(spek.id, spek);
    s.news_subscriptions.insert(PlayerId::new(102), NewsTier::Free);

    // 1 Tüccar — arbitraj.
    let mut tuc = Player::new(
        PlayerId::new(103),
        "NPC-Tuccar",
        Role::Tuccar,
        Money::from_lira(20_000).unwrap(),
        true,
    )
    .unwrap()
    .with_kind(NpcKind::Tuccar)
    .with_personality(Personality::MeanReverter);
    for c in CityId::ALL {
        for p in ProductKind::ALL {
            let _ = tuc.inventory.add(c, p, 200);
        }
    }
    s.players.insert(tuc.id, tuc);
    s.news_subscriptions.insert(PlayerId::new(103), NewsTier::Free);

    let mut human_kumas_sold = 0u32;
    let mut total_matches = 0u32;
    let mut esnaf_kumas_bought = 0u32;
    let mut human_orders = 0u32;
    let mut esnaf_buy_orders_total = 0u32; // Esnaf'ın toplam bid sayısı
    let mut esnaf_sell_orders_total = 0u32; // referans için ask sayısı

    for t in 1..=90u32 {
        let tick = Tick::new(t);
        let mut npc_rng = rng_for(s.room_id, tick);
        let npc_cmds = decide_all_npcs(&s, &mut npc_rng, tick, Difficulty::Easy);

        // NPC komutlarında Esnaf'ların bid/ask sayılarını ölç.
        for cmd in &npc_cmds {
            if let Command::SubmitOrder(o) = cmd {
                if o.player == PlayerId::new(100) || o.player == PlayerId::new(101) {
                    match o.side {
                        OrderSide::Buy => esnaf_buy_orders_total += 1,
                        OrderSide::Sell => esnaf_sell_orders_total += 1,
                    }
                }
            }
        }

        let mut cmds = npc_cmds;

        // İnsan oyuncu her 10 tickte bir Kumaş satış emri verir — düşük fiyat
        // (12₺) ki Esnaf'ın market×0.93 bid'i ile eşleşsin.
        if t % 10 == 5 {
            human_orders += 1;
            cmds.push(Command::SubmitOrder(
                MarketOrder::new(
                    OrderId::new(u64::from(t)),
                    PlayerId::new(1),
                    CityId::Istanbul,
                    ProductKind::Kumas,
                    OrderSide::Sell,
                    25,
                    Money::from_lira(12).unwrap(),
                    tick,
                )
                .unwrap(),
            ));
        }

        let (next, report) = advance_tick(&s, &cmds).unwrap();
        for entry in &report.entries {
            use moneywar_engine::LogEvent;
            if let LogEvent::OrderMatched {
                buyer,
                seller,
                quantity,
                product,
                ..
            } = &entry.event
            {
                total_matches += 1;
                if *seller == PlayerId::new(1) && *product == ProductKind::Kumas {
                    human_kumas_sold += quantity;
                }
                if (*buyer == PlayerId::new(100) || *buyer == PlayerId::new(101))
                    && *product == ProductKind::Kumas
                {
                    esnaf_kumas_bought += quantity;
                }
            }
        }
        s = next;
    }

    let final_human_cash = s.players[&PlayerId::new(1)].cash.as_cents();
    let final_human_kumas: u32 = CityId::ALL
        .iter()
        .map(|c| {
            s.players[&PlayerId::new(1)]
                .inventory
                .get(*c, ProductKind::Kumas)
        })
        .sum();

    SimResult {
        seed,
        total_matches,
        human_orders,
        human_kumas_sold,
        esnaf_kumas_bought,
        esnaf_buy_orders_total,
        esnaf_sell_orders_total,
        final_human_cash,
        final_human_kumas,
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct SimResult {
    seed: u64,
    total_matches: u32,
    human_orders: u32,
    human_kumas_sold: u32,
    esnaf_kumas_bought: u32,
    esnaf_buy_orders_total: u32,
    esnaf_sell_orders_total: u32,
    final_human_cash: i64,
    final_human_kumas: u32,
}

#[test]
fn run_three_seeds_and_print() {
    println!();
    println!(
        "{:>4} {:>7} {:>8} {:>10} {:>10} {:>9} {:>9} {:>10}",
        "seed", "match", "h_sold", "esnaf_buy", "esnaf_ask", "esnaf_₺", "cash_end", "k_end"
    );
    for seed in [1u64, 7, 42] {
        let r = run_seed(seed);
        println!(
            "{:>4} {:>7} {:>8} {:>10} {:>10} {:>9} {:>9} {:>10}",
            r.seed,
            r.total_matches,
            r.human_kumas_sold,
            r.esnaf_buy_orders_total,
            r.esnaf_sell_orders_total,
            r.esnaf_kumas_bought,
            r.final_human_cash / 100,
            r.final_human_kumas,
        );
    }
}
