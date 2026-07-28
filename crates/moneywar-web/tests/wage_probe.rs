//! Hayat pahalılığı endeksi ve ücret yükü.
//!
//! Ücret ekonomideki tek piyasa dışı fiyattı: her mal arz-talebe göre
//! fiyatlanırken işçinin fiyatı sabit çakılıydı, dolayısıyla hanenin **reel**
//! geliri sezon boyunca eriyordu. Endeksleme bunu düzeltti ama bu sefer ücret
//! yükü firmaları ezdi.
//!
//! Bu probe endeksin sezon boyunca nereye çıktığını ve kimin ne kadar ücret
//! ödediğini gösterir — ayar tahminle değil ölçümle yapılsın diye.

use moneywar_domain::{CityId, NpcKind, ProductKind};
use moneywar_web::driver::SimDriver;

/// `economy.rs::cost_of_living_index` ile **aynı** formül: hane sepetindeki
/// mamullerin sezon başı çapasına oranı.
fn cpi(d: &SimDriver) -> i64 {
    let (mut now, mut base) = (0i64, 0i64);
    for city in CityId::ALL {
        for product in ProductKind::FINISHED_GOODS {
            if product.need_tier().is_none() {
                continue;
            }
            let Some(b) = d.state.price_baseline_initial.get(&(city, product)) else {
                continue;
            };
            if b.as_cents() <= 0 {
                continue;
            }
            let Some(n) = d.state.reference_price(city, product) else {
                continue;
            };
            now += n.as_cents();
            base += b.as_cents();
        }
    }
    if base <= 0 { 100 } else { now * 100 / base }
}

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test wage_probe -- --ignored --nocapture`"]
fn how_high_does_the_cost_of_living_climb() {
    let mut d = SimDriver::new(
        moneywar_web::DEFAULT_SEED,
        moneywar_web::SEASON_TICKS,
        3,
        moneywar_web::DIFFICULTY,
    );

    println!("\n── Hayat pahalılığı endeksi (100 = sezon başı) ─────────────");
    println!(
        "{:>6} {:>8} {:>12} {:>12} {:>12} {:>10}",
        "tick", "endeks", "istihdam", "Alıcı nakit", "Sanayici", "Tüccar"
    );
    println!("{}", "-".repeat(64));

    for _ in 0..moneywar_web::SEASON_TICKS {
        d.step();
        let t = d.state.current_tick.value();
        if !t.is_multiple_of(50) {
            continue;
        }
        let idx = cpi(&d);
        let employed: u32 = d.state.factories.values().map(|f| f.employees).sum::<u32>()
            + d.state.private_farms.values().map(|f| f.employees).sum::<u32>();
        let cash_of = |kind: NpcKind| -> i64 {
            d.state
                .players
                .values()
                .filter(|p| p.npc_kind == Some(kind))
                .map(|p| p.cash.as_cents() / 100)
                .sum()
        };
        println!(
            "{:>6} {:>8} {:>12} {:>12} {:>12} {:>10}",
            t,
            idx,
            employed,
            cash_of(NpcKind::Alici),
            cash_of(NpcKind::Sanayici),
            cash_of(NpcKind::Tuccar),
        );
    }

    println!("\n  «endeks» tavanı economy.rs'te INDEX_CAP ile sınırlı.");
    println!("  Nakit sütunları rolün **toplam** nakdi (lira).");
}
