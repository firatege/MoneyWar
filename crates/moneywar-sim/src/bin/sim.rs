//! `cargo run -p moneywar-sim -- [args]` CLI.
//!
//! Args:
//!   --seed <N>          Default: 42
//!   --ticks <N>         Default: 90
//!   --difficulty <X>    easy|hard  (Expert kaldırıldı; medium Faz 1'de gelir)
//!   --scenario <NAME>   passive | active_sanayici | active_tuccar
//!   --report-out <P>    Markdown rapor dosyaya yaz (yoksa stdout)
//!   --multi-seed        1,7,42 ile koştur, üç rapor + ortalama
//!
//! Hızlı baseline:
//!   cargo run -p moneywar-sim -- --multi-seed --report-out artifacts/baseline_v02.md

use std::env;
use std::fs;
use std::path::Path;

use moneywar_npc::Difficulty;
use moneywar_sim::{
    render_markdown, PerRunMetrics, QualityScore, Scenario, SimRunner, Stats,
};

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut seed: u64 = 42;
    let mut ticks: u32 = 90;
    let mut diff = Difficulty::Hard;
    let mut scenario: &'static Scenario = &Scenario::ACTIVE_SANAYICI;
    let mut report_out: Option<String> = None;
    let mut multi_seed = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                seed = args[i + 1].parse().expect("--seed: u64");
                i += 2;
            }
            "--ticks" => {
                ticks = args[i + 1].parse().expect("--ticks: u32");
                i += 2;
            }
            "--difficulty" => {
                diff = match args[i + 1].as_str() {
                    "easy" => Difficulty::Easy,
                    "hard" => Difficulty::Hard,
                    "medium" => Difficulty::Medium,
                    other => panic!("bilinmeyen difficulty: {other}"),
                };
                i += 2;
            }
            "--scenario" => {
                scenario = match args[i + 1].as_str() {
                    "passive" => &Scenario::PASSIVE,
                    "active_sanayici" => &Scenario::ACTIVE_SANAYICI,
                    "active_tuccar" => &Scenario::ACTIVE_TUCCAR,
                    other => panic!("bilinmeyen scenario: {other}"),
                };
                i += 2;
            }
            "--report-out" => {
                report_out = Some(args[i + 1].clone());
                i += 2;
            }
            "--multi-seed" => {
                multi_seed = true;
                i += 1;
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            other => {
                eprintln!("bilinmeyen arg: {other}");
                print_help();
                std::process::exit(1);
            }
        }
    }

    let mut combined = String::new();
    let seeds: Vec<u64> = if multi_seed {
        // Genişletilmiş seed seti — daha güçlü istatistik (10 seed).
        vec![1, 7, 42, 100, 256, 512, 1024, 2048, 4096, 8192]
    } else {
        vec![seed]
    };

    let mut all_metrics: Vec<PerRunMetrics> = Vec::new();
    for s in &seeds {
        let runner = SimRunner::new(*s, scenario)
            .with_ticks(ticks)
            .with_difficulty(diff);
        let result = runner.run();
        let metrics = PerRunMetrics::from_result(&result);
        all_metrics.push(metrics);
        // Sadece tek seed'de detaylı rapor; multi-seed'de agregat yeterli.
        if !multi_seed {
            let md = render_markdown(&result);
            combined.push_str(&md);
            combined.push_str("\n---\n\n");
        }
    }

    // Multi-seed'de agregat istatistik raporu üret.
    if multi_seed && !all_metrics.is_empty() {
        let stats = Stats::collect(diff, &all_metrics);
        let quality = QualityScore::from_stats(&stats);
        combined.push_str(&render_aggregate_report(&stats, &quality, &all_metrics));
    }

    if let Some(path) = report_out {
        if let Some(parent) = Path::new(&path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(&path, &combined).expect("write report");
        eprintln!("Rapor yazıldı: {path}");
    } else {
        print!("{combined}");
    }
}

/// Multi-seed agregat raporunu Markdown olarak üret.
fn render_aggregate_report(
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
    let _ = writeln!(out, "## Kalite Kapısı");
    let _ = writeln!(
        out,
        "**Skor: {}/{}** — {:?}",
        quality.passed, quality.total, stats.difficulty
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "| Madde | Geçti | Değer |");
    let _ = writeln!(out, "|---|---|---|");
    for (item, ok, value) in &quality.details {
        let icon = if *ok { "✅" } else { "❌" };
        let _ = writeln!(out, "| {item} | {icon} | {value} |");
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Genel Metrik İstatistikleri (mean ± std, [min, max])");
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

    let _ = writeln!(out, "## Rol Başına PnL Dağılımı");
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

    let _ = writeln!(out, "## Per-Seed Detayları");
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
    let _ = writeln!(out);

    out
}

fn print_help() {
    println!(
        "moneywar-sim — headless deterministic simulation\n\n\
         USAGE:\n  cargo run -p moneywar-sim -- [OPTIONS]\n\n\
         OPTIONS:\n\
           --seed <N>          Default: 42\n\
           --ticks <N>         Default: 90\n\
           --difficulty <X>    easy|hard|expert  Default: hard\n\
           --scenario <NAME>   passive | active_sanayici | active_tuccar\n\
           --report-out <P>    Markdown rapor dosya yolu (yoksa stdout)\n\
           --multi-seed        seeds 1,7,42 üçü için koştur\n"
    );
}
