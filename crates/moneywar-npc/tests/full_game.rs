//! Integration: CLI playtest hazırlığı — insan + NPC'ler 20 tick birlikte koşar.
//!
//! Amacı oynanabilirlik doğrulamak: engine crash etmiyor, NPC'ler düzgün
//! likidite sağlıyor, insan oyuncu komutlarıyla NPC'ler karışıyor, tick
//! lifecycle tamamı (events + production + transport + contracts + loans +
//! clearing) beklenen sıra ile işliyor.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::map_unwrap_or,
    clippy::doc_markdown,
    clippy::uninlined_format_args,
    clippy::too_many_lines,
    clippy::unnested_or_patterns,
    clippy::semicolon_if_nothing_returned,
    clippy::needless_pass_by_value,
    clippy::missing_panics_doc
)]

use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, NpcKind, OrderId, OrderSide, Personality,
    Player, PlayerId, ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::{advance_tick, rng_for};
use moneywar_npc::{Difficulty, decide_all_npcs};

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

    // Money conservation testi: news fee ekonomi dışı sink olduğu için Free'ye geç.
    s.news_subscriptions.insert(PlayerId::new(1), moneywar_domain::NewsTier::Free);
    s.news_subscriptions.insert(PlayerId::new(100), moneywar_domain::NewsTier::Free);
    s.news_subscriptions.insert(PlayerId::new(101), moneywar_domain::NewsTier::Free);

    s
}

#[test]
fn twenty_tick_simulation_with_humans_and_npcs() {
    let mut state = init_world();
    let total_cash_before: i64 = state.players.values().map(|p| p.cash.as_cents()).sum();

    for t in 1..=20u32 {
        // NPC komutları RNG'den deterministik üretilir.
        let mut npc_rng = rng_for(state.room_id, Tick::new(t));
        let npc_cmds = decide_all_npcs(&state, &mut npc_rng, Tick::new(t), Difficulty::Easy);

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

// ---------------------------------------------------------------------------
// Likidite smoke — yeni AliciNpc + 1/1/3 kompozisyon piyasa aktivitesi üretir.
// ---------------------------------------------------------------------------

fn seed_with_composition() -> GameState {
    // RoomConfig default balance kullanır (composition 1/1/3).
    let mut s = GameState::new(RoomId::new(42), RoomConfig::hizli());

    // İnsan Sanayici — fabrika kuracak, sonra kumaş üretecek.
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

    // Kompozisyon: 1 Tüccar + 1 Sanayici + 3 Alıcı.
    let mut tuccar = Player::new(
        PlayerId::new(100),
        "Hasan Bey",
        Role::Tuccar,
        Money::from_lira(15_000).unwrap(),
        true,
    )
    .unwrap()
    .with_kind(NpcKind::Tuccar);
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            tuccar.inventory.add(city, product, 25).unwrap();
        }
    }
    s.players.insert(tuccar.id, tuccar);

    let mut sanayici_npc = Player::new(
        PlayerId::new(101),
        "Mehmet Usta",
        Role::Sanayici,
        Money::from_lira(30_000).unwrap(),
        true,
    )
    .unwrap()
    .with_kind(NpcKind::Sanayici);
    sanayici_npc
        .inventory
        .add(CityId::Istanbul, ProductKind::Kumas, 20)
        .unwrap();
    s.players.insert(sanayici_npc.id, sanayici_npc);

    let alici_names = ["Selim Bey", "Ali Bey", "Ömer Bey"];
    for (i, name) in alici_names.iter().enumerate() {
        let alici = Player::new(
            PlayerId::new(102 + i as u64),
            *name,
            Role::Tuccar,
            Money::from_lira(100_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        s.players.insert(alici.id, alici);
    }

    // 2 NPC-Esnaf — saf satıcı. Her biri devasa stok.
    let esnaf_names = ["Zeynep Hanım", "Fatma Hanım"];
    for (i, name) in esnaf_names.iter().enumerate() {
        let mut esnaf = Player::new(
            PlayerId::new(105 + i as u64),
            *name,
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Esnaf);
        // Her şehir × ürün için 100 birim (toplam 2100 birim dengeli dağıtım).
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                let _ = esnaf.inventory.add(city, product, 100);
            }
        }
        s.players.insert(esnaf.id, esnaf);
    }
    s
}

#[test]
fn liquidity_smoke_twenty_ticks_produces_matches() {
    use moneywar_engine::LogEvent;
    let mut state = seed_with_composition();
    let mut total_matches: u32 = 0;
    let mut total_expired: u32 = 0;
    let mut total_cleared_buckets: u32 = 0;

    for t in 1..=20u32 {
        let mut npc_rng = rng_for(state.room_id, Tick::new(t));
        let cmds = decide_all_npcs(&state, &mut npc_rng, Tick::new(t), Difficulty::Hard);
        let (new_state, report) = advance_tick(&state, &cmds).expect("advance");
        state = new_state;
        for entry in &report.entries {
            match &entry.event {
                LogEvent::OrderMatched { .. } => total_matches += 1,
                LogEvent::OrderExpired { .. } => total_expired += 1,
                LogEvent::MarketCleared {
                    matched_qty,
                    clearing_price: Some(_),
                    ..
                } => {
                    if *matched_qty > 0 {
                        total_cleared_buckets += 1;
                    }
                }
                _ => {}
            }
        }
    }

    // 20 tick'te en az 20 eşleşme. AliciNpc + Tuccar/Sanayici satışı
    // overlap yaratmalı. Relist cooldown likiditeyi biraz kısar (istenen
    // tasarım — flash-place manipülasyonunu önler); eşik ona göre set edildi.
    println!(
        "smoke: {total_matches} match, {total_expired} expired, {total_cleared_buckets} cleared-bucket-with-price"
    );
    assert!(
        total_matches >= 20,
        "likidite düşük: {total_matches} match, {total_expired} expired, {total_cleared_buckets} cleared bucket"
    );
}

// ---------------------------------------------------------------------------
// Cooldown correctness — aynı (player, city, product) için TTL=1 emir
// bittikten sonra ardışık tick'te yeni emir reddedilmeli.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// FuzzyExpert smoke — DSS NPC'leri 30 tick crash etmiyor ve emir üretiyor.
// ---------------------------------------------------------------------------

#[test]
fn fuzzy_expert_runs_without_crash_and_produces_commands() {
    let mut state = seed_with_composition();
    // 4 NPC'ye DSS personality'si ata
    let archetypes = [
        Personality::Aggressive,
        Personality::Arbitrageur,
        Personality::Hoarder,
        Personality::EventTrader,
    ];
    let pids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| matches!(p.npc_kind, Some(NpcKind::Sanayici) | Some(NpcKind::Tuccar)))
        .map(|(id, _)| *id)
        .collect();
    for (i, pid) in pids.iter().enumerate() {
        let p = archetypes[i % archetypes.len()];
        if let Some(player) = state.players.get_mut(pid) {
            player.personality = Some(p);
        }
    }

    let mut total_dss_commands = 0;
    for t in 1..=30u32 {
        let mut npc_rng = rng_for(state.room_id, Tick::new(t));
        let cmds = decide_all_npcs(&state, &mut npc_rng, Tick::new(t), Difficulty::Expert);
        total_dss_commands += cmds.len();
        let (new_state, _report) = advance_tick(&state, &cmds).expect("advance");
        state = new_state;
    }
    // 30 tick'te DSS NPC'leri en az birkaç komut üretmeli.
    assert!(
        total_dss_commands > 10,
        "FuzzyExpert NPC'leri sessiz: {total_dss_commands} komut"
    );
    // State sağlam.
    assert_eq!(state.current_tick, Tick::new(30));
}

#[test]
fn relist_cooldown_rejects_immediate_resubmit() {
    use moneywar_engine::LogEvent;
    let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
    let mut human = Player::new(
        PlayerId::new(1),
        "H",
        Role::Tuccar,
        Money::from_lira(10_000).unwrap(),
        false,
    )
    .unwrap();
    let _ = human
        .inventory
        .add(CityId::Istanbul, ProductKind::Pamuk, 100);
    s.players.insert(human.id, human);

    // Tick 1: TTL=1 emir yolla → clear'de expire, cooldown başlar.
    let o1 = MarketOrder::new(
        OrderId::new(1),
        PlayerId::new(1),
        CityId::Istanbul,
        ProductKind::Pamuk,
        OrderSide::Sell,
        10,
        Money::from_lira(5).unwrap(),
        Tick::new(1),
    )
    .unwrap();
    let (s1, _r1) = advance_tick(&s, &[Command::SubmitOrder(o1)]).unwrap();

    // Tick 2: aynı (player, city, product) için yeni emir → reject.
    let o2 = MarketOrder::new(
        OrderId::new(2),
        PlayerId::new(1),
        CityId::Istanbul,
        ProductKind::Pamuk,
        OrderSide::Sell,
        5,
        Money::from_lira(5).unwrap(),
        Tick::new(2),
    )
    .unwrap();
    let (_s2, r2) = advance_tick(&s1, &[Command::SubmitOrder(o2)]).unwrap();
    let rejected = r2
        .entries
        .iter()
        .any(|e| matches!(&e.event, LogEvent::CommandRejected { reason, .. } if reason.contains("cooldown")));
    assert!(rejected, "cooldown reject bekleniyordu");
}
