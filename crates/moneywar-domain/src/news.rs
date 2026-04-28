//! Haber servisi — 4 tier abonelik (recurring tick fee) + haber maddeleri.
//!
//! Olay motoru bir `GameEvent` tetikleyince haber servisi her abonelik
//! tier'ı için `NewsItem` üretir ve tier'ın lead-time'ına göre oyuncuya
//! farklı tick'lerde ulaştırır:
//!
//! | Tier   | Kime                            | Lead-time | Tick ücreti (norm/Tüccar) |
//! |--------|---------------------------------|-----------|---------------------------|
//! | Free   | Herkese — varsayılan            | yok       | 0 / 0                     |
//! | Bronz  | Tüccar bedava, diğeri ücretli   | 0 tick    | 5 / 0                     |
//! | Gümüş  | Ücretli (Tüccar indirimli)      | 1 tick    | 15 / 5                    |
//! | Altın  | Premium (Tüccar indirimli)      | 2 tick    | 40 / 15                   |
//!
//! Free'de olay haberi gelmez; Bronze'dan itibaren gelir. Ücret oyuncudan
//! tick başına çekilir; cash bitince 1 tick uyarı, sonra Free'ye düşer.

use serde::{Deserialize, Serialize};

use crate::{DomainError, GameEvent, Money, NewsId, Role, Tick};

/// Haber abonelik tier'ı. Sıralama: `Free < Bronze < Silver < Gold`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NewsTier {
    Free,
    Bronze,
    Silver,
    Gold,
}

impl NewsTier {
    /// Olayın gerçekleştiği tick'ten kaç tick önce haber gelir.
    /// Free'de hiç haber gelmez (lead 0 ama ayrı kontrol var).
    #[must_use]
    pub const fn lead_time(self) -> u32 {
        match self {
            Self::Free => crate::balance::NEWS_LEAD_FREE,
            Self::Bronze => crate::balance::NEWS_LEAD_BRONZE,
            Self::Silver => crate::balance::NEWS_LEAD_SILVER,
            Self::Gold => crate::balance::NEWS_LEAD_GOLD,
        }
    }

    /// Bu tier olay haberi alır mı? Free almaz; diğerleri alır.
    #[must_use]
    pub const fn receives_event_news(self) -> bool {
        !matches!(self, Self::Free)
    }

    /// Tick başı abonelik ücreti. Role'e göre Tüccar indirimli tarife alır.
    pub fn tick_cost(self, role: Role) -> Result<Money, DomainError> {
        let lira = match (self, role) {
            (Self::Free, _) => crate::balance::NEWS_TICK_COST_FREE_LIRA,
            (Self::Bronze, Role::Tuccar) => crate::balance::NEWS_TICK_COST_BRONZE_TUCCAR_LIRA,
            (Self::Bronze, _) => crate::balance::NEWS_TICK_COST_BRONZE_LIRA,
            (Self::Silver, Role::Tuccar) => crate::balance::NEWS_TICK_COST_SILVER_TUCCAR_LIRA,
            (Self::Silver, _) => crate::balance::NEWS_TICK_COST_SILVER_LIRA,
            (Self::Gold, Role::Tuccar) => crate::balance::NEWS_TICK_COST_GOLD_TUCCAR_LIRA,
            (Self::Gold, _) => crate::balance::NEWS_TICK_COST_GOLD_LIRA,
        };
        Money::from_lira(lira)
    }

    /// Bir basamak aşağı düş (Free hariç). Free → Free (idempotent).
    #[must_use]
    pub const fn downgrade(self) -> Self {
        match self {
            Self::Gold => Self::Silver,
            Self::Silver => Self::Bronze,
            Self::Bronze | Self::Free => Self::Free,
        }
    }

    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Free => "Ücretsiz",
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
        assert_eq!(NewsTier::Free.lead_time(), 0);
        assert_eq!(NewsTier::Bronze.lead_time(), 0);
        assert_eq!(NewsTier::Silver.lead_time(), 1);
        assert_eq!(NewsTier::Gold.lead_time(), 2);
    }

    #[test]
    fn tier_tick_costs_monotone_for_default_role() {
        // Default rol = Sanayici (Tüccar değil) — full tarife.
        let f = NewsTier::Free.tick_cost(Role::Sanayici).unwrap();
        let b = NewsTier::Bronze.tick_cost(Role::Sanayici).unwrap();
        let s = NewsTier::Silver.tick_cost(Role::Sanayici).unwrap();
        let g = NewsTier::Gold.tick_cost(Role::Sanayici).unwrap();
        assert_eq!(f, Money::ZERO);
        assert!(f < b);
        assert!(b < s);
        assert!(s < g);
    }

    #[test]
    fn tier_tick_costs_tuccar_discount() {
        // Tüccar her tier'da indirimli ama hiçbiri bedava değil.
        let bronze_n = NewsTier::Bronze.tick_cost(Role::Sanayici).unwrap();
        let bronze_t = NewsTier::Bronze.tick_cost(Role::Tuccar).unwrap();
        assert!(bronze_t < bronze_n);
        assert!(!bronze_t.is_zero(), "Bronze artık herkes için ücretli");
        let silver_n = NewsTier::Silver.tick_cost(Role::Sanayici).unwrap();
        let silver_t = NewsTier::Silver.tick_cost(Role::Tuccar).unwrap();
        assert!(silver_t < silver_n);
        let gold_n = NewsTier::Gold.tick_cost(Role::Sanayici).unwrap();
        let gold_t = NewsTier::Gold.tick_cost(Role::Tuccar).unwrap();
        assert!(gold_t < gold_n);
    }

    #[test]
    fn tier_ordering_free_lt_bronze_lt_silver_lt_gold() {
        assert!(NewsTier::Free < NewsTier::Bronze);
        assert!(NewsTier::Bronze < NewsTier::Silver);
        assert!(NewsTier::Silver < NewsTier::Gold);
    }

    #[test]
    fn tier_downgrade_steps_one_level_down() {
        assert_eq!(NewsTier::Gold.downgrade(), NewsTier::Silver);
        assert_eq!(NewsTier::Silver.downgrade(), NewsTier::Bronze);
        assert_eq!(NewsTier::Bronze.downgrade(), NewsTier::Free);
        assert_eq!(NewsTier::Free.downgrade(), NewsTier::Free);
    }

    #[test]
    fn tier_receives_event_news_excludes_free() {
        assert!(!NewsTier::Free.receives_event_news());
        assert!(NewsTier::Bronze.receives_event_news());
        assert!(NewsTier::Silver.receives_event_news());
        assert!(NewsTier::Gold.receives_event_news());
    }

    #[test]
    fn tier_display_names() {
        assert_eq!(NewsTier::Free.to_string(), "Ücretsiz");
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
