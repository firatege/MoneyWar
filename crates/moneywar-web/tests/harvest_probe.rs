//! Dinamik hasat tepkisi doyuyor mu?
//!
//! Çiftçi'nin hasadı fiyata bakar: referans fiyat baz fiyatın 3 katını
//! aşarsa kıtlık kabul edip daha çok üretir. Kod bunu iddia ediyordu ama
//! tepkinin **tavana çarpıp çarpmadığı** hiç ölçülmemişti.
//!
//! Ölçüm net çıktı: zamanın %60'ında en üst dal seçiliyor. Yani kıtlık
//! sinyali sürekli var, tepki doymuş — tavan gerçek kısıt. Bu, sabiti
//! büyütmenin işe yarayacağının kanıtı oldu (300 → 600).

use std::collections::BTreeMap;

use moneywar_domain::{CityId, ProductKind};
use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test harvest_probe -- --ignored --nocapture`"]
fn does_the_scarcity_response_saturate() {
    let mut d = SimDriver::new(
        moneywar_web::DEFAULT_SEED,
        moneywar_web::SEASON_TICKS,
        3,
        moneywar_web::DIFFICULTY,
    );
    let mut hist: BTreeMap<i64, u64> = BTreeMap::new();

    for _ in 0..moneywar_web::SEASON_TICKS {
        d.step();
        for city in CityId::ALL {
            for product in ProductKind::RAW_MATERIALS {
                let reference = d
                    .state
                    .reference_price(city, product)
                    .map_or(0, moneywar_domain::Money::as_cents);
                if reference == 0 {
                    continue;
                }
                let baseline = d
                    .state
                    .price_baseline
                    .get(&(city, product))
                    .map_or(1, |m| m.as_cents())
                    .max(1);
                // Motordaki eşiklerin aynısı (bkz. `economy.rs`, price_factor_pct).
                let ratio = reference * 100 / baseline;
                let bucket = if ratio > 300 {
                    300
                } else if ratio > 150 {
                    150
                } else if ratio > 110 {
                    110
                } else if ratio < 50 {
                    50
                } else if ratio < 80 {
                    80
                } else {
                    100
                };
                *hist.entry(bucket).or_default() += 1;
            }
        }
    }

    let total: u64 = hist.values().sum::<u64>().max(1);
    println!("\n── Dinamik hasat tepkisi ────────────────────────────────");
    println!("(referans fiyat ÷ baz fiyat — hangi dal ne sıklıkla seçiliyor)");
    for (bucket, n) in &hist {
        let label = match bucket {
            300 => "kıtlık tavanı  (>%300)",
            150 => "güçlü kıtlık   (>%150)",
            110 => "hafif kıtlık   (>%110)",
            80 => "düşük fiyat    (<%80)",
            50 => "bolluk         (<%50)",
            _ => "tepki yok  (%80-110)",
        };
        let pct = *n as f64 / total as f64 * 100.0;
        let bar = "█".repeat((pct / 2.0).round() as usize);
        println!("   {label:<24} %{pct:>5.1}  {bar}");
    }
    println!(
        "\ntavan sabiti şu an %{}",
        moneywar_domain::balance::HARVEST_SCARCITY_CAP_PCT
    );
    println!("─────────────────────────────────────────────────────────\n");
}
