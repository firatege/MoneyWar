//! Ölçek sorusu: her şeyi 100/1000 ile çarpınca ne oluyor?
//!
//! Üç şey ölçülüyor:
//!
//! 1. **Gerçekten "1'er 1'er" mi alınıyor?** Emir büyüklüğü ile *gerçekleşen
//!    eşleşme* büyüklüğü farklı olabilir: emir 10 birim isteyip 1 birim
//!    dolabilir. İzleyicinin gördüğü ikincisi.
//! 2. **Emir sayısı ölçekle artar mı?** Alıcı'nın emir tavanı şehir × ürün
//!    ile sabit; ama mal ucuzlayınca hâlihazırda düşen emirler geçmeye
//!    başlayabilir — bu da "10 kat fazla sipariş" hissi verir.
//! 3. **Ne patlıyor?** Rol kârları, para arzı, kısmi doluluk.

use std::collections::BTreeMap;

use moneywar_domain::NpcKind;
use moneywar_engine::LogEvent;
use moneywar_web::driver::SimDriver;

#[derive(Default)]
struct Acc {
    orders: u64,
    order_units: u64,
    fills: u64,
    fill_units: u64,
}

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test scale_probe -- --ignored --nocapture`"]
fn how_big_is_an_actual_fill() {
    let mut d = SimDriver::new(moneywar_web::DEFAULT_SEED, 350, 3, moneywar_web::DIFFICULTY);
    let mut acc: BTreeMap<NpcKind, Acc> = BTreeMap::new();
    // Eşleşme büyüklüğü dağılımı — "1'er 1'er" gerçekten öyle mi.
    let mut fill_hist: BTreeMap<u32, u64> = BTreeMap::new();

    for _ in 0..350 {
        d.step();
        for entry in &d.last_report.entries {
            match &entry.event {
                LogEvent::CommandAccepted {
                    command: moneywar_domain::Command::SubmitOrder(o),
                } => {
                    if !o.side.is_buy() {
                        continue;
                    }
                    if let Some(k) = d.state.players.get(&o.player).and_then(|p| p.npc_kind) {
                        let e = acc.entry(k).or_default();
                        e.orders += 1;
                        e.order_units += u64::from(o.quantity);
                    }
                }
                LogEvent::OrderMatched {
                    quantity, buyer, ..
                } => {
                    *fill_hist.entry(*quantity).or_default() += 1;
                    if let Some(k) = d.state.players.get(buyer).and_then(|p| p.npc_kind) {
                        let e = acc.entry(k).or_default();
                        e.fills += 1;
                        e.fill_units += u64::from(*quantity);
                    }
                }
                _ => {}
            }
        }
    }

    let scale = moneywar_domain::balance::PRODUCTION_SCALE_PCT;
    println!("\n── Emir vs gerçekleşen eşleşme (ölçek %{scale}) ─────────────────");
    println!(
        "{:<12} {:>8} {:>11} {:>8} {:>11} {:>9}",
        "rol", "emir", "emir/birim", "eşleşme", "eşleşme/br", "doluluk"
    );
    println!("{}", "-".repeat(64));
    for (k, a) in &acc {
        let ou = if a.orders == 0 { 0.0 } else { a.order_units as f64 / a.orders as f64 };
        let fu = if a.fills == 0 { 0.0 } else { a.fill_units as f64 / a.fills as f64 };
        let cover = if a.order_units == 0 {
            0.0
        } else {
            a.fill_units as f64 / a.order_units as f64 * 100.0
        };
        println!(
            "{:<12} {:>8} {ou:>11.1} {:>8} {fu:>11.1} {cover:>8.0}%",
            k.label(),
            a.orders,
            a.fills
        );
    }

    println!("\nEşleşme büyüklüğü dağılımı (kaç birim el değiştirdi):");
    let total: u64 = fill_hist.values().sum();
    let buckets = [(1u32, 1u32), (2, 3), (4, 9), (10, 24), (25, 49), (50, u32::MAX)];
    for (lo, hi) in buckets {
        let n: u64 = fill_hist
            .iter()
            .filter(|(q, _)| **q >= lo && **q <= hi)
            .map(|(_, c)| *c)
            .sum();
        let pct = n as f64 / total.max(1) as f64 * 100.0;
        let bar = "█".repeat((pct / 2.0).round() as usize);
        let label = if hi == u32::MAX {
            format!("{lo}+")
        } else if lo == hi {
            format!("{lo}")
        } else {
            format!("{lo}-{hi}")
        };
        println!("   {label:>6} birim  {pct:>5.1}%  {bar}");
    }
    println!("────────────────────────────────────────────────────────────────\n");
}
