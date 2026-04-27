//! DSS tuning analizi — 90-tick simülasyon raporu.
//!
//! `cargo test -p moneywar-npc --test dss_tuning -- --nocapture` ile çıktı
//! gözükür. Test her zaman geçer (assertion'lar gevşek), amaç **rapor**.

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
    CityId, GameState, Money, NpcKind, Personality, Player, PlayerId, ProductKind, Role,
    RoomConfig, RoomId, Tick,
};
use moneywar_engine::{advance_tick, leaderboard, rng_for};
use moneywar_npc::{Difficulty, decide_all_npcs};

fn run_full_season(difficulty: Difficulty, personality_dist: &[Personality]) -> SeasonReport {
    run_full_season_with_human_cash(difficulty, personality_dist, 40_000)
}

/// Aynı senaryo 12 NPC ile (2 sanayici + 2 tüccar + 3 alıcı + 3 esnaf + 2 spek)
/// + 1 insan (pasif izleyici, parametrik cash). Personality dağılımı parametre.
fn run_full_season_with_human_cash(
    difficulty: Difficulty,
    personality_dist: &[Personality],
    human_cash_lira: i64,
) -> SeasonReport {
    let mut s = GameState::new(RoomId::new(7777), RoomConfig::hizli());

    // Price baseline — şehirler arası %20 spread, arbitraj fırsatı.
    // Domain constants kullan ki balance.rs değişikliği test'e yansısın.
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let base_lira: i64 = if product.is_raw() {
                moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA
            } else {
                moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA
            };
            let mult = match city {
                CityId::Istanbul => 100,
                CityId::Ankara => 80,
                CityId::Izmir => 115,
            };
            let cents = base_lira * 100 * mult / 100;
            s.price_baseline
                .insert((city, product), Money::from_cents(cents));
        }
    }

    // Pasif insan oyuncu — komut göndermez. Cash parametrik (lider/normal/geride).
    let human = Player::new(
        PlayerId::new(1),
        "Spectator",
        Role::Tuccar,
        Money::from_lira(human_cash_lira).unwrap(),
        false,
    )
    .unwrap();
    s.players.insert(human.id, human);

    let mut next_id: u64 = 100;

    // 2 Sanayici
    for i in 0..2 {
        let p = personality_dist[i % personality_dist.len()];
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("San{i}"),
            Role::Sanayici,
            Money::from_lira(30_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Sanayici)
        .with_personality(p);
        let _ = npc.inventory.add(CityId::Istanbul, ProductKind::Pamuk, 50);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }
    // 2 Tüccar
    for i in 0..2 {
        let p = personality_dist[(i + 2) % personality_dist.len()];
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("Tuc{i}"),
            Role::Tuccar,
            Money::from_lira(15_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Tuccar)
        .with_personality(p);
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                let _ = npc.inventory.add(city, product, 25);
            }
        }
        s.players.insert(npc.id, npc);
        next_id += 1;
    }
    // 3 Alıcı
    for i in 0..3 {
        let npc = Player::new(
            PlayerId::new(next_id),
            format!("Ali{i}"),
            Role::Tuccar,
            Money::from_lira(100_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }
    // 3 Esnaf
    for i in 0..3 {
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("Esn{i}"),
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Esnaf);
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                let _ = npc.inventory.add(city, product, 150);
            }
        }
        s.players.insert(npc.id, npc);
        next_id += 1;
    }
    // 2 Spekülatör
    for i in 0..2 {
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("Spe{i}"),
            Role::Tuccar,
            Money::from_lira(40_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Spekulator);
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                let _ = npc.inventory.add(city, product, 50);
            }
        }
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // Başlangıç cash'leri kaydet
    let starting_cash: std::collections::BTreeMap<PlayerId, i64> = s
        .players
        .iter()
        .map(|(id, p)| (*id, p.cash.as_cents()))
        .collect();

    let mut total_matches: u32 = 0;
    let mut total_dispatched: u32 = 0;
    let mut total_caravans_bought: u32 = 0;
    let mut total_factories_built: u32 = 0;
    let mut total_events: u32 = 0;
    let mut total_contracts_proposed: u32 = 0;
    let mut total_contracts_accepted: u32 = 0;
    let mut total_contracts_fulfilled: u32 = 0;
    let mut total_contracts_breached: u32 = 0;

    for t in 1..=90u32 {
        let mut npc_rng = rng_for(s.room_id, Tick::new(t));
        let cmds = decide_all_npcs(&s, &mut npc_rng, Tick::new(t), difficulty);
        let (new_state, report) = advance_tick(&s, &cmds).expect("advance");
        s = new_state;
        for entry in &report.entries {
            use moneywar_engine::LogEvent;
            match &entry.event {
                LogEvent::OrderMatched { .. } => total_matches += 1,
                LogEvent::CaravanDispatched { .. } => total_dispatched += 1,
                LogEvent::CaravanBought { .. } => total_caravans_bought += 1,
                LogEvent::FactoryBuilt { .. } => total_factories_built += 1,
                LogEvent::EventScheduled { .. } => total_events += 1,
                LogEvent::ContractProposed { .. } => total_contracts_proposed += 1,
                LogEvent::ContractAccepted { .. } => total_contracts_accepted += 1,
                LogEvent::ContractSettled { final_state, .. } => match final_state {
                    moneywar_domain::ContractState::Fulfilled => total_contracts_fulfilled += 1,
                    moneywar_domain::ContractState::Breached { .. } => {
                        total_contracts_breached += 1
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }

    // Final cash → PnL hesabı
    let mut npc_pnl_by_kind: std::collections::BTreeMap<NpcKind, Vec<f64>> =
        std::collections::BTreeMap::new();
    let mut npc_pnl_by_personality: std::collections::BTreeMap<Personality, Vec<f64>> =
        std::collections::BTreeMap::new();

    for (id, player) in &s.players {
        if !player.is_npc {
            continue;
        }
        let starting = starting_cash.get(id).copied().unwrap_or(0);
        let pnl = (player.cash.as_cents() - starting) as f64 / 100.0;
        if let Some(kind) = player.npc_kind {
            npc_pnl_by_kind.entry(kind).or_default().push(pnl);
        }
        if let Some(p) = player.personality {
            npc_pnl_by_personality.entry(p).or_default().push(pnl);
        }
    }

    let board = leaderboard(&s);

    SeasonReport {
        difficulty,
        total_matches,
        total_dispatched,
        total_caravans_bought,
        total_factories_built,
        total_events,
        total_contracts_proposed,
        total_contracts_accepted,
        total_contracts_fulfilled,
        total_contracts_breached,
        npc_pnl_by_kind,
        npc_pnl_by_personality,
        leaderboard_top: board
            .iter()
            .take(8)
            .map(|sc| {
                let name = s
                    .players
                    .get(&sc.player_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let personality = s.players.get(&sc.player_id).and_then(|p| p.personality);
                (name, personality, sc.total)
            })
            .collect(),
    }
}

#[derive(Debug)]
struct SeasonReport {
    difficulty: Difficulty,
    total_matches: u32,
    total_dispatched: u32,
    total_caravans_bought: u32,
    total_factories_built: u32,
    total_events: u32,
    total_contracts_proposed: u32,
    total_contracts_accepted: u32,
    total_contracts_fulfilled: u32,
    total_contracts_breached: u32,
    npc_pnl_by_kind: std::collections::BTreeMap<NpcKind, Vec<f64>>,
    npc_pnl_by_personality: std::collections::BTreeMap<Personality, Vec<f64>>,
    leaderboard_top: Vec<(String, Option<Personality>, Money)>,
}

fn print_report(r: &SeasonReport) {
    println!("\n╔══════════════════════════════════════════════════╗");
    println!("║  Difficulty: {:?}  ", r.difficulty);
    println!("╠══════════════════════════════════════════════════╣");
    println!("║  Toplam match:        {}", r.total_matches);
    println!("║  Toplam dispatch:     {}", r.total_dispatched);
    println!("║  Kervan satın alındı: {}", r.total_caravans_bought);
    println!("║  Fabrika kuruldu:     {}", r.total_factories_built);
    println!("║  Olay sayısı:         {}", r.total_events);
    println!(
        "║  Kontrat: {} öneri / {} kabul / {} fulfill / {} breach",
        r.total_contracts_proposed,
        r.total_contracts_accepted,
        r.total_contracts_fulfilled,
        r.total_contracts_breached
    );
    println!("╠══════════════════════════════════════════════════╣");
    println!("║  NPC tipine göre ortalama PnL:");
    for (kind, pnls) in &r.npc_pnl_by_kind {
        let avg = pnls.iter().sum::<f64>() / pnls.len() as f64;
        println!(
            "║    {:<12} avg={:+8.0}₺  (n={})",
            format!("{kind:?}"),
            avg,
            pnls.len()
        );
    }
    if !r.npc_pnl_by_personality.is_empty() {
        println!("╠══════════════════════════════════════════════════╣");
        println!("║  Personality'ye göre ortalama PnL:");
        let mut entries: Vec<_> = r.npc_pnl_by_personality.iter().collect();
        entries.sort_by(|a, b| {
            let a_avg = a.1.iter().sum::<f64>() / a.1.len() as f64;
            let b_avg = b.1.iter().sum::<f64>() / b.1.len() as f64;
            b_avg
                .partial_cmp(&a_avg)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (p, pnls) in entries {
            let avg = pnls.iter().sum::<f64>() / pnls.len() as f64;
            println!(
                "║    {} {:<14} avg={:+8.0}₺  (n={})",
                p.emoji(),
                p.label(),
                avg,
                pnls.len()
            );
        }
    }
    println!("╠══════════════════════════════════════════════════╣");
    println!("║  Leaderboard top 8:");
    for (i, (name, p, total)) in r.leaderboard_top.iter().enumerate() {
        let emoji = p.map_or("", Personality::emoji);
        println!("║    {}. {emoji}{name:<14} {total}", i + 1);
    }
    println!("╚══════════════════════════════════════════════════╝");
}

#[test]
fn tuning_easy_baseline() {
    let report = run_full_season(Difficulty::Easy, &[Personality::Aggressive]);
    print_report(&report);
    assert!(report.total_matches > 50, "Easy yine de likidite üretmeli");
}

#[test]
fn tuning_hard_baseline() {
    let report = run_full_season(Difficulty::Hard, &[Personality::Aggressive]);
    print_report(&report);
    assert!(report.total_matches > 100, "Hard'da daha çok aktivite");
}

#[test]
fn tuning_expert_mixed_personalities() {
    let report = run_full_season(Difficulty::Expert, &Personality::ALL);
    print_report(&report);
    assert!(report.total_matches > 100, "Expert akıllı pazar yapmalı");
}

#[test]
fn tuning_expert_only_aggressive() {
    let report = run_full_season(
        Difficulty::Expert,
        &[Personality::Aggressive, Personality::Aggressive],
    );
    print_report(&report);
}

#[test]
fn tuning_expert_only_hoarder() {
    let report = run_full_season(
        Difficulty::Expert,
        &[Personality::Hoarder, Personality::Hoarder],
    );
    print_report(&report);
}

#[test]
fn tuning_expert_only_arbitrageur() {
    let report = run_full_season(
        Difficulty::Expert,
        &[Personality::Arbitrageur, Personality::Arbitrageur],
    );
    print_report(&report);
}

/// Kişilik karşılaştırma — her kişiliği tek tek test et, NPC ortalama PnL.
#[test]
fn tuning_personality_comparison() {
    println!("\n╔══════════════════════════════════════════════════╗");
    println!("║  KİŞİLİK KARŞILAŞTIRMA — 90 tick, tek-arketip     ║");
    println!("╠══════════════════════════════════════════════════╣");
    println!("║  Arketip      Tüccar    Sanayici  Dispatch       ║");
    println!("║  ──────────   ───────   ────────  ────────       ║");
    let mut results: Vec<(Personality, f64, f64, u32)> = Vec::new();
    for p in Personality::ALL {
        let report = run_full_season(Difficulty::Expert, &[p]);
        let tuccar_avg = report
            .npc_pnl_by_kind
            .get(&NpcKind::Tuccar)
            .map(|v| v.iter().sum::<f64>() / v.len() as f64)
            .unwrap_or(0.0);
        let sanayici_avg = report
            .npc_pnl_by_kind
            .get(&NpcKind::Sanayici)
            .map(|v| v.iter().sum::<f64>() / v.len() as f64)
            .unwrap_or(0.0);
        results.push((p, tuccar_avg, sanayici_avg, report.total_dispatched));
    }
    results.sort_by(|a, b| {
        let ta = a.1 + a.2;
        let tb = b.1 + b.2;
        tb.partial_cmp(&ta).unwrap_or(std::cmp::Ordering::Equal)
    });
    for (p, t, s, d) in &results {
        println!(
            "║  {} {:<10}  {:+7.0}₺  {:+7.0}₺  {:>3}            ║",
            p.emoji(),
            p.label(),
            t,
            s,
            d
        );
    }
    println!("╚══════════════════════════════════════════════════╝");
}

/// Adaptive difficulty test — insan lider olunca NPC'lerin reaksiyonu.
#[test]
fn tuning_adaptive_difficulty() {
    println!("\n╔══════════════════════════════════════════════════╗");
    println!("║  ADAPTIVE DIFFICULTY — insan cash etkisi          ║");
    println!("╠══════════════════════════════════════════════════╣");
    let cases = [
        ("Geride (5K)", 5_000),
        ("Normal (40K)", 40_000),
        ("Lider (200K)", 200_000),
        ("Aşırı lider (500K)", 500_000),
    ];
    for (label, cash) in cases {
        let report = run_full_season_with_human_cash(Difficulty::Expert, &Personality::ALL, cash);
        let tuccar_avg = report
            .npc_pnl_by_kind
            .get(&NpcKind::Tuccar)
            .map(|v| v.iter().sum::<f64>() / v.len() as f64)
            .unwrap_or(0.0);
        let sanayici_avg = report
            .npc_pnl_by_kind
            .get(&NpcKind::Sanayici)
            .map(|v| v.iter().sum::<f64>() / v.len() as f64)
            .unwrap_or(0.0);
        let total_npc = tuccar_avg + sanayici_avg;
        println!(
            "║  {:<18}  Tüccar={:+7.0}  Sanayici={:+7.0}  Toplam={:+8.0} ║",
            label, tuccar_avg, sanayici_avg, total_npc
        );
    }
    println!("║                                                  ║");
    println!("║  Beklenen: insan ne kadar lider, NPC PnL o kadar  ║");
    println!("║  yüksek (catch-up boost). Mevcut tuning            ║");
    println!("║  utility × (1 + lead_boost × 0.3) — agresifleşme.  ║");
    println!("╚══════════════════════════════════════════════════╝");
}
