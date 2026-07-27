//! İşgücü havuzu: kadrosuz duran fabrika sayısı ve havuz doluluğu.
//!
//! Canlıda t250'de 44 fabrikanın 18'i sıfır kadroyla duruyordu — kurulmuş,
//! parası ödenmiş, tek işçisi yok. Havuzu büyütmek bunu düzeltir mi, yoksa
//! yalnız kuyruğu mu kaydırır?

use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test labor_probe -- --ignored --nocapture`"]
fn how_many_factories_stand_unstaffed() {
    let mut d = SimDriver::new(moneywar_web::DEFAULT_SEED, moneywar_web::SEASON_TICKS, 3, moneywar_web::DIFFICULTY);
    let mut rows: Vec<(u32, usize, usize, u32, u32, u32)> = Vec::new();

    for _ in 0..moneywar_web::SEASON_TICKS {
        d.step();
        let t = d.state.current_tick.value();
        if !t.is_multiple_of(70) {
            continue;
        }
        let total = d.state.factories.len();
        let empty = d.state.factories.values().filter(|f| f.employees == 0).count();
        let employed: u32 = d.state.factories.values().map(|f| f.employees).sum();
        let farm_staff: u32 = d.state.private_farms.values().map(|f| f.employees).sum();
        let wanted: u32 = d.state.factories.values().map(|f| f.required_employees()).sum();
        let pool = moneywar_domain::balance::labor_pool_at(t);
        rows.push((t, total, empty, employed + farm_staff, wanted, pool));
    }

    let growth = moneywar_domain::balance::LABOR_POOL_GROWTH_PER_100_TICKS;
    println!("\n── İşgücü (havuz büyüme +{growth}/100 tick) ──────────────────────");
    println!(
        "{:>5} {:>8} {:>10} {:>9} {:>9} {:>7} {:>9}",
        "tick", "fabrika", "kadrosuz", "istihdam", "istenen", "havuz", "doluluk"
    );
    println!("{}", "-".repeat(62));
    for (t, total, empty, employed, wanted, pool) in rows {
        println!(
            "{t:>5} {total:>8} {empty:>10} {employed:>9} {wanted:>9} {pool:>7} {:>8.0}%",
            f64::from(employed) / f64::from(pool.max(1)) * 100.0
        );
    }
    println!("──────────────────────────────────────────────────────────────\n");
}
