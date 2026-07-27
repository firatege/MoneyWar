//! İki soru:
//!
//! 1. **Sanayici Tüccar'a niye satmıyor?** Rol akışında Tüccar → Alıcı
//!    531B₺, Sanayici → Tüccar yalnız 19.9B₺. Tüccar sattığı malı
//!    nereden alıyor? Aracı gerçekten aracılık ediyor mu, yoksa
//!    başlangıç stoğunu mu eritiyor?
//!
//! 2. **Ekmek ve Ziyafet fabrikaları neden hep atıl?** Atıllık sabit mi,
//!    yoksa sezon boyunca değişiyor mu; ve duruş sebebi ne?

use std::collections::BTreeMap;

use moneywar_domain::{NpcKind, ProductKind};
use moneywar_engine::LogEvent;
use moneywar_web::driver::SimDriver;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test merchant_probe -- --ignored --nocapture`"]
fn does_the_merchant_actually_intermediate() {
    let mut d = SimDriver::new(moneywar_web::DEFAULT_SEED, 350, 3, moneywar_web::DIFFICULTY);

    // Tüccar'ın ürün başına alımı ve satımı (birim).
    let mut tuccar_buy: BTreeMap<ProductKind, u64> = BTreeMap::new();
    let mut tuccar_sell: BTreeMap<ProductKind, u64> = BTreeMap::new();
    // Sanayici mamulünü kime satıyor.
    let mut sanayici_sell_to: BTreeMap<NpcKind, u64> = BTreeMap::new();
    // Duruş sebepleri, ürün başına.
    let mut idle_reason: BTreeMap<(ProductKind, String), u64> = BTreeMap::new();
    // Atıllık zaman içinde: (50'lik dilim, ürün) → atıl fabrika-tick.
    let mut idle_phase: BTreeMap<(u32, ProductKind), u64> = BTreeMap::new();

    let kind_of = |d: &SimDriver, id: moneywar_domain::PlayerId| {
        d.state.players.get(&id).and_then(|p| p.npc_kind)
    };

    for _ in 0..350 {
        d.step();
        let phase = d.state.current_tick.value() / 50 * 50;

        for entry in &d.last_report.entries {
            match &entry.event {
                LogEvent::OrderMatched {
                    product,
                    quantity,
                    buyer,
                    seller,
                    ..
                } => {
                    let q = u64::from(*quantity);
                    if kind_of(&d, *buyer) == Some(NpcKind::Tuccar) {
                        *tuccar_buy.entry(*product).or_default() += q;
                    }
                    if kind_of(&d, *seller) == Some(NpcKind::Tuccar) {
                        *tuccar_sell.entry(*product).or_default() += q;
                    }
                    if kind_of(&d, *seller) == Some(NpcKind::Sanayici)
                        && let Some(bk) = kind_of(&d, *buyer)
                    {
                        *sanayici_sell_to.entry(bk).or_default() += q;
                    }
                }
                LogEvent::FactoryIdle {
                    factory_id, reason, ..
                } => {
                    if let Some(f) = d.state.factories.get(factory_id) {
                        // Sebep metni oyuncu/şehir detayı taşıyor; ilk iki
                        // kelime sınıfı yeterince ayırıyor.
                        let cls: String =
                            reason.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
                        *idle_reason.entry((f.product, cls)).or_default() += 1;
                        *idle_phase.entry((phase, f.product)).or_default() += 1;
                    }
                }
                _ => {}
            }
        }
    }

    println!("\n── 1. Tüccar gerçekten aracılık ediyor mu? ─────────────────────");
    println!("{:<18} {:>10} {:>10} {:>12}", "ürün", "aldı", "sattı", "kendi stoğu");
    println!("{}", "-".repeat(54));
    let (mut tot_buy, mut tot_sell) = (0u64, 0u64);
    for p in ProductKind::ALL {
        let b = tuccar_buy.get(&p).copied().unwrap_or(0);
        let s = tuccar_sell.get(&p).copied().unwrap_or(0);
        if b == 0 && s == 0 {
            continue;
        }
        tot_buy += b;
        tot_sell += s;
        // Aldığından fazla sattıysa aradaki fark başlangıç stoğundan gelmiş.
        let from_stock = s.saturating_sub(b);
        println!(
            "{:<18} {b:>10} {s:>10} {:>12}",
            p.display_name(),
            if from_stock > 0 { from_stock.to_string() } else { "—".into() }
        );
    }
    println!("{}", "-".repeat(54));
    println!(
        "{:<18} {tot_buy:>10} {tot_sell:>10} {:>12}",
        "TOPLAM",
        tot_sell.saturating_sub(tot_buy)
    );
    let intermediation = if tot_sell == 0 {
        0.0
    } else {
        tot_buy.min(tot_sell) as f64 / tot_sell as f64 * 100.0
    };
    println!("sattığının %{intermediation:.0}'ini piyasadan almış (gerisi kendi stoğu)");

    println!("\nSanayici mamulünü kime satıyor:");
    let total: u64 = sanayici_sell_to.values().sum();
    let mut rows: Vec<_> = sanayici_sell_to.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (k, n) in rows {
        println!(
            "   {:<12} {n:>8} birim  %{:.0}",
            k.label(),
            n as f64 / total.max(1) as f64 * 100.0
        );
    }

    println!("\n── 2. Ekmek / Ziyafet neden atıl? ──────────────────────────────");
    for target in [ProductKind::Ekmek, ProductKind::Ziyafet, ProductKind::Kumas] {
        println!("\n{}:", target.display_name());
        let mut reasons: Vec<_> = idle_reason
            .iter()
            .filter(|((p, _), _)| *p == target)
            .map(|((_, r), n)| (r.clone(), *n))
            .collect();
        reasons.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (r, n) in reasons.iter().take(4) {
            println!("   {n:>6}× {r}");
        }
        if reasons.is_empty() {
            println!("   (hiç durmadı)");
        }
        let phases: Vec<String> = (0..7)
            .map(|i| {
                let n = idle_phase.get(&(i * 50, target)).copied().unwrap_or(0);
                format!("t{:03}:{n}", i * 50)
            })
            .collect();
        println!("   zaman içinde: {}", phases.join("  "));
    }
    println!("────────────────────────────────────────────────────────────────\n");
}
