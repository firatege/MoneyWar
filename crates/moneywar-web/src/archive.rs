//! Sezon arşivi — kapanan her sezonun denge denetimini diske yazar.
//!
//! # Neden ham log değil
//!
//! Tick-by-tick olay akışı sezonda ~48 MB tutar; sezon 17.5 dakika olduğuna
//! göre günde ~4 GB eder. Ne saklanabilir ne de okunabilir. Üstelik "bir
//! sorun var mı?" sorusunu da cevaplamaz — o soru tek tek işlemlerde değil,
//! toplulaştırılmış tablolarda (rol adaleti, arz/talep, üretim marjı, para
//! arzı) yaşar.
//!
//! Bu modül sezon başına ~10 KB yazar: makine için JSON, insan için MD.
//! Günde ~1 MB. Karşılaştırma sorusu ("ne zaman bozuldu, hangi sezondan
//! sonra kötüleşti") ancak böyle cevaplanabilir hale gelir.
//!
//! Anlık soru için diske hiç gerek yok — `GET /api/audit` sezon ortasında da
//! aynı raporu döndürür.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::balance::BalanceReport;
use crate::driver::CompletedSeason;

/// Sezon denetimlerinin yazıldığı dizin.
#[derive(Debug, Clone)]
pub struct SeasonArchive {
    dir: PathBuf,
}

impl SeasonArchive {
    /// Dizini kullanıma hazırlar. Oluşturulamazsa `None` — arşiv olmadan da
    /// oyun çalışmalı, disk sorunu sezonu düşürmemeli.
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Option<Self> {
        let dir = dir.into();
        match std::fs::create_dir_all(&dir) {
            Ok(()) => Some(Self { dir }),
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "sezon arşivi açılamadı");
                None
            }
        }
    }

    /// Kapanan sezonu JSON + MD olarak yazar. Hata log'lanır, yutulmaz ama
    /// çağırana da taşınmaz — arşiv yazamamak oyunu durdurmaz.
    pub fn write(&self, finished: &CompletedSeason) {
        let base = format!("season-{:05}", finished.season);
        self.write_file(&format!("{base}.json"), || {
            serde_json::to_string_pretty(&finished.report)
                .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
        });
        self.write_file(&format!("{base}.md"), || {
            render_markdown(finished.season, &finished.report)
        });
    }

    fn write_file(&self, name: &str, body: impl FnOnce() -> String) {
        let path = self.dir.join(name);
        let result = std::fs::File::create(&path).and_then(|mut f| f.write_all(body().as_bytes()));
        match result {
            Ok(()) => tracing::info!(path = %path.display(), "sezon arşivlendi"),
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "arşiv yazılamadı"),
        }
    }

    /// Arşiv dizini (endpoint listelemesi için).
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Bir sezonun denetimini okunabilir özet olarak yazar.
///
/// Sim'in stdout tablolarıyla aynı bilgiyi taşır; fark, dosyaya gidip
/// sezonlar arası kıyaslanabilmesi.
#[must_use]
pub fn render_markdown(season: u64, r: &BalanceReport) -> String {
    let lira = |cents: i64| cents as f64 / 100.0;
    let mut out = String::new();

    out.push_str(&format!("# Sezon {season} — denge denetimi\n\n"));

    // ── Rol adaleti ─────────────────────────────────────────────────────────
    out.push_str("## Rol adaleti\n\n");
    out.push_str("| rol | n | kişi başı ₺ | zarar eden | iflas |\n");
    out.push_str("|---|---:|---:|---:|---:|\n");
    for role in &r.roles {
        out.push_str(&format!(
            "| {} | {} | {:.0} | {} | {} |\n",
            role.kind.label(),
            role.count,
            role.pnl_per_capita,
            role.losers,
            role.bankrupt,
        ));
    }
    match r.fairness_spread() {
        Some(s) => out.push_str(&format!("\nAdalet makası: **{s:.1}×** (hedef <3×)\n\n")),
        None => out.push_str("\nAdalet makası ölçülemedi — en fakir kâr rolü zararda.\n\n"),
    }

    // ── Para arzı ───────────────────────────────────────────────────────────
    let m = &r.money;
    out.push_str("## Para arzı\n\n");
    out.push_str(&format!(
        "{:.0}₺ → {:.0}₺ (**{:+.1}%**)\n\n",
        lira(m.supply_start),
        lira(m.supply_end),
        m.supply_change_pct(),
    ));
    out.push_str(&format!(
        "- maaş + ücret: {:.0}₺\n- sermaye harcaması: {:.0}₺\n- kredi anaparası: {:.0}₺\n\n",
        lira(m.salary_paid),
        lira(m.capex),
        lira(m.loan_principal),
    ));

    // ── Üretim marjı ────────────────────────────────────────────────────────
    if !r.margins.is_empty() {
        out.push_str("## Üretim marjı\n\n");
        out.push_str("Fiyatın ne kadarı kâr: (fiyat − maliyet) ÷ fiyat. Hedef >%30. ");
        out.push_str("Sıfıra yaklaşan ürün üretilemez hale gelir.\n\n");
        for (p, pct) in &r.margins {
            out.push_str(&format!("- {}: %{:.0}\n", p.display_name(), pct));
        }
        out.push('\n');
    }

    // ── Arz / talep ─────────────────────────────────────────────────────────
    out.push_str("## Arz / talep\n\n");
    out.push_str("Mutlak sayılar TTL boyunca tekrar sayılır — **oran** güvenilir, ");
    out.push_str("\"arz satıl\" alt sınırdır.\n\n");
    out.push_str("| ürün | tal/arz | eşleşen | fiyat oluşan pass |\n");
    out.push_str("|---|---:|---:|---:|\n");
    for (p, f) in &r.market {
        out.push_str(&format!(
            "| {} | {:.1}× | {} | %{:.0} |\n",
            p.display_name(),
            f.demand_supply_ratio(),
            f.matched,
            f.priced_rate() * 100.0,
        ));
    }
    out.push('\n');

    // ── Üretim tıkanıklığı ──────────────────────────────────────────────────
    let attempts = r.production_started + r.factory_idle_ticks;
    if attempts > 0 {
        out.push_str(&format!(
            "## Üretim\n\n{} batch başladı, {} tamamlandı. Fabrika başlatmak isteyip \
             girdi bulamadı: denemelerin **%{:.0}**'i.\n\n",
            r.production_started,
            r.production_completed,
            r.factory_idle_ticks as f64 / attempts as f64 * 100.0,
        ));
    }

    // ── Ürün defteri ────────────────────────────────────────────────────────
    if !r.product_flow.is_empty() {
        out.push_str("## Ürün defteri — kim alıyor, kim satıyor\n\n");
        out.push_str("| ürün | eşleşen | alan | satan |\n|---|---:|---|---|\n");
        for pl in &r.product_flow {
            let fmt = |v: &[(moneywar_domain::NpcKind, u64)]| -> String {
                let total: u64 = v.iter().map(|(_, n)| *n).sum();
                if total == 0 {
                    return "—".into();
                }
                v.iter()
                    .take(3)
                    .map(|(k, n)| format!("{} %{:.0}", k.label(), *n as f64 / total as f64 * 100.0))
                    .collect::<Vec<_>>()
                    .join(" · ")
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                pl.product.display_name(),
                pl.matched,
                fmt(&pl.buyers),
                fmt(&pl.sellers),
            ));
        }
        out.push('\n');
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::SimDriver;

    fn finished_season() -> CompletedSeason {
        // Kısa sezon — dönüş anında tamamlanan raporu verir.
        let mut d = SimDriver::new(crate::DEFAULT_SEED, 6, 3, crate::DIFFICULTY);
        loop {
            if let Some(done) = d.step() {
                return done;
            }
        }
    }

    #[test]
    fn markdown_contains_the_headline_tables() {
        let done = finished_season();
        let md = render_markdown(done.season, &done.report);
        for heading in ["Rol adaleti", "Para arzı", "Arz / talep"] {
            assert!(md.contains(heading), "eksik başlık: {heading}");
        }
    }

    #[test]
    fn archive_writes_both_formats() {
        let dir = std::env::temp_dir().join(format!("mw-arch-{}", std::process::id()));
        let a = SeasonArchive::new(&dir).expect("geçici dizin açılmalı");
        let done = finished_season();
        a.write(&done);

        let base = format!("season-{:05}", done.season);
        assert!(dir.join(format!("{base}.json")).is_file(), "JSON yazılmalı");
        assert!(dir.join(format!("{base}.md")).is_file(), "MD yazılmalı");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwritable_directory_does_not_panic() {
        // Disk sorunu sezonu düşürmemeli.
        assert!(SeasonArchive::new("/proc/moneywar-yazilamaz").is_none());
    }
}
