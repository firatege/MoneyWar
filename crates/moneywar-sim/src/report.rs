//! Sim sonuçlarından insan-okur Markdown rapor üretici.
//!
//! Sorulara cevap veren bölümler:
//! 1. Kim battı? (cash < bankrupt eşiği)
//! 2. Doygunluk eğrisi — toplam talep tick'e göre
//! 3. Piyasa altı emirler — uzun süredir bekleyen, market×0.85 altı
//! 4. NPC PnL evolution
//! 5. Match/reject oranı
//! 6. Fiyat trend per (city, product)
//! 7. NPC × action_kind dağılımı

use std::fmt::Write;

use crate::runner::SimResult;

const BANKRUPT_THRESHOLD_LIRA: i64 = 100;
const STALE_ORDER_AGE: u32 = 10;

/// Sim sonucundan tam Markdown rapor üret.
pub fn render_markdown(result: &SimResult) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Sim Rapor: seed={} ticks={} difficulty={:?} scenario={}",
        result.seed, result.ticks, result.difficulty, result.scenario_name
    );
    let _ = writeln!(out);

    section_summary(&mut out, result);
    section_bankruptcies(&mut out, result);
    section_demand_curve(&mut out, result);
    section_stale_orders(&mut out, result);
    section_pnl_evolution(&mut out, result);
    section_clearing_metrics(&mut out, result);
    section_action_distribution(&mut out, result);

    out
}

fn section_summary(out: &mut String, r: &SimResult) {
    let last = r.snapshots.last();
    let _ = writeln!(out, "## Özet");
    if let Some(s) = last {
        let _ = writeln!(out, "- Final tick: {}", s.tick);
        let _ = writeln!(out, "- Toplam oyuncu/NPC: {}", s.players.len());
        let total_cash: i64 = s.players.iter().map(|p| p.cash_cents).sum();
        let total_stock: u64 = s.players.iter().map(|p| p.inventory_total).sum();
        let _ = writeln!(out, "- Toplam nakit: {}₺", total_cash / 100);
        let _ = writeln!(out, "- Toplam stok birim: {}", total_stock);
    }
    let total_accepted: u32 = r.snapshots.iter().map(|s| s.commands_accepted).sum();
    let total_rejected: u32 = r.snapshots.iter().map(|s| s.commands_rejected).sum();
    let _ = writeln!(out, "- Toplam komut accept: {total_accepted}");
    let _ = writeln!(out, "- Toplam komut reject: {total_rejected}");
    let _ = writeln!(out);
}

fn section_bankruptcies(out: &mut String, r: &SimResult) {
    let _ = writeln!(out, "## 🪦 Battı (cash < {}₺)", BANKRUPT_THRESHOLD_LIRA);
    let last = match r.snapshots.last() {
        Some(s) => s,
        None => return,
    };
    let bankrupt: Vec<_> = last
        .players
        .iter()
        .filter(|p| p.cash_cents < BANKRUPT_THRESHOLD_LIRA * 100)
        .collect();
    if bankrupt.is_empty() {
        let _ = writeln!(out, "- Hiç batan yok.");
    } else {
        let _ = writeln!(out, "| Oyuncu | Kind | Cash | Stok |");
        let _ = writeln!(out, "|---|---|---|---|");
        for p in bankrupt {
            let _ = writeln!(
                out,
                "| {} | {:?} | {}₺ | {} |",
                p.name,
                p.npc_kind.clone().unwrap_or_else(|| "Human".into()),
                p.cash_cents / 100,
                p.inventory_total
            );
        }
    }
    let _ = writeln!(out);
}

fn section_demand_curve(out: &mut String, r: &SimResult) {
    let _ = writeln!(out, "## 📈 Talep Eğrisi (toplam BUY qty per tick)");
    let _ = writeln!(out, "10 tick'lik bucket'larda agregre — doygunluk tespiti.");
    let _ = writeln!(out, "| Tick aralığı | Toplam BUY | Toplam SELL | Spread |");
    let _ = writeln!(out, "|---|---|---|---|");
    let bucket_size = 10u32;
    let total_buckets = (r.ticks + bucket_size - 1) / bucket_size;
    for b in 0..total_buckets {
        let lo = b * bucket_size + 1;
        let hi = (lo + bucket_size - 1).min(r.ticks);
        let mut buy_total = 0u64;
        let mut sell_total = 0u64;
        for s in &r.snapshots {
            if s.tick < lo || s.tick > hi {
                continue;
            }
            for ob in &s.order_book {
                buy_total += u64::from(ob.bid_total_qty);
                sell_total += u64::from(ob.ask_total_qty);
            }
        }
        let spread_cmt = if sell_total == 0 {
            "talep yüklü"
        } else if buy_total * 2 < sell_total {
            "talep zayıf ⚠"
        } else {
            "denge"
        };
        let _ = writeln!(
            out,
            "| t{lo}-{hi} | {buy_total} | {sell_total} | {spread_cmt} |"
        );
    }
    let _ = writeln!(out);
}

fn section_stale_orders(out: &mut String, r: &SimResult) {
    let _ = writeln!(
        out,
        "## ⏰ Bekleyen Emirler ({}+ tick stale)",
        STALE_ORDER_AGE
    );
    let last = match r.snapshots.last() {
        Some(s) => s,
        None => return,
    };
    let stale: Vec<_> = last
        .order_book
        .iter()
        .filter(|ob| ob.oldest_order_age >= STALE_ORDER_AGE)
        .collect();
    if stale.is_empty() {
        let _ = writeln!(out, "- Hiç stale emir yok ({}+ tick).", STALE_ORDER_AGE);
    } else {
        let _ = writeln!(
            out,
            "| Şehir/Ürün | En eski yaş | Bid count/qty | Ask count/qty |"
        );
        let _ = writeln!(out, "|---|---|---|---|");
        for ob in stale {
            let _ = writeln!(
                out,
                "| {}/{}  | {} tick | {}/{} | {}/{} |",
                ob.city,
                ob.product,
                ob.oldest_order_age,
                ob.bid_count,
                ob.bid_total_qty,
                ob.ask_count,
                ob.ask_total_qty
            );
        }
    }
    let _ = writeln!(out);
}

fn section_pnl_evolution(out: &mut String, r: &SimResult) {
    let _ = writeln!(out, "## 💰 PnL Evolution (her oyuncu cash + stok birimleri)");
    if r.snapshots.is_empty() {
        return;
    }
    let first = &r.snapshots[0];
    let last = r.snapshots.last().unwrap();

    let _ = writeln!(
        out,
        "| Oyuncu | Kind | Başlangıç ₺ | Son ₺ | Δ ₺ | Stok Δ |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");

    for p_first in &first.players {
        let p_last = match last.players.iter().find(|p| p.id == p_first.id) {
            Some(p) => p,
            None => continue,
        };
        let cash_delta = p_last.cash_cents - p_first.cash_cents;
        let stock_delta = i64::try_from(p_last.inventory_total)
            .unwrap_or(i64::MAX)
            .saturating_sub(i64::try_from(p_first.inventory_total).unwrap_or(0));
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {:+} | {:+} |",
            p_first.name,
            p_first.npc_kind.as_deref().unwrap_or("Human"),
            p_first.cash_cents / 100,
            p_last.cash_cents / 100,
            cash_delta / 100,
            stock_delta
        );
    }
    let _ = writeln!(out);
}

fn section_clearing_metrics(out: &mut String, r: &SimResult) {
    let _ = writeln!(out, "## 🔄 Clearing Metrikleri");
    let mut total_clearings = 0u32;
    let mut total_matched = 0u64;
    let mut total_submitted_buy = 0u64;
    let mut total_submitted_sell = 0u64;
    for s in &r.snapshots {
        for c in &s.clearings {
            total_clearings += 1;
            total_matched += u64::from(c.matched_qty);
            total_submitted_buy += u64::from(c.submitted_buy_qty);
            total_submitted_sell += u64::from(c.submitted_sell_qty);
        }
    }
    let _ = writeln!(out, "- Toplam clearing: {total_clearings}");
    let _ = writeln!(out, "- Toplam match qty: {total_matched}");
    let _ = writeln!(out, "- Submitted BUY qty: {total_submitted_buy}");
    let _ = writeln!(out, "- Submitted SELL qty: {total_submitted_sell}");
    if total_submitted_buy + total_submitted_sell > 0 {
        let efficiency =
            (total_matched as f64) * 100.0 / (total_submitted_buy + total_submitted_sell) as f64;
        let _ = writeln!(out, "- Match verimliliği: {:.1}%", efficiency);
    }
    let _ = writeln!(out);
}

fn section_action_distribution(out: &mut String, r: &SimResult) {
    let _ = writeln!(out, "## 🎯 Aksiyon Dağılımı (NPC başına)");
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<(u64, String), u32> = BTreeMap::new();
    let mut npc_names: BTreeMap<u64, String> = BTreeMap::new();
    for trace in &r.traces {
        for d in &trace.npc_decisions {
            npc_names.insert(d.npc_id, d.npc_name.clone());
            for action in &d.actions_emitted {
                let kind = action.split_whitespace().next().unwrap_or("?").to_string();
                *counts.entry((d.npc_id, kind)).or_insert(0) += 1;
            }
        }
    }
    if counts.is_empty() {
        let _ = writeln!(out, "- (NPC trace boş)");
        let _ = writeln!(out);
        return;
    }
    let _ = writeln!(out, "| NPC | Aksiyon | Sayı |");
    let _ = writeln!(out, "|---|---|---|");
    for ((id, kind), n) in counts {
        let name = npc_names.get(&id).cloned().unwrap_or_else(|| format!("?{id}"));
        let _ = writeln!(out, "| {name} | {kind} | {n} |");
    }
    let _ = writeln!(out);
}
