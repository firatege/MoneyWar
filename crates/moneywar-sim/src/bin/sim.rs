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
use moneywar_sim::{render_markdown, Scenario, SimRunner};

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
        vec![1, 7, 42]
    } else {
        vec![seed]
    };

    for s in &seeds {
        let runner = SimRunner::new(*s, scenario)
            .with_ticks(ticks)
            .with_difficulty(diff);
        let result = runner.run();
        let md = render_markdown(&result);
        combined.push_str(&md);
        combined.push_str("\n---\n\n");
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
