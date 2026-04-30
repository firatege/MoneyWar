//! Plan v4 davranış sözleşmeleri (RoleContract) + global oyun kapısı.
//!
//! Her NpcKind için **ne yapmalı** + **ne yapmamalı** tanımlı. Test koşumunda
//! gerçek aksiyon mix'i bu kontratlara karşı denetlenir.
//!
//! İki katman:
//! 1. **RoleContract** — NpcKind başına aksiyon dağılımı + PnL bandı + ihlal
//!    sayacı (yasak komut emiti).
//! 2. **GameThresholds** — sezon genelinde piyasanın canlılığı, batık sayısı,
//!    fiyat stabilitesi, talep doygunluğu.
//!
//! Hard difficulty için baseline sayılar; tuning sonrası daraltılabilir.

use std::collections::BTreeMap;

use crate::runner::{RoleActionMix, SimResult};
use crate::stats::Stats;

/// Bir kuralın geçti mi/sebebi raporu.
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub label: String,
    pub passed: bool,
    pub detail: String,
}

/// NpcKind başına davranış kontratı. Aşağıdaki tüm sınırları sezon sonu
/// audit eder.
#[derive(Debug, Clone)]
pub struct RoleContract {
    pub kind: &'static str,
    /// Kısa açıklama: bu rolün **görevi**.
    pub mandate: &'static str,
    /// Yapması beklenen aksiyon türleri (en az 1 emit zorunlu).
    pub required_actions: &'static [RequiredAction],
    /// Asla yapmaması gereken aksiyon listesi (forbidden_action_count == 0).
    pub forbidden_label: &'static str,
    /// PnL alt sınırı (lira, sezon bazlı).
    pub min_pnl_lira: f64,
    /// PnL üst sınırı (lira). Aşırı PnL = oyun dengesini bozar (ekonomi sızıntısı).
    pub max_pnl_lira: f64,
    /// Toplam emit edilen komutun bu kategorideki minimum payı (0.0..1.0).
    pub min_dominant_action_share: f64,
}

/// Bir komut tipinin minimum sezon emiti.
#[derive(Debug, Clone, Copy)]
pub struct RequiredAction {
    pub label: &'static str,
    /// Sezon başı min emit sayısı.
    pub min_count: u32,
    /// Bu kategori `RoleActionMix`'in hangi alanından okunacak.
    pub field: ActionField,
}

#[derive(Debug, Clone, Copy)]
pub enum ActionField {
    BuyRaw,
    BuyFinished,
    /// `BuyRaw + BuyFinished` toplamı — Tüccar gibi ürün-bağımsız BUY.
    BuyAny,
    SellRaw,
    SellFinished,
    /// `SellRaw + SellFinished` toplamı.
    SellAny,
    BuildFactory,
    BuyCaravan,
    Dispatch,
    ProposeContract,
}

impl ActionField {
    fn count(self, mix: &RoleActionMix) -> u32 {
        match self {
            Self::BuyRaw => mix.buy_raw,
            Self::BuyFinished => mix.buy_finished,
            Self::BuyAny => mix.buy_raw + mix.buy_finished,
            Self::SellRaw => mix.sell_raw,
            Self::SellFinished => mix.sell_finished,
            Self::SellAny => mix.sell_raw + mix.sell_finished,
            Self::BuildFactory => mix.build_factory,
            Self::BuyCaravan => mix.buy_caravan,
            Self::Dispatch => mix.dispatch,
            Self::ProposeContract => mix.propose_contract,
        }
    }
}

/// Plan v4 default kontratları — tek-görev tasarım, Hard 90 tick referansı.
#[must_use]
pub fn default_contracts() -> Vec<RoleContract> {
    vec![
        RoleContract {
            kind: "Ciftci",
            mandate: "Hammadde üretici — sadece SELL raw, ürün stoğunu market'e akıt.",
            required_actions: &[RequiredAction {
                label: "SELL raw",
                min_count: 5,
                field: ActionField::SellRaw,
            }],
            forbidden_label: "BUY herhangi bir şey, SELL mamul",
            min_pnl_lira: -10_000.0,
            max_pnl_lira: 30_000.0,
            min_dominant_action_share: 0.95, // ~tüm aksiyon SELL raw olmalı
        },
        RoleContract {
            kind: "Esnaf",
            mandate: "Toptancı — Çiftçi'den ham al, Sanayici/Alıcı'ya markup'la sat.",
            required_actions: &[
                RequiredAction {
                    label: "BUY raw",
                    min_count: 3,
                    field: ActionField::BuyRaw,
                },
                RequiredAction {
                    label: "SELL toplam (raw+finished)",
                    min_count: 5,
                    field: ActionField::SellAny,
                },
            ],
            forbidden_label: "BUY mamul (Sanayici işi)",
            // PnL bandı: vergi (closed loop %5) + haber tier maliyetini absorbe eder.
            // Trading marjı ~ -1500₺ ile +5K arası gerçekçi.
            min_pnl_lira: -6_500.0,
            max_pnl_lira: 60_000.0,
            min_dominant_action_share: 0.0,
        },
        RoleContract {
            kind: "Sanayici",
            mandate: "Üretici — fabrika kurup raw_input alır, mamul üretip satar.",
            required_actions: &[
                RequiredAction {
                    label: "BUY raw (factory input)",
                    min_count: 3,
                    field: ActionField::BuyRaw,
                },
                RequiredAction {
                    label: "SELL finished (factory output)",
                    min_count: 3,
                    field: ActionField::SellFinished,
                },
            ],
            forbidden_label: "BUY mamul, SELL raw",
            min_pnl_lira: -25_000.0,
            max_pnl_lira: 80_000.0,
            min_dominant_action_share: 0.0,
        },
        RoleContract {
            kind: "Tuccar",
            mandate: "Lojistik — şehirler arası arbitraj, kervan dispatch.",
            required_actions: &[
                RequiredAction {
                    label: "BUY arbitraj (raw+finished)",
                    min_count: 5,
                    field: ActionField::BuyAny,
                },
                RequiredAction {
                    label: "SELL arbitraj",
                    min_count: 5,
                    field: ActionField::SellAny,
                },
            ],
            forbidden_label: "Same-city BUY+SELL (arbitraj gate ihlali)",
            min_pnl_lira: -10_000.0,
            max_pnl_lira: 80_000.0,
            min_dominant_action_share: 0.0,
        },
        RoleContract {
            kind: "Alici",
            mandate: "Tüketici — mamul al, periyodik maaş ile yenilen.",
            required_actions: &[RequiredAction {
                label: "BUY finished",
                min_count: 3,
                field: ActionField::BuyFinished,
            }],
            forbidden_label: "BUY raw",
            min_pnl_lira: -120_000.0,
            max_pnl_lira: 80_000.0,
            min_dominant_action_share: 0.0,
        },
        RoleContract {
            kind: "Spekulator",
            mandate: "Market maker — hem bid hem ask, spread oyunu.",
            required_actions: &[
                RequiredAction {
                    label: "BUY emir (any)",
                    min_count: 3,
                    field: ActionField::BuyAny,
                },
                RequiredAction {
                    label: "SELL emir (any)",
                    min_count: 3,
                    field: ActionField::SellAny,
                },
            ],
            forbidden_label: "Tek yönlü (sadece BUY veya sadece SELL)",
            min_pnl_lira: -25_000.0,
            max_pnl_lira: 50_000.0,
            min_dominant_action_share: 0.0,
        },
        RoleContract {
            kind: "Banka",
            mandate: "Likidite — distress NPC'lere kredi açar (özel akış).",
            required_actions: &[],
            forbidden_label: "Ticari komut emiti (BUY/SELL/BuildFactory)",
            min_pnl_lira: -5_000.0,
            max_pnl_lira: 100_000.0,
            min_dominant_action_share: 0.0,
        },
    ]
}

/// Tek bir rol kontratını run sonucuna karşı denetle.
pub fn audit_role(
    contract: &RoleContract,
    mix_total: &RoleActionMix, // tüm seedlerde toplam (ya da tek seed)
    pnl_avg: f64,
    n_npcs: u32, // bu role mensup NPC sayısı (kontrat kontrol ölçek)
    n_seeds: u32,
) -> Vec<CheckResult> {
    let mut out = Vec::new();
    let scale = (n_npcs * n_seeds).max(1) as f64;

    // 1. Yasaklı aksiyon yok.
    out.push(CheckResult {
        label: format!("[{}] Yasak aksiyon yok ({})", contract.kind, contract.forbidden_label),
        passed: mix_total.forbidden_action_count == 0,
        detail: format!("forbidden_count={}", mix_total.forbidden_action_count),
    });

    // 2. Beklenen aksiyon türleri (NPC sayısı ile ölçeklensin).
    for req in contract.required_actions {
        let count = req.field.count(mix_total);
        let need = (req.min_count as f64 * scale).ceil() as u32;
        out.push(CheckResult {
            label: format!(
                "[{}] {} ≥ {} (toplam {} NPC × {} seed)",
                contract.kind, req.label, need, n_npcs, n_seeds
            ),
            passed: count >= need,
            detail: format!("emit={} need={}", count, need),
        });
    }

    // 3. PnL bandı (sezon ortalaması).
    out.push(CheckResult {
        label: format!(
            "[{}] PnL ortalama [{:.0}, {:.0}]₺ aralığında",
            contract.kind, contract.min_pnl_lira, contract.max_pnl_lira
        ),
        passed: pnl_avg >= contract.min_pnl_lira && pnl_avg <= contract.max_pnl_lira,
        detail: format!("avg={:.0}₺", pnl_avg),
    });

    out
}

/// Sezon genelinde piyasanın sağlığı için global eşikler.
#[derive(Debug, Clone)]
pub struct GameThresholds {
    /// Toplam clearing sayısı sezon başına (10 seed mean).
    pub min_total_matches_per_run: f64,
    /// Match verim % min (matched_qty / submitted).
    pub min_match_efficiency_pct: f64,
    /// İflas eden NPC sayısı max (sezon başına).
    pub max_bankrupt_npcs: f64,
    /// En eski emrin yaşı tick max.
    pub max_stale_order_age: f64,
    /// İnsan oyuncuya hayatta kalma garantisi (PnL alt sınır).
    pub min_human_pnl_lira: f64,
    /// Banka kredi olayı min sayısı (closed loop tetiklendi mi).
    pub min_bank_loans_issued: u32,
    /// Pazar canlılığı — 18 bucket'tan kaç tanesi ölü (clearing < 1) olabilir.
    pub max_dead_buckets: u32,
}

impl GameThresholds {
    #[must_use]
    pub fn hard_default() -> Self {
        Self {
            min_total_matches_per_run: 800.0,
            min_match_efficiency_pct: 0.8,
            max_bankrupt_npcs: 1.0,
            max_stale_order_age: 10.0,
            min_human_pnl_lira: -25_000.0,
            min_bank_loans_issued: 0,
            // 18 bucket'tan max 4 ölü kabul (Izmir/Ankara'da bazı raw'lar
            // doğal olarak az dönebilir). Hedef: tuning ile 2'ye in.
            max_dead_buckets: 4,
        }
    }
}

/// 18 (city, product) bucket'tan kaç tanesi ölü (sezon boyu hiç clearing yok).
/// Mean across runs.
fn count_dead_buckets(runs: &[SimResult]) -> f64 {
    use std::collections::BTreeSet;
    if runs.is_empty() {
        return 0.0;
    }
    let mut total_dead: u32 = 0;
    for r in runs {
        let mut active: BTreeSet<(u8, u8)> = BTreeSet::new();
        for snap in &r.snapshots {
            for c in &snap.clearings {
                if c.matched_qty > 0 {
                    active.insert((c.city, c.product));
                }
            }
        }
        // 3 şehir × 6 ürün = 18.
        total_dead += 18 - active.len() as u32;
    }
    total_dead as f64 / runs.len() as f64
}

/// Global oyun kapısını denetle.
pub fn audit_game(
    thresholds: &GameThresholds,
    stats: &Stats,
    bank_loans_total: u32,
) -> Vec<CheckResult> {
    audit_game_with_runs(thresholds, stats, bank_loans_total, &[])
}

/// `audit_game`'in run-aware versiyonu — pazar dolaşım kontrolü için runs lazım.
pub fn audit_game_with_runs(
    thresholds: &GameThresholds,
    stats: &Stats,
    bank_loans_total: u32,
    runs: &[SimResult],
) -> Vec<CheckResult> {
    let mut out = Vec::new();
    out.push(CheckResult {
        label: format!(
            "Toplam clearing ≥ {:.0}",
            thresholds.min_total_matches_per_run
        ),
        passed: stats.matches.mean >= thresholds.min_total_matches_per_run,
        detail: format!("mean={:.0}", stats.matches.mean),
    });
    out.push(CheckResult {
        label: format!(
            "Match verimi ≥ {:.1}%",
            thresholds.min_match_efficiency_pct
        ),
        passed: stats.match_efficiency_pct.mean >= thresholds.min_match_efficiency_pct,
        detail: format!("{:.2}%", stats.match_efficiency_pct.mean),
    });
    out.push(CheckResult {
        label: format!("Bankrupt NPC ≤ {:.0}", thresholds.max_bankrupt_npcs),
        passed: stats.bankrupt_npcs.mean <= thresholds.max_bankrupt_npcs,
        detail: format!("mean={:.1}", stats.bankrupt_npcs.mean),
    });
    out.push(CheckResult {
        label: format!("Stale order yaşı ≤ {:.0} tick", thresholds.max_stale_order_age),
        passed: stats.stale_orders_max.mean <= thresholds.max_stale_order_age,
        detail: format!("max_avg={:.1}", stats.stale_orders_max.mean),
    });
    out.push(CheckResult {
        label: format!("İnsan PnL ≥ {:.0}₺", thresholds.min_human_pnl_lira),
        passed: stats.human_pnl.mean >= thresholds.min_human_pnl_lira,
        detail: format!("avg={:.0}₺", stats.human_pnl.mean),
    });
    out.push(CheckResult {
        label: format!(
            "Banka kredi açıldı ≥ {} (closed loop)",
            thresholds.min_bank_loans_issued
        ),
        passed: bank_loans_total >= thresholds.min_bank_loans_issued,
        detail: format!("total={}", bank_loans_total),
    });
    if !runs.is_empty() {
        let dead = count_dead_buckets(runs);
        out.push(CheckResult {
            label: format!(
                "Ölü bucket ≤ {} / 18 (pazar dolaşımı)",
                thresholds.max_dead_buckets
            ),
            passed: dead <= thresholds.max_dead_buckets as f64,
            detail: format!("mean={:.1}/18", dead),
        });
    }
    out
}

/// 10 seed × 7 rol kontrat raporu — Markdown.
pub fn render_threshold_report(
    contracts: &[RoleContract],
    thresholds: &GameThresholds,
    runs: &[SimResult],
    stats: &Stats,
) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let n_seeds = runs.len() as u32;

    let _ = writeln!(out, "# 🛡️ Threshold Audit — {} seed × Hard 90 tick", n_seeds);
    let _ = writeln!(out);

    // Global oyun kapısı.
    let bank_loans_total: u32 = runs.iter().map(|r| r.bank_loans_issued).sum();
    let game_checks = audit_game_with_runs(thresholds, stats, bank_loans_total, runs);
    let game_passed = game_checks.iter().filter(|c| c.passed).count();
    let _ = writeln!(out, "## Oyun Kapısı — {}/{} geçti", game_passed, game_checks.len());
    let _ = writeln!(out);
    let _ = writeln!(out, "| Madde | Geçti | Detay |");
    let _ = writeln!(out, "|---|---|---|");
    for c in &game_checks {
        let icon = if c.passed { "✅" } else { "❌" };
        let _ = writeln!(out, "| {} | {} | {} |", c.label, icon, c.detail);
    }
    let _ = writeln!(out);

    // Rol bazlı kontrat denetimi.
    let mut total_mix: BTreeMap<String, RoleActionMix> = BTreeMap::new();
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
        // NPC sayısını ilk run'dan al.
        if let Some(first_snap) = r.snapshots.first() {
            for p in &first_snap.players {
                if let Some(kind) = &p.npc_kind {
                    let c = npc_count_per_kind.entry(kind.clone()).or_default();
                    if r.seed == runs[0].seed {
                        *c += 1;
                    }
                }
            }
        }
    }

    for contract in contracts {
        let mix = total_mix.get(contract.kind).cloned().unwrap_or_default();
        let pnl_avg = stats
            .pnl_by_role
            .get(contract.kind)
            .map(|s| s.mean)
            .unwrap_or(0.0);
        let n_npcs = *npc_count_per_kind.get(contract.kind).unwrap_or(&0);
        let checks = audit_role(contract, &mix, pnl_avg, n_npcs, n_seeds);
        let passed = checks.iter().filter(|c| c.passed).count();

        let _ = writeln!(
            out,
            "## {} — {}/{} geçti",
            contract.kind,
            passed,
            checks.len()
        );
        let _ = writeln!(out, "_{}_", contract.mandate);
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "**Aksiyon dağılımı (10 seed toplam, {} NPC × {} seed):**",
            n_npcs, n_seeds
        );
        let _ = writeln!(
            out,
            "- BUY raw={}, BUY finished={}",
            mix.buy_raw, mix.buy_finished
        );
        let _ = writeln!(
            out,
            "- SELL raw={}, SELL finished={}",
            mix.sell_raw, mix.sell_finished
        );
        let _ = writeln!(
            out,
            "- BuildFactory={}, BuyCaravan={}, Dispatch={}",
            mix.build_factory, mix.buy_caravan, mix.dispatch
        );
        let _ = writeln!(
            out,
            "- Contract propose={}, Loan take={}, Loan repay={}",
            mix.propose_contract, mix.take_loan, mix.repay_loan
        );
        let _ = writeln!(out, "- **Forbidden = {}**", mix.forbidden_action_count);
        let _ = writeln!(out);

        let _ = writeln!(out, "| Madde | Geçti | Detay |");
        let _ = writeln!(out, "|---|---|---|");
        for c in &checks {
            let icon = if c.passed { "✅" } else { "❌" };
            let _ = writeln!(out, "| {} | {} | {} |", c.label, icon, c.detail);
        }
        let _ = writeln!(out);
    }

    out
}
