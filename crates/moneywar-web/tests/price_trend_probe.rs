//! Fiyatlar inip çıkıyor mu, yoksa hep mi yukarı?
//!
//! Sağlıklı bir pazarda fiyat iki yöne de hareket eder: arz bollaşınca düşer,
//! kıtlaşınca çıkar. Oyunda fiyatların sezon boyunca tek yönlü tırmandığı
//! gözlemlendi. Bu probe iddiayı sayıyla sınar: her (şehir, ürün) kovası için
//! fiyatın kaç tick yükseldiğini, kaç tick düştüğünü ve sezon sonundaki net
//! kaymayı ölçer.
//!
//! Yukarı/aşağı tick sayısı dengeliyken net kayma büyükse sorun adımların
//! **boyunda** (yukarı adımlar daha iri). Tick sayısı da yukarı ağırlıklıysa
//! sorun adımların **sıklığında**.

use std::collections::BTreeMap;

use moneywar_domain::{CityId, ProductKind};
use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test price_trend_probe -- --ignored --nocapture`"]
fn do_prices_ever_fall() {
    let mut d = SimDriver::new(
        moneywar_web::DEFAULT_SEED,
        moneywar_web::SEASON_TICKS,
        3,
        moneywar_web::DIFFICULTY,
    );

    // (şehir, ürün) → (yukarı tick, aşağı tick, sabit tick, ilk fiyat, son fiyat)
    let mut stats: BTreeMap<(CityId, ProductKind), (u32, u32, u32, i64, i64)> = BTreeMap::new();
    let mut prev: BTreeMap<(CityId, ProductKind), i64> = BTreeMap::new();

    for _ in 0..moneywar_web::SEASON_TICKS {
        d.step();
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                let Some(now) = d.state.reference_price(city, product) else {
                    continue;
                };
                let now = now.as_cents();
                if now <= 0 {
                    continue;
                }
                let key = (city, product);
                if let Some(&before) = prev.get(&key) {
                    let e = stats.entry(key).or_insert((0, 0, 0, before, now));
                    match now.cmp(&before) {
                        std::cmp::Ordering::Greater => e.0 += 1,
                        std::cmp::Ordering::Less => e.1 += 1,
                        std::cmp::Ordering::Equal => e.2 += 1,
                    }
                    e.4 = now;
                }
                prev.insert(key, now);
            }
        }
    }

    // Ürün bazında topla — 5 şehrin ortalaması.
    let mut per_product: BTreeMap<String, (u32, u32, u32, f64, u32)> = BTreeMap::new();
    for ((_, product), (up, down, flat, first, last)) in &stats {
        let e = per_product
            .entry(format!("{product:?}"))
            .or_insert((0, 0, 0, 0.0, 0));
        e.0 += up;
        e.1 += down;
        e.2 += flat;
        e.3 += *last as f64 / (*first).max(1) as f64;
        e.4 += 1;
    }

    println!("\n── Fiyat yönü (500 tick, 5 şehir toplamı) ──────────────────");
    println!(
        "{:<18} {:>8} {:>8} {:>8} {:>10} {:>12}",
        "ürün", "yukarı", "aşağı", "sabit", "yukarı%", "net kayma"
    );
    println!("{}", "-".repeat(70));

    let mut rows: Vec<_> = per_product.into_iter().collect();
    rows.sort_by(|a, b| {
        let ra = a.1.3 / f64::from(a.1.4.max(1));
        let rb = b.1.3 / f64::from(b.1.4.max(1));
        rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
    });
    let (mut tot_up, mut tot_down) = (0u32, 0u32);
    for (product, (up, down, flat, drift_sum, n)) in &rows {
        let moves = up + down;
        let up_pct = if moves == 0 {
            0.0
        } else {
            f64::from(*up) / f64::from(moves) * 100.0
        };
        let drift = drift_sum / f64::from((*n).max(1));
        println!(
            "{product:<18} {up:>8} {down:>8} {flat:>8} {up_pct:>9.0}% {drift:>11.2}x"
        );
        tot_up += up;
        tot_down += down;
    }
    let moves = tot_up + tot_down;
    println!("{}", "-".repeat(70));
    println!(
        "{:<18} {:>8} {:>8} {:>8} {:>9.0}%",
        "TOPLAM",
        tot_up,
        tot_down,
        "",
        f64::from(tot_up) / f64::from(moves.max(1)) * 100.0
    );
    // Arzın fiyata tepki verip vermediğini görmek için: kim kaç tarla kurdu?
    let mut farms_by_kind: BTreeMap<String, (usize, u32)> = BTreeMap::new();
    for f in d.state.private_farms.values() {
        let kind = d
            .state
            .players
            .get(&f.owner)
            .and_then(|p| p.npc_kind)
            .map_or_else(|| "?".to_string(), |k| format!("{k:?}"));
        let e = farms_by_kind.entry(kind).or_insert((0, 0));
        e.0 += 1;
        e.1 += u32::from(f.level);
    }
    // Çapa (price_baseline) clamp sınırına yapıştı mı? Yapıştıysa maliyet
    // çapası fiyat sinyali olmaktan çıkar ve ona göre satan rol piyasanın
    // altında kalır.
    let (mut at_ceiling, mut at_floor, mut total_b) = (0u32, 0u32, 0u32);
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let (Some(b), Some(i)) = (
                d.state.price_baseline.get(&(city, product)),
                d.state.price_baseline_initial.get(&(city, product)),
            ) else {
                continue;
            };
            if i.as_cents() <= 0 {
                continue;
            }
            total_b += 1;
            let pct = b.as_cents() * 100 / i.as_cents();
            if pct >= 159 {
                at_ceiling += 1;
            } else if pct <= 61 {
                at_floor += 1;
            }
        }
    }
    println!("\n── Çapa clamp durumu (sezon sonu) ──────────────────────────");
    println!(
        "  {at_ceiling}/{total_b} kova tavanda (%160) · {at_floor}/{total_b} tabanda (%60)"
    );

    println!("\n── Tarla sahipliği (sezon sonu) ────────────────────────────");
    if farms_by_kind.is_empty() {
        println!("  (hiç tarla yok)");
    }
    for (kind, (n, lvl_sum)) in &farms_by_kind {
        println!("  {kind:<12} {n:>3} tarla · ort. seviye {:.1}", f64::from(*lvl_sum) / *n as f64);
    }

    println!("\n  «yukarı%» 50 civarı = dengeli salınım · 50'nin çok üstü = tek yönlü tırmanış");
    println!("  «net kayma» sezon sonu ÷ sezon başı fiyat");
}
