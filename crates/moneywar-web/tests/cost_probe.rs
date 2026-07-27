//! Tick başına iş yükü — "bir tick kaç hesap ediyor?"
//!
//! Aritmetik işlem sayısını doğrudan saymak için motoru sayaçlarla delmek
//! gerekirdi. Bunun yerine **kardinaliteleri** ölçüyoruz: kaç oyuncu, kaç
//! emir, kaç kova, kaç fabrika, kaç eşleşme. İşlem sayısı bunların üstüne
//! kod okunarak çarpılır — hangi döngünün kaç kez döndüğü buradan çıkar.
//!
//! Tick 1 ile tick 500 arasındaki fark önemli: emir kitabı ve fabrika sayısı
//! sezon boyunca büyüyor, yani maliyet sabit değil.

use moneywar_domain::OrderSide;
use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test cost_probe -- --ignored --nocapture`"]
fn how_much_work_does_a_tick_do() {
    let mut d = SimDriver::new(
        moneywar_web::DEFAULT_SEED,
        moneywar_web::SEASON_TICKS,
        3,
        moneywar_web::DIFFICULTY,
    );

    struct Row {
        tick: u32,
        players: usize,
        npcs: usize,
        orders: usize,
        buckets: usize,
        max_bucket: usize,
        factories: usize,
        farms: usize,
        caravans: usize,
        inv_entries: usize,
        log_entries: usize,
        matches: usize,
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut total_orders: u64 = 0;
    let mut total_matches: u64 = 0;
    let mut total_inv: u64 = 0;
    let mut total_bucket_sq: u64 = 0;

    for _ in 0..moneywar_web::SEASON_TICKS {
        d.step();
        let t = d.state.current_tick.value();

        let orders: usize = d.state.order_book.values().map(Vec::len).sum();
        let buckets = d.state.order_book.len();
        let max_bucket = d.state.order_book.values().map(Vec::len).max().unwrap_or(0);
        let inv_entries: usize = d
            .state
            .players
            .values()
            .map(|p| p.inventory.entries().count())
            .sum();
        let matches = d
            .state
            .order_book
            .values()
            .flat_map(|o| o.iter())
            .filter(|o| o.side == OrderSide::Buy)
            .count();
        let log_entries = d.last_report.entries.len();

        total_orders += orders as u64;
        total_matches += log_entries as u64;
        total_inv += inv_entries as u64;
        // Eşleştirme kova içinde çift taraflı gezdiği için kare terimi taşır.
        total_bucket_sq += d
            .state
            .order_book
            .values()
            .map(|v| (v.len() as u64).pow(2))
            .sum::<u64>();

        if t == 1 || t == 250 || t == moneywar_web::SEASON_TICKS {
            rows.push(Row {
                tick: t,
                players: d.state.players.len(),
                npcs: d.state.players.values().filter(|p| p.is_npc).count(),
                orders,
                buckets,
                max_bucket,
                factories: d.state.factories.len(),
                farms: d.state.private_farms.len(),
                caravans: d.state.caravans.len(),
                inv_entries,
                log_entries,
                matches,
            });
        }
    }

    println!("\n── Tick başına kardinalite ─────────────────────────────────");
    println!(
        "{:>6} {:>8} {:>7} {:>8} {:>7} {:>10} {:>8} {:>6} {:>8} {:>8} {:>8}",
        "tick", "oyuncu", "npc", "emir", "kova", "enbuyukkova", "fabrika", "tarla", "kervan", "envanter", "logsatır"
    );
    println!("{}", "-".repeat(100));
    for r in &rows {
        println!(
            "{:>6} {:>8} {:>7} {:>8} {:>7} {:>10} {:>8} {:>6} {:>8} {:>8} {:>8}",
            r.tick, r.players, r.npcs, r.orders, r.buckets, r.max_bucket,
            r.factories, r.farms, r.caravans, r.inv_entries, r.log_entries
        );
        let _ = r.matches;
    }

    let ticks = u64::from(moneywar_web::SEASON_TICKS);
    println!("\n── Sezon toplamı ({ticks} tick) ─────────────────────────────");
    println!("  emir·tick toplamı        {total_orders:>12}");
    println!("  emir² ·tick toplamı      {total_bucket_sq:>12}   (eşleştirme kare terimi)");
    println!("  envanter girdisi·tick    {total_inv:>12}");
    println!("  log satırı (olay)        {total_matches:>12}");
    println!("  ortalama emir/tick       {:>12}", total_orders / ticks);
    println!("  ortalama olay/tick       {:>12}", total_matches / ticks);
}
