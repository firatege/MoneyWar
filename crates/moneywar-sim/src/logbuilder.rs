//! Tek bir koşumun tüm raporlarını disk'e timestamped bir klasörde yazar.
//!
//! Layout:
//! ```text
//! <root>/run_<YYYYMMDD_HHMMSS>/
//! ├── manifest.json                 # Argümanlar, seed listesi, git sha, tarih
//! ├── aggregate.md                  # 10-seed istatistik (mean/std/min/max)
//! ├── thresholds.md                 # Rol kontrat + oyun kapısı (✅/❌)
//! ├── tuning_issues.md              # Otomatik: yalnızca ❌ olan maddeler
//! └── per_seed/
//!     ├── seed_<N>.md               # Full Markdown rapor
//!     ├── seed_<N>_actions.jsonl    # Her tick: NpcDecisionTrace
//!     └── seed_<N>_clearings.csv    # Her clearing: tick,city,product,price,qty
//! ```
//!
//! Kullanım: `moneywar_sim::logbuilder::write_full_log(&dir, ...)`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use moneywar_npc::Difficulty;

use crate::report::render_markdown;
use crate::runner::SimResult;
use crate::stats::{PerRunMetrics, QualityScore, Stats};
use crate::thresholds::{
    audit_game_with_runs, audit_role, default_contracts, render_threshold_report, GameThresholds,
};

/// Şehir id (0..2) → display name.
fn city_name(id: u8) -> &'static str {
    match id {
        0 => "Istanbul",
        1 => "Ankara",
        2 => "Izmir",
        _ => "?",
    }
}

/// Ürün id (0..5) → display name.
fn product_name(id: u8) -> &'static str {
    match id {
        0 => "Pamuk",
        1 => "Bugday",
        2 => "Zeytin",
        3 => "Kumas",
        4 => "Un",
        5 => "Zeytinyagi",
        _ => "?",
    }
}

/// Timestamp'lı run klasörü oluştur (`root/run_YYYYMMDD_HHMMSS`).
#[must_use]
pub fn create_run_dir(root: &Path) -> PathBuf {
    let ts = format_timestamp();
    let dir = root.join(format!("run_{ts}"));
    let _ = fs::create_dir_all(&dir);
    let _ = fs::create_dir_all(dir.join("per_seed"));
    dir
}

/// Tek run için: bucket × tick aktivite matrisi.
/// (`city_id`, `product_id`) → (`clearing_count`, `total_matched_qty`, prices)
struct BucketStats {
    clearing_count: u32,
    matched_qty: u64,
    prices_cents: Vec<i64>,
    total_buy_submitted: u64,
    total_sell_submitted: u64,
}

impl BucketStats {
    fn new() -> Self {
        Self {
            clearing_count: 0,
            matched_qty: 0,
            prices_cents: Vec::new(),
            total_buy_submitted: 0,
            total_sell_submitted: 0,
        }
    }
}

/// 18 bucket (3 şehir × 6 ürün) için seed başına aktivite + 10-seed ortalama.
fn render_circulation_md(runs: &[SimResult]) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "## 🔄 Pazar Dolaşımı — Şehir × Ürün Aktivite Matrisi");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "_Her hücre: 10 seed mean clearing sayısı. {{boş bucket sayısı}}_"
    );
    let _ = writeln!(out);

    // Seed başına bucket map → 10 seed ortalama
    let mut agg: BTreeMap<(u8, u8), BucketStats> = BTreeMap::new();
    let mut empty_buckets_per_seed: Vec<u32> = Vec::new();

    for r in runs {
        let mut seed_buckets: BTreeMap<(u8, u8), BucketStats> = BTreeMap::new();
        for snap in &r.snapshots {
            for c in &snap.clearings {
                let key = (c.city, c.product);
                let bs = seed_buckets.entry(key).or_insert_with(BucketStats::new);
                if c.matched_qty > 0 {
                    bs.clearing_count += 1;
                    bs.matched_qty += u64::from(c.matched_qty);
                    if let Some(p) = c.clearing_price_cents {
                        bs.prices_cents.push(p);
                    }
                }
                bs.total_buy_submitted += u64::from(c.submitted_buy_qty);
                bs.total_sell_submitted += u64::from(c.submitted_sell_qty);
            }
        }
        // Aggregate'a ekle
        for (k, bs) in &seed_buckets {
            let agg_bs = agg.entry(*k).or_insert_with(BucketStats::new);
            agg_bs.clearing_count += bs.clearing_count;
            agg_bs.matched_qty += bs.matched_qty;
            agg_bs.prices_cents.extend_from_slice(&bs.prices_cents);
            agg_bs.total_buy_submitted += bs.total_buy_submitted;
            agg_bs.total_sell_submitted += bs.total_sell_submitted;
        }
        // Bu seedde kaç bucket boş kaldı (3*6=18 toplam)
        let active = seed_buckets
            .values()
            .filter(|b| b.clearing_count > 0)
            .count();
        empty_buckets_per_seed.push(18 - active as u32);
    }
    let n_seeds = runs.len() as f64;

    // Tablo: satır şehir, sütun ürün
    let _ = writeln!(
        out,
        "| Şehir \\ Ürün | Pamuk | Bugday | Zeytin | Kumas | Un | Zeytinyagi |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|---|");
    for city_id in 0..3u8 {
        let mut row = format!("| **{}** |", city_name(city_id));
        for product_id in 0..6u8 {
            let bs = agg.get(&(city_id, product_id));
            let count = bs.map_or(0, |b| b.clearing_count);
            let avg = f64::from(count) / n_seeds;
            let icon = if avg < 1.0 {
                "💀"
            } else if avg < 10.0 {
                "🟡"
            } else if avg < 30.0 {
                "🟢"
            } else {
                "🔥"
            };
            row.push_str(&format!(" {icon} {avg:.1} |"));
        }
        let _ = writeln!(out, "{row}");
    }
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "_Lejant: 💀 ölü (<1), 🟡 zayıf (<10), 🟢 sağlıklı (<30), 🔥 yoğun (≥30)_"
    );
    let _ = writeln!(out);

    let avg_empty = f64::from(empty_buckets_per_seed.iter().sum::<u32>()) / n_seeds;
    let max_empty = empty_buckets_per_seed.iter().copied().max().unwrap_or(0);
    let _ = writeln!(
        out,
        "**Ölü bucket:** sezon başına ortalama {avg_empty:.1}/18 (max {max_empty} seed'de)"
    );
    let _ = writeln!(out);

    // Şehir başına toplam clearing + match qty
    let _ = writeln!(out, "### Şehir Toplam");
    let _ = writeln!(out, "| Şehir | Clearing | Match qty | Buy submit | Sell submit |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for city_id in 0..3u8 {
        let mut total_clearing = 0u32;
        let mut total_qty = 0u64;
        let mut total_buy = 0u64;
        let mut total_sell = 0u64;
        for product_id in 0..6u8 {
            if let Some(bs) = agg.get(&(city_id, product_id)) {
                total_clearing += bs.clearing_count;
                total_qty += bs.matched_qty;
                total_buy += bs.total_buy_submitted;
                total_sell += bs.total_sell_submitted;
            }
        }
        let _ = writeln!(
            out,
            "| {} | {:.0} | {:.0} | {:.0} | {:.0} |",
            city_name(city_id),
            f64::from(total_clearing) / n_seeds,
            total_qty as f64 / n_seeds,
            total_buy as f64 / n_seeds,
            total_sell as f64 / n_seeds
        );
    }
    let _ = writeln!(out);

    // Ürün başına toplam — hangi ürün dönüyor en çok
    let _ = writeln!(out, "### Ürün Toplam");
    let _ = writeln!(out, "| Ürün | Clearing | Match qty | Fiyat min/max ₺ |");
    let _ = writeln!(out, "|---|---|---|---|");
    for product_id in 0..6u8 {
        let mut total_clearing = 0u32;
        let mut total_qty = 0u64;
        let mut all_prices: Vec<i64> = Vec::new();
        for city_id in 0..3u8 {
            if let Some(bs) = agg.get(&(city_id, product_id)) {
                total_clearing += bs.clearing_count;
                total_qty += bs.matched_qty;
                all_prices.extend_from_slice(&bs.prices_cents);
            }
        }
        let (pmin, pmax) = if all_prices.is_empty() {
            (0, 0)
        } else {
            (
                all_prices.iter().copied().min().unwrap_or(0) / 100,
                all_prices.iter().copied().max().unwrap_or(0) / 100,
            )
        };
        let _ = writeln!(
            out,
            "| {} | {:.0} | {:.0} | {}—{}₺ |",
            product_name(product_id),
            f64::from(total_clearing) / n_seeds,
            total_qty as f64 / n_seeds,
            pmin,
            pmax
        );
    }
    let _ = writeln!(out);
    out
}

/// NPC kind başına sezon başı vs son envanter delta — mal kim biriktirdi/erit?
fn render_inventory_flow_md(runs: &[SimResult]) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "## 📦 Stok Akışı — NpcKind Başına Δ Inventory");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "_Sezon başı → son toplam birim. Pozitif = biriktirdi, negatif = erit._"
    );
    let _ = writeln!(out);

    let mut deltas_by_kind: BTreeMap<String, Vec<i64>> = BTreeMap::new();

    for r in runs {
        let first = match r.snapshots.first() {
            Some(s) => s,
            None => continue,
        };
        let last = match r.snapshots.last() {
            Some(s) => s,
            None => continue,
        };
        for p_first in &first.players {
            if let Some(kind) = &p_first.npc_kind {
                if let Some(p_last) = last.players.iter().find(|p| p.id == p_first.id) {
                    let delta = p_last.inventory_total as i64 - p_first.inventory_total as i64;
                    deltas_by_kind.entry(kind.clone()).or_default().push(delta);
                }
            }
        }
    }

    let _ = writeln!(out, "| Rol | Mean Δ | Min | Max |");
    let _ = writeln!(out, "|---|---|---|---|");
    for (kind, ds) in &deltas_by_kind {
        let n = ds.len() as f64;
        let mean = ds.iter().sum::<i64>() as f64 / n;
        let min = *ds.iter().min().unwrap_or(&0);
        let max = *ds.iter().max().unwrap_or(&0);
        let icon = if mean.abs() < 5.0 {
            "⚠️ donuk"
        } else if mean > 0.0 {
            "📈 birikiyor"
        } else {
            "📉 eriyor"
        };
        let _ = writeln!(
            out,
            "| {kind} | {mean:.0} {icon} | {min} | {max} |"
        );
    }
    let _ = writeln!(out);
    out
}

/// Tüm raporları diske yaz. `cmdline` sadece manifest için.
pub fn write_full_log(
    dir: &Path,
    cmdline: &str,
    seeds: &[u64],
    ticks: u32,
    diff: Difficulty,
    scenario_name: &str,
    runs: &[SimResult],
    metrics: &[PerRunMetrics],
    elapsed_ms: u128,
) {
    // 1. Manifest.
    let manifest = build_manifest(cmdline, seeds, ticks, diff, scenario_name, elapsed_ms);
    let _ = fs::write(dir.join("manifest.json"), manifest);

    // 2. Aggregate Markdown + pazar dolaşımı + stok akışı.
    let stats = Stats::collect(diff, metrics);
    let quality = QualityScore::from_stats(&stats);
    let mut aggregate_md = render_aggregate_md(&stats, &quality, metrics);
    aggregate_md.push_str(&render_circulation_md(runs));
    aggregate_md.push_str(&render_inventory_flow_md(runs));
    let _ = fs::write(dir.join("aggregate.md"), aggregate_md);

    // 3. Threshold + tuning issues.
    let contracts = default_contracts();
    let thresholds = GameThresholds::hard_default();
    let threshold_md = render_threshold_report(&contracts, &thresholds, runs, &stats);
    let _ = fs::write(dir.join("thresholds.md"), &threshold_md);
    let tuning_md = extract_tuning_issues(&contracts, &thresholds, runs, &stats);
    let _ = fs::write(dir.join("tuning_issues.md"), tuning_md);

    // 4. Per-seed: full md + actions JSONL + clearings CSV.
    let per_seed = dir.join("per_seed");
    for r in runs {
        let _ = fs::write(
            per_seed.join(format!("seed_{}.md", r.seed)),
            render_markdown(r),
        );
        let _ = write_actions_jsonl(&per_seed.join(format!("seed_{}_actions.jsonl", r.seed)), r);
        let _ = write_clearings_csv(&per_seed.join(format!("seed_{}_clearings.csv", r.seed)), r);
    }
}

fn render_aggregate_md(
    stats: &Stats,
    quality: &QualityScore,
    runs: &[PerRunMetrics],
) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# 📊 Agregat İstatistik — {} run × {:?}",
        stats.n_runs, stats.difficulty
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "## Kalite Kapısı: {}/{}", quality.passed, quality.total);
    let _ = writeln!(out);
    let _ = writeln!(out, "| Madde | Geçti | Değer |");
    let _ = writeln!(out, "|---|---|---|");
    for (item, ok, value) in &quality.details {
        let icon = if *ok { "✅" } else { "❌" };
        let _ = writeln!(out, "| {item} | {icon} | {value} |");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Genel Metrikler (mean ± std)");
    let _ = writeln!(out, "| Metrik | Mean | Std | Min | Max | Median |");
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for (name, s) in [
        ("Toplam clearing", &stats.matches),
        ("Match qty", &stats.matched_qty),
        ("Match verim %", &stats.match_efficiency_pct),
        ("Bankrupt NPC", &stats.bankrupt_npcs),
        ("Stale yaş max", &stats.stale_orders_max),
        ("İnsan PnL ₺", &stats.human_pnl),
    ] {
        let _ = writeln!(
            out,
            "| {name} | {:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
            s.mean, s.std_dev, s.min, s.max, s.median
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Rol PnL");
    let _ = writeln!(out, "| Rol | Mean | Std | Min | Max | Median |");
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for (role, s) in &stats.pnl_by_role {
        let _ = writeln!(
            out,
            "| {role} | {:.0}₺ | {:.0}₺ | {:.0}₺ | {:.0}₺ | {:.0}₺ |",
            s.mean, s.std_dev, s.min, s.max, s.median
        );
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Per-Seed Özet");
    let _ = writeln!(
        out,
        "| Seed | Match | Verim % | Bankrupt | Stale | İnsan PnL |"
    );
    let _ = writeln!(out, "|---|---|---|---|---|---|");
    for r in runs {
        let total = r.submitted_buy + r.submitted_sell;
        let eff = if total == 0 {
            0.0
        } else {
            (r.matched_qty as f64) * 100.0 / total as f64
        };
        let _ = writeln!(
            out,
            "| {} | {} | {:.2} | {} | {} | {} |",
            r.seed, r.total_matches, eff, r.bankrupt_npcs, r.stale_orders_max_age, r.human_pnl_lira
        );
    }
    out
}

/// Sadece ❌ olan maddeleri ayrı bir markdown'a çıkar — hızlı tuning bakışı.
fn extract_tuning_issues(
    contracts: &[crate::thresholds::RoleContract],
    thresholds: &GameThresholds,
    runs: &[SimResult],
    stats: &Stats,
) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "# 🔧 Tuning Issues");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "_Otomatik üretildi: yalnızca threshold audit'te ❌ olan maddeler._"
    );
    let _ = writeln!(out);

    let bank_total: u32 = runs.iter().map(|r| r.bank_loans_issued).sum();
    let game_checks = audit_game_with_runs(thresholds, stats, bank_total, runs);
    let game_fails: Vec<_> = game_checks.iter().filter(|c| !c.passed).collect();
    if !game_fails.is_empty() {
        let _ = writeln!(out, "## Oyun Kapısı");
        for c in &game_fails {
            let _ = writeln!(out, "- ❌ **{}** — {}", c.label, c.detail);
        }
        let _ = writeln!(out);
    }

    let mut total_mix: BTreeMap<String, crate::runner::RoleActionMix> = BTreeMap::new();
    let mut npc_count_per_kind: BTreeMap<String, u32> = BTreeMap::new();
    for r in runs {
        for (kind, mix) in &r.action_mix_by_kind {
            let agg = total_mix.entry(kind.clone()).or_default();
            agg.buy_raw += mix.buy_raw;
            agg.buy_finished += mix.buy_finished;
            agg.sell_raw += mix.sell_raw;
            agg.sell_finished += mix.sell_finished;
            agg.build_factory += mix.build_factory;
            agg.buy_caravan += mix.buy_caravan;
            agg.dispatch += mix.dispatch;
            agg.propose_contract += mix.propose_contract;
            agg.accept_contract += mix.accept_contract;
            agg.take_loan += mix.take_loan;
            agg.repay_loan += mix.repay_loan;
            agg.forbidden_action_count += mix.forbidden_action_count;
            agg.total_commands += mix.total_commands;
        }
        if r.seed == runs[0].seed {
            if let Some(s0) = r.snapshots.first() {
                for p in &s0.players {
                    if let Some(kind) = &p.npc_kind {
                        *npc_count_per_kind.entry(kind.clone()).or_default() += 1;
                    }
                }
            }
        }
    }
    let n_seeds = runs.len() as u32;
    for c in contracts {
        let mix = total_mix.get(c.kind).cloned().unwrap_or_default();
        let pnl = stats.pnl_by_role.get(c.kind).map_or(0.0, |s| s.mean);
        let n = *npc_count_per_kind.get(c.kind).unwrap_or(&0);
        let checks = audit_role(c, &mix, pnl, n, n_seeds);
        let fails: Vec<_> = checks.iter().filter(|x| !x.passed).collect();
        if fails.is_empty() {
            continue;
        }
        let _ = writeln!(out, "## {} ({})", c.kind, c.mandate);
        for f in fails {
            let _ = writeln!(out, "- ❌ **{}** — {}", f.label, f.detail);
        }
        let _ = writeln!(out);
    }
    out
}

fn write_actions_jsonl(path: &Path, run: &SimResult) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    for tick_trace in &run.traces {
        // Her tick için tek satır JSON (tick + npc kararları array)
        let line = serde_json::to_string(tick_trace).unwrap_or_default();
        writeln!(f, "{line}")?;
    }
    Ok(())
}

fn write_clearings_csv(path: &Path, run: &SimResult) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    writeln!(
        f,
        "tick,city_id,product_id,clearing_price_cents,matched_qty,submitted_buy,submitted_sell"
    )?;
    for snap in &run.snapshots {
        for c in &snap.clearings {
            let price = c.clearing_price_cents.unwrap_or(0);
            writeln!(
                f,
                "{},{},{},{},{},{},{}",
                snap.tick,
                c.city,
                c.product,
                price,
                c.matched_qty,
                c.submitted_buy_qty,
                c.submitted_sell_qty
            )?;
        }
    }
    Ok(())
}

fn build_manifest(
    cmdline: &str,
    seeds: &[u64],
    ticks: u32,
    diff: Difficulty,
    scenario_name: &str,
    elapsed_ms: u128,
) -> String {
    let git_sha = git_sha().unwrap_or_else(|| "unknown".to_string());
    let timestamp = format_timestamp();
    let seeds_json = seeds
        .iter()
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\n  \"timestamp\": \"{timestamp}\",\n  \"git_sha\": \"{git_sha}\",\n  \"cmdline\": {cmdline:?},\n  \"seeds\": [{seeds_json}],\n  \"ticks\": {ticks},\n  \"difficulty\": \"{diff:?}\",\n  \"scenario\": \"{scenario_name}\",\n  \"elapsed_ms\": {elapsed_ms}\n}}\n"
    )
}

fn git_sha() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn format_timestamp() -> String {
    // Std lib ile yerel saat olmadan UNIX epoch → human readable.
    // Düşük çözünürlük yeter; klasör adı için.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Basit dönüşüm: epoch → YYYYMMDD_HHMMSS (UTC)
    // Tam timezone hassasiyetine gerek yok; klasör isimleri için sabit.
    let (y, mo, d, h, mi, s) = epoch_to_ymdhms(secs);
    format!("{y:04}{mo:02}{d:02}_{h:02}{mi:02}{s:02}")
}

/// Epoch saniyesini UTC YYYY-MM-DD HH:MM:SS'e çevirir (chrono'suz).
fn epoch_to_ymdhms(secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let s = (secs % 60) as u32;
    let m = ((secs / 60) % 60) as u32;
    let h = ((secs / 3600) % 24) as u32;
    let mut days = (secs / 86400) as i64;
    // 1970-01-01 → ileri.
    let mut year: i64 = 1970;
    loop {
        let yd = if is_leap(year as u32) { 366 } else { 365 };
        if days >= yd {
            days -= yd;
            year += 1;
        } else {
            break;
        }
    }
    let months = [31, 0, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month: u32 = 1;
    for &md in &months {
        let md = if month == 2 {
            if is_leap(year as u32) {
                29
            } else {
                28
            }
        } else {
            md
        };
        if days >= md {
            days -= md;
            month += 1;
        } else {
            break;
        }
    }
    (year as u32, month, (days + 1) as u32, h, m, s)
}

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
