//! Ham madde yarışını kim kazanıyor?
//!
//! Ölçümde ham maddenin **%37'sini Spekülatör, %30'unu Tüccar** alıyor;
//! malı gerçekten mala çeviren Sanayici'ye %32 kalıyor. Arzı büyütmek bu
//! oranı değiştirmedi (hasat tavanı %300 → %600 denendi): su ekleniyor ama
//! musluk aynı.
//!
//! Bu araç yarışın **neden** kaybedildiğini ayırır: teklif fiyatı mı düşük,
//! emir mi az, yoksa emir büyük ama doluluk mu düşük.

use std::collections::BTreeMap;

use moneywar_domain::{NpcKind, OrderSide, ProductKind};
use moneywar_engine::LogEvent;
use moneywar_web::driver::SimDriver;

#[derive(Default)]
struct Acc {
    orders: u64,
    order_units: u64,
    price_cents: i64,
    fills: u64,
    fill_units: u64,
    fill_price_cents: i64,
}

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test raw_race_probe -- --ignored --nocapture`"]
fn who_wins_the_raw_material_race() {
    let mut d = SimDriver::new(
        moneywar_web::DEFAULT_SEED,
        moneywar_web::SEASON_TICKS,
        3,
        moneywar_web::DIFFICULTY,
    );
    // (ürün, rol) → sayaçlar. Yalnız ham madde.
    let mut acc: BTreeMap<(ProductKind, NpcKind), Acc> = BTreeMap::new();

    for _ in 0..moneywar_web::SEASON_TICKS {
        d.step();
        for entry in &d.last_report.entries {
            match &entry.event {
                LogEvent::CommandAccepted {
                    command: moneywar_domain::Command::SubmitOrder(o),
                } => {
                    if o.side != OrderSide::Buy || !o.product.is_raw() {
                        continue;
                    }
                    let Some(k) = d.state.players.get(&o.player).and_then(|p| p.npc_kind) else {
                        continue;
                    };
                    let e = acc.entry((o.product, k)).or_default();
                    e.orders += 1;
                    e.order_units += u64::from(o.quantity);
                    e.price_cents += o.unit_price.as_cents();
                }
                LogEvent::OrderMatched {
                    product,
                    quantity,
                    price,
                    buyer,
                    ..
                } => {
                    if !product.is_raw() {
                        continue;
                    }
                    let Some(k) = d.state.players.get(buyer).and_then(|p| p.npc_kind) else {
                        continue;
                    };
                    let e = acc.entry((*product, k)).or_default();
                    e.fills += 1;
                    e.fill_units += u64::from(*quantity);
                    e.fill_price_cents += price.as_cents();
                }
                _ => {}
            }
        }
    }

    println!("\n── Ham madde yarışı: kim kazanıyor, neden ────────────────────────");
    println!(
        "{:<10} {:<12} {:>7} {:>9} {:>11} {:>10} {:>9} {:>10}",
        "ham", "rol", "emir", "emir/br", "ort teklif", "aldığı br", "doluluk", "ort ödeme"
    );
    println!("{}", "-".repeat(84));

    for raw in ProductKind::RAW_MATERIALS {
        let mut rows: Vec<(NpcKind, &Acc)> = acc
            .iter()
            .filter(|((p, _), _)| *p == raw)
            .map(|((_, k), a)| (*k, a))
            .collect();
        if rows.is_empty() {
            continue;
        }
        rows.sort_by_key(|(_, a)| std::cmp::Reverse(a.fill_units));
        let total_fill: u64 = rows.iter().map(|(_, a)| a.fill_units).sum::<u64>().max(1);
        for (k, a) in rows {
            let bid = if a.orders == 0 {
                0.0
            } else {
                a.price_cents as f64 / a.orders as f64 / 100.0
            };
            let paid = if a.fills == 0 {
                0.0
            } else {
                a.fill_price_cents as f64 / a.fills as f64 / 100.0
            };
            let ou = if a.orders == 0 {
                0.0
            } else {
                a.order_units as f64 / a.orders as f64
            };
            let cover = if a.order_units == 0 {
                0.0
            } else {
                a.fill_units as f64 / a.order_units as f64 * 100.0
            };
            println!(
                "{:<10} {:<12} {:>7} {ou:>9.1} {bid:>10.2}₺ {:>9} (%{:>2.0}) {cover:>8.0}% {paid:>9.2}₺",
                raw.display_name(),
                k.label(),
                a.orders,
                a.fill_units,
                a.fill_units as f64 / total_fill as f64 * 100.0,
            );
        }
        println!();
    }
    println!("──────────────────────────────────────────────────────────────────\n");
}
