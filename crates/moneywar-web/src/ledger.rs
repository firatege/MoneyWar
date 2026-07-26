//! İşlem defteri — son N eşleşmenin ve üretimin halka tamponu.
//!
//! # Neden gerekli
//!
//! `GameState` "şu an" tutar: kimin nesi var, hangi fabrika ne üretiyor.
//! Ama izleyicinin sorduğu sorular geçmişe bakar — "bu firma en son ne
//! sattı", "bu şehirde kim kimle iş yapıyor", "bu fabrika kaç tick'tir
//! üretiyor". `relationships` bunun sadece toplamını taşır; hangi şehirde,
//! hangi üründe, ne fiyata olduğunu değil.
//!
//! Ham log bu soruyu cevaplar ama sezonda ~48 MB tutar. Burada sabit
//! kapasiteli halka tampon var: son `TRADE_CAP` eşleşme + son
//! `PRODUCTION_CAP` üretim. Sezonun tamamı değil, son birkaç yüz tick'i —
//! drill-down sayfalarının ihtiyacı bu.

use std::collections::VecDeque;

use moneywar_domain::{CityId, FactoryId, Money, PlayerId, ProductKind, Tick};
use moneywar_engine::{LogEvent, TickReport};

/// Tutulan eşleşme sayısı. ~700 tick'lik pazar hareketi; 5 şehir × 12 ürün
/// için firma başına anlamlı bir geçmiş bırakır, bellekte ~500 KB tutar.
const TRADE_CAP: usize = 8_000;

/// Tutulan üretim kaydı sayısı. Üretim eşleşmeden seyrek olduğu için daha az.
const PRODUCTION_CAP: usize = 4_000;

/// Gerçekleşmiş tek bir pazar eşleşmesi.
#[derive(Debug, Clone, Copy)]
pub struct Trade {
    pub tick: Tick,
    pub city: CityId,
    pub product: ProductKind,
    pub quantity: u32,
    pub price: Money,
    pub buyer: PlayerId,
    pub seller: PlayerId,
}

impl Trade {
    /// İşlemin toplam tutarı (adet × birim fiyat).
    #[must_use]
    pub fn value(&self) -> Money {
        Money::from_cents(
            self.price
                .as_cents()
                .saturating_mul(i64::from(self.quantity)),
        )
    }

    /// İşlemin tarafı mı?
    #[must_use]
    pub fn involves(&self, player: PlayerId) -> bool {
        self.buyer == player || self.seller == player
    }
}

/// Tamamlanmış tek bir üretim batch'i.
#[derive(Debug, Clone, Copy)]
pub struct Production {
    pub tick: Tick,
    /// Fabrikanın sahibi. Olay bu alanı taşımaz — `LogEntry.actor` taşır.
    pub owner: PlayerId,
    pub factory: FactoryId,
    pub city: CityId,
    pub product: ProductKind,
    pub units: u32,
}

/// Son işlemlerin ve üretimlerin halka tamponu.
#[derive(Debug, Default)]
pub struct Ledger {
    trades: VecDeque<Trade>,
    productions: VecDeque<Production>,
}

impl Ledger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bir tick'in raporundaki eşleşme ve üretimleri deftere yazar.
    pub fn ingest(&mut self, report: &TickReport) {
        for entry in &report.entries {
            match &entry.event {
                LogEvent::OrderMatched {
                    city,
                    product,
                    quantity,
                    price,
                    buyer,
                    seller,
                    ..
                } => self.push_trade(Trade {
                    tick: report.tick,
                    city: *city,
                    product: *product,
                    quantity: *quantity,
                    price: *price,
                    buyer: *buyer,
                    seller: *seller,
                }),
                LogEvent::ProductionCompleted {
                    factory_id,
                    city,
                    product,
                    units,
                } => {
                    // Sahip olayda değil, kaydın aktöründe. Aktörsüz üretim
                    // olmamalı; olursa kaydı atlamak sessiz kayıp değil —
                    // fabrika sahibi bilinmeden satır kime yazılacağı yok.
                    if let Some(owner) = entry.actor {
                        self.push_production(Production {
                            tick: report.tick,
                            owner,
                            factory: *factory_id,
                            city: *city,
                            product: *product,
                            units: *units,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn push_trade(&mut self, t: Trade) {
        if self.trades.len() == TRADE_CAP {
            self.trades.pop_front();
        }
        self.trades.push_back(t);
    }

    fn push_production(&mut self, p: Production) {
        if self.productions.len() == PRODUCTION_CAP {
            self.productions.pop_front();
        }
        self.productions.push_back(p);
    }

    /// Sezon değişiminde defter sıfırlanır — geçen sezonun işlemleri yeni
    /// sezonun firmalarına ait değil.
    pub fn clear(&mut self) {
        self.trades.clear();
        self.productions.clear();
    }

    /// En eskiden en yeniye işlemler.
    pub fn trades(&self) -> impl DoubleEndedIterator<Item = &Trade> {
        self.trades.iter()
    }

    /// En eskiden en yeniye üretimler.
    pub fn productions(&self) -> impl DoubleEndedIterator<Item = &Production> {
        self.productions.iter()
    }

    /// Defterin kapsadığı en eski tick — "bu istatistik ne kadar geriyi
    /// görüyor" sorusunun cevabı. Boşsa `None`.
    #[must_use]
    pub fn earliest_tick(&self) -> Option<Tick> {
        let t = self.trades.front().map(|t| t.tick);
        let p = self.productions.front().map(|p| p.tick);
        match (t, p) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::OrderId;
    use moneywar_engine::LogEntry;

    fn matched(tick: u32, qty: u32) -> TickReport {
        let t = Tick::new(tick);
        let mut r = TickReport::new(t);
        r.push(LogEntry::order_matched(
            t,
            CityId::Istanbul,
            ProductKind::Bugday,
            OrderId::new(1),
            OrderId::new(2),
            PlayerId::new(1),
            PlayerId::new(2),
            qty,
            Money::from_cents(1_000),
        ));
        r
    }

    fn produced(tick: u32, units: u32) -> TickReport {
        let t = Tick::new(tick);
        let mut r = TickReport::new(t);
        r.push(LogEntry::production_completed(
            t,
            PlayerId::new(7),
            FactoryId::new(3),
            CityId::Istanbul,
            ProductKind::Un,
            units,
        ));
        r
    }

    #[test]
    fn production_owner_comes_from_the_entry_actor() {
        let mut l = Ledger::new();
        l.ingest(&produced(4, 12));
        let p = *l.productions().next().expect("üretim kaydı");
        assert_eq!(p.owner, PlayerId::new(7));
        assert_eq!(p.factory, FactoryId::new(3));
        assert_eq!(p.units, 12);
    }

    #[test]
    fn ingest_records_matches() {
        let mut l = Ledger::new();
        l.ingest(&matched(1, 5));
        let all: Vec<_> = l.trades().collect();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].quantity, 5);
        assert_eq!(all[0].value(), Money::from_cents(5_000));
    }

    #[test]
    fn ring_buffer_drops_the_oldest_not_the_newest() {
        let mut l = Ledger::new();
        for i in 0..(TRADE_CAP + 10) {
            l.ingest(&matched(u32::try_from(i).unwrap(), 1));
        }
        assert_eq!(l.trades().count(), TRADE_CAP, "kapasite aşılmamalı");
        let first = l.trades().next().expect("dolu defter");
        assert_eq!(first.tick.value(), 10, "en eski atılmalı, en yeni değil");
    }

    #[test]
    fn clear_resets_the_season() {
        let mut l = Ledger::new();
        l.ingest(&matched(1, 5));
        l.clear();
        assert_eq!(l.trades().count(), 0);
        assert_eq!(l.earliest_tick(), None);
    }

    #[test]
    fn involves_matches_both_sides() {
        let mut l = Ledger::new();
        l.ingest(&matched(1, 5));
        let t = *l.trades().next().unwrap();
        assert!(t.involves(PlayerId::new(1)));
        assert!(t.involves(PlayerId::new(2)));
        assert!(!t.involves(PlayerId::new(3)));
    }
}
