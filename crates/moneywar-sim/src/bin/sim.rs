//! `cargo run -p moneywar-sim -- [args]` CLI.
//!
//! Args:
//!   --seed <N>          Default: 42
//!   --ticks <N>         Default: 90
//!   --difficulty <X>    easy|medium|hard  Default: hard
//!   --scenario <NAME>   passive | `active_sanayici` | `active_tuccar`
//!   --report-out <P>    Markdown rapor dosyaya yaz (yoksa stdout)
//!   --multi-seed        10 seed paralel: 1,7,42,100,256,512,1024,2048,4096,8192
//!   --serial            Multi-seed'i sıralı koştur (debug için)
//!   --per-seed-dir <D>  Multi-seed: her seed için ayrı markdown (D/seed_<N>.md)
//!
//! Hızlı baseline:
//!   cargo run -p moneywar-sim -- --multi-seed --report-out artifacts/baseline.md
//!
//! Paralel 10 oyun + per-seed dosyalar:
//!   cargo run -p moneywar-sim -- --multi-seed \
//!       --report-out artifacts/aggregate.md \
//!       --per-seed-dir artifacts/per-seed

use std::env;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Instant;

use moneywar_npc::Difficulty;
use moneywar_sim::{
    GameThresholds, PerRunMetrics, QualityScore, Scenario, SimResult, SimRunner, Stats,
    default_contracts, logbuilder, render_markdown, render_threshold_report,
};

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut seed: u64 = 42;
    let mut ticks: u32 = 90;
    let mut diff = Difficulty::Hard;
    let mut scenario: &'static Scenario = &Scenario::ACTIVE_SANAYICI;
    let mut report_out: Option<String> = None;
    let mut multi_seed = false;
    let mut serial = false;
    let mut per_seed_dir: Option<String> = None;
    let mut threshold_report_out: Option<String> = None;
    let mut log_dir: Option<String> = None;

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
                    "synthetic" => Difficulty::Synthetic,
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
            "--serial" => {
                serial = true;
                i += 1;
            }
            "--per-seed-dir" => {
                per_seed_dir = Some(args[i + 1].clone());
                i += 2;
            }
            "--threshold-out" => {
                threshold_report_out = Some(args[i + 1].clone());
                i += 2;
            }
            "--log-dir" => {
                log_dir = Some(args[i + 1].clone());
                i += 2;
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

    let seeds: Vec<u64> = if multi_seed {
        vec![1, 7, 42, 100, 256, 512, 1024, 2048, 4096, 8192]
    } else {
        vec![seed]
    };

    let started = Instant::now();
    let results: Vec<SimResult> = if multi_seed && !serial {
        run_parallel(&seeds, scenario, ticks, diff)
    } else {
        seeds
            .iter()
            .map(|s| {
                SimRunner::new(*s, scenario)
                    .with_ticks(ticks)
                    .with_difficulty(diff)
                    .run()
            })
            .collect()
    };
    let elapsed_ms = started.elapsed().as_millis();
    eprintln!(
        "🏁 {} run × {} tick → {} ms ({})",
        results.len(),
        ticks,
        elapsed_ms,
        if multi_seed && !serial {
            "parallel"
        } else {
            "serial"
        }
    );

    let all_metrics: Vec<PerRunMetrics> = results.iter().map(PerRunMetrics::from_result).collect();

    // Per-seed dosyaları yaz (multi-seed'de istenirse).
    if multi_seed {
        if let Some(dir) = &per_seed_dir {
            let dir_path = Path::new(dir);
            let _ = fs::create_dir_all(dir_path);
            for r in &results {
                let path = dir_path.join(format!("seed_{}.md", r.seed));
                fs::write(&path, render_markdown(r)).expect("write per-seed");
            }
            eprintln!("Per-seed raporları: {dir}/seed_*.md");
        }
    }

    // Birleşik rapor (stdout veya report-out).
    let mut combined = String::new();
    if multi_seed && !all_metrics.is_empty() {
        let stats = Stats::collect(diff, &all_metrics);
        let quality = QualityScore::from_stats(&stats);
        combined.push_str(&render_aggregate_report(&stats, &quality, &all_metrics));
        combined.push_str(&render_per_seed_summary(&results, &all_metrics));
    } else {
        for r in &results {
            combined.push_str(&render_markdown(r));
            combined.push_str("\n---\n\n");
        }
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

    // Threshold raporu (rol kontrat + oyun kapısı denetimi).
    if multi_seed {
        if let Some(path) = threshold_report_out {
            let stats = Stats::collect(diff, &all_metrics);
            let contracts = default_contracts();
            let thresholds = GameThresholds::hard_default();
            let md = render_threshold_report(&contracts, &thresholds, &results, &stats);
            if let Some(parent) = Path::new(&path).parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&path, &md).expect("write threshold report");
            eprintln!("Threshold raporu: {path}");
        }
    }

    // Tek-flag log builder: timestamped klasör + tüm raporlar/data.
    if let Some(root) = log_dir {
        let root_path = Path::new(&root);
        let _ = fs::create_dir_all(root_path);
        let run_dir = logbuilder::create_run_dir(root_path);
        let cmdline = args.join(" ");
        logbuilder::write_full_log(
            &run_dir,
            &cmdline,
            &seeds,
            ticks,
            diff,
            scenario.name,
            &results,
            &all_metrics,
            elapsed_ms,
        );
        eprintln!("📁 Log klasörü: {}", run_dir.display());
        eprintln!("   ├── manifest.json");
        eprintln!("   ├── aggregate.md");
        eprintln!("   ├── thresholds.md");
        eprintln!("   ├── tuning_issues.md");
        eprintln!("   └── per_seed/seed_<N>.{{md, _actions.jsonl, _clearings.csv}}");
    }
}

/// 10 seed'i paralel iş parçacıklarında koştur. Her thread bir `SimRunner` çalıştırır.
fn run_parallel(
    seeds: &[u64],
    scenario: &'static Scenario,
    ticks: u32,
    diff: Difficulty,
) -> Vec<SimResult> {
    let handles: Vec<_> = seeds
        .iter()
        .map(|&s| {
            thread::spawn(move || {
                SimRunner::new(s, scenario)
                    .with_ticks(ticks)
                    .with_difficulty(diff)
                    .run()
            })
        })
        .collect();
    let mut out: Vec<SimResult> = handles.into_iter().map(|h| h.join().expect("thread")).collect();
    // Seed sırasına göre sırala (deterministik output).
    out.sort_by_key(|r| r.seed);
    out
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
    let _ = writeln!(out);

    out
}

/// Tüm run'ların full Markdown raporunu agregate'in altına ekler — tek dosyada
/// hem ortalama hem detay görmek isteyenler için.
fn render_per_seed_summary(results: &[SimResult], _metrics: &[PerRunMetrics]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "# 🔎 Per-Seed Detay Raporları");
    let _ = writeln!(out);
    for r in results {
        let _ = writeln!(out, "## Seed {}", r.seed);
        out.push_str(&render_markdown(r));
        let _ = writeln!(out, "\n---\n");
    }
    out
}

fn print_help() {
    println!(
        "moneywar-sim — headless deterministic simulation\n\n\
         USAGE:\n  cargo run -p moneywar-sim -- [OPTIONS]\n\n\
         OPTIONS:\n\
           --seed <N>          Default: 42\n\
           --ticks <N>         Default: 90\n\
           --difficulty <X>    easy|medium|hard  Default: hard\n\
           --scenario <NAME>   passive | active_sanayici | active_tuccar\n\
           --report-out <P>    Birleşik markdown rapor dosya yolu\n\
           --multi-seed        10 seed paralel koştur (1,7,42,100,256,512,1024,2048,4096,8192)\n\
           --serial            Multi-seed'i sıralı koştur (debug için)\n\
           --per-seed-dir <D>  Her seed için ayrı dosya: D/seed_<N>.md\n\
           --threshold-out <P> Rol kontrat + oyun kapısı denetim raporu\n\
           --log-dir <D>       Tek flag: D/run_<timestamp>/ altında manifest+aggregate+\n\
                                thresholds+tuning_issues+per_seed/(md+jsonl+csv)\n"
    );
}
