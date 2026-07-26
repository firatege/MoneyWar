//! Özel çiftliğin neden hiç kurulmadığını ölçer.
//!
//! Canlıda `özel çiftlik: 0` görünüyordu. Kurulmama sebebi üç yerden biri
//! olabilir: aday hiç üretilmiyor, komut üretiliyor ama reddediliyor, ya da
//! aday üretiliyor fakat skorlamada eleniyor. Bu test üçünü ayırt eder.

use moneywar_domain::balance::{PRIVATE_FARM_BUILD_COST_LIRA, PRIVATE_FARM_MAX_PER_OWNER};
use moneywar_domain::{Money, NpcKind};
use moneywar_engine::LogEvent;
use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test private_farm_probe -- --ignored --nocapture`"]
fn why_are_private_farms_never_built() {
    let mut d = SimDriver::new(moneywar_web::DEFAULT_SEED, 350, 3, moneywar_web::DIFFICULTY);

    let build_cost = Money::from_lira(PRIVATE_FARM_BUILD_COST_LIRA).expect("maliyet");
    // NPC'nin aradığı tampon: maliyet × 1.5.
    let npc_gate = Money::from_cents(build_cost.as_cents() * 3 / 2);

    let mut built = 0u32;
    let mut build_ticks: Vec<u32> = Vec::new();
    // Kaç Sanayici kapıyı (>=5 fabrika) geçiyor, tick'e göre.
    let mut gate_pass_by_phase: std::collections::BTreeMap<u32, u64> = Default::default();
    let mut rejected = 0u32;
    let mut reject_reasons: std::collections::BTreeMap<String, u32> = Default::default();

    // Sanayici nakdinin dağılımı — eşiği kaç tick karşılıyor.
    let mut sanayici_samples = 0u64;
    let mut above_gate = 0u64;
    let mut above_cost = 0u64;
    let mut peak_cash = Money::ZERO;
    // Tarlaya ihtiyaç duyan (girdisi eksik) fabrikası olan Sanayici sayısı.
    let mut with_shortage = 0u64;
    // Fabrika sayısı dağılımı — aday üretiminin asıl kapısı burada.
    let mut fab_hist: std::collections::BTreeMap<usize, u64> = Default::default();
    let mut max_fabs = 0usize;

    for _ in 0..350 {
        d.step();

        for entry in &d.last_report.entries {
            match &entry.event {
                LogEvent::PrivateFarmBuilt { .. } => {
                    built += 1;
                    build_ticks.push(d.state.current_tick.value());
                }
                LogEvent::CommandRejected { command, reason } => {
                    if matches!(command, moneywar_domain::Command::BuildPrivateFarm { .. }) {
                        rejected += 1;
                        *reject_reasons.entry(reason.clone()).or_default() += 1;
                    }
                }
                _ => {}
            }
        }

        for p in d.state.players.values() {
            if p.npc_kind != Some(NpcKind::Sanayici) {
                continue;
            }
            sanayici_samples += 1;
            if p.cash >= npc_gate {
                above_gate += 1;
            }
            if p.cash >= build_cost {
                above_cost += 1;
            }
            peak_cash = peak_cash.max(p.cash);

            // Kendi fabrikalarında ham madde açığı var mı?
            let short = d.state.factories.values().any(|f| {
                f.owner == p.id
                    && f.product.raw_input().is_some_and(|raw| {
                        raw.is_raw() && p.inventory.get(f.city, raw) < f.batch_size()
                    })
            });
            if short {
                with_shortage += 1;
            }

            let owned = d.state.factories.values().filter(|f| f.owner == p.id).count();
            *fab_hist.entry(owned).or_default() += 1;
            max_fabs = max_fabs.max(owned);
            if owned >= 5 {
                *gate_pass_by_phase
                    .entry(d.state.current_tick.value() / 50 * 50)
                    .or_default() += 1;
            }
        }
    }

    let pct = |n: u64| n as f64 / sanayici_samples.max(1) as f64 * 100.0;

    println!("\n── Özel çiftlik tanısı (350 tick) ─────────────────────────");
    println!("kurulan tarla:            {built}");
    println!("reddedilen komut:         {rejected}");
    for (reason, n) in &reject_reasons {
        println!("   {n}× {reason}");
    }
    println!();
    println!("Sanayici örneklemi:       {sanayici_samples} (oyuncu×tick)");
    println!(
        "girdi açığı olan:         {:.0}%  ← tarlaya ihtiyaç var mı",
        pct(with_shortage)
    );
    println!(
        "nakit ≥ maliyet ({:>6}₺): {:.1}%",
        PRIVATE_FARM_BUILD_COST_LIRA,
        pct(above_cost)
    );
    println!(
        "nakit ≥ NPC eşiği ({:>6}₺): {:.1}%  ← aday üretmenin ön koşulu",
        npc_gate.as_cents() / 100,
        pct(above_gate)
    );
    println!("en yüksek Sanayici nakdi: {}₺", peak_cash.as_cents() / 100);
    println!("tarla kotası/sahip:       {PRIVATE_FARM_MAX_PER_OWNER}");
    println!();
    println!("Sanayici fabrika sayısı dağılımı (aday kapısı: >= 8):");
    for (n, count) in &fab_hist {
        let bar = "█".repeat((pct(*count) / 2.0).round() as usize);
        println!("   {n:>2} fabrika  {:>5.1}%  {bar}", pct(*count));
    }
    println!("en çok fabrikası olan:    {max_fabs}");
    println!();
    println!("tarla kurulma tick'leri:  {build_ticks:?}");
    println!("kapıyı (>=5 fab) geçen Sanayici sayısı, 50'lik dilimlerde:");
    for (phase, n) in &gate_pass_by_phase {
        println!("   t{phase:>3}-{:>3}  {n}", phase + 49);
    }
    println!("───────────────────────────────────────────────────────────\n");
}
