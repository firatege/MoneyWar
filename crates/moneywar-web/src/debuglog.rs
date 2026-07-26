//! Tick-by-tick ekonomi debug log'u — sim runner ile aynı format.
//!
//! Her tick: event akışı + tick özeti + 30 bucket snapshot satırı. Bellekteki
//! halka tampona (ring buffer) biriktirilir; `GET /api/log` ile okunur.
//! İsteğe bağlı olarak dosyaya da yazılır (`MONEYWAR_LOG` / `/app/debug`).

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use moneywar_domain::{CityId, GameState, ProductKind};
use moneywar_engine::{LogEvent, TickReport};

/// Bellekte tutulan max satır (yaklaşık; aşılınca en eskiler düşer).
const RING_CAP: usize = 12_000;

/// Log toplayıcı — ring buffer + opsiyonel dosya.
#[derive(Debug)]
pub struct LogSink {
    ring: Mutex<VecDeque<String>>,
    file: Mutex<Option<BufWriter<File>>>,
}

impl LogSink {
    #[must_use]
    pub fn new(file_path: Option<PathBuf>) -> Self {
        let file = file_path.and_then(|p| File::create(p).map(BufWriter::new).ok());
        Self {
            ring: Mutex::new(VecDeque::with_capacity(RING_CAP)),
            file: Mutex::new(file),
        }
    }

    /// Çok satırlı bir blok ekle (her satır ayrı kaydedilir).
    pub fn push_block(&self, block: &str) {
        if let Ok(mut ring) = self.ring.lock() {
            for line in block.lines() {
                if ring.len() >= RING_CAP {
                    ring.pop_front();
                }
                ring.push_back(line.to_string());
            }
        }
        if let Ok(mut f) = self.file.lock()
            && let Some(w) = f.as_mut()
        {
            let _ = w.write_all(block.as_bytes());
            let _ = w.write_all(b"\n");
            let _ = w.flush();
        }
    }

    /// Son `n` satırı düz metin olarak döndür (n=0 → tümü).
    #[must_use]
    pub fn tail(&self, n: usize) -> String {
        let ring = match self.ring.lock() {
            Ok(r) => r,
            Err(_) => return String::new(),
        };
        let start = if n == 0 || n >= ring.len() {
            0
        } else {
            ring.len() - n
        };
        ring.iter()
            .skip(start)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Bir tick'in debug bloğunu üret (sim `write_tick_debug` ile aynı format).
#[must_use]
pub fn format_tick_block(state: &GameState, report: &TickReport, season: u64) -> String {
    let mut out = String::new();
    let tick = state.current_tick.value();

    // Event akışı.
    for entry in &report.entries {
        let actor = entry
            .actor
            .map(|a| {
                state
                    .players
                    .get(&a)
                    .map_or_else(|| format!("#{}", a.value()), |p| p.name.clone())
            })
            .unwrap_or_else(|| "system".into());
        out.push_str(&format!(
            "s{season} t{:>3}  {actor:<22}  {:?}\n",
            entry.tick.value(),
            entry.event
        ));
    }

    // Tick özeti.
    let mut matched_cnt = 0u32;
    let mut matched_qty = 0u32;
    let mut expired_cnt = 0u32;
    let mut rejected_cnt = 0u32;
    let mut fill_rejected_cnt = 0u32;
    for entry in &report.entries {
        match &entry.event {
            LogEvent::OrderMatched { quantity, .. } => {
                matched_cnt += 1;
                matched_qty += quantity;
            }
            LogEvent::OrderExpired { .. } => expired_cnt += 1,
            LogEvent::CommandRejected { .. } => rejected_cnt += 1,
            LogEvent::FillRejected { .. } => fill_rejected_cnt += 1,
            _ => {}
        }
    }
    out.push_str(&format!(
        "s{season} t{tick:>3}  <tick-summary>           matched={matched_qty}u/{matched_cnt}fill expired={expired_cnt} cmd_reject={rejected_cnt} fill_reject={fill_rejected_cnt}\n"
    ));

    // Bucket snapshot.
    out.push_str(&format!(
        "s{season} t{tick:>3}  <bucket-header>          bucket               last      avg5      base      bid       ask       spread  buy_q  sell_q\n"
    ));
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let last = state
                .price_history
                .get(&(city, product))
                .and_then(|h| h.last().map(|(_, p)| format!("{p:>8}")))
                .unwrap_or_else(|| "       -".into());
            let avg5 = state
                .rolling_avg_price(city, product, 5)
                .map_or_else(|| "       -".into(), |p| format!("{p:>8}"));
            let base = state
                .price_baseline
                .get(&(city, product))
                .map_or_else(|| "       -".into(), |p| format!("{p:>8}"));
            let bid = state
                .best_bid(city, product)
                .map_or_else(|| "       -".into(), |(p, _)| format!("{p:>8}"));
            let ask = state
                .best_ask(city, product)
                .map_or_else(|| "       -".into(), |(p, _)| format!("{p:>8}"));
            let spread = match (state.best_bid(city, product), state.best_ask(city, product)) {
                (Some((b, _)), Some((a, _))) => {
                    format!("{:>6}", a.as_cents().saturating_sub(b.as_cents()))
                }
                _ => "     -".into(),
            };
            let (buy_q, sell_q) = state
                .order_book
                .get(&(city, product))
                .map_or((0, 0), |book| {
                    let bq: u32 = book
                        .iter()
                        .filter(|o| o.side.is_buy())
                        .map(|o| o.quantity)
                        .sum();
                    let sq: u32 = book
                        .iter()
                        .filter(|o| o.side.is_sell())
                        .map(|o| o.quantity)
                        .sum();
                    (bq, sq)
                });
            let bucket_label = format!("{}/{}", city.display_name(), product.display_name());
            out.push_str(&format!(
                "s{season} t{tick:>3}  <bucket>                 {bucket_label:<20} {last} {avg5} {base} {bid} {ask} {spread} {buy_q:>6} {sell_q:>6}\n"
            ));
        }
    }

    out
}
