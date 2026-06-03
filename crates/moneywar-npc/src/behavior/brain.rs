//! Ajan hafıza katmanı (Faz 2 + 3) — tick'ler arası yaşayan `AgentBrain`.
//!
//! Her NPC'nin bir `AgentBrain`'i vardır; `BrainPool` tüm beyinleri tutar.
//! Motor (`advance_tick`) saf ekonomik mekanik olarak kalır; beyin AI'nın
//! derdidir. Her tick:
//!   1. `brain.observe(state, pid)` — gözlem güncelle (deterministik)
//!   2. `inject_brain_signals` — sinyalleri inputs map'ine ekle
//!   3. `decide_behavior` normal skor hesabını yapar, beyin sinyalleri dahil
//!
//! ## Faz 3 — Beklenti + sınırlı rasyonellik
//!
//! - **Fiyat beklentisi (EWMA):** brain geçmişten beklediği fiyatı öğrenir.
//! - **Skill noise:** kazanan keskin, kaybeden panikler.
//!
//! ## Faz 5 — Çok-tick strateji (Goal state machine)
//!
//! Brain'de kalıcı bir `Goal` yaşar: `Expand | Corner | PriceWar | Consolidate | Retreat`.
//! Her tick `update_goal()` koşulları değerlendirir ve geçiş yapar.
//! `goal_alignment` sinyali (0..1): bu (city,product) mevcut hedefle ne kadar uyumlu?
//! Hedefe uygun bucket'larda aksiyonlar güçlendirilir, uyumsuzlarda zayıflar.
//!
//! # Determinizm
//!
//! `observe()` sadece `GameState`'e bakar, RNG kullanmaz. Aynı seed → aynı
//! state geçmişi → aynı beyin durumu → aynı karar.

use std::collections::{BTreeMap, VecDeque};

use moneywar_domain::{CityId, GameState, OrderSide, PlayerId, ProductKind};
use moneywar_engine::score_player;

/// Son kaç tick'in PnL deltası trend hesabına girer.
const PNL_WINDOW: usize = 8;
/// Saniyede N lira artış = "iyi trend" referansı (normalize için).
/// 150K lira/sezon = 1000₺/tick → bir tick'te 1000₺ kazanmak çok iyi.
const PNL_TREND_REF_CENTS: f64 = 100_000.0; // 1000₺ = 100_000 cent
/// Cash surplus referans eşiği — bu naditin üstü "harcayacak para var".
const CASH_SURPLUS_REF: f64 = 40_000.0; // 40K₺

// Trait drift hızı — her tick max bu kadar kayar (çok hızlı salınmayı önler).
const TRAIT_DRIFT_RATE: f64 = 0.02;

/// Sürekli kişilik trait vektörü — tüm değerler [0,1].
/// Statik Personality enum'un yerine geçmez; onu başlangıç noktası olarak
/// kullanır ve sonra deneyimle kayar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PersonalityTraits {
    /// Risk iştahı: 0=aşırı temkinli, 1=tam spekülatif.
    pub risk: f64,
    /// Agresiflik: 0=pasif bekle, 1=rakibe agresif saldır.
    pub aggression: f64,
    /// Sabır: 0=hemen sat/al (panik), 1=fiyat olgunlaşana kadar bekle.
    pub patience: f64,
    /// Kâr açgözlülüğü: 0=makul kâra razı, 1=maksimum marj ısrar.
    pub greed: f64,
}

impl Default for PersonalityTraits {
    fn default() -> Self {
        // Başlangıç: orta seviye her şey.
        Self { risk: 0.5, aggression: 0.5, patience: 0.5, greed: 0.5 }
    }
}

impl PersonalityTraits {
    /// Trait'lerin mevcut `Weights`'i nasıl modüle ettiği:
    /// - risk: arbitrage + expected_edge ölçeklenir
    /// - aggression: rival_threat yanıtı + bucket_dominance
    /// - patience: urgency karşıtı (yüksek sabır → urgency ağırlığı düşer)
    /// - greed: price_rel_avg amplify (pahalıya sat, ucuza al ısrarı)
    pub fn modulate(&self, mut w: super::scoring::Weights) -> super::scoring::Weights {
        // Risk: yüksekse spekülatif sinyallere daha duyarlı
        let risk_scale = 0.5 + self.risk;        // 0.5..1.5
        w.arbitrage *= risk_scale;
        w.expected_edge *= risk_scale;

        // Aggression: rakip ve hâkimiyet sinyallerine tepki
        let agg_scale = 0.5 + self.aggression;   // 0.5..1.5
        w.rival_threat *= agg_scale;
        w.bucket_dominance *= agg_scale;
        w.competition *= agg_scale;              // negatif ağırlıksa daha güçlü kaç

        // Patience: düşük sabır → urgency güçlenir (hemen işlem yap)
        let impatience = 1.0 - self.patience;    // 0..1
        w.urgency *= 0.5 + impatience;           // 0.5..1.5

        // Greed: yüksek açgözlülük → fiyat rel avg'ye daha duyarlı
        let greed_scale = 0.5 + self.greed;
        w.price_rel_avg *= greed_scale;

        w
    }
}

// Goal geçiş eşikleri
/// Corner hedefi için gereken minimum sahiplik oranı.
const CORNER_OWNERSHIP_THRESHOLD: f64 = 0.25;
/// PriceWar başlatmak için rakip tehdidin yeterince yüksek olması.
const PRICE_WAR_THREAT_THRESHOLD: f64 = 0.3;
/// Consolidate'ten Expand'e dönmek için gereken nakit fazlası.
const EXPAND_CASH_THRESHOLD: f64 = 0.7;
/// Retreat için kritik PnL eşiği.
const RETREAT_PNL_THRESHOLD: f64 = 0.2;
/// Retreat'ten çıkış için iyileşme eşiği.
const RECOVER_PNL_THRESHOLD: f64 = 0.45;

/// Ajan'ın bu an takip ettiği çok-tick strateji.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Goal {
    /// Yeni pazar veya fabrika aç — büyüme modu.
    Expand,
    /// Belirli bir bucket'ı ele geçir — odak modu.
    Corner { city: CityId, product: ProductKind },
    /// Rakibi belirli bir bucket'ta ez — savaş modu.
    PriceWar { city: CityId, product: ProductKind },
    /// Mevcut pozisyonu güçlendir ve savun — istikrar modu.
    Consolidate,
    /// Zararı kes, sermayeyi koru — geri çekilme modu.
    Retreat,
}
/// Fiyat beklentisi EWMA katsayısı (0=donmuş hafıza, 1=anlık).
/// 0.2 → ~5 tick gecikmeli beklenti; ani şoklara aşırı tepki vermez.
const BELIEF_ALPHA: f64 = 0.2;
/// Kaybeden ajanın kazanan üstündeki ek gürültüsü (noise scale).
const LOSER_NOISE_PENALTY: f64 = 0.15;

/// Bir NPC'nin tick'ler arası hatırladığı durum.
#[derive(Debug, Clone)]
pub struct AgentBrain {
    // ── Faz 2 sinyalleri ─────────────────────────────────────────────────────
    /// PnL trendi: 0.0 = hızla düşüyor, 0.5 = sabit, 1.0 = hızla yükseliyor.
    pub pnl_trend: f64,
    /// Nakit fazlası: 0.0 = kasada para yok, 1.0 = bol para.
    pub cash_surplus: f64,
    /// Bucket bazlı pazar sahipliği: sell emri payı (0..1).
    pub market_ownership: BTreeMap<(CityId, ProductKind), f64>,
    /// Kendi bucket'larıma rakip baskısı: PlayerId → ağırlıklı tehdit skoru.
    pub rival_threat: BTreeMap<PlayerId, f64>,

    // ── Faz 3: fiyat beklentisi ───────────────────────────────────────────────
    /// EWMA fiyat beklentisi (lira) — geçmişten öğrenilen "adil fiyat" tahmini.
    pub price_beliefs: BTreeMap<(CityId, ProductKind), f64>,

    // ── Faz 5: çok-tick strateji ─────────────────────────────────────────────
    /// Mevcut stratejik hedef.
    pub goal: Goal,
    /// Hedefe kaç tick'tir devam ediliyor (geçiş hızını kontrol eder).
    goal_age: u32,

    // ── Faz 6: dinamik kişilik ────────────────────────────────────────────────
    /// Deneyimle kayan sürekli trait vektörü.
    pub traits: PersonalityTraits,

    // -- İç izleme (dışarıya kapalı) --
    prev_pnl_cents: i64,
    pnl_deltas: VecDeque<i64>,
}

impl Default for AgentBrain {
    fn default() -> Self {
        Self {
            pnl_trend: 0.5,
            cash_surplus: 0.5,
            market_ownership: BTreeMap::new(),
            rival_threat: BTreeMap::new(),
            price_beliefs: BTreeMap::new(),
            goal: Goal::Expand,
            goal_age: 0,
            traits: PersonalityTraits::default(),
            prev_pnl_cents: 0,
            pnl_deltas: VecDeque::with_capacity(PNL_WINDOW + 1),
        }
    }
}

impl AgentBrain {
    /// Mevcut `state`'i gözlemle, hafızayı güncelle.
    /// Deterministik — RNG yok.
    pub fn observe(&mut self, state: &GameState, player_id: PlayerId) {
        self.update_pnl_trend(state, player_id);
        self.update_cash_surplus(state, player_id);
        self.update_market_ownership(state, player_id);
        self.update_rival_threat(state, player_id);
        self.update_price_beliefs(state);
        self.update_goal(player_id);
        self.update_traits();
    }

    /// Bu (city, product) mevcut hedefle ne kadar uyumlu? [0..1].
    /// 1.0 = hedefin tam merkezinde, 0.0 = hedefle ilgisiz.
    #[must_use]
    pub fn goal_alignment(&self, city: CityId, product: ProductKind) -> f64 {
        match &self.goal {
            Goal::Expand => {
                // Expand: az sahip olduğum bucket'lar (yeni pazar açmak için).
                let ownership = self.ownership_of(city, product);
                if ownership < 0.1 { 0.8 } else { 0.3 }
            }
            Goal::Corner { city: tc, product: tp } => {
                // Corner: sadece hedef bucket maksimum, diğerleri düşük.
                if city == *tc && product == *tp { 1.0 } else { 0.1 }
            }
            Goal::PriceWar { city: tc, product: tp } => {
                // PriceWar: hedef bucket'ta çok agresif.
                if city == *tc && product == *tp { 1.0 } else { 0.2 }
            }
            Goal::Consolidate => {
                // Consolidate: sahip olduklarımı koru.
                let ownership = self.ownership_of(city, product);
                (ownership * 1.5).clamp(0.0, 1.0)
            }
            Goal::Retreat => {
                // Retreat: tüm genişleme bastırılır.
                0.1
            }
        }
    }

    /// Mevcut hedefin kısa etiketi (log/debug için).
    #[must_use]
    pub fn goal_label(&self) -> &'static str {
        match &self.goal {
            Goal::Expand => "EXPAND",
            Goal::Corner { .. } => "CORNER",
            Goal::PriceWar { .. } => "PRICE_WAR",
            Goal::Consolidate => "CONSOLIDATE",
            Goal::Retreat => "RETREAT",
        }
    }

    /// Bu (city, product) için beklenen fiyat (lira). Hiç görülmediyse 0.
    #[must_use]
    pub fn expected_price(&self, city: CityId, product: ProductKind) -> Option<f64> {
        self.price_beliefs.get(&(city, product)).copied()
    }

    /// Sınırlı rasyonellik gürültüsü — kazanan az hata yapar, kaybeden panikler.
    /// `base_noise` difficulty'den gelen taban gürültü.
    #[must_use]
    pub fn skill_noise(&self, base_noise: f64) -> f64 {
        // pnl_trend 0.5 = nötr, 1.0 = kazanıyor, 0.0 = kaybediyor.
        // Kaybeden (trend < 0.5): taban gürültüye ceza eklenir.
        // Kazanan (trend > 0.5): gürültü azaltılır (min base_noise * 0.5).
        let adj = (0.5 - self.pnl_trend) * LOSER_NOISE_PENALTY * 2.0;
        (base_noise + adj).clamp(base_noise * 0.5, base_noise + LOSER_NOISE_PENALTY)
    }

    /// Bu (city, product) için market_ownership sinyali (0..1).
    #[must_use]
    pub fn ownership_of(&self, city: CityId, product: ProductKind) -> f64 {
        self.market_ownership
            .get(&(city, product))
            .copied()
            .unwrap_or(0.0)
    }

    /// Bu (city, product) için toplam rakip tehdit skoru (0..1 normalize).
    #[must_use]
    pub fn rival_threat_for(&self, city: CityId, product: ProductKind, player_id: PlayerId) -> f64 {
        // Bu bucket'ta rakip order baskısını hesapla — anlık okuma.
        let _ = (city, product, player_id);
        // rival_threat BTreeMap toplam tehdidi tutuyor; bucket bazlı için
        // finalize'de hesaplanmış max'a normalize et.
        let total: f64 = self.rival_threat.values().sum();
        if total <= 0.0 {
            return 0.0;
        }
        (total / (total + 1.0)).clamp(0.0, 1.0)
    }

    // ── İç güncellemeler ─────────────────────────────────────────────────────

    fn update_pnl_trend(&mut self, state: &GameState, player_id: PlayerId) {
        let current = score_player(state, player_id).total.as_cents();
        let delta = current - self.prev_pnl_cents;
        self.prev_pnl_cents = current;

        self.pnl_deltas.push_back(delta);
        if self.pnl_deltas.len() > PNL_WINDOW {
            self.pnl_deltas.pop_front();
        }

        if self.pnl_deltas.is_empty() {
            self.pnl_trend = 0.5;
            return;
        }
        let avg: f64 =
            self.pnl_deltas.iter().sum::<i64>() as f64 / self.pnl_deltas.len() as f64;
        // sigmoid yumuşatma: 0 → 0.5, +ref → ~0.8, -ref → ~0.2
        self.pnl_trend = sigmoid(avg / PNL_TREND_REF_CENTS);
    }

    fn update_cash_surplus(&mut self, state: &GameState, player_id: PlayerId) {
        let cash = state
            .players
            .get(&player_id)
            .map_or(0.0, |p| p.cash.as_cents() as f64 / 100.0);
        self.cash_surplus = (cash / CASH_SURPLUS_REF).clamp(0.0, 1.0);
    }

    fn update_market_ownership(&mut self, state: &GameState, player_id: PlayerId) {
        self.market_ownership.clear();
        for ((city, product), orders) in &state.order_book {
            let my_sell: u32 = orders
                .iter()
                .filter(|o| o.player == player_id && o.side == OrderSide::Sell)
                .map(|o| o.quantity)
                .sum();
            let total_sell: u32 = orders
                .iter()
                .filter(|o| o.side == OrderSide::Sell)
                .map(|o| o.quantity)
                .sum();
            if total_sell > 0 {
                let share = my_sell as f64 / total_sell as f64;
                if share > 0.0 {
                    self.market_ownership.insert((*city, *product), share);
                }
            }
        }
    }

    /// Trait'leri pnl_trend'e göre kaydır.
    fn update_traits(&mut self) {
        let t = &mut self.traits;
        let trend = self.pnl_trend; // 0=kaybediyor, 0.5=sabit, 1=kazanıyor

        if trend > 0.6 {
            // Kazanıyor: özgüvenle büyü
            drift(&mut t.risk,       0.7, TRAIT_DRIFT_RATE);
            drift(&mut t.aggression, 0.65, TRAIT_DRIFT_RATE);
            drift(&mut t.patience,   0.7, TRAIT_DRIFT_RATE);
            drift(&mut t.greed,      0.7, TRAIT_DRIFT_RATE);
        } else if trend < 0.35 {
            // Kaybediyor: panik + çaresiz saldırganlık
            drift(&mut t.risk,       0.3, TRAIT_DRIFT_RATE);  // korkak
            drift(&mut t.aggression, 0.75, TRAIT_DRIFT_RATE); // çaresiz saldırı
            drift(&mut t.patience,   0.25, TRAIT_DRIFT_RATE); // panik sat
            drift(&mut t.greed,      0.3, TRAIT_DRIFT_RATE);  // daha ucuza satar
        } else {
            // Nötr: yavaşça ortaya dön
            drift(&mut t.risk,       0.5, TRAIT_DRIFT_RATE * 0.3);
            drift(&mut t.aggression, 0.5, TRAIT_DRIFT_RATE * 0.3);
            drift(&mut t.patience,   0.5, TRAIT_DRIFT_RATE * 0.3);
            drift(&mut t.greed,      0.5, TRAIT_DRIFT_RATE * 0.3);
        }
    }

    /// Goal state machine geçişleri — deterministik.
    fn update_goal(&mut self, _player_id: PlayerId) {
        self.goal_age += 1;
        // Minimum hedef süresi: gereksiz titreşimi önle.
        const MIN_GOAL_TICKS: u32 = 5;
        if self.goal_age < MIN_GOAL_TICKS {
            return;
        }

        let new_goal = match &self.goal {
            Goal::Retreat => {
                // İyileşiyorsa çıkış: Consolidate.
                if self.pnl_trend > RECOVER_PNL_THRESHOLD {
                    Some(Goal::Consolidate)
                } else {
                    None
                }
            }
            Goal::Expand => {
                // Kritik durumda geri çekil.
                if self.pnl_trend < RETREAT_PNL_THRESHOLD && self.cash_surplus < 0.1 {
                    return self.transition(Goal::Retreat);
                }
                // Bir bucket'ta baskın pozisyon kazandıysam → Corner.
                if let Some((city, product)) = self.strongest_owned_bucket() {
                    if self.ownership_of(city, product) >= CORNER_OWNERSHIP_THRESHOLD {
                        Some(Goal::Corner { city, product })
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            Goal::Corner { city, product } => {
                let (c, p) = (*city, *product);
                if self.pnl_trend < RETREAT_PNL_THRESHOLD {
                    return self.transition(Goal::Retreat);
                }
                // Rakip tehdidi yüksekse → PriceWar.
                let threat = self.rival_threat_for(c, p, _player_id);
                if threat > PRICE_WAR_THREAT_THRESHOLD {
                    Some(Goal::PriceWar { city: c, product: p })
                // Pekiştirildiyse (yüksek sahiplik) → Consolidate.
                } else if self.ownership_of(c, p) > 0.6 {
                    Some(Goal::Consolidate)
                } else {
                    None
                }
            }
            Goal::PriceWar { city, product } => {
                let (c, p) = (*city, *product);
                // Kazanıyorsam → Consolidate.
                if self.pnl_trend > 0.65 {
                    Some(Goal::Consolidate)
                // Kaybediyorsam → Retreat.
                } else if self.pnl_trend < RETREAT_PNL_THRESHOLD {
                    Some(Goal::Retreat)
                // Rakip gitti → Corner'a dön.
                } else if self.rival_threat_for(c, p, _player_id) < 0.1 {
                    Some(Goal::Corner { city: c, product: p })
                } else {
                    None
                }
            }
            Goal::Consolidate => {
                if self.pnl_trend < RETREAT_PNL_THRESHOLD {
                    return self.transition(Goal::Retreat);
                }
                // Bol nakit + iyiydim → Expand.
                if self.cash_surplus > EXPAND_CASH_THRESHOLD && self.pnl_trend > 0.6 {
                    Some(Goal::Expand)
                } else {
                    None
                }
            }
        };

        if let Some(next) = new_goal {
            self.transition(next);
        }
    }

    fn transition(&mut self, new_goal: Goal) {
        self.goal = new_goal;
        self.goal_age = 0;
    }

    /// En güçlü sahip olunan bucket (market_ownership en yüksek).
    fn strongest_owned_bucket(&self) -> Option<(CityId, ProductKind)> {
        self.market_ownership
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(k, _)| *k)
    }

    fn update_price_beliefs(&mut self, state: &GameState) {
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                let current = state
                    .price_history
                    .get(&(city, product))
                    .and_then(|h| h.last().map(|(_, p)| p.as_cents() as f64 / 100.0));
                if let Some(cur) = current {
                    let belief = self.price_beliefs.entry((city, product)).or_insert(cur);
                    // EWMA: yeni beklenti = α × anlık + (1-α) × eski beklenti
                    *belief = BELIEF_ALPHA * cur + (1.0 - BELIEF_ALPHA) * *belief;
                }
            }
        }
    }

    fn update_rival_threat(&mut self, state: &GameState, player_id: PlayerId) {
        self.rival_threat.clear();
        for ((city, product), ownership) in &self.market_ownership {
            if *ownership < 0.05 {
                continue; // bu bucket benim değil
            }
            if let Some(orders) = state.order_book.get(&(*city, *product)) {
                for o in orders {
                    if o.player != player_id {
                        *self.rival_threat.entry(o.player).or_default() +=
                            o.quantity as f64 * ownership;
                    }
                }
            }
        }
    }
}

/// Tüm NPC beyinleri. Sürücü ve sim tick'ler arası saklar.
#[derive(Debug, Default)]
pub struct BrainPool(pub BTreeMap<PlayerId, AgentBrain>);

impl BrainPool {
    /// NPC için beyin döndür (yoksa yeni oluştur).
    pub fn get_or_insert(&mut self, player_id: PlayerId) -> &mut AgentBrain {
        self.0.entry(player_id).or_default()
    }

    /// Mevcut state'i gözlemleyerek tüm beyinleri güncelle.
    pub fn observe_all(&mut self, state: &GameState) {
        for (pid, brain) in &mut self.0 {
            brain.observe(state, *pid);
        }
    }

    /// Oyuncu seti değiştiyse yeni NPC'lere boş beyin ekle.
    pub fn sync_players(&mut self, state: &GameState) {
        for (pid, p) in &state.players {
            if p.is_npc {
                self.0.entry(*pid).or_default();
            }
        }
    }
}

// `1 / (1 + e^{-x})` — sonuç (0,1), x=0 → 0.5.
fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x.clamp(-10.0, 10.0)).exp())
}

/// Değeri hedefe doğru `rate` kadar kaydır (lerp adımı).
fn drift(value: &mut f64, target: f64, rate: f64) {
    *value += (target - *value) * rate;
    *value = value.clamp(0.0, 1.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{Money, NpcKind, Player, PlayerId, Role, RoomConfig, RoomId};

    fn make_state_with_npc(cash_lira: i64) -> (GameState, PlayerId) {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let pid = PlayerId::new(100);
        let p = Player::new(pid, "Test", Role::Sanayici, Money::from_lira(cash_lira).unwrap(), true)
            .unwrap()
            .with_kind(NpcKind::Sanayici);
        s.players.insert(pid, p);
        (s, pid)
    }

    #[test]
    fn default_brain_is_neutral() {
        let brain = AgentBrain::default();
        assert!((brain.pnl_trend - 0.5).abs() < 1e-9);
        assert!(brain.market_ownership.is_empty());
    }

    #[test]
    fn observe_does_not_panic_on_empty_state() {
        let (s, pid) = make_state_with_npc(10_000);
        let mut brain = AgentBrain::default();
        brain.observe(&s, pid); // panik çıkmamalı
    }

    #[test]
    fn cash_surplus_normalized() {
        let (s, pid) = make_state_with_npc(40_000); // = referans
        let mut brain = AgentBrain::default();
        brain.observe(&s, pid);
        assert!((brain.cash_surplus - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cash_surplus_zero_when_broke() {
        let (s, pid) = make_state_with_npc(0);
        let mut brain = AgentBrain::default();
        brain.observe(&s, pid);
        assert_eq!(brain.cash_surplus, 0.0);
    }

    #[test]
    fn pnl_trend_stays_neutral_on_flat_pnl() {
        let (s, pid) = make_state_with_npc(10_000);
        let mut brain = AgentBrain::default();
        // Aynı state'i 10 kez gözlemle → delta her seferinde 0.
        for _ in 0..10 {
            brain.observe(&s, pid);
        }
        assert!((brain.pnl_trend - 0.5).abs() < 0.1);
    }

    #[test]
    fn brain_pool_sync_adds_npc() {
        let (s, pid) = make_state_with_npc(10_000);
        let mut pool = BrainPool::default();
        assert!(pool.0.is_empty());
        pool.sync_players(&s);
        assert!(pool.0.contains_key(&pid));
    }

    #[test]
    fn sigmoid_midpoint_is_half() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-9);
        assert!(sigmoid(10.0) > 0.99);
        assert!(sigmoid(-10.0) < 0.01);
    }

    #[test]
    fn price_belief_converges_toward_current_price() {
        let (mut s, pid) = make_state_with_npc(10_000);
        // Fiyat geçmişine birkaç nokta ekle.
        let price = moneywar_domain::Money::from_lira(100).unwrap();
        s.price_history
            .entry((CityId::Istanbul, ProductKind::Pamuk))
            .or_default()
            .push((moneywar_domain::Tick::new(1), price));

        let mut brain = AgentBrain::default();
        // İlk observe → beklenti anlık fiyattan başlar.
        brain.observe(&s, pid);
        let b1 = brain.expected_price(CityId::Istanbul, ProductKind::Pamuk).unwrap();
        assert!((b1 - 100.0).abs() < 1.0);
    }

    #[test]
    fn skill_noise_winner_lower_than_loser() {
        let base = 0.10;
        let mut winner = AgentBrain::default();
        winner.pnl_trend = 0.9; // kazanıyor
        let mut loser = AgentBrain::default();
        loser.pnl_trend = 0.1; // kaybediyor

        assert!(winner.skill_noise(base) < loser.skill_noise(base));
    }

    #[test]
    fn skill_noise_neutral_equals_base() {
        let base = 0.10;
        let neutral = AgentBrain::default(); // pnl_trend = 0.5
        let noise = neutral.skill_noise(base);
        // Nötr → base ± küçük tolerans (0.5 - 0.5 = 0, adj = 0)
        assert!((noise - base).abs() < 0.01);
    }
}
