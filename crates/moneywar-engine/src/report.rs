//! Tick çıktı raporu — analitik + audit için.
//!
//! `advance_tick` her çağrıda yeni state'in yanında bir `TickReport` döndürür.
//! Rapor:
//! - Motor içinde tüketilmez — server Faz 10'da DB'ye yazar (journal tablosu).
//! - Saf veri: I/O yok, global state yok.
//! - İleri fazlarda yeni `LogEvent` variant'ları ile genişler (order match,
//!   production complete, caravan arrived, event trigger, vb).
//!
//! Analitik kullanım: SQL `WHERE event_kind = 'CommandRejected'` ile en çok
//! hangi komutun reddedildiğini ölçebilirsin, rol bazlı `PnL` çıkarabilirsin.

use moneywar_domain::{CityId, Command, Money, OrderId, PlayerId, ProductKind, Tick};
use serde::{Deserialize, Serialize};

/// Bir tick boyunca motor tarafından üretilmiş tüm gözlemler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickReport {
    pub tick: Tick,
    pub entries: Vec<LogEntry>,
}

impl TickReport {
    /// Verilen tick için boş rapor.
    #[must_use]
    pub fn new(tick: Tick) -> Self {
        Self {
            tick,
            entries: Vec::new(),
        }
    }

    /// Bu rapora tek bir log entry ekler.
    pub fn push(&mut self, entry: LogEntry) {
        self.entries.push(entry);
    }

    /// Kabul edilmiş entry sayısı (analitik kolaylık).
    #[must_use]
    pub fn accepted_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e.event, LogEvent::CommandAccepted { .. }))
            .count()
    }

    /// Reddedilmiş entry sayısı.
    #[must_use]
    pub fn rejected_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e.event, LogEvent::CommandRejected { .. }))
            .count()
    }
}

/// Motorun ürettiği atomik log kaydı.
///
/// Her kayıt şu soruları cevaplar: **ne zaman** (tick), **kim** (actor),
/// **ne oldu** (event). Server bu kayıtları append-only bir tabloya yazar;
/// replay, audit ve balance tuning analizi bu tablodan koşar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub tick: Tick,
    /// Olayı tetikleyen oyuncu. `None` → sistem eventi (henüz yok, Faz 6+
    /// haber/olay trigger'ları için ayrılmış).
    pub actor: Option<PlayerId>,
    pub event: LogEvent,
}

impl LogEntry {
    #[must_use]
    pub fn command_accepted(tick: Tick, actor: PlayerId, command: Command) -> Self {
        Self {
            tick,
            actor: Some(actor),
            event: LogEvent::CommandAccepted { command },
        }
    }

    pub fn command_rejected(
        tick: Tick,
        actor: PlayerId,
        command: Command,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tick,
            actor: Some(actor),
            event: LogEvent::CommandRejected {
                command,
                reason: reason.into(),
            },
        }
    }

    /// Tek bir eşleşme (iki emrin kesişimi). Sistem eventi → `actor = None`.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn order_matched(
        tick: Tick,
        city: CityId,
        product: ProductKind,
        buy_order_id: OrderId,
        sell_order_id: OrderId,
        buyer: PlayerId,
        seller: PlayerId,
        quantity: u32,
        price: Money,
    ) -> Self {
        Self {
            tick,
            actor: None,
            event: LogEvent::OrderMatched {
                city,
                product,
                buy_order_id,
                sell_order_id,
                buyer,
                seller,
                quantity,
                price,
            },
        }
    }

    /// Bir `(city, product)` bucket'ının temizlenme özeti. `clearing_price`
    /// `None` → hiç eşleşme yok (spread), tüm emirler çöpe atıldı.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn market_cleared(
        tick: Tick,
        city: CityId,
        product: ProductKind,
        clearing_price: Option<Money>,
        matched_qty: u32,
        submitted_buy_qty: u32,
        submitted_sell_qty: u32,
        saturation_threshold: u32,
        saturation_qty: u32,
    ) -> Self {
        Self {
            tick,
            actor: None,
            event: LogEvent::MarketCleared {
                city,
                product,
                clearing_price,
                matched_qty,
                submitted_buy_qty,
                submitted_sell_qty,
                saturation_threshold,
                saturation_qty,
            },
        }
    }

    /// Bir fill settle edilemedi (buyer cash yetmiyor veya seller stok yetmiyor).
    /// State dokunulmaz, para korunumu ihlal edilmez. Sadece log'a kayıt.
    #[must_use]
    pub fn fill_rejected(
        tick: Tick,
        city: CityId,
        product: ProductKind,
        buyer: PlayerId,
        seller: PlayerId,
        quantity: u32,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tick,
            actor: None,
            event: LogEvent::FillRejected {
                city,
                product,
                buyer,
                seller,
                quantity,
                reason: reason.into(),
            },
        }
    }
}

/// Motor'un ürettiği semantik event'ler.
///
/// Faz 2 iskeletinde sadece komut dispatch sonuçları var. Faz 3-8 arası
/// domain event'leri eklenecek:
/// - `OrderMatched { buyer, seller, product, qty, price }`
/// - `ProductionCompleted { factory, product, qty }`
/// - `CaravanArrived { caravan, city, cargo }`
/// - `ContractSettled { contract, outcome }`
/// - `NewsPublished { tier, item }`
/// - `LoanTaken` / `LoanRepaid` (Faz 5.5)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LogEvent {
    /// Oyuncu komutu motor tarafından uygulandı.
    CommandAccepted { command: Command },

    /// Oyuncu komutu reddedildi — reason insan okunur string (validation,
    /// insufficient funds, capacity, vb).
    CommandRejected { command: Command, reason: String },

    /// Batch auction'da iki emir eşleşti. `price` = uniform clearing fiyatı
    /// (`(marjinal_buy + marjinal_sell) / 2` midpoint). Faz 3C'de bu eventler
    /// settlement (cash/inventory) için okunacak.
    OrderMatched {
        city: CityId,
        product: ProductKind,
        buy_order_id: OrderId,
        sell_order_id: OrderId,
        buyer: PlayerId,
        seller: PlayerId,
        quantity: u32,
        price: Money,
    },

    /// Bir `(city, product)` pazarının tick kapanış özeti. `clearing_price`
    /// `None` → eşleşme yok (spread). `matched_qty` total değişim, `submitted_*`
    /// iptal sonrası tick'e giren toplam arz/talep (analitik için).
    /// `saturation_threshold` = oyuncu sayısından türeyen şehir soğurma kapasitesi
    /// (§10). `saturation_qty` = eşiği aştığı için `clearing_price / 2`'de
    /// settle edilmiş birim sayısı.
    MarketCleared {
        city: CityId,
        product: ProductKind,
        clearing_price: Option<Money>,
        matched_qty: u32,
        submitted_buy_qty: u32,
        submitted_sell_qty: u32,
        saturation_threshold: u32,
        saturation_qty: u32,
    },

    /// Settlement aşamasında bir fill uygulanamadı (cash/stok yetmez, overflow).
    /// State değişmez, para korunumu korunur. Fill analitik için hala kayıtlı.
    FillRejected {
        city: CityId,
        product: ProductKind,
        buyer: PlayerId,
        seller: PlayerId,
        quantity: u32,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CityId, MarketOrder, Money, OrderId, OrderSide, PlayerId, ProductKind, RoomId, Tick,
    };

    fn sample_command() -> Command {
        Command::SubmitOrder(
            MarketOrder::new(
                OrderId::new(1),
                PlayerId::new(7),
                CityId::Istanbul,
                ProductKind::Pamuk,
                OrderSide::Buy,
                10,
                Money::from_lira(5).unwrap(),
                Tick::new(1),
            )
            .unwrap(),
        )
    }

    #[test]
    fn new_report_starts_empty() {
        let r = TickReport::new(Tick::new(5));
        assert_eq!(r.tick, Tick::new(5));
        assert!(r.entries.is_empty());
        assert_eq!(r.accepted_count(), 0);
        assert_eq!(r.rejected_count(), 0);
    }

    #[test]
    fn accepted_entry_counts_in_accepted_only() {
        let mut r = TickReport::new(Tick::new(1));
        r.push(LogEntry::command_accepted(
            Tick::new(1),
            PlayerId::new(7),
            sample_command(),
        ));
        assert_eq!(r.accepted_count(), 1);
        assert_eq!(r.rejected_count(), 0);
    }

    #[test]
    fn rejected_entry_counts_in_rejected_only() {
        let mut r = TickReport::new(Tick::new(1));
        r.push(LogEntry::command_rejected(
            Tick::new(1),
            PlayerId::new(7),
            sample_command(),
            "insufficient funds",
        ));
        assert_eq!(r.accepted_count(), 0);
        assert_eq!(r.rejected_count(), 1);
    }

    #[test]
    fn entry_preserves_actor_and_event() {
        let cmd = sample_command();
        let entry = LogEntry::command_accepted(Tick::new(3), PlayerId::new(42), cmd.clone());
        assert_eq!(entry.tick, Tick::new(3));
        assert_eq!(entry.actor, Some(PlayerId::new(42)));
        assert!(matches!(entry.event, LogEvent::CommandAccepted { .. }));
    }

    #[test]
    fn rejected_event_carries_reason() {
        let entry = LogEntry::command_rejected(
            Tick::new(1),
            PlayerId::new(1),
            sample_command(),
            "bad state",
        );
        match entry.event {
            LogEvent::CommandRejected { reason, .. } => assert_eq!(reason, "bad state"),
            other => panic!("expected rejected, got {other:?}"),
        }
    }

    #[test]
    fn serde_roundtrip_report() {
        let mut r = TickReport::new(Tick::new(10));
        r.push(LogEntry::command_accepted(
            Tick::new(10),
            PlayerId::new(1),
            sample_command(),
        ));
        r.push(LogEntry::command_rejected(
            Tick::new(10),
            PlayerId::new(2),
            sample_command(),
            "denied",
        ));
        let back: TickReport = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn log_event_serializes_with_kind_tag() {
        // Analitik DB indexing için `kind` alanının sabit olması önemli.
        let ev = LogEvent::CommandAccepted {
            command: sample_command(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"kind\":\"command_accepted\""));
    }

    // RoomId import kaydı, ileride report'a room_id eklenirse hazır.
    #[allow(dead_code)]
    fn _room_import_marker(_: RoomId) {}
}
