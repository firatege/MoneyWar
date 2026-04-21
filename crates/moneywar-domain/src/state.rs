//! Oyun durumu kökü (`GameState`).
//!
//! Tek bir oda'nın tüm state'i bu struct'ta tutulur. Motor bu state'i
//! saf fonksiyon olarak okuyup yeni state üretir:
//!
//! ```text
//! advance_tick(state, commands, seed) → (new_state, report)
//! ```
//!
//! Tüm koleksiyonlar `BTreeMap` — deterministik iterasyon için
//! (`HashMap` yasak, replay kırılır).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    Caravan, CaravanId, CityId, Contract, ContractId, Factory, FactoryId, Loan, LoanId,
    MarketOrder, Money, NewsItem, NewsTier, Player, PlayerId, ProductKind, RoomConfig, RoomId,
    Tick,
};

/// Tek bir oda'nın tüm oyun durumu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameState {
    pub room_id: RoomId,
    pub config: RoomConfig,
    pub current_tick: Tick,

    /// İnsan + NPC oyuncuları.
    pub players: BTreeMap<PlayerId, Player>,
    pub factories: BTreeMap<FactoryId, Factory>,
    pub caravans: BTreeMap<CaravanId, Caravan>,

    /// Hal Pazarı emir defteri. `(city, product) → [emirler]`.
    /// Tick sınırında batch auction ile eşleşir, boşaltılır.
    pub order_book: BTreeMap<(CityId, ProductKind), Vec<MarketOrder>>,

    pub contracts: BTreeMap<ContractId, Contract>,

    /// Oyuncunun şu anki haber abonelik tier'ı. Tüccar Silver'ı bedava alır
    /// ama motor yine de burada kayıt tutar (uniform kod yolu).
    pub news_subscriptions: BTreeMap<PlayerId, NewsTier>,

    /// Oyuncu başı haber kutusu (en yenisi sondadır).
    pub news_inbox: BTreeMap<PlayerId, Vec<NewsItem>>,

    /// Aktif krediler (Faz 5.5). Ödenen krediler buradan kaldırılır.
    pub loans: BTreeMap<LoanId, Loan>,

    /// `(city, product) → [(tick, takas_fiyati), ...]`. Skor için son 5 tick
    /// ortalaması buradan hesaplanır (§9).
    pub price_history: BTreeMap<(CityId, ProductKind), Vec<(Tick, Money)>>,

    /// Deterministik ID üretimi için sayaçlar. Engine yeni entity kurduğunda bunları artırır.
    pub counters: IdCounters,
}

/// Deterministik ID üretimi için monoton sayaçlar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IdCounters {
    pub next_order_id: u64,
    pub next_contract_id: u64,
    pub next_factory_id: u64,
    pub next_caravan_id: u64,
    pub next_news_id: u64,
    pub next_event_id: u64,
    pub next_loan_id: u64,
}

impl GameState {
    /// Boş oda state'i. Oyuncular ve entity'ler sonradan eklenir (Faz 9'da server).
    #[must_use]
    pub fn new(room_id: RoomId, config: RoomConfig) -> Self {
        Self {
            room_id,
            config,
            current_tick: Tick::ZERO,
            players: BTreeMap::new(),
            factories: BTreeMap::new(),
            caravans: BTreeMap::new(),
            order_book: BTreeMap::new(),
            contracts: BTreeMap::new(),
            news_subscriptions: BTreeMap::new(),
            news_inbox: BTreeMap::new(),
            loans: BTreeMap::new(),
            price_history: BTreeMap::new(),
            counters: IdCounters::default(),
        }
    }

    /// Sezonun ilerleme yüzdesi. Kolaylık getter'ı.
    #[must_use]
    pub fn season_progress(&self) -> crate::SeasonProgress {
        // Config valid → season_ticks > 0 garantili; unwrap_or_else fallback.
        crate::SeasonProgress::from_ticks(self.current_tick, self.config.season_ticks)
            .unwrap_or(crate::SeasonProgress::START)
    }

    /// Toplam katılımcı sayısı (insan + NPC).
    #[must_use]
    pub fn participant_count(&self) -> u8 {
        u8::try_from(self.players.len()).unwrap_or(u8::MAX)
    }

    /// Belirli (şehir, ürün) için son N tick'in ortalama takas fiyatı.
    /// Tarihçe boşsa `None`. §9 skor formülü `N=5` ile kullanır.
    #[must_use]
    pub fn rolling_avg_price(
        &self,
        city: CityId,
        product: ProductKind,
        window: usize,
    ) -> Option<Money> {
        let hist = self.price_history.get(&(city, product))?;
        if hist.is_empty() || window == 0 {
            return None;
        }
        let slice_start = hist.len().saturating_sub(window);
        let slice = &hist[slice_start..];
        let count = i64::try_from(slice.len()).ok()?;
        let sum_cents: i64 = slice
            .iter()
            .map(|(_, m)| m.as_cents())
            .try_fold(0i64, i64::checked_add)?;
        let avg = sum_cents.checked_div(count)?;
        Some(Money::from_cents(avg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RoomId;

    fn empty_state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    #[test]
    fn new_state_is_empty() {
        let s = empty_state();
        assert_eq!(s.current_tick, Tick::ZERO);
        assert!(s.players.is_empty());
        assert!(s.factories.is_empty());
        assert!(s.caravans.is_empty());
        assert!(s.order_book.is_empty());
        assert!(s.contracts.is_empty());
    }

    #[test]
    fn new_state_uses_default_counters() {
        let s = empty_state();
        assert_eq!(s.counters, IdCounters::default());
    }

    #[test]
    fn season_progress_zero_at_start() {
        let s = empty_state();
        assert_eq!(s.season_progress().value(), 0);
    }

    #[test]
    fn season_progress_mid_at_halfway() {
        let mut s = empty_state();
        // Hızlı: 90 ticks. Tick 45 = 50%
        s.current_tick = Tick::new(45);
        assert_eq!(s.season_progress().value(), 50);
    }

    #[test]
    fn rolling_avg_none_when_empty() {
        let s = empty_state();
        assert!(
            s.rolling_avg_price(CityId::Istanbul, ProductKind::Pamuk, 5)
                .is_none()
        );
    }

    #[test]
    fn rolling_avg_computes_mean() {
        let mut s = empty_state();
        s.price_history.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![
                (Tick::new(1), Money::from_cents(100)),
                (Tick::new(2), Money::from_cents(200)),
                (Tick::new(3), Money::from_cents(300)),
            ],
        );
        let avg = s
            .rolling_avg_price(CityId::Istanbul, ProductKind::Pamuk, 5)
            .unwrap();
        assert_eq!(avg, Money::from_cents(200));
    }

    #[test]
    fn rolling_avg_uses_last_n_only() {
        let mut s = empty_state();
        s.price_history.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![
                (Tick::new(1), Money::from_cents(100)),
                (Tick::new(2), Money::from_cents(200)),
                (Tick::new(3), Money::from_cents(300)),
                (Tick::new(4), Money::from_cents(400)),
                (Tick::new(5), Money::from_cents(500)),
                (Tick::new(6), Money::from_cents(600)),
            ],
        );
        // Son 2 tick: 500, 600 → avg 550
        let avg = s
            .rolling_avg_price(CityId::Istanbul, ProductKind::Pamuk, 2)
            .unwrap();
        assert_eq!(avg, Money::from_cents(550));
    }

    #[test]
    fn rolling_avg_window_zero_returns_none() {
        let mut s = empty_state();
        s.price_history.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![(Tick::new(1), Money::from_cents(100))],
        );
        assert!(
            s.rolling_avg_price(CityId::Istanbul, ProductKind::Pamuk, 0)
                .is_none()
        );
    }

    #[test]
    fn serde_roundtrip_empty() {
        let s = empty_state();
        let back: GameState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
