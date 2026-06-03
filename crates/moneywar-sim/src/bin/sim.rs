//! Headless MoneyWar sim — frontend'de gözüken oyunun **birebir aynısını** koşar.
//!
//! Canlı sunucuyla (moneywar-web) aynı `SimDriver`, aynı dünya kurulumu
//! (`new_season`) ve aynı tick-by-tick log formatını (`format_tick_block`)
//! kullanır; tek fark actix/WS yok, motor tam hızda döner. Her oyun için
//! tam log dosyası + stdout'a kısa özet üretir.
//!
//! Kullanım:
//!   cargo run -p moneywar-sim                 # 1 oyun, 150 tick (frontend sezonu)
//!   cargo run -p moneywar-sim -- --parallel   # 5 oyunu aynı anda koş
//!   cargo run -p moneywar-sim -- --games 3    # 3 oyun, sıralı
//!   cargo run -p moneywar-sim -- --games 8 --parallel --ticks 300 --out /tmp/mw
//!
//! Argümanlar:
//!   --games <N>     Kaç oyun simüle edilsin. Default 1 (--parallel verilince 5).
//!   --parallel      Oyunları paralel (thread) koştur. Default sıralı.
//!   --ticks <N>     Oyun başına tick. Default 150 (moneywar-web SEASON_TICKS).
//!   --seed <N>      Base seed. Default frontend ile aynı; oyun #1 birebir o oyundur.
//!   --out <DIR>     Log çıktı dizini. Default "artifacts/sim".

use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;

use moneywar_domain::Money;
use moneywar_engine::{LogEvent, leaderboard};
use moneywar_sim::metrics::{Metrics, MetricsAccumulator};
use moneywar_web::driver::SimDriver;
use moneywar_web::{DEFAULT_SEED, DIFFICULTY, SEASON_TICKS, TICK_SECONDS, debuglog::format_tick_block};

/// Tek oyunun simülasyon sonucu — özet tablosu için.
struct Outcome {
    label: usize,
    seed: u64,
    ticks: u32,
    is_frontend: bool,
    matched_qty: u64,
    matched_fills: u64,
    expired: u64,
    rejected: u64,
    top: Vec<(String, f64)>,
    metrics: Metrics,
    log_path: PathBuf,
}

/// CLI ayarları.
struct Config {
    games: usize,
    parallel: bool,
    ticks: u32,
    base_seed: u64,
    out_dir: PathBuf,
}

fn money_to_lira(m: Money) -> f64 {
    m.as_cents() as f64 / 100.0
}

fn main() {
    let cfg = parse_args();

    if let Err(e) = std::fs::create_dir_all(&cfg.out_dir) {
        eprintln!("çıktı dizini oluşturulamadı ({}): {e}", cfg.out_dir.display());
        std::process::exit(1);
    }

    println!(
        "MoneyWar sim — {} oyun × {} tick · {} · seed taban={:#x} · çıktı={}",
        cfg.games,
        cfg.ticks,
        if cfg.parallel { "paralel" } else { "sıralı" },
        cfg.base_seed,
        cfg.out_dir.display(),
    );

    let started = Instant::now();
    let outcomes = if cfg.parallel {
        run_parallel(&cfg)
    } else {
        run_sequential(&cfg)
    };
    let elapsed = started.elapsed();

    print_summary(&outcomes, elapsed.as_secs_f64());
}

/// Oyun `i` (0-tabanlı) için base seed. i=0 + varsayılan taban → frontend oyunu.
fn seed_for(base: u64, i: usize) -> u64 {
    base.wrapping_add(i as u64)
}

fn run_sequential(cfg: &Config) -> Vec<Outcome> {
    (0..cfg.games)
        .map(|i| run_game(i, seed_for(cfg.base_seed, i), cfg))
        .collect()
}

fn run_parallel(cfg: &Config) -> Vec<Outcome> {
    let handles: Vec<_> = (0..cfg.games)
        .map(|i| {
            let seed = seed_for(cfg.base_seed, i);
            let ticks = cfg.ticks;
            let out_dir = cfg.out_dir.clone();
            let is_frontend = i == 0 && cfg.base_seed == DEFAULT_SEED;
            thread::spawn(move || run_game_inner(i, seed, ticks, &out_dir, is_frontend))
        })
        .collect();

    let mut outcomes: Vec<Outcome> = handles
        .into_iter()
        .filter_map(|h| h.join().ok())
        .collect();
    outcomes.sort_by_key(|o| o.label);
    outcomes
}

fn run_game(i: usize, seed: u64, cfg: &Config) -> Outcome {
    let is_frontend = i == 0 && cfg.base_seed == DEFAULT_SEED;
    run_game_inner(i, seed, cfg.ticks, &cfg.out_dir, is_frontend)
}

/// Bir oyunu baştan sona koşar: her tick `SimDriver::step` + log bloğu +
/// istatistik biriktir, sonra log dosyasını yazar ve özet döner.
fn run_game_inner(i: usize, seed: u64, ticks: u32, out_dir: &Path, is_frontend: bool) -> Outcome {
    let mut driver = SimDriver::new(
        seed,
        ticks,
        u32::try_from(TICK_SECONDS).unwrap_or(5),
        DIFFICULTY,
    );

    let mut log = String::new();
    let mut matched_qty: u64 = 0;
    let mut matched_fills: u64 = 0;
    let mut expired: u64 = 0;
    let mut rejected: u64 = 0;
    let mut acc = MetricsAccumulator::default();

    for _ in 0..ticks {
        driver.step();
        log.push_str(&format_tick_block(&driver.state, &driver.last_report, driver.season));

        for entry in &driver.last_report.entries {
            match &entry.event {
                LogEvent::OrderMatched {
                    city,
                    product,
                    buyer,
                    seller,
                    quantity,
                    ..
                } => {
                    matched_fills += 1;
                    matched_qty += u64::from(*quantity);
                    acc.record_match(*city, *product, *buyer, *seller, *quantity);
                }
                LogEvent::OrderExpired { .. } => expired += 1,
                LogEvent::CommandRejected { .. } => rejected += 1,
                _ => {}
            }
        }
    }
    let metrics = acc.finalize(&driver.state);

    let log_path = out_dir.join(format!("game_{:02}.log", i + 1));
    if let Err(e) = std::fs::write(&log_path, &log) {
        eprintln!("log yazılamadı ({}): {e}", log_path.display());
    }

    let top: Vec<(String, f64)> = leaderboard(&driver.state)
        .into_iter()
        .take(3)
        .map(|score| {
            let name = driver
                .state
                .players
                .get(&score.player_id)
                .map_or_else(|| format!("#{}", score.player_id.value()), |p| p.name.clone());
            (name, money_to_lira(score.total))
        })
        .collect();

    Outcome {
        label: i,
        seed,
        ticks,
        is_frontend,
        matched_qty,
        matched_fills,
        expired,
        rejected,
        top,
        metrics,
        log_path,
    }
}

fn print_summary(outcomes: &[Outcome], elapsed_s: f64) {
    println!("\n{:=<82}", "");
    println!(
        "{:>5}  {:>18}  {:>8}  {:>6}  {:>8}  {:>8}  {}",
        "oyun", "seed", "eşleşen", "fill", "expired", "reddet", "lider (PnL₺)"
    );
    println!("{:-<82}", "");

    for o in outcomes {
        let leader = o
            .top
            .first()
            .map_or_else(|| "—".to_string(), |(name, pnl)| format!("{name} ({pnl:+.0})"));
        let tag = if o.is_frontend { "★" } else { " " };
        println!(
            "{tag}{:>4}  {:>18}  {:>8}  {:>6}  {:>8}  {:>8}  {leader}",
            o.label + 1,
            format!("{:#x}", o.seed),
            o.matched_qty,
            o.matched_fills,
            o.expired,
            o.rejected,
        );
    }

    println!("{:-<82}", "");

    // Yoğunlaşma / rekabet metrikleri (Faz 0).
    println!(
        "\n{:>5}  {:>9}  {:>8}  {:>9}  {:>6}   {}",
        "oyun", "arz HHI", "top %", "fab HHI", "Gini", "rol PnL (lider→)"
    );
    println!("{:-<82}", "");
    for o in outcomes {
        let m = &o.metrics;
        let roles = m
            .role_pnl
            .iter()
            .take(3)
            .map(|(r, pnl)| format!("{r} {pnl:+.0}"))
            .collect::<Vec<_>>()
            .join(" · ");
        let tag = if o.is_frontend { "★" } else { " " };
        println!(
            "{tag}{:>4}  {:>9.0}  {:>7.1}%  {:>9.0}  {:>6.3}   {roles}",
            o.label + 1,
            m.supply_hhi,
            m.top_firm_share,
            m.factory_hhi,
            m.wealth_gini,
        );
    }
    println!("{:-<82}", "");

    let total_matched: u64 = outcomes.iter().map(|o| o.matched_qty).sum();
    let ticks = outcomes.first().map_or(0, |o| o.ticks);
    println!(
        "{} oyun · {} tick/oyun · toplam {} birim eşleşti · {:.2} sn",
        outcomes.len(),
        ticks,
        total_matched,
        elapsed_s,
    );
    println!(
        "HHI: 0–10000 (>2500 yoğun, →10000 monopol) · Gini: 0 eşit → 1 tek elde · ★ = frontend oyunu"
    );
    if let Some(dir) = outcomes.first().and_then(|o| o.log_path.parent()) {
        println!("loglar: {}/game_NN.log", dir.display());
    }
}

/// Argümanları çok basit elle parse et (clap bağımlılığı yok).
fn parse_args() -> Config {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut games: Option<usize> = None;
    let mut parallel = false;
    let mut ticks: u32 = SEASON_TICKS;
    let mut base_seed: u64 = DEFAULT_SEED;
    let mut out_dir = PathBuf::from("artifacts/sim");

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--parallel" | "-p" => parallel = true,
            "--games" | "-g" => {
                games = it.next().and_then(|v| v.parse().ok());
            }
            "--ticks" | "-t" => {
                if let Some(v) = it.next().and_then(|v| v.parse().ok()) {
                    ticks = v;
                }
            }
            "--seed" | "-s" => {
                if let Some(v) = it.next().and_then(|v| parse_seed(v)) {
                    base_seed = v;
                }
            }
            "--out" | "-o" => {
                if let Some(v) = it.next() {
                    out_dir = PathBuf::from(v);
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("bilinmeyen argüman: {other} (--help)");
                std::process::exit(2);
            }
        }
    }

    // --parallel verilip --games verilmediyse mantıklı varsayılan: 5 oyun.
    let games = games.unwrap_or(if parallel { 5 } else { 1 }).max(1);

    Config { games, parallel, ticks, base_seed, out_dir }
}

/// Seed'i ondalık ya da `0x`-önekli hex olarak parse et.
fn parse_seed(s: &str) -> Option<u64> {
    s.strip_prefix("0x")
        .map_or_else(|| s.parse().ok(), |hex| u64::from_str_radix(hex, 16).ok())
}

fn print_help() {
    println!(
        "MoneyWar headless sim — frontend ile aynı oyunu koşar\n\n\
         cargo run -p moneywar-sim -- [SEÇENEKLER]\n\n\
         --games <N>    Kaç oyun (default 1; --parallel ile 5)\n\
         --parallel,-p  Oyunları aynı anda koş\n\
         --ticks <N>    Oyun başına tick (default {SEASON_TICKS})\n\
         --seed <N>     Base seed (ondalık veya 0xHEX)\n\
         --out <DIR>    Log dizini (default artifacts/sim)\n\
         --help,-h      Bu yardım"
    );
}
