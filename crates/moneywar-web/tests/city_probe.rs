//! Fabrikalar neden bir şehirde toplanıyor?
//!
//! Gözlem: fabrikaların çoğu İstanbul'a kuruluyor. İki olası sebep var:
//!
//!   a) **Uzmanlık çekimi**: ilk fabrika seçimi şehrin prime ham maddesine
//!      bakıyor; birden çok ürünün girdisi aynı şehre denk geliyorsa hepsi
//!      oraya yığılır.
//!   b) **Slot sırası**: aday listesi `CityId::ALL` sırasında geziliyor ve
//!      eşitlik durumunda ilk şehir kazanıyor.
//!
//! Tablo şehir başına fabrikayı, o şehrin uzmanlığını ve ürün dökümünü yan
//! yana koyar; hangisi olduğu buradan görünür.

use std::collections::BTreeMap;

use moneywar_domain::CityId;
use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test city_probe -- --ignored --nocapture`"]
fn where_do_factories_cluster() {
    // Tek seed yanıltır (o oyunun uzmanlık dağılımına bakarız). Birkaç seed
    // koşup toplarız ki "İstanbul" mu yoksa "listenin ilk şehri" mi belli olsun.
    const SEEDS: usize = 8;

    let mut city_totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut specialty_hits: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_seed: Vec<(u64, Vec<(String, usize, String)>)> = Vec::new();

    for s in 0..SEEDS {
        let seed = moneywar_web::DEFAULT_SEED.wrapping_add(s as u64);
        let mut d = SimDriver::new(seed, moneywar_web::SEASON_TICKS, 3, moneywar_web::DIFFICULTY);
        for _ in 0..moneywar_web::SEASON_TICKS {
            d.step();
        }

        let mut rows: Vec<(String, usize, String)> = Vec::new();
        for city in CityId::ALL {
            let n = d.state.factories.values().filter(|f| f.city == city).count();
            let spec = d
                .state
                .city_specialty
                .get(&city)
                .map_or_else(|| "-".to_string(), |p| format!("{p:?}"));
            *city_totals.entry(format!("{city:?}")).or_insert(0) += n;
            rows.push((format!("{city:?}"), n, spec));
        }

        // Bu oyunda en çok fabrika alan şehrin uzmanlığı neydi?
        if let Some((_, _, spec)) = rows.iter().max_by_key(|(_, n, _)| *n) {
            *specialty_hits.entry(spec.clone()).or_insert(0) += 1;
        }
        per_seed.push((seed, rows));
    }

    println!("\n── Şehir başına fabrika (seed başına) ──────────────────────");
    print!("{:<10}", "seed");
    for city in CityId::ALL {
        print!("{:>12}", format!("{city:?}"));
    }
    println!();
    println!("{}", "-".repeat(10 + 12 * CityId::ALL.len()));
    for (seed, rows) in &per_seed {
        print!("{:<10}", format!("{:#x}", seed & 0xffff));
        for (_, n, spec) in rows {
            print!("{:>12}", format!("{n} ({spec:.4})"));
        }
        println!();
    }

    println!("\n── {SEEDS} seed toplamı ─────────────────────────────────────");
    let grand: usize = city_totals.values().sum();
    let mut totals: Vec<_> = city_totals.into_iter().collect();
    totals.sort_by(|a, b| b.1.cmp(&a.1));
    for (city, n) in &totals {
        let pct = *n as f64 / grand.max(1) as f64 * 100.0;
        println!("  {city:<12} {n:>5} fabrika  {pct:>5.1}%");
    }

    println!("\n── En kalabalık şehrin uzmanlığı (kaç seed'de) ─────────────");
    let mut hits: Vec<_> = specialty_hits.into_iter().collect();
    hits.sort_by(|a, b| b.1.cmp(&a.1));
    for (spec, n) in hits {
        println!("  {spec:<14} {n}/{SEEDS}");
    }
    println!("\n  Şehir sabitse sebep slot sırası; uzmanlık sabitse çekim.");
}
