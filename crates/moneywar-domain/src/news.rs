//! Haber servisi — abonelik tier'ları + haber maddeleri (game-design.md §6).
//!
//! Olay motoru bir `GameEvent` tetikleyince haber servisi her abonelik
//! tier'ı için `NewsItem` üretir ve tier'ın lead-time'ına göre oyuncuya
//! farklı tick'lerde ulaştırır:
//!
//! | Tier | Kime | Lead-time |
//! |------|------|-----------|
//! | Bronz | Herkes (bedava) | 0 tick (anında) |
//! | Gümüş | Abonelik (Tüccar bedava) | 1 tick önceden |
//! | Altın | Pahalı abonelik | 2 tick önceden |

use serde::{Deserialize, Serialize};

use crate::{DomainError, GameEvent, Money, NewsId, Tick};

/// Haber abonelik tier'ı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NewsTier {
    Bronze,
    Silver,
    Gold,
}

impl NewsTier {
    /// Olayın gerçekleştiği tick'ten kaç tick önce haber gelir.
    #[must_use]
    pub const fn lead_time(self) -> u32 {
        match self {
            Self::Bronze => 0,
            Self::Silver => 1,
            Self::Gold => 2,
        }
    }

    /// Sezon başı abonelik ücreti (varsayılan). Tüccar Gümüş'ü bedava alır,
    /// bu ücret normal oyunculara uygulanır.
    pub fn subscription_cost(self) -> Result<Money, DomainError> {
        match self {
            Self::Bronze => Ok(Money::ZERO),
            Self::Silver => Money::from_lira(500),
            Self::Gold => Money::from_lira(2_000),
        }
    }

    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Bronze => "Bronz",
            Self::Silver => "Gümüş",
            Self::Gold => "Altın",
        }
    }
}

impl std::fmt::Display for NewsTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Oyuncuya dağıtılan haber maddesi.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NewsItem {
    pub id: NewsId,
    pub tier: NewsTier,
    /// Haberin oyuncuya görüneceği tick.
    pub disclosed_tick: Tick,
    /// Olayın gerçekleşeceği tick (`disclosed_tick` + `tier.lead_time()`).
    pub event_tick: Tick,
    /// Altında yatan oyun olayı.
    pub event: GameEvent,
}

impl NewsItem {
    /// Olaydan `NewsItem` üretir. `disclosed_tick = event_tick - tier.lead_time()`.
    /// `event_tick < lead_time` ise hata.
    pub fn from_event(
        id: NewsId,
        tier: NewsTier,
        event_tick: Tick,
        event: GameEvent,
    ) -> Result<Self, DomainError> {
        let disclosed = event_tick
            .value()
            .checked_sub(tier.lead_time())
            .ok_or_else(|| {
                DomainError::Validation(format!(
                    "event at {event_tick} is too early for {tier} lead-time {}",
                    tier.lead_time()
                ))
            })?;
        Ok(Self {
            id,
            tier,
            disclosed_tick: Tick::new(disclosed),
            event_tick,
            event,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CityId, EventSeverity, ProductKind};

    fn sample_event() -> GameEvent {
        GameEvent::Drought {
            city: CityId::Ankara,
            product: ProductKind::Bugday,
            severity: EventSeverity::Major,
        }
    }

    #[test]
    fn tier_lead_times_match_design() {
        assert_eq!(NewsTier::Bronze.lead_time(), 0);
        assert_eq!(NewsTier::Silver.lead_time(), 1);
        assert_eq!(NewsTier::Gold.lead_time(), 2);
    }

    #[test]
    fn tier_subscription_costs_monotone() {
        let b = NewsTier::Bronze.subscription_cost().unwrap();
        let s = NewsTier::Silver.subscription_cost().unwrap();
        let g = NewsTier::Gold.subscription_cost().unwrap();
        assert!(b < s);
        assert!(s < g);
        assert_eq!(b, Money::ZERO);
    }

    #[test]
    fn tier_ordering_bronze_lt_silver_lt_gold() {
        assert!(NewsTier::Bronze < NewsTier::Silver);
        assert!(NewsTier::Silver < NewsTier::Gold);
    }

    #[test]
    fn tier_display_names() {
        assert_eq!(NewsTier::Bronze.to_string(), "Bronz");
        assert_eq!(NewsTier::Silver.to_string(), "Gümüş");
        assert_eq!(NewsTier::Gold.to_string(), "Altın");
    }

    #[test]
    fn news_item_bronze_disclosed_at_event_tick() {
        let n = NewsItem::from_event(
            NewsId::new(1),
            NewsTier::Bronze,
            Tick::new(30),
            sample_event(),
        )
        .unwrap();
        assert_eq!(n.disclosed_tick, Tick::new(30));
        assert_eq!(n.event_tick, Tick::new(30));
    }

    #[test]
    fn news_item_silver_disclosed_1_tick_before() {
        let n = NewsItem::from_event(
            NewsId::new(1),
            NewsTier::Silver,
            Tick::new(30),
            sample_event(),
        )
        .unwrap();
        assert_eq!(n.disclosed_tick, Tick::new(29));
    }

    #[test]
    fn news_item_gold_disclosed_2_ticks_before() {
        let n = NewsItem::from_event(
            NewsId::new(1),
            NewsTier::Gold,
            Tick::new(30),
            sample_event(),
        )
        .unwrap();
        assert_eq!(n.disclosed_tick, Tick::new(28));
    }

    #[test]
    fn news_item_rejects_underflow() {
        let err = NewsItem::from_event(
            NewsId::new(1),
            NewsTier::Gold,
            Tick::new(1), // need 2 tick lead, but we only have 1
            sample_event(),
        )
        .expect_err("underflow");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn news_item_serde_roundtrip() {
        let n = NewsItem::from_event(
            NewsId::new(7),
            NewsTier::Gold,
            Tick::new(50),
            sample_event(),
        )
        .unwrap();
        let back: NewsItem = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(n, back);
    }
}
