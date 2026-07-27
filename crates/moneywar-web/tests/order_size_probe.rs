//! Emir büyüklüğü — kim tek seferde kaç birim istiyor?
//!
//! Zincirin tepesi doluyor ama tüketici ara malı kapıyor. Yüzdelik iştah
//! ayarı bunu ancak kısmen düzeltti; asıl şüphe **orantıda**: 100 birim
//! ekmek üretiliyorsa bir hane 5 birim almalı, bir fabrika 20-50 —
//! fabrika bir batch'i doldurmak zorunda çünkü.
//!
//! Bu test rol × ürün bazında emir büyüklüğünü **ve teklif fiyatını**
//! çıkarır.
//!
//! # Bulgu
//!
//! Miktar zaten doğru — tüketici emir başına 6 birim, fabrika 41 istiyor.
//! Belirleyici olan **fiyat** ve tablo kendi kontrol grubunu içeriyor:
//!
//! ```text
//! ürün        rol         ort teklif    o zincirin karşılaması
//! Kumaş       Sanayici        66.05₺  →  Elbise  %97-108  sağlıklı
//! Kumaş       Alıcı           60.84₺
//!
//! Ekmek       Sanayici       108.03₺  →  Ziyafet %13-32   aç
//! Ekmek       Alıcı          145.98₺
//! ```
//!
//! Zincir, fabrikanın tüketiciyi **geçebildiği** yerde çalışıyor; geçemediği
//! yerde tıkanıyor. Sebebi de görünür: Ziyafet 180₺'ye satılıyor ama tek
//! ana girdisi Ekmek piyasada 146₺ — o fiyata üretim zaten kârsız.
//!
//! Dolaylı kaldıraçların hiçbiri bu sayıyı değiştirmediği için hiçbiri
//! çözmedi (miktar, kuyruk önceliği, iştah, ölçek, fiyat merdiveni —
//! ölçümler `alici.rs` ve `balance.rs`'te). Çözüm doğrudan
//! `pricing::derived_input_ceiling`'in ileri bakmasında.

use std::collections::BTreeMap;

use moneywar_domain::{NpcKind, ProductKind};
use moneywar_engine::LogEvent;
use moneywar_web::driver::SimDriver;

#[derive(Default)]
struct Stat {
    orders: u64,
    units: u64,
    max: u32,
    /// Teklif fiyatlarının toplamı (cent) — ortalama için.
    price_cents: i64,
}

impl Stat {
    fn avg(&self) -> f64 {
        if self.orders == 0 {
            0.0
        } else {
            self.units as f64 / self.orders as f64
        }
    }

    /// Ortalama teklif fiyatı (lira). Yarışı kimin kazandığı buradan okunur.
    fn avg_price(&self) -> f64 {
        if self.orders == 0 {
            0.0
        } else {
            self.price_cents as f64 / self.orders as f64 / 100.0
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
            s.price_cents += order.unit_price.as_cents();
        }
    }

    println!("\n── Alım emri büyüklüğü (350 tick) ──────────────────────────");
    println!(
        "{:<16} {:<12} {:>8} {:>10} {:>8} {:>11}",
        "ürün", "rol", "emir", "ort birim", "en çok", "ort teklif"
    );
    println!("{}", "-".repeat(72));

    // Ara mallar en kritik: fabrika ile tüketici burada yarışıyor.
    for product in [
        ProductKind::Sarap,
        ProductKind::Zeytinyagi,
        ProductKind::Ekmek,
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
                "{:<16} {:<12} {:>8} {:>10.1} {:>8} {:>10.2}₺",
                product.display_name(),
                kind.label(),
                s.orders,
                s.avg(),
                s.max,
                s.avg_price()
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
