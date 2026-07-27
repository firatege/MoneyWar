//! Devralma mekaniği neden ateşlemiyor?
//!
//! `AcquireFactory` eklendiğinde sezonda **sıfır** devralma oldu — kartel ve
//! özel çiftlikle aynı hata sınıfı: kapı ulaşılamaz. Bu araç kapıları tek tek
//! sayar, hangisinin kapalı olduğu tabloda görünür.

use moneywar_domain::balance::{ACQUISITION_DISTRESS_CASH_LIRA, ACQUISITION_IDLE_TICKS};
use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test acquisition_probe -- --ignored --nocapture`"]
fn which_gate_blocks_acquisition() {
    let mut d = SimDriver::new(
        moneywar_web::DEFAULT_SEED,
        moneywar_web::SEASON_TICKS,
        3,
        moneywar_web::DIFFICULTY,
    );

    let (mut samples, mut idle_ok, mut broke_ok, mut both_ok) = (0u64, 0u64, 0u64, 0u64);
    let mut min_owner_cash = i64::MAX;
    let mut broke_owner_ticks = 0u64;

    for _ in 0..moneywar_web::SEASON_TICKS {
        d.step();
        let tick = d.state.current_tick;
        for f in d.state.factories.values() {
            samples += 1;
            let idle = f.is_atil(tick, ACQUISITION_IDLE_TICKS);
            let cash = d
                .state
                .players
                .get(&f.owner)
                .map_or(0, |p| p.cash.as_cents());
            min_owner_cash = min_owner_cash.min(cash);
            let broke = cash <= ACQUISITION_DISTRESS_CASH_LIRA.saturating_mul(100);
            if idle {
                idle_ok += 1;
            }
            if broke {
                broke_ok += 1;
                broke_owner_ticks += 1;
            }
            if idle && broke {
                both_ok += 1;
            }
        }
    }

    let pct = |n: u64| n as f64 / samples.max(1) as f64 * 100.0;
    println!("\n── Devralma kapıları ────────────────────────────────────");
    println!("örneklem (fabrika × tick):        {samples}");
    println!(
        "kapı 1 — {ACQUISITION_IDLE_TICKS}+ tick atıl:            %{:.1}",
        pct(idle_ok)
    );
    println!(
        "kapı 2 — sahibi <{ACQUISITION_DISTRESS_CASH_LIRA}₺ nakit:  %{:.2}",
        pct(broke_ok)
    );
    println!("kapı 1 + 2 birlikte:              %{:.3}", pct(both_ok));
    println!(
        "\nen düşük gözlenen sahip nakdi:    {}₺",
        min_owner_cash / 100
    );
    println!("nakitsiz sahip görülen tick:      {broke_owner_ticks}");
    println!("─────────────────────────────────────────────────────────\n");
}
