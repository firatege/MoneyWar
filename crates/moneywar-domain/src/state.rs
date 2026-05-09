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
    Caravan, CaravanId, CityId, Contract, ContractId, Factory, FactoryId, GameEvent, Loan, LoanId,
    MarketOrder, Money, NewsItem, NewsTier, Player, PlayerId, ProductKind, RoomConfig, RoomId,
    Tick,
};

/// `ProductKind::RAW_MATERIALS` order'ında bir sonraki raw'ı döner.
/// `seed_city_profiles` rotation'ı için yardımcı.
fn next_raw(p: ProductKind) -> ProductKind {
    let raws = ProductKind::RAW_MATERIALS;
    let idx = raws.iter().position(|&r| r == p).unwrap_or(0);
    raws[(idx + 1) % raws.len()]
}

/// Aktif piyasa şoku — bir olayın `(city, product)` üzerinde geçici fiyat
/// etkisi. Pozitif `multiplier_pct` baseline'ı yukarı (kıtlık), negatif
/// aşağı (bolluk) iter. `expires_at` tick'inden itibaren kaldırılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveShock {
    /// Şokun yüzde etkisi. Örn. +18 → baseline × 1.18, -10 → × 0.90.
    pub multiplier_pct: i32,
    /// Bu tick'e gelindiğinde şok temizlenir.
    pub expires_at: Tick,
    /// Şoku üreten olay (UI için "Drought in Ankara" gibi etiketleme).
    pub source: GameEvent,
}

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

    /// Oyuncunun şu anki haber abonelik tier'ı. Recurring fee — tick başına
    /// `tier.tick_cost(role)` çekilir. Yokluk = Free.
    pub news_subscriptions: BTreeMap<PlayerId, NewsTier>,

    /// Cash yetersizliği nedeniyle uyarılan oyuncular. Bir sonraki tick yine
    /// ödenemezse Free'ye düşer. Ödeme yapılırsa silinir.
    #[serde(default)]
    pub news_payment_warned: std::collections::BTreeSet<PlayerId>,

    /// Oyuncu başı haber kutusu (en yenisi sondadır).
    pub news_inbox: BTreeMap<PlayerId, Vec<NewsItem>>,

    /// Aktif krediler (Faz 5.5). Ödenen krediler buradan kaldırılır.
    pub loans: BTreeMap<LoanId, Loan>,

    /// `(city, product) → [(tick, takas_fiyati), ...]`. Skor için son 5 tick
    /// ortalaması buradan hesaplanır (§9).
    pub price_history: BTreeMap<(CityId, ProductKind), Vec<(Tick, Money)>>,

    /// Oyun başında seed RNG ile üretilen baz fiyat dağılımı. Her `(city, product)`
    /// için sabit. NPC davranışları bu tabloyu "fair value" olarak kullanır:
    /// son clearing yoksa buradan, varsa karışımla. Her oyun farklı olduğu için
    /// gerçek arbitraj fırsatı yaratır (bazı şehirler pahalı, bazıları ucuz).
    /// Boş olursa NPC eski hardcoded base'e düşer (`balance::NPC_BASE_PRICE_*`).
    #[serde(default)]
    pub price_baseline: BTreeMap<(CityId, ProductKind), Money>,

    /// Relist cooldown: `(player, city, product) → earliest_allowed_tick`.
    /// Bir emir bittiğinde (expire / cancel / full fill), bu anahtar
    /// `current_tick + balance.relist_cooldown_ticks` değeriyle yazılır.
    /// Submit sırasında `current_tick < earliest_allowed_tick` ise emir reddedilir.
    /// Flash-place manipülasyonunu engeller.
    #[serde(default)]
    pub relist_cooldown: BTreeMap<(PlayerId, CityId, ProductKind), Tick>,

    /// Aktif piyasa şokları — olay motoru ekler, tick başında expire olanlar
    /// temizlenir. NPC fair-value hesabı ve UI bu map'ten okur. Aynı
    /// `(city, product)` için yeni şok eskisinin üstüne yazılır (override).
    #[serde(default)]
    pub active_shocks: BTreeMap<(CityId, ProductKind), ActiveShock>,

    /// Bu oyunun şehir-uzmanlaşma haritası. Hangi şehir hangi ham maddeyi
    /// doğal olarak ucuza üretir? Seed sırasında 3 raw'ı 3 şehre **shuffled**
    /// atar — her oyun farklı: bir oyunda İstanbul → Buğday, sonraki oyunda
    /// İstanbul → Zeytin. "Ezbere strateji" sorununu kırar (oyuncu her sezon
    /// haritayı keşfeder). Boş ise fallback `CityId::cheap_raw()`'a düşer
    /// (geriye uyumluluk).
    #[serde(default)]
    pub city_specialty: BTreeMap<CityId, ProductKind>,

    /// Şehrin "ikincil" hamı — Çiftçi prime'ın ~%25'i kadar üretir, kendi
    /// ihtiyacını karşılar ama dışarıya cılız ihraç eder. Seed sırasında
    /// `city_specialty`'den farklı bir ham olarak atanır. Boş ise harvest
    /// secondary'i atlar.
    #[serde(default)]
    pub city_secondary: BTreeMap<CityId, ProductKind>,

    /// Şehrin "talep" hamı — üretim YOK, Alıcı bu üründe ağırlıklı BUY eder.
    /// İthalat çekme noktası → Tüccar arbitrage doğal hedefi. Seed sırasında
    /// `city_specialty` ve `city_secondary`'den farklı kalan tek raw atanır.
    #[serde(default)]
    pub city_demand: BTreeMap<CityId, ProductKind>,

    /// Deterministik ID üretimi için sayaçlar. Engine yeni entity kurduğunda bunları artırır.
    pub counters: IdCounters,

    /// v8.20: Patience erosion sayacı — `(player, city, product)` için art arda
    /// match olmadan geçen tick sayısı. Match olduğunda 0'lanır, her clearing'de
    /// match yoksa +1. NPC pricing helper'ları bunu okuyup uyumsuzluk varsa
    /// fiyatı yumuşatır → kilit-anti deadlock garantisi.
    /// Cap MAX_NO_MATCH_STREAK (15) — sonsuz büyümez, jitter'ın hesabı
    /// taşmaz, deterministic.
    #[serde(default)]
    pub no_match_streak: BTreeMap<(PlayerId, CityId, ProductKind), u32>,

    /// v8.22: Easy/Hard difficulty için fiyat cömertliği. NPC pricing
    /// helper'ları bunu urgency_pct'ye ekler (SELL floor düşer, BUY ceiling
    /// yükselir) → human için kâr fırsatı genişler.
    /// - 0  = nötr (Hard)
    /// - 5  = hafif (Medium)
    /// - 15 = cömert (Easy) — NPC fiyat marjları %15 human lehine kayar
    #[serde(default)]
    pub market_softener_pct: u32,

    /// v8.24: Sezon başı orijinal fiyat çapaları — tâtonnement clamp için.
    /// Tâtonnement her tick price_baseline'i ±%0.5 kaydırır; bu map sezon
    /// başında snapshot tutar. Engine clamp'i `initial × [%70, %140]` aralığı
    /// uygular → kümülatif fiyat patlaması yok. Sezon başında set edilir
    /// (CLI/sim seed) ve sezon boyu sabit kalır.
    #[serde(default)]
    pub price_baseline_initial: BTreeMap<(CityId, ProductKind), Money>,
}

/// Patience erosion'in üst sınırı — bu eşikten sonra %15 sabit yumuşama.
/// 15 tick ≈ sezon'un %16'sı (90 tick), yeterince hızlı tepki.
pub const MAX_NO_MATCH_STREAK: u32 = 15;

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
            news_payment_warned: std::collections::BTreeSet::new(),
            news_inbox: BTreeMap::new(),
            loans: BTreeMap::new(),
            price_history: BTreeMap::new(),
            price_baseline: BTreeMap::new(),
            relist_cooldown: BTreeMap::new(),
            active_shocks: BTreeMap::new(),
            city_specialty: BTreeMap::new(),
            city_secondary: BTreeMap::new(),
            city_demand: BTreeMap::new(),
            counters: IdCounters::default(),
            no_match_streak: BTreeMap::new(),
            market_softener_pct: 0,
            price_baseline_initial: BTreeMap::new(),
        }
    }

    /// `(player, city, product)` için art arda match olmadan geçen tick sayısı.
    /// `MAX_NO_MATCH_STREAK`'a clamp'lenir. Engine clearing tarafından güncellenir.
    #[must_use]
    pub fn no_match_streak(&self, player: PlayerId, city: CityId, product: ProductKind) -> u32 {
        self.no_match_streak
            .get(&(player, city, product))
            .copied()
            .unwrap_or(0)
            .min(MAX_NO_MATCH_STREAK)
    }

    /// Bu oyundaki "ucuz ham" eşleşmesini döner. State'te tanımlı ise onu,
    /// yoksa derleme zamanı default'una düşer. NPC ve UI bu helper'ı kullanır.
    #[must_use]
    pub fn cheap_raw_for(&self, city: CityId) -> ProductKind {
        self.city_specialty
            .get(&city)
            .copied()
            .unwrap_or_else(|| city.cheap_raw())
    }

    /// Baseline fiyatı + aktif şok çarpımı. Baseline yoksa `None`. Şok yoksa
    /// baseline aynen döner. NPC ve UI bu helper'ı çağırır.
    #[must_use]
    pub fn effective_baseline(&self, city: CityId, product: ProductKind) -> Option<Money> {
        let base = self.price_baseline.get(&(city, product)).copied()?;
        let shock = self.active_shocks.get(&(city, product));
        let pct = shock.map_or(0, |s| s.multiplier_pct);
        if pct == 0 {
            return Some(base);
        }
        // base × (100 + pct) / 100. i32 → i64 widening ile overflow güvenli.
        let multiplier = 100i64 + i64::from(pct);
        let cents = base
            .as_cents()
            .saturating_mul(multiplier)
            .saturating_div(100);
        Some(Money::from_cents(cents.max(1)))
    }

    /// NPC karar referans fiyatı — fiyat keşfi (price discovery) için.
    /// Son 5 clearing'in ortalamasını döndürür (rolling avg). Trade history
    /// yoksa `effective_baseline`'a düşer (sezon başı durumu). Şok çarpımı
    /// rolling_avg üzerine uygulanmaz çünkü clearing fiyatları zaten şoku
    /// içerir; baseline fallback'inde ise effective_baseline şoku ekler.
    /// Engine clearing path'i bu helper'ı KULLANMAZ — sadece NPC karar
    /// mantığı içindir (engine `effective_baseline`'ı pay-as-bid için
    /// kullanır, oraya dokunmuyoruz).
    #[must_use]
    pub fn reference_price(&self, city: CityId, product: ProductKind) -> Option<Money> {
        if let Some(rolling) = self.rolling_avg_price(city, product, 5) {
            return Some(rolling);
        }
        self.effective_baseline(city, product)
    }

    /// Order book'ta `(city, product)` için en yüksek BUY fiyatı + miktarı.
    /// v8.20: Tek doğru kaynak — Tüccar'ın eski lokal `best_bid_in_city`
    /// helper'ı buraya taşındı. NPC pricing (Çiftçi cross, Sanayici cross,
    /// Tüccar dispatch) hep buna bakar.
    #[must_use]
    pub fn best_bid(&self, city: CityId, product: ProductKind) -> Option<(Money, u32)> {
        self.order_book
            .get(&(city, product))?
            .iter()
            .filter(|o| matches!(o.side, crate::OrderSide::Buy))
            .map(|o| (o.unit_price, o.quantity))
            .max_by_key(|(p, _)| *p)
    }

    /// Order book'ta `(city, product)` için en düşük SELL fiyatı + miktarı.
    #[must_use]
    pub fn best_ask(&self, city: CityId, product: ProductKind) -> Option<(Money, u32)> {
        self.order_book
            .get(&(city, product))?
            .iter()
            .filter(|o| matches!(o.side, crate::OrderSide::Sell))
            .map(|o| (o.unit_price, o.quantity))
            .min_by_key(|(p, _)| *p)
    }

    /// Order book midpoint — iki taraf da varsa avg. NPC pricing'in
    /// "spread'i adil paylaş" yaklaşımı için.
    #[must_use]
    pub fn midpoint(&self, city: CityId, product: ProductKind) -> Option<Money> {
        let bid = self.best_bid(city, product)?.0;
        let ask = self.best_ask(city, product)?.0;
        Some(Money::from_cents(
            bid.as_cents().saturating_add(ask.as_cents()) / 2,
        ))
    }

    /// Tick'e gelindiğinde expire olan tüm şokları temizler. Tick lifecycle'ın
    /// en başında çağrılır.
    pub fn clear_expired_shocks(&mut self, current: Tick) {
        self.active_shocks
            .retain(|_, shock| current.is_before(shock.expires_at));
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

    /// Şehir profil slotlarını populate eder. `prime_per_city`: caller'ın
    /// shuffle ettiği her şehrin **prime** hamı. Secondary ve demand rotation
    /// ile türetilir (`ProductKind::RAW_MATERIALS` order: Pamuk→Bugday→Zeytin):
    /// `secondary = prime'ın bir sonraki raw`, `demand = onun bir sonrası`.
    /// Her şehirde 3 slot 3 farklı raw → 9 (şehir × ham) bucket'tan 9'u da
    /// ekonomik anlamlı (3 prime, 3 secondary, 3 demand).
    pub fn seed_city_profiles(&mut self, prime_per_city: [(CityId, ProductKind); 5]) {
        for (city, prime) in prime_per_city {
            let secondary = next_raw(prime);
            let demand = next_raw(secondary);
            self.city_specialty.insert(city, prime);
            self.city_secondary.insert(city, secondary);
            self.city_demand.insert(city, demand);
        }
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

    fn order(side: crate::OrderSide, price_cents: i64, qty: u32) -> MarketOrder {
        MarketOrder::new(
            crate::OrderId::new(qty as u64),
            crate::PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
            side,
            qty,
            Money::from_cents(price_cents),
            Tick::new(1),
        )
        .unwrap()
    }

    #[test]
    fn best_bid_returns_highest_buy() {
        let mut s = empty_state();
        s.order_book.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![
                order(crate::OrderSide::Buy, 100, 10),
                order(crate::OrderSide::Buy, 150, 20),
                order(crate::OrderSide::Sell, 200, 5),
            ],
        );
        let (price, qty) = s.best_bid(CityId::Istanbul, ProductKind::Pamuk).unwrap();
        assert_eq!(price.as_cents(), 150);
        assert_eq!(qty, 20);
    }

    #[test]
    fn best_ask_returns_lowest_sell() {
        let mut s = empty_state();
        s.order_book.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![
                order(crate::OrderSide::Sell, 200, 5),
                order(crate::OrderSide::Sell, 180, 15),
                order(crate::OrderSide::Buy, 100, 10),
            ],
        );
        let (price, qty) = s.best_ask(CityId::Istanbul, ProductKind::Pamuk).unwrap();
        assert_eq!(price.as_cents(), 180);
        assert_eq!(qty, 15);
    }

    #[test]
    fn best_bid_none_for_empty_book() {
        let s = empty_state();
        assert!(s.best_bid(CityId::Istanbul, ProductKind::Pamuk).is_none());
        assert!(s.best_ask(CityId::Istanbul, ProductKind::Pamuk).is_none());
    }

    #[test]
    fn midpoint_avg_of_bid_ask() {
        let mut s = empty_state();
        s.order_book.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![
                order(crate::OrderSide::Buy, 100, 10),
                order(crate::OrderSide::Sell, 200, 5),
            ],
        );
        let mid = s.midpoint(CityId::Istanbul, ProductKind::Pamuk).unwrap();
        assert_eq!(mid.as_cents(), 150);
    }

    #[test]
    fn midpoint_none_when_one_side_missing() {
        let mut s = empty_state();
        s.order_book.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![order(crate::OrderSide::Buy, 100, 10)],
        );
        assert!(s.midpoint(CityId::Istanbul, ProductKind::Pamuk).is_none());
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

    #[test]
    fn seed_city_profiles_assigns_disjoint_slots_per_city() {
        let mut s = empty_state();
        // İstanbul prime=Pamuk → secondary=Bugday, demand=Zeytin
        // Ankara prime=Bugday → secondary=Zeytin, demand=Pamuk
        // İzmir prime=Zeytin → secondary=Pamuk, demand=Bugday
        s.seed_city_profiles([
            (CityId::Istanbul, ProductKind::Pamuk),
            (CityId::Ankara, ProductKind::Bugday),
            (CityId::Izmir, ProductKind::Zeytin),
            (CityId::Bursa, ProductKind::Pamuk),
            (CityId::Konya, ProductKind::Bugday),
        ]);
        for city in CityId::ALL {
            let prime = s.city_specialty[&city];
            let secondary = s.city_secondary[&city];
            let demand = s.city_demand[&city];
            // 3 slot 3 farklı raw — şehir başına bucket çakışması yok.
            assert_ne!(prime, secondary, "{city}: prime/secondary aynı");
            assert_ne!(prime, demand, "{city}: prime/demand aynı");
            assert_ne!(secondary, demand, "{city}: secondary/demand aynı");
        }
    }

    #[test]
    fn seed_city_profiles_covers_buckets() {
        let mut s = empty_state();
        s.seed_city_profiles([
            (CityId::Istanbul, ProductKind::Pamuk),
            (CityId::Ankara, ProductKind::Bugday),
            (CityId::Izmir, ProductKind::Zeytin),
            (CityId::Bursa, ProductKind::Pamuk),
            (CityId::Konya, ProductKind::Bugday),
        ]);
        // 5 şehir × 3 slot = 15 entry. Tek bucket çakışmaz.
        let mut covered = std::collections::BTreeSet::new();
        for city in CityId::ALL {
            covered.insert((city, s.city_specialty[&city]));
            covered.insert((city, s.city_secondary[&city]));
            covered.insert((city, s.city_demand[&city]));
        }
        // 5 şehir × 3 slot = 15
        assert_eq!(covered.len(), 15);
    }
}
