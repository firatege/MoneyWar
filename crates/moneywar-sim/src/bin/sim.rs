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
use moneywar_engine::{LogEvent, leaderboard, story_headline};
use moneywar_sim::balance::{BalanceAccumulator, BalanceReport};
use moneywar_sim::drama::DramaScorecard;
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
    drama: DramaScorecard,
    balance: BalanceReport,
    /// İsimlerle çözülmüş manşetler — "kim ne yaptı" satırları.
    headlines: Vec<String>,
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
    let mut drama = DramaScorecard::default();
    let mut bal = BalanceAccumulator::default();

    for _ in 0..ticks {
        driver.step();
        log.push_str(&format_tick_block(&driver.state, &driver.last_report, driver.season));

        for entry in &driver.last_report.entries {
            drama.record(entry.tick, &entry.event);
            bal.record(&driver.state, &entry.event);
            match &entry.event {
                LogEvent::OrderMatched { city, product, buyer, seller, quantity, .. } => {
                    matched_fills += 1;
                    matched_qty += u64::from(*quantity);
                    let buyer_kind = driver.state.players.get(buyer).and_then(|p| p.npc_kind);
                    acc.record_match(*city, *product, *buyer, buyer_kind, *seller, *quantity);
                }
                LogEvent::ProductionCompleted { factory_id, city, product, units } => {
                    if let Some(owner) = driver.state.factories.get(factory_id).map(|f| f.owner) {
                        acc.record_production(*city, *product, owner, *units);
                    }
                }
                LogEvent::CaravanDispatched { from, to, cargo_total, .. } => {
                    if let Some(owner) = entry.actor {
                        let owner_kind = driver.state.players.get(&owner).and_then(|p| p.npc_kind);
                        acc.record_caravan(*from, *to, owner, owner_kind,
                            u32::try_from(*cargo_total).unwrap_or(u32::MAX));
                    }
                }
                LogEvent::OrderExpired { .. } => expired += 1,
                LogEvent::CommandRejected { .. } => rejected += 1,
                _ => {}
            }
        }
    }
    let metrics = acc.finalize(&driver.state);
    let balance = bal.finalize(&driver.state);
    let headlines: Vec<String> = drama
        .headlines
        .iter()
        .filter_map(|(tick, ev)| {
            story_headline(&driver.state, ev)
                .map(|line| format!("t{:<4} {line}", tick.value()))
        })
        .collect();

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
        drama,
        balance,
        headlines,
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

    // ── SANAYİCİ metrikleri ──────────────────────────────────────────────────
    println!("\n  SANAYİCİ");
    println!(
        "{:>5}  {:>9}  {:>9}  {:>9}  {:>9}   {}",
        "oyun", "fab HHI", "ürt HHI", "ham HHI", "ürt birim", "fabrika dağılımı"
    );
    println!("{:-<82}", "");
    for o in outcomes {
        let m = &o.metrics;
        let fab_dist = m
            .factory_counts
            .iter()
            .take(3)
            .map(|(n, c)| format!("{n}:{c}"))
            .collect::<Vec<_>>()
            .join(" · ");
        let tag = if o.is_frontend { "★" } else { " " };
        println!(
            "{tag}{:>4}  {:>9.0}  {:>9.0}  {:>9.0}  {:>9}   {fab_dist}",
            o.label + 1,
            m.factory_hhi,
            m.production_hhi,
            m.raw_supply_hhi,
            m.total_produced,
        );
    }

    // ── TÜCCAR metrikleri ────────────────────────────────────────────────────
    println!("\n  TÜCCAR");
    println!(
        "{:>5}  {:>9}  {:>9}  {:>9}   {}",
        "oyun", "kervan HHI", "rota HHI", "taşınan", "en aktif rota"
    );
    println!("{:-<82}", "");
    for o in outcomes {
        let m = &o.metrics;
        let top_route = m.top_route.map_or_else(
            || "—".to_string(),
            |((from, to), vol)| format!("{} → {} ({})", from.display_name(), to.display_name(), vol),
        );
        let tag = if o.is_frontend { "★" } else { " " };
        println!(
            "{tag}{:>4}  {:>10.0}  {:>9.0}  {:>9}   {top_route}",
            o.label + 1,
            m.caravan_hhi,
            m.route_dominance_hhi,
            m.total_caravan_vol,
        );
    }

    // ── DRAMA (docs/finish-plan.md Faz 0: hikâyelik olay skorkartı) ─────────
    println!("\n  DRAMA");
    println!("{:>5}  {:>6}  {:>5}  {}", "oyun", "olay", "fren", "döküm");
    println!("{:-<82}", "");
    for o in outcomes {
        let v = o.drama.verdict(o.matched_qty);
        let brake = if v.brakes_ok() { "OK" } else { "İHLAL" };
        let target = if v.story_target_met { "✓" } else { " " };
        let tag = if o.is_frontend { "★" } else { " " };
        println!(
            "{tag}{:>4}  {:>5}{target}  {:>5}  {}",
            o.label + 1,
            v.story_events,
            brake,
            o.drama.summary_line(),
        );
    }

    // Manşetler: izleyicinin göreceği "kim ne yapıyor" akışının örneklemi.
    if let Some(o) = outcomes.iter().find(|o| !o.headlines.is_empty()) {
        println!("\n  MANŞETLER (oyun {})", o.label + 1);
        println!("{:-<82}", "");
        // Sezonun ilk ve son olayları — açılış hamleleri + finalde kim battı.
        let n = o.headlines.len();
        for line in o.headlines.iter().take(6) {
            println!("  {line}");
        }
        if n > 12 {
            println!("  … ({} olay daha) …", n - 12);
        }
        for line in o.headlines.iter().skip(n.saturating_sub(6)) {
            println!("  {line}");
        }
    }

    print_balance(outcomes);

    // ── ORTAK ────────────────────────────────────────────────────────────────
    println!("\n  ORTAK");
    println!("{:>5}  {:>6}   {}", "oyun", "Gini", "rol PnL");
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
        println!("{tag}{:>4}  {:>6.3}   {roles}", o.label + 1, m.wealth_gini);
    }
    println!("{:-<82}", "");

    let total_matched: u64 = outcomes.iter().map(|o| o.matched_qty).sum();
    let ticks = outcomes.first().map_or(0, |o| o.ticks);
    println!(
        "\n{} oyun · {} tick/oyun · toplam {} birim eşleşti · {:.2} sn",
        outcomes.len(), ticks, total_matched, elapsed_s,
    );
    println!("HHI: 0–10000 (>2500 yoğun → 10000 monopol) · Gini: 0 eşit → 1 tek elde · ★ = frontend");
    if let Some(dir) = outcomes.first().and_then(|o| o.log_path.parent()) {
        println!("loglar: {}/game_NN.log", dir.display());
    }
}

/// Denge denetimi bölümü — tüm oyunların ortalaması üstünden rol adaleti,
/// emir akışı sağlığı, fiyat kayması ve red sebepleri.
///
/// Tek oyun gürültülüdür (seed'e göre bir rol şanslı olabilir); denge sorusu
/// ancak oyunlar arası ortalamayla cevaplanır. Bu yüzden rol tablosu
/// toplulaştırılmış, oyun-spesifik olan yalnız adalet makası sütunudur.
fn print_balance(outcomes: &[Outcome]) {
    let n_games = outcomes.len() as f64;
    if outcomes.is_empty() {
        return;
    }

    // ── Rol adaleti (oyunlar arası ortalama) ─────────────────────────────────
    println!("\n  ROL ADALETİ ({} oyun ortalaması)", outcomes.len());
    println!(
        "{:>12}  {:>4}  {:>12}  {:>11}  {:>11}  {:>7}  {:>6}  {:>9}  {:>6}",
        "rol", "n", "kişi başı ₺", "en kötü ₺", "en iyi ₺", "zarar", "iflas", "fill/emir", "red%"
    );
    println!("{:-<95}", "");

    let mut rows: Vec<(f64, String)> = Vec::new();
    for &kind in &moneywar_sim::balance::ROLE_ORDER {
        let per: Vec<_> = outcomes
            .iter()
            .filter_map(|o| o.balance.roles.iter().find(|r| r.kind == kind))
            .collect();
        if per.is_empty() {
            continue;
        }
        let k = per.len() as f64;
        let avg = |f: &dyn Fn(&moneywar_sim::balance::RoleBalance) -> f64| -> f64 {
            per.iter().map(|r| f(r)).sum::<f64>() / k
        };
        let per_capita = avg(&|r| r.pnl_per_capita);
        rows.push((
            per_capita,
            format!(
                "{:>12}  {:>4}  {:>12.0}  {:>11.0}  {:>11.0}  {:>7.1}  {:>6.1}  {:>9.2}  {:>5.0}%",
                kind.label(),
                per[0].count,
                per_capita,
                avg(&|r| r.pnl_min),
                avg(&|r| r.pnl_max),
                avg(&|r| r.losers as f64),
                avg(&|r| r.bankrupt as f64),
                avg(&|r| r.flow.fills_per_order()),
                avg(&|r| r.flow.reject_ratio()) * 100.0,
            ),
        ));
    }
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    for (_, line) in &rows {
        println!("{line}");
    }

    let spreads: Vec<f64> = outcomes
        .iter()
        .filter_map(|o| o.balance.fairness_spread())
        .collect();
    println!("{:-<95}", "");
    if spreads.is_empty() {
        println!("  adalet makası ölçülemedi — en fakir kâr rolü zararda kapattı");
    } else {
        let mean = spreads.iter().sum::<f64>() / spreads.len() as f64;
        let worst = spreads.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        println!(
            "  adalet makası (en zengin kâr rolü ÷ en fakir): ort {mean:.1}× · en kötü {worst:.1}× \
             · hedef <3× · ölçülebilen {}/{} oyun",
            spreads.len(),
            outcomes.len(),
        );
    }

    // ── Arz / talep dengesi ──────────────────────────────────────────────────
    // Fabrika neden atıl, fiyat neden kayıyor sorusunun tek cevap kaynağı:
    // talep arzın kaç katı ve kitaba giren arzın kaçta kaçı gerçekten satıldı.
    let mut flows: std::collections::BTreeMap<&'static str, Vec<moneywar_sim::balance::MarketFlow>> =
        std::collections::BTreeMap::new();
    for o in outcomes {
        for (product, flow) in &o.balance.market {
            flows.entry(product.display_name()).or_default().push(*flow);
        }
    }
    let mut flow_rows: Vec<(f64, String)> = flows
        .into_iter()
        .map(|(name, fs)| {
            let k = fs.len() as f64;
            let mean = |f: &dyn Fn(&moneywar_sim::balance::MarketFlow) -> f64| -> f64 {
                fs.iter().map(|x| f(x)).sum::<f64>() / k
            };
            let ratio = mean(&|f| f.demand_supply_ratio().min(99.0));
            let verdict = if ratio > 2.0 {
                "ARZ AÇIĞI"
            } else if ratio < 0.5 {
                "ARZ FAZLASI"
            } else {
                ""
            };
            (
                ratio,
                format!(
                    "{:>16}  {:>10.0}  {:>10.0}  {:>10.0}  {:>7.1}×  {:>7.0}%  {:>7.0}%  {verdict}",
                    name,
                    mean(&|f| f.demand as f64),
                    mean(&|f| f.supply as f64),
                    mean(&|f| f.matched as f64),
                    ratio,
                    mean(&|f| f.supply_clear_rate()) * 100.0,
                    mean(&|f| f.priced_rate()) * 100.0,
                ),
            )
        })
        .collect();
    flow_rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    if !flow_rows.is_empty() {
        println!("\n  ARZ / TALEP (oyun başına ortalama)");
        println!(
            "{:>16}  {:>10}  {:>10}  {:>10}  {:>8}  {:>8}  {:>8}",
            "ürün", "talep", "arz", "eşleşen", "tal/arz", "arz satıl", "fiyat var"
        );
        println!("{:-<95}", "");
        for (_, line) in &flow_rows {
            println!("{line}");
        }
        println!(
            "{:-<95}\n  not: talep/arz kitapta görülen miktardır — bir emir TTL'i boyunca her \
             tick tekrar sayılır.\n  «tal/arz» ve «fiyat var» güvenilir; «arz satıl» gerçek \
             oranın alt sınırıdır.",
            ""
        );
    }

    // ── Ham maddeyi kim kapıyor ──────────────────────────────────────────────
    // Fabrika aç kalıyorsa iki ihtimal var: ham hiç üretilmiyor, ya da
    // üretiliyor ama Sanayici piyasada kaybediyor. Bu tablo ikincisini ölçer.
    let mut raw_rows: Vec<(u64, String)> = Vec::new();
    let mut raw_total = 0.0;
    for &kind in &moneywar_sim::balance::ROLE_ORDER {
        let qty: u64 = outcomes
            .iter()
            .filter_map(|o| o.balance.raw_buy_by_role.iter().find(|(k, _)| *k == kind))
            .map(|(_, q)| *q)
            .sum();
        if qty > 0 {
            raw_total += qty as f64 / n_games;
            raw_rows.push((qty, kind.label().to_owned()));
        }
    }
    if !raw_rows.is_empty() {
        raw_rows.sort_by(|a, b| b.0.cmp(&a.0));
        println!("\n  HAM MADDEYİ KİM ALIYOR (oyun başına ortalama)");
        println!("{:-<95}", "");
        let line = raw_rows
            .iter()
            .map(|(qty, name)| {
                let per_game = *qty as f64 / n_games;
                format!("{name} {per_game:.0} (%{:.0})", per_game / raw_total * 100.0)
            })
            .collect::<Vec<_>>()
            .join(" · ");
        println!("  {line}");
    }

    // ── Çevirme oranı ────────────────────────────────────────────────────────
    // Aldığının ne kadarını aynı pazarda geri sattı. Üretici için düşük olmalı
    // (aldığı hammaddeyi tüketir), aracı için yüksek. Tüccar'ın kârı taşımadan
    // mı geliyor sorusunun doğrudan cevabı.
    let mut churn_rows: Vec<String> = Vec::new();
    for &kind in &moneywar_sim::balance::ROLE_ORDER {
        let vals: Vec<f64> = outcomes
            .iter()
            .filter_map(|o| o.balance.churn_by_role.iter().find(|(k, _)| *k == kind))
            .map(|(_, r)| *r)
            .collect();
        if !vals.is_empty() {
            let mean = vals.iter().sum::<f64>() / vals.len() as f64;
            churn_rows.push(format!("{} %{:.0}", kind.label(), mean * 100.0));
        }
    }
    if !churn_rows.is_empty() {
        println!("\n  ÇEVİRME ORANI (aldığının ne kadarını aynı pazarda geri sattı)");
        println!("{:-<95}", "");
        println!("  {}", churn_rows.join(" · "));
    }

    // ── Piyasa mekaniği sağlığı ──────────────────────────────────────────────
    let sum = |f: &dyn Fn(&BalanceReport) -> u64| -> f64 {
        outcomes.iter().map(|o| f(&o.balance) as f64).sum::<f64>() / n_games
    };
    let started = sum(&|b| b.production_started);
    let completed = sum(&|b| b.production_completed);
    let idle = sum(&|b| b.factory_idle_ticks);
    println!("\n  PİYASA MEKANİĞİ (oyun başına ortalama)");
    println!("{:-<95}", "");
    // Payda "batch başlatma fırsatı" = başlayan + girdi bulamayan. Üretim
    // süren tick'ler ikisine de girmez, o yüzden bu ham atıl-tick oranı değil.
    println!(
        "  üretim: {started:.0} batch başladı → {completed:.0} tamamlandı · girdi bulamayan tick \
         {idle:.0}\n  fabrika başlatmak istedi ama girdi yoktu: denemelerin %{:.0}'i",
        if started + idle > 0.0 { idle / (started + idle) * 100.0 } else { 0.0 }
    );
    println!(
        "  settlement'ta düşen fill: {:.0} · kredi {:.0} alındı / {:.0} default",
        sum(&|b| b.fill_rejected),
        sum(&|b| b.loans_taken),
        sum(&|b| b.loans_defaulted),
    );

    // ── Red sebepleri (tüm oyunlar toplamı) ──────────────────────────────────
    let mut reasons: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for o in outcomes {
        for (reason, count) in &o.balance.top_rejects {
            *reasons.entry(reason.as_str()).or_default() += count;
        }
    }
    let mut reasons: Vec<_> = reasons.into_iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1));
    if !reasons.is_empty() {
        println!("\n  RED SEBEPLERİ (tüm oyunlar)");
        println!("{:-<95}", "");
        for (reason, count) in reasons.iter().take(4) {
            println!("  {count:>8}  {reason}");
        }
    }

    // ── Fiyat kayması ────────────────────────────────────────────────────────
    let mut drift: std::collections::BTreeMap<String, Vec<f64>> =
        std::collections::BTreeMap::new();
    for o in outcomes {
        for (product, ratio) in &o.balance.price_drift {
            drift
                .entry(product.display_name().to_owned())
                .or_default()
                .push(*ratio);
        }
    }
    let mut drift: Vec<(String, f64)> = drift
        .into_iter()
        .map(|(name, v)| {
            let mean = v.iter().sum::<f64>() / v.len() as f64;
            (name, mean)
        })
        .collect();
    drift.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if !drift.is_empty() {
        println!("\n  FİYAT KAYMASI (sezon sonu ÷ sezon başı baseline, ortalama)");
        println!("{:-<95}", "");
        let line = drift
            .iter()
            .map(|(name, r)| format!("{name} {r:.2}×"))
            .collect::<Vec<_>>()
            .join(" · ");
        println!("  {line}");
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
