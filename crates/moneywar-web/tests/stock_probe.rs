//! Sanayici mamul stoğu neden erimiyor?
//!
//! Gözlem: bazı Sanayici'ler elinde çok sayıda mamulle oturuyor. İki ayrı
//! açıklama var ve ayırmak şart:
//!
//! - **satmıyor**  → emri hiç açmıyor (fiyat tabanı tutmuyor, aday üretilmiyor)
//! - **satamıyor** → emri açıyor ama karşısında alıcı yok (fill düşük)
//!
//! Üçüncü bir ihtimal daha var: emri açıyor, doluyor, ama **açtığı miktar
//! ürettiğinden küçük**. `sanayici.rs`'te satış miktarı `(qty/2).clamp(1,50)`
//! — stok ne olursa olsun emir başına 50 birim tavanı var. Stok 100'ü geçince
//! tavan bağlar ve fazlası kitapta hiç görünmez.
//!
//! Bu probe üçünü ayırt eder: elde duran stok, kitaba çıkan miktar, ve
//! tavanın bağladığı kova sayısı.

use std::collections::BTreeMap;

use moneywar_domain::{NpcKind, OrderSide, PlayerId};
use moneywar_web::driver::SimDriver;

/// `sanayici.rs`'teki satış miktarı tavanı — `(qty/2).clamp(1, 50)`.
/// Tavan `qty/2 > 50`, yani stok 100'ü aştığında bağlar.
const SELL_QTY_CAP: u32 = 50;
const CAP_BINDS_ABOVE: u32 = SELL_QTY_CAP * 2;

#[test]
#[ignore = "ölçüm aracı — `cargo test -p moneywar-web --test stock_probe -- --ignored --nocapture`"]
fn why_does_finished_stock_pile_up() {
    let mut d = SimDriver::new(
        moneywar_web::DEFAULT_SEED,
        moneywar_web::SEASON_TICKS,
        3,
        moneywar_web::DIFFICULTY,
    );

    struct Row {
        tick: u32,
        held: u64,
        on_book: u64,
        buckets: usize,
        capped_buckets: usize,
        capped_held: u64,
    }
    let mut rows: Vec<Row> = Vec::new();

    for _ in 0..moneywar_web::SEASON_TICKS {
        d.step();
        let t = d.state.current_tick.value();
        if !t.is_multiple_of(70) {
            continue;
        }

        let sanayiciler: Vec<PlayerId> = d
            .state
            .players
            .iter()
            .filter(|(_, p)| p.npc_kind == Some(NpcKind::Sanayici))
            .map(|(id, _)| *id)
            .collect();

        let mut held = 0u64;
        let mut buckets = 0usize;
        let mut capped_buckets = 0usize;
        let mut capped_held = 0u64;
        for id in &sanayiciler {
            let Some(p) = d.state.players.get(id) else {
                continue;
            };
            for (_city, product, qty) in p.inventory.entries() {
                if !product.is_finished() || qty == 0 {
                    continue;
                }
                held += u64::from(qty);
                buckets += 1;
                if qty > CAP_BINDS_ABOVE {
                    capped_buckets += 1;
                    capped_held += u64::from(qty);
                }
            }
        }

        // Kitapta duran Sanayici SELL miktarı — "satışa çıkardığı".
        let on_book: u64 = d
            .state
            .order_book
            .values()
            .flat_map(|orders| orders.iter())
            .filter(|o| o.side == OrderSide::Sell && sanayiciler.contains(&o.player))
            .map(|o| u64::from(o.quantity))
            .sum();

        rows.push(Row {
            tick: t,
            held,
            on_book,
            buckets,
            capped_buckets,
            capped_held,
        });
    }

    println!("\n── Sanayici mamul stoğu (emir tavanı {SELL_QTY_CAP} birim) ─────────────────");
    println!(
        "{:>5} {:>10} {:>10} {:>7} {:>8} {:>8} {:>12} {:>9}",
        "tick", "elde", "kitapta", "kova", "tavanlı", "tavan%", "tavanlı stok", "kitap/elde"
    );
    println!("{}", "-".repeat(80));
    for r in &rows {
        let cap_pct = if r.buckets == 0 {
            0.0
        } else {
            r.capped_buckets as f64 / r.buckets as f64 * 100.0
        };
        let ratio = if r.held == 0 {
            0.0
        } else {
            r.on_book as f64 / r.held as f64 * 100.0
        };
        println!(
            "{:>5} {:>10} {:>10} {:>7} {:>8} {:>7.0}% {:>12} {:>8.0}%",
            r.tick, r.held, r.on_book, r.buckets, r.capped_buckets, cap_pct, r.capped_held, ratio
        );
    }

    println!("\n  «elde»    Sanayici envanterindeki toplam mamul birimi");
    println!("  «kitapta» aynı anda pazara çıkardığı SELL miktarı");
    println!("  «tavanlı» stoğu {CAP_BINDS_ABOVE} birimi aşan kova — emir tavanı burada bağlıyor");

    // Sezon sonu: en çok stok tutan firmalar ve o stok hangi üründe.
    let mut per_firm: BTreeMap<String, u64> = BTreeMap::new();
    let mut per_product: BTreeMap<String, u64> = BTreeMap::new();
    for (id, p) in &d.state.players {
        if p.npc_kind != Some(NpcKind::Sanayici) {
            continue;
        }
        let _ = id;
        for (_city, product, qty) in p.inventory.entries() {
            if !product.is_finished() || qty == 0 {
                continue;
            }
            *per_firm.entry(p.name.clone()).or_insert(0) += u64::from(qty);
            *per_product.entry(format!("{product:?}")).or_insert(0) += u64::from(qty);
        }
    }

    let mut firms: Vec<_> = per_firm.into_iter().collect();
    firms.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n── Sezon sonu: en çok mamul tutan Sanayici'ler ──────────────");
    for (name, qty) in firms.iter().take(8) {
        println!("  {name:<22} {qty:>8} birim");
    }

    let mut prods: Vec<_> = per_product.into_iter().collect();
    prods.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n── Sezon sonu: hangi üründe birikiyor ──────────────────────");
    for (product, qty) in prods.iter().take(8) {
        println!("  {product:<22} {qty:>8} birim");
    }
}
