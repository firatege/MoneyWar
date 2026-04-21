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

use moneywar_domain::{
    CaravanId, CityId, Command, ContractId, ContractState, FactoryId, ListingKind, LoanId, Money,
    OrderId, PlayerId, ProductKind, Tick,
};
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

    /// Yeni fabrika kuruldu.
    #[must_use]
    pub fn factory_built(
        tick: Tick,
        owner: PlayerId,
        factory_id: FactoryId,
        city: CityId,
        product: ProductKind,
        cost: Money,
    ) -> Self {
        Self {
            tick,
            actor: Some(owner),
            event: LogEvent::FactoryBuilt {
                factory_id,
                owner,
                city,
                product,
                cost,
            },
        }
    }

    /// Fabrika yeni bir batch başlattı (ham madde tüketildi).
    #[must_use]
    pub fn production_started(
        tick: Tick,
        owner: PlayerId,
        factory_id: FactoryId,
        city: CityId,
        product: ProductKind,
        units: u32,
        completion_tick: Tick,
    ) -> Self {
        Self {
            tick,
            actor: Some(owner),
            event: LogEvent::ProductionStarted {
                factory_id,
                city,
                product,
                units,
                completion_tick,
            },
        }
    }

    /// Fabrika batch'i tamamlandı, envantere eklendi.
    #[must_use]
    pub fn production_completed(
        tick: Tick,
        owner: PlayerId,
        factory_id: FactoryId,
        city: CityId,
        product: ProductKind,
        units: u32,
    ) -> Self {
        Self {
            tick,
            actor: Some(owner),
            event: LogEvent::ProductionCompleted {
                factory_id,
                city,
                product,
                units,
            },
        }
    }

    /// Yeni kervan satın alındı.
    #[must_use]
    pub fn caravan_bought(
        tick: Tick,
        owner: PlayerId,
        caravan_id: CaravanId,
        starting_city: CityId,
        capacity: u32,
        cost: Money,
    ) -> Self {
        Self {
            tick,
            actor: Some(owner),
            event: LogEvent::CaravanBought {
                caravan_id,
                owner,
                starting_city,
                capacity,
                cost,
            },
        }
    }

    /// Kervan yola çıktı — cargo envanter'den çıkarıldı, `EnRoute` state'ine geçti.
    #[must_use]
    pub fn caravan_dispatched(
        tick: Tick,
        owner: PlayerId,
        caravan_id: CaravanId,
        from: CityId,
        to: CityId,
        arrival_tick: Tick,
        cargo_total: u64,
    ) -> Self {
        Self {
            tick,
            actor: Some(owner),
            event: LogEvent::CaravanDispatched {
                caravan_id,
                from,
                to,
                arrival_tick,
                cargo_total,
            },
        }
    }

    /// Yeni kontrat önerildi, satıcı kaporası escrow'a kilitlendi.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn contract_proposed(
        tick: Tick,
        seller: PlayerId,
        contract_id: ContractId,
        listing: ListingKind,
        product: ProductKind,
        quantity: u32,
        unit_price: Money,
        delivery_city: CityId,
        delivery_tick: Tick,
        seller_deposit: Money,
        buyer_deposit: Money,
    ) -> Self {
        Self {
            tick,
            actor: Some(seller),
            event: LogEvent::ContractProposed {
                contract_id,
                seller,
                listing,
                product,
                quantity,
                unit_price,
                delivery_city,
                delivery_tick,
                seller_deposit,
                buyer_deposit,
            },
        }
    }

    /// Kontrat kabul edildi, alıcı kaporası escrow'a kilitlendi → Active.
    #[must_use]
    pub fn contract_accepted(
        tick: Tick,
        acceptor: PlayerId,
        contract_id: ContractId,
        buyer_deposit: Money,
    ) -> Self {
        Self {
            tick,
            actor: Some(acceptor),
            event: LogEvent::ContractAccepted {
                contract_id,
                acceptor,
                buyer_deposit,
            },
        }
    }

    /// Kontrat önerisi geri çekildi (sadece `Proposed` state'te).
    /// Satıcı kaporası iade edildi.
    #[must_use]
    pub fn contract_cancelled(
        tick: Tick,
        seller: PlayerId,
        contract_id: ContractId,
        refunded_deposit: Money,
    ) -> Self {
        Self {
            tick,
            actor: Some(seller),
            event: LogEvent::ContractCancelled {
                contract_id,
                seller,
                refunded_deposit,
            },
        }
    }

    /// NPC bankasından kredi alındı — principal borçlunun cash'ine eklendi.
    #[must_use]
    pub fn loan_taken(
        tick: Tick,
        borrower: PlayerId,
        loan_id: LoanId,
        principal: Money,
        interest_rate_percent: u32,
        due_tick: Tick,
        total_due: Money,
    ) -> Self {
        Self {
            tick,
            actor: Some(borrower),
            event: LogEvent::LoanTaken {
                loan_id,
                borrower,
                principal,
                interest_rate_percent,
                due_tick,
                total_due,
            },
        }
    }

    /// Kredi ödendi — `on_time` false ise vadeyi geçti ama motor çekebildi.
    #[must_use]
    pub fn loan_repaid(
        tick: Tick,
        borrower: PlayerId,
        loan_id: LoanId,
        amount_paid: Money,
        on_time: bool,
    ) -> Self {
        Self {
            tick,
            actor: Some(borrower),
            event: LogEvent::LoanRepaid {
                loan_id,
                borrower,
                amount_paid,
                on_time,
            },
        }
    }

    /// Kredi default — vade geçti, borçlu yeterli nakit bulamadı.
    /// Mevcut tüm nakti çekildi, kalan borç banka tarafından silindi.
    #[must_use]
    pub fn loan_defaulted(
        tick: Tick,
        borrower: PlayerId,
        loan_id: LoanId,
        seized: Money,
        unpaid_balance: Money,
    ) -> Self {
        Self {
            tick,
            actor: Some(borrower),
            event: LogEvent::LoanDefaulted {
                loan_id,
                borrower,
                seized,
                unpaid_balance,
            },
        }
    }

    /// Kontrat durum geçişi (5B — delivery settlement).
    #[must_use]
    pub fn contract_settled(
        tick: Tick,
        contract_id: ContractId,
        final_state: ContractState,
    ) -> Self {
        Self {
            tick,
            actor: None,
            event: LogEvent::ContractSettled {
                contract_id,
                final_state,
            },
        }
    }

    /// Kervan vardı — cargo hedef şehir envanterine yatırıldı, `Idle` oldu.
    #[must_use]
    pub fn caravan_arrived(
        tick: Tick,
        owner: PlayerId,
        caravan_id: CaravanId,
        city: CityId,
        cargo_total: u64,
    ) -> Self {
        Self {
            tick,
            actor: Some(owner),
            event: LogEvent::CaravanArrived {
                caravan_id,
                city,
                cargo_total,
            },
        }
    }

    /// Fabrika bu tick atıl kaldı (ham madde yok, üretim başlamadı).
    #[must_use]
    pub fn factory_idle(
        tick: Tick,
        owner: PlayerId,
        factory_id: FactoryId,
        city: CityId,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            tick,
            actor: Some(owner),
            event: LogEvent::FactoryIdle {
                factory_id,
                city,
                reason: reason.into(),
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

    /// Oyuncu yeni fabrika kurdu. Sanayici tekeli.
    FactoryBuilt {
        factory_id: FactoryId,
        owner: PlayerId,
        city: CityId,
        product: ProductKind,
        cost: Money,
    },

    /// Fabrika bu tick yeni batch başlattı (ham madde consume + timer start).
    ProductionStarted {
        factory_id: FactoryId,
        city: CityId,
        product: ProductKind,
        units: u32,
        completion_tick: Tick,
    },

    /// Fabrika batch'i tamamlandı; bitmiş ürün sahibinin envanterine eklendi.
    ProductionCompleted {
        factory_id: FactoryId,
        city: CityId,
        product: ProductKind,
        units: u32,
    },

    /// Fabrika bu tick atıl kaldı — çoğunlukla ham madde envanter'de yoktu.
    /// §9 skor formülünde "10 tick atıl" fabrika değeri sıfıra düşer.
    FactoryIdle {
        factory_id: FactoryId,
        city: CityId,
        reason: String,
    },

    /// Oyuncu yeni kervan satın aldı. Kapasite ve maliyet role'e bağlıdır (§10).
    CaravanBought {
        caravan_id: CaravanId,
        owner: PlayerId,
        starting_city: CityId,
        capacity: u32,
        cost: Money,
    },

    /// Kervan yola çıktı. `arrival_tick` deterministik varış zamanı (§4:
    /// kayıp yok, süre riski var). `cargo_total` taşınan toplam birim.
    CaravanDispatched {
        caravan_id: CaravanId,
        from: CityId,
        to: CityId,
        arrival_tick: Tick,
        cargo_total: u64,
    },

    /// Kervan vardı — cargo hedef şehirde sahibin envanterine eklendi.
    CaravanArrived {
        caravan_id: CaravanId,
        city: CityId,
        cargo_total: u64,
    },

    /// Yeni kontrat önerildi, satıcı kaporası escrow'a kilitlendi (§2 Katman 2).
    ContractProposed {
        contract_id: ContractId,
        seller: PlayerId,
        listing: ListingKind,
        product: ProductKind,
        quantity: u32,
        unit_price: Money,
        delivery_city: CityId,
        delivery_tick: Tick,
        seller_deposit: Money,
        buyer_deposit: Money,
    },

    /// Kontrat kabul edildi → `Active`. Alıcı kaporası da escrow'da artık.
    ContractAccepted {
        contract_id: ContractId,
        acceptor: PlayerId,
        buyer_deposit: Money,
    },

    /// Kontrat önerisi satıcı tarafından geri çekildi (yalnız `Proposed`'ta).
    ContractCancelled {
        contract_id: ContractId,
        seller: PlayerId,
        refunded_deposit: Money,
    },

    /// Teslimat tick'inde kontrat kapandı — `Fulfilled` veya `Breached`.
    /// Breach'te breacher'ın kaporası karşı tarafa tazminat olarak gider.
    ContractSettled {
        contract_id: ContractId,
        final_state: ContractState,
    },

    /// NPC bankasından kredi alındı. Bu event sistem-dışı (oyun bankası)
    /// para yaratır; money conservation oyun içi transferler için geçerli,
    /// banka ile toplamda değil.
    LoanTaken {
        loan_id: LoanId,
        borrower: PlayerId,
        principal: Money,
        interest_rate_percent: u32,
        due_tick: Tick,
        total_due: Money,
    },

    /// Kredi tam olarak geri ödendi. `on_time=false` → motor vadeden sonra
    /// otomatik çekti ama nakit yetti.
    LoanRepaid {
        loan_id: LoanId,
        borrower: PlayerId,
        amount_paid: Money,
        on_time: bool,
    },

    /// Kredi default — motor borçlunun tüm nakdini çekti, kalanı silindi.
    LoanDefaulted {
        loan_id: LoanId,
        borrower: PlayerId,
        seized: Money,
        unpaid_balance: Money,
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
