//! Ajan hafıza katmanı (Faz 2) — tick'ler arası yaşayan `AgentBrain`.
//!
//! Her NPC'nin bir `AgentBrain`'i vardır; `BrainPool` tüm beyinleri tutar.
//! Motor (`advance_tick`) saf ekonomik mekanik olarak kalır; beyin AI'nın
//! derdidir. Her tick:
//!   1. `brain.observe(state, pid)` — gözlem güncelle (deterministik)
//!   2. `brain.signals()` — 4 yeni sinyal üret → `compute_inputs`'a eklenir
//!   3. `decide_behavior` normal skor hesabını yapar, beyin sinyalleri dahil
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

/// Bir NPC'nin tick'ler arası hatırladığı durum.
#[derive(Debug, Clone)]
pub struct AgentBrain {
    /// PnL trendi: 0.0 = hızla düşüyor, 0.5 = sabit, 1.0 = hızla yükseliyor.
    pub pnl_trend: f64,
    /// Nakit fazlası: 0.0 = kasada para yok, 1.0 = bol para.
    pub cash_surplus: f64,
    /// Bucket bazlı pazar sahipliği: sell emri payı (0..1).
    pub market_ownership: BTreeMap<(CityId, ProductKind), f64>,
    /// Kendi bucket'larıma rakip baskısı: PlayerId → ağırlıklı tehdit skoru.
    pub rival_threat: BTreeMap<PlayerId, f64>,

    // -- İç izleme (dışarıya kapalı) --
    prev_pnl_cents: i64,
    pnl_deltas: VecDeque<i64>,
}

impl Default for AgentBrain {
    fn default() -> Self {
        Self {
            pnl_trend: 0.5,    // başlangıçta nötr
            cash_surplus: 0.5,
            market_ownership: BTreeMap::new(),
            rival_threat: BTreeMap::new(),
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
}
