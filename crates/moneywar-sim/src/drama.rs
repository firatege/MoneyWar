//! Drama skorkartı — sezonun "hikâyelik olay" sayacı (docs/finish-plan.md Faz 0).
//!
//! Eski yaklaşım ekonomi sağlığını ölçüyordu; bu modül tersini ölçer:
//! izleyiciye anlatılabilir olay (tekel, fiyat savaşı, iflas, kartel...)
//! sezon başına yeterince çıkıyor mu? Ekonomi sağlığından geriye yalnız
//! felaket frenleri kaldı (piyasa toptan ölmesin, iflas zinciri oyunu
//! bitirmesin).

use moneywar_domain::Tick;
use moneywar_engine::LogEvent;

/// Sezon başına hedeflenen minimum hikâyelik olay (docs/finish-plan.md).
pub const STORY_EVENT_TARGET: u64 = 8;
/// Felaket freni: sezon başına tolere edilen azami iflas.
///
/// Bu eşik iflas mekaniği çalışmadan önce tahminle (3) konmuştu. Ölçüldü:
/// ~35 firmalık dünyada seed'e göre 1-5 iflas çıkıyor ve piyasa her koşumda
/// canlı kalıyor — bu ölüm sarmalı değil, sağlıklı tasfiye. Fren gerçek
/// felaketi (dünyanın boşalması) yakalasın diye ~%20 kayba çekildi.
pub const MAX_BANKRUPTCIES: u64 = 8;
/// Felaket freni: sezonda eşleşmesi gereken minimum birim (piyasa ölmedi).
pub const MIN_MATCHED_QTY: u64 = 500;

/// Anlatı olaylarının tür bazında sayacı. `record` ile beslenir.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DramaScorecard {
    pub monopolies_formed: u64,
    pub monopolies_broken: u64,
    pub undercut_campaigns: u64,
    pub price_wars_declared: u64,
    pub price_wars_won: u64,
    pub bankruptcies: u64,
    pub grudges_formed: u64,
    pub supply_chokes: u64,
    pub cartels_formed: u64,
    pub cartels_betrayed: u64,
    /// Manşetlik olayların ham kaydı (tick + olay), en fazla `HEADLINE_CAP`.
    /// İzleyicinin gördüğü akışın sim tarafındaki karşılığı: kim, neyi,
    /// nerede yaptı. Sim özeti bunları isimlerle basar.
    pub headlines: Vec<(Tick, LogEvent)>,
}

/// Saklanan azami manşet — sim özetinde örneklem göstermeye yeter.
pub const HEADLINE_CAP: usize = 200;

impl DramaScorecard {
    /// Bir motor olayını işle; anlatı olayı değilse dokunma.
    pub fn record(&mut self, tick: Tick, event: &LogEvent) {
        let before = self.story_event_total();
        self.tally(event);
        // Sayaç arttıysa bu manşetlik bir olaydı.
        if self.story_event_total() > before && self.headlines.len() < HEADLINE_CAP {
            self.headlines.push((tick, event.clone()));
        }
    }

    fn tally(&mut self, event: &LogEvent) {
        match event {
            LogEvent::MonopolyFormed { .. } => self.monopolies_formed += 1,
            LogEvent::MonopolyBroken { .. } => self.monopolies_broken += 1,
            LogEvent::UndercutCampaign { .. } => self.undercut_campaigns += 1,
            LogEvent::PriceWarDeclared { .. } => self.price_wars_declared += 1,
            LogEvent::PriceWarWon { .. } => self.price_wars_won += 1,
            LogEvent::FirmBankrupt { .. } => self.bankruptcies += 1,
            LogEvent::GrudgeFormed { .. } => self.grudges_formed += 1,
            LogEvent::SupplyChoke { .. } => self.supply_chokes += 1,
            LogEvent::CartelFormed { .. } => self.cartels_formed += 1,
            LogEvent::CartelBetrayed { .. } => self.cartels_betrayed += 1,
            _ => {}
        }
    }

    /// Toplam hikâyelik olay. Kin tek başına hikâye sayılmaz (öncülü olan
    /// undercut/ihanet zaten sayıldı); iflas felaket frenine de tabi ama
    /// izleyici için olay olduğundan burada sayılır.
    #[must_use]
    pub fn story_event_total(&self) -> u64 {
        self.monopolies_formed
            + self.monopolies_broken
            + self.undercut_campaigns
            + self.price_wars_declared
            + self.price_wars_won
            + self.bankruptcies
            + self.supply_chokes
            + self.cartels_formed
            + self.cartels_betrayed
    }

    /// Skorkart + felaket frenlerinden sezon hükmü üret.
    #[must_use]
    pub fn verdict(&self, matched_qty: u64) -> Verdict {
        Verdict {
            story_events: self.story_event_total(),
            story_target_met: self.story_event_total() >= STORY_EVENT_TARGET,
            market_alive: matched_qty >= MIN_MATCHED_QTY,
            bankruptcy_ok: self.bankruptcies <= MAX_BANKRUPTCIES,
        }
    }

    /// Sim özetindeki tek satırlık DRAMA raporu.
    #[must_use]
    pub fn summary_line(&self) -> String {
        format!(
            "tekel {}↑/{}↓ · undercut {} · savaş {}⚔/{}🏳 · iflas {} · boğma {} · kartel {}/{}kin{}",
            self.monopolies_formed,
            self.monopolies_broken,
            self.undercut_campaigns,
            self.price_wars_declared,
            self.price_wars_won,
            self.bankruptcies,
            self.supply_chokes,
            self.cartels_formed,
            self.cartels_betrayed,
            if self.grudges_formed > 0 {
                format!(" · kin {}", self.grudges_formed)
            } else {
                String::new()
            },
        )
    }
}

/// Sezon hükmü: drama hedefi + felaket frenleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// Toplam hikâyelik olay.
    pub story_events: u64,
    /// `STORY_EVENT_TARGET`'a ulaşıldı mı? (Faz 1 öncesi doğal olarak false.)
    pub story_target_met: bool,
    /// Piyasa toptan ölmedi (eşleşme hacmi taban üstünde).
    pub market_alive: bool,
    /// İflas zinciri oyunu bitirmedi.
    pub bankruptcy_ok: bool,
}

impl Verdict {
    /// Felaket frenleri (drama hedefi hariç) temiz mi?
    #[must_use]
    pub const fn brakes_ok(&self) -> bool {
        self.market_alive && self.bankruptcy_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{CityId, PlayerId, ProductKind};

    fn monopoly() -> LogEvent {
        LogEvent::MonopolyFormed {
            city: CityId::Istanbul,
            product: ProductKind::Un,
            firm: PlayerId::new(7),
            share_percent: 72,
        }
    }

    #[test]
    fn counts_story_events_by_kind() {
        let mut card = DramaScorecard::default();
        card.record(Tick::new(1), &monopoly());
        card.record(Tick::new(1), &monopoly());
        card.record(Tick::new(1), &LogEvent::FirmBankrupt { firm: PlayerId::new(3) });
        assert_eq!(card.monopolies_formed, 2);
        assert_eq!(card.bankruptcies, 1);
        assert_eq!(card.story_event_total(), 3);
    }

    #[test]
    fn ignores_non_story_events() {
        let mut card = DramaScorecard::default();
        card.record(Tick::new(1), &LogEvent::EconomySalary {
            player: PlayerId::new(1),
            amount: moneywar_domain::Money::from_cents(100),
        });
        assert_eq!(card, DramaScorecard::default());
    }

    #[test]
    fn grudge_counts_separately_not_as_story() {
        let mut card = DramaScorecard::default();
        card.record(Tick::new(1), &LogEvent::GrudgeFormed {
            holder: PlayerId::new(1),
            against: PlayerId::new(2),
        });
        assert_eq!(card.grudges_formed, 1);
        assert_eq!(card.story_event_total(), 0);
    }

    #[test]
    fn verdict_flags_dead_market_and_bankruptcy_chain() {
        let mut card = DramaScorecard::default();
        // Eşiği aşan iflas zinciri — sabit değiştiğinde test bayatlamasın.
        for i in 0..=MAX_BANKRUPTCIES {
            card.record(Tick::new(1), &LogEvent::FirmBankrupt { firm: PlayerId::new(i) });
        }
        let v = card.verdict(MIN_MATCHED_QTY - 1);
        assert!(!v.market_alive);
        assert!(!v.bankruptcy_ok);
        assert!(!v.brakes_ok());
    }

    #[test]
    fn verdict_passes_with_healthy_brakes_and_enough_drama() {
        let mut card = DramaScorecard::default();
        for _ in 0..STORY_EVENT_TARGET {
            card.record(Tick::new(1), &monopoly());
        }
        let v = card.verdict(MIN_MATCHED_QTY);
        assert!(v.story_target_met);
        assert!(v.brakes_ok());
    }
}
