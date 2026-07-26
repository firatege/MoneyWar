//! Emir büyüklüğü — kim tek seferde kaç birim istiyor?
//!
//! Zincirin tepesi doluyor ama tüketici ara malı kapıyor. Yüzdelik iştah
//! ayarı bunu ancak kısmen düzeltti; asıl şüphe **orantıda**: 100 birim
//! ekmek üretiliyorsa bir hane 5 birim almalı, bir fabrika 20-50 —
//! fabrika bir batch'i doldurmak zorunda çünkü.
//!
//! Bu test rol × ürün bazında emir büyüklüğü dağıtımını çıkarır. Tüketici
//! emri fabrika emriyle aynı büyüklükteyse yarışı kim kazanırsa kazansın
//! zincir tıkanır.

use std::collections::BTreeMap;

use moneywar_domain::{NpcKind, ProductKind};
use moneywar_engine::LogEvent;
use moneywar_web::driver::SimDriver;

#[derive(Default)]
struct Stat {
    orders: u64,
    units: u64,
    max: u32,
}

impl Stat {
    fn avg(&self) -> f64 {
        if self.orders == 0 {
            0.0
        } else {
            self.units as f64 / self.orders as f64
        }
    }
}

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test order_size_probe -- --ignored --nocapture`"]
fn how_big_is_each_role_order() {
    let mut d = SimDriver::new(moneywar_web::DEFAULT_SEED, 350, 3, moneywar_web::DIFFICULTY);

    // (rol, ürün) → alım emri istatistiği.
    let mut buy: BTreeMap<(NpcKind, ProductKind), Stat> = BTreeMap::new();

    for _ in 0..350 {
        d.step();
        for entry in &d.last_report.entries {
            let LogEvent::CommandAccepted { command } = &entry.event else {
                continue;
            };
            let moneywar_domain::Command::SubmitOrder(order) = command else {
                continue;
            };
            if order.side != moneywar_domain::OrderSide::Buy {
                continue;
            }
            let Some(kind) = d.state.players.get(&order.player).and_then(|p| p.npc_kind) else {
                continue;
            };
            let s = buy.entry((kind, order.product)).or_default();
            s.orders += 1;
            s.units += u64::from(order.quantity);
            s.max = s.max.max(order.quantity);
        }
    }

    println!("\n── Alım emri büyüklüğü (350 tick) ──────────────────────────");
    println!("{:<16} {:<12} {:>8} {:>10} {:>8}", "ürün", "rol", "emir", "ort birim", "en çok");
    println!("{}", "-".repeat(60));

    // Ara mallar en kritik: fabrika ile tüketici burada yarışıyor.
    for product in [
        ProductKind::Un,
        ProductKind::Ekmek,
        ProductKind::Zeytinyagi,
        ProductKind::Kumas,
    ] {
        let mut rows: Vec<_> = buy
            .iter()
            .filter(|((_, p), _)| *p == product)
            .map(|((k, _), s)| (*k, s))
            .collect();
        rows.sort_by(|a, b| b.1.avg().partial_cmp(&a.1.avg()).unwrap());
        for (kind, s) in rows {
            println!(
                "{:<16} {:<12} {:>8} {:>10.1} {:>8}",
                product.display_name(),
                kind.label(),
                s.orders,
                s.avg(),
                s.max
            );
        }
        println!();
    }

    // Karşılaştırma noktası: bir fabrika batch'i kaç birim girdi ister.
    println!("referans — bir batch için gereken girdi:");
    for p in [ProductKind::Ekmek, ProductKind::Ziyafet, ProductKind::Elbise] {
        if let Some(f) = d.state.factories.values().find(|f| f.product == p) {
            println!(
                "   {:<16} batch {:>4} birim → ana girdi {} × {}",
                p.display_name(),
                f.batch_size(),
                f.batch_size(),
                p.raw_input().map_or("—", ProductKind::display_name),
            );
        }
    }
    println!("────────────────────────────────────────────────────────────\n");
}
