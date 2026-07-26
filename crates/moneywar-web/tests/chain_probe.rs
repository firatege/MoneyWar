//! Üretim zincirinin nerede tıkandığını ölçer.
//!
//! Denetim üst katmanda büyük arz açığı gösteriyor (Ekmek 22.5× talep/arz,
//! Ziyafet pazarında yalnız %15 ihtimalle fiyat oluşuyor). İki olası sebep
//! var ve ayırt edilmesi gerekiyor:
//!
//!   a) **Kapasite**: üst katmanda yeterli fabrika yok.
//!   b) **Tıkanıklık**: fabrika var ama girdisi gelmiyor.
//!
//! Bu test katman katman fabrika sayısı, üretilen birim ve atıl oranını
//! yan yana koyar; hangisi olduğu tabloda görünür.

use std::collections::BTreeMap;

use moneywar_domain::{NpcKind, ProductKind};
use moneywar_engine::LogEvent;
use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test chain_probe -- --ignored --nocapture`"]
fn where_does_the_production_chain_choke() {
    let mut d = SimDriver::new(moneywar_web::DEFAULT_SEED, 350, 3, moneywar_web::DIFFICULTY);

    let mut produced: BTreeMap<ProductKind, u64> = BTreeMap::new();
    let mut idle_events: BTreeMap<ProductKind, u64> = BTreeMap::new();
    // Alıcı'nın tükettiği birim — talebin gerçek büyüklüğü.
    let mut consumed: BTreeMap<ProductKind, u64> = BTreeMap::new();

    for _ in 0..350 {
        d.step();
        for entry in &d.last_report.entries {
            match &entry.event {
                LogEvent::ProductionCompleted { product, units, .. } => {
                    *produced.entry(*product).or_default() += u64::from(*units);
                }
                // Olay ürünü taşımıyor, fabrika kimliğini taşıyor.
                LogEvent::FactoryIdle { factory_id, .. } => {
                    if let Some(f) = d.state.factories.get(factory_id) {
                        *idle_events.entry(f.product).or_default() += 1;
                    }
                }
                LogEvent::OrderMatched {
                    product,
                    quantity,
                    buyer,
                    ..
                } => {
                    // Alıcı satın aldığı malı tüketir — zincirin çıkışı.
                    if d.state
                        .players
                        .get(buyer)
                        .and_then(|p| p.npc_kind)
                        .is_some_and(|k| k == NpcKind::Alici)
                    {
                        *consumed.entry(*product).or_default() += u64::from(*quantity);
                    }
                }
                _ => {}
            }
        }
    }

    // Katman başına fabrika sayısı (sezon sonu).
    let mut fab_by_product: BTreeMap<ProductKind, (u32, u32)> = BTreeMap::new();
    let idle_threshold = moneywar_engine::IDLE_FACTORY_THRESHOLD;
    for f in d.state.factories.values() {
        let e = fab_by_product.entry(f.product).or_default();
        e.0 += 1;
        if f.is_atil(d.state.current_tick, idle_threshold) {
            e.1 += 1;
        }
    }

    println!("\n── Üretim zinciri tanısı (350 tick) ────────────────────────────────");
    println!(
        "{:<18} {:>4} {:>7} {:>6} {:>10} {:>10} {:>9}",
        "ürün", "kat", "fabrika", "atıl", "üretildi", "tüketildi", "karşılama"
    );
    println!("{}", "-".repeat(70));

    let mut rows: Vec<_> = ProductKind::ALL
        .into_iter()
        .filter(|p| !p.is_raw())
        .collect();
    rows.sort_by_key(|p| p.tier());

    for p in rows {
        let (fabs, idle) = fab_by_product.get(&p).copied().unwrap_or((0, 0));
        let made = produced.get(&p).copied().unwrap_or(0);
        let eaten = consumed.get(&p).copied().unwrap_or(0);
        // Üretilen, tüketilenin kaçta kaçını karşılıyor.
        let cover = if eaten == 0 {
            "—".to_string()
        } else {
            format!("{:.0}%", made as f64 / eaten as f64 * 100.0)
        };
        println!(
            "{:<18} {:>4} {:>7} {:>6} {:>10} {:>10} {:>9}",
            p.display_name(),
            p.tier(),
            fabs,
            idle,
            made,
            eaten,
            cover
        );
    }

    println!("\nfabrika 'girdi yok' olayı (ürün başına):");
    let mut idle_rows: Vec<_> = idle_events.into_iter().collect();
    idle_rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (p, n) in idle_rows.iter().take(8) {
        println!("   {:<18} {n}", p.display_name());
    }
    println!("────────────────────────────────────────────────────────────────────\n");
}
