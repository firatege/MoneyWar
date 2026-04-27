//! NPC davranış implementasyonları.
//!
//! # Mimari
//!
//! - [`NpcBehavior`] trait: tek giriş noktası `decide(state, pid, rng, tick)`.
//! - [`MarketMaker`] (Easy): basit likidite — rastgele şehir/ürün, baz
//!   fiyat etrafında al/sat. Rol/strateji yok. v1 iskeletinden gelir.
//! - [`SmartTrader`] (Hard): role-aware. Sanayici NPC fabrika kurar,
//!   ham madde alır, üretir, bitmiş ürünü satar. Tüccar NPC arbitraj
//!   yapar (ucuz şehir → kervanla → pahalı şehir).
//!
//! # Zorluk seviyeleri
//!
//! [`Difficulty::Easy`] tüm NPC'lere [`MarketMaker`] uygular. Tüccar/Sanayici
//! arasında fark yok. Likidite verir, ciddi rekabet yok.
//!
//! [`Difficulty::Hard`] her NPC'nin rolüne göre [`SmartTrader`] uygular.
//! Sanayici fabrika kurar, üretim zincirini işletir; Tüccar şehirler arası
//! arbitraj yapar. Tick başına 1-3 emir yollarlar — likidite hep var,
//! rekabet ciddi.
//!
//! # Determinism
//!
//! `decide` RNG alır; aynı (state, rng) → aynı komut seti. Motor, NPC'leri
//! `decide_all_npcs` üzerinden sıralı işler (`BTreeMap` `player_id` ASC).

pub mod dss;
mod error;
pub mod fuzzy;

pub use error::NpcError;

use moneywar_domain::{
    Caravan, CargoSpec, CityId, Command, GameState, MarketOrder, Money, NpcKind, OrderId,
    OrderSide, PlayerId, ProductKind, Role, Tick,
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// NPC zorluk seviyesi. Oyun başlangıcında seçilir, tüm NPC'lere uygulanır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Difficulty {
    /// Basit likidite NPC'si — rastgele al/sat, strateji yok.
    #[default]
    Easy,
    /// Akıllı NPC — role göre fabrika/kervan kullanır, multi-emir verir.
    Hard,
    /// DSS NPC — kişilik arketipi + utility AI ile gerçek strateji.
    /// 7 farklı archetype (Aggressive/TrendFollower/MeanReverter/Arbitrageur/
    /// EventTrader/Hoarder/Cartel) seed RNG ile NPC'lere atanır.
    Expert,
}

impl Difficulty {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Easy => "Easy (basit likidite)",
            Self::Hard => "Hard (akıllı NPC, rekabetçi)",
            Self::Expert => "Expert (DSS kişilikli AI)",
        }
    }
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Easy => Self::Hard,
            Self::Hard => Self::Expert,
            Self::Expert => Self::Easy,
        }
    }
}

/// Bir NPC'nin bu tick için komut üreten davranışı.
pub trait NpcBehavior {
    fn decide(
        &self,
        state: &GameState,
        self_id: PlayerId,
        rng: &mut ChaCha8Rng,
        tick: Tick,
    ) -> Vec<Command>;
}

// =============================================================================
// MarketMaker — Easy (basit likidite)
// =============================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct MarketMaker;

impl NpcBehavior for MarketMaker {
    fn decide(
        &self,
        state: &GameState,
        self_id: PlayerId,
        rng: &mut ChaCha8Rng,
        tick: Tick,
    ) -> Vec<Command> {
        let Some(player) = state.players.get(&self_id) else {
            return Vec::new();
        };
        let ttl = state.config.balance.default_order_ttl;
        let city = pick_city(rng);
        let product = pick_product(rng);
        // Son clearing fiyatı varsa onu kullan (price discovery), yoksa base.
        let reference = market_or_base(state, city, product);
        let have = player.inventory.get(city, product);
        let order_id = OrderId::new(npc_order_id(self_id, tick, 0));

        if have > 0 {
            let qty = rng.random_range(1u32..=have.min(20));
            let sell_price = Money::from_cents(
                reference.as_cents() * moneywar_domain::balance::NPC_SELL_MARKUP_PCT / 100,
            );
            if let Ok(order) = MarketOrder::new_with_ttl(
                order_id,
                self_id,
                city,
                product,
                OrderSide::Sell,
                qty,
                sell_price,
                tick,
                ttl,
            ) {
                return vec![Command::SubmitOrder(order)];
            }
            return Vec::new();
        }

        let min_cash_cents = reference.as_cents().saturating_mul(10);
        if player.cash.as_cents() < min_cash_cents {
            return Vec::new();
        }
        let qty: u32 = rng.random_range(1u32..=10);
        let buy_price = Money::from_cents(
            reference.as_cents() * moneywar_domain::balance::NPC_BUY_MARKDOWN_PCT / 100,
        );
        MarketOrder::new_with_ttl(
            order_id,
            self_id,
            city,
            product,
            OrderSide::Buy,
            qty,
            buy_price,
            tick,
            ttl,
        )
        .map(|o| vec![Command::SubmitOrder(o)])
        .unwrap_or_default()
    }
}

// =============================================================================
// SmartTrader — Hard (role-aware)
// =============================================================================

#[derive(Debug, Default, Clone, Copy)]
pub struct SmartTrader;

impl NpcBehavior for SmartTrader {
    fn decide(
        &self,
        state: &GameState,
        self_id: PlayerId,
        rng: &mut ChaCha8Rng,
        tick: Tick,
    ) -> Vec<Command> {
        let Some(player) = state.players.get(&self_id) else {
            return Vec::new();
        };
        match player.role {
            Role::Sanayici => decide_sanayici(state, self_id, rng, tick),
            Role::Tuccar => decide_tuccar(state, self_id, rng, tick),
        }
    }
}

/// Sanayici NPC stratejisi — fabrika kur, ham madde al, üret, sat.
fn decide_sanayici(
    state: &GameState,
    pid: PlayerId,
    _rng: &mut ChaCha8Rng,
    tick: Tick,
) -> Vec<Command> {
    let mut cmds: Vec<Command> = Vec::new();
    let mut seq: u32 = 0;
    let player = state.players.get(&pid).expect("checked");
    let ttl = state.config.balance.default_order_ttl;

    let factory_count = u32::try_from(state.factories.values().filter(|f| f.owner == pid).count())
        .unwrap_or(u32::MAX);

    // 1) Sadece 1 fabrika cap. Matematik: 2. fabrika 15K maliyet, 30 batch ×
    // 10 × 12₺ marj = 3.6K ekstra kâr → -11.4K net. Tek fabrika kâr getirir.
    // Eski Hard SmartTrader 2-3 fabrika kuruyordu, sezon boyu zarar.
    if factory_count == 0 {
        let city = best_factory_city(state);
        let (_, product) = city_specialty(state, city);
        cmds.push(Command::BuildFactory {
            owner: pid,
            city,
            product,
        });
    }

    // 2) Her fabrika için ham madde al (üretim için).
    let my_factories: Vec<_> = state
        .factories
        .values()
        .filter(|f| f.owner == pid)
        .collect();
    for factory in &my_factories {
        let raw = factory.raw_input();
        let have_raw = player.inventory.get(factory.city, raw);
        if have_raw < 30 {
            // Az ham var → al. Piyasa fiyatından %5 yüksek (eşleşme şansı için).
            let target = market_or_base(state, factory.city, raw);
            let buy_price = Money::from_cents((target.as_cents() * 105) / 100);
            let qty = 30u32.saturating_sub(have_raw).min(20);
            let total = buy_price.as_cents().saturating_mul(i64::from(qty));
            if player.cash.as_cents() >= total {
                let id = OrderId::new(npc_order_id(pid, tick, seq));
                seq += 1;
                if let Ok(o) = MarketOrder::new_with_ttl(
                    id,
                    pid,
                    factory.city,
                    raw,
                    OrderSide::Buy,
                    qty,
                    buy_price,
                    tick,
                    ttl,
                ) {
                    cmds.push(Command::SubmitOrder(o));
                }
            }
        }
    }

    // 3) Bitmiş ürünleri sat — piyasa fiyatından %5 düşük (eşleşme önceliği).
    let mut finished_entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, p, q)| p.is_finished() && *q > 0)
        .collect();
    finished_entries.sort_by_key(|(_, _, q)| std::cmp::Reverse(*q));
    for (city, product, qty) in finished_entries.into_iter().take(2) {
        let target = market_or_base(state, city, product);
        let sell_price = Money::from_cents((target.as_cents() * 95) / 100);
        let sell_qty = qty.min(15);
        let id = OrderId::new(npc_order_id(pid, tick, seq));
        seq += 1;
        if let Ok(o) = MarketOrder::new_with_ttl(
            id,
            pid,
            city,
            product,
            OrderSide::Sell,
            sell_qty,
            sell_price,
            tick,
            ttl,
        ) {
            cmds.push(Command::SubmitOrder(o));
        }
    }

    cmds.truncate(4);
    cmds
}

/// Tüccar NPC stratejisi — arbitraj. Ucuz şehirden al, kervanla pahalı
/// şehre taşı, satma emri ver.
#[allow(clippy::too_many_lines)]
fn decide_tuccar(
    state: &GameState,
    pid: PlayerId,
    rng: &mut ChaCha8Rng,
    tick: Tick,
) -> Vec<Command> {
    let mut cmds: Vec<Command> = Vec::new();
    let mut seq: u32 = 0;
    let player = state.players.get(&pid).expect("checked");
    let ttl = state.config.balance.default_order_ttl;

    // 1) Kervan yoksa al — ilki bedava. Cap 2 — 2. kervan 6K maliyet,
    // arbitraj kâr 100₺/dispatch × ~20 dispatch/sezon = 2K. Net -4K, kabul
    // edilebilir. 3+ kervan zarar ekonomisi (eski 4 cap → -10K Tüccar).
    let caravan_count = u32::try_from(state.caravans.values().filter(|c| c.owner == pid).count())
        .unwrap_or(u32::MAX);
    let buy_caravan = caravan_count == 0
        || (caravan_count < 2
            && player.cash >= moneywar_domain::Caravan::buy_cost(Role::Tuccar, caravan_count)
            && rng.random_ratio(1, 4));
    if buy_caravan {
        cmds.push(Command::BuyCaravan {
            owner: pid,
            starting_city: pick_city(rng),
        });
    }

    // 2) Idle kervanları arbitraj fırsatı için dispatch et.
    let my_idle_caravans: Vec<&Caravan> = state
        .caravans
        .values()
        .filter(|c| c.owner == pid && c.is_idle())
        .collect();
    // Eski take(2) → take(4) — birden çok kervan aynı tickte dispatch eder.
    for caravan in my_idle_caravans.iter().take(4) {
        let here = caravan
            .state
            .current_city()
            .expect("idle caravan has location");
        let mut best: Option<(ProductKind, CityId, i64)> = None;
        for product in ProductKind::ALL {
            let here_price = market_or_base(state, here, product).as_cents();
            for to in CityId::ALL {
                if to == here {
                    continue;
                }
                let there_price = market_or_base(state, to, product).as_cents();
                let profit = there_price - here_price;
                // Eski 50 cents (0.50₺) → 25 cents — daha düşük kâr
                // eşiği, daha çok dispatch fırsatı.
                if profit > best.map_or(0, |(_, _, p)| p) && profit > 25 {
                    best = Some((product, to, profit));
                }
            }
        }
        if let Some((product, to, _)) = best {
            let have = player.inventory.get(here, product);
            let cap = caravan.capacity;
            let qty = have.min(cap);
            if qty > 0 {
                let mut cargo = CargoSpec::new();
                if cargo.add(product, qty).is_ok() {
                    cmds.push(Command::DispatchCaravan {
                        caravan_id: caravan.id,
                        from: here,
                        to,
                        cargo,
                    });
                    continue;
                }
            }
            // Stok yok — bu şehirde bu ürünü almak için Buy emri.
            let here_price = market_or_base(state, here, product);
            let buy_price = Money::from_cents((here_price.as_cents() * 105) / 100);
            let qty_target = cap.min(20);
            let total = buy_price.as_cents().saturating_mul(i64::from(qty_target));
            if player.cash.as_cents() >= total {
                let id = OrderId::new(npc_order_id(pid, tick, seq));
                seq += 1;
                if let Ok(o) = MarketOrder::new_with_ttl(
                    id,
                    pid,
                    here,
                    product,
                    OrderSide::Buy,
                    qty_target,
                    buy_price,
                    tick,
                    ttl,
                ) {
                    cmds.push(Command::SubmitOrder(o));
                }
            }
        }
    }

    // 3) Stoğu olan ürünleri bulundukları şehirde sat.
    // Raw ve finished dengesi önemli — eski kod top-2 qty alıyordu,
    // finished goods dominant kalınca ham madde HİÇ satılmıyordu ve
    // Sanayici fabrikaları aç kalıyordu. Yeni strateji:
    //   - En yüksek qty'li 1 raw (varsa)
    //   - En yüksek qty'li 1 finished (varsa)
    //   - Fallback: genel top-2
    let mut raw_entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, p, q)| p.is_raw() && *q > 0)
        .collect();
    raw_entries.sort_by_key(|(_, _, q)| std::cmp::Reverse(*q));

    let mut finished_entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, p, q)| p.is_finished() && *q > 0)
        .collect();
    finished_entries.sort_by_key(|(_, _, q)| std::cmp::Reverse(*q));

    let mut to_sell: Vec<(CityId, ProductKind, u32)> = Vec::new();
    if let Some(raw) = raw_entries.first() {
        to_sell.push(*raw);
    }
    if let Some(fin) = finished_entries.first() {
        to_sell.push(*fin);
    }
    // İki slot dolmadıysa genel listeyle tamamla.
    if to_sell.len() < 2 {
        let mut all: Vec<(CityId, ProductKind, u32)> = player
            .inventory
            .entries()
            .filter(|(_, _, q)| *q > 0)
            .collect();
        all.sort_by_key(|(_, _, q)| std::cmp::Reverse(*q));
        for e in all {
            if !to_sell.iter().any(|t| t.0 == e.0 && t.1 == e.1) {
                to_sell.push(e);
                if to_sell.len() >= 2 {
                    break;
                }
            }
        }
    }

    for (city, product, qty) in to_sell {
        let target = market_or_base(state, city, product);
        let sell_price = Money::from_cents((target.as_cents() * 105) / 100);
        let sell_qty = qty.min(15);
        let id = OrderId::new(npc_order_id(pid, tick, seq));
        seq += 1;
        if let Ok(o) = MarketOrder::new_with_ttl(
            id,
            pid,
            city,
            product,
            OrderSide::Sell,
            sell_qty,
            sell_price,
            tick,
            ttl,
        ) {
            cmds.push(Command::SubmitOrder(o));
        }
    }

    cmds.truncate(4);
    cmds
}

// =============================================================================
// AliciNpc — saf alıcı (kervan/fabrika yok, sadece buy emri)
// =============================================================================

/// Saf alıcı NPC. Seed'de `name` "NPC-Alıcı" prefix'i ile eklenen oyuncular
/// bu davranışı alır. Ne rol oynadıkları önemli değil — her tick 3 buy emri
/// verir, bias **finished goods**'a. Finished goods talebi piyasaya insan
/// oyuncunun üretimi için alıcı bulmasını sağlar.
#[derive(Debug, Default, Clone, Copy)]
pub struct AliciNpc;

impl NpcBehavior for AliciNpc {
    fn decide(
        &self,
        state: &GameState,
        self_id: PlayerId,
        rng: &mut ChaCha8Rng,
        tick: Tick,
    ) -> Vec<Command> {
        let Some(player) = state.players.get(&self_id) else {
            return Vec::new();
        };
        let ttl = state.config.balance.default_order_ttl;

        // 3 emir: %70 finished (oyuncu üretimi alıcı bulsun), %30 raw.
        // Deterministic: city × product RNG ile seçilir.
        let mut cmds = Vec::new();
        for seq in 0..3u32 {
            let product = if rng.random_ratio(7, 10) {
                // Finished goods — oyuncu üretimine talep.
                let idx = rng.random_range(0usize..ProductKind::FINISHED_GOODS.len());
                ProductKind::FINISHED_GOODS[idx]
            } else {
                pick_product(rng)
            };
            let city = pick_city(rng);
            let market = market_or_base(state, city, product);
            // %8 üzerine ödemeye hazır — Tüccar satışıyla (ask ~%105) overlap
            // yaratır, piyasa likiditesi kırılmasın. Her tick rastgele
            // %5-%10 arasında oynayarak fiyat dinamiği verir.
            let premium = rng.random_range(105u32..=110);
            let bid_price = Money::from_cents((market.as_cents() * i64::from(premium)) / 100);
            let qty: u32 = rng.random_range(10u32..=25);
            let total = bid_price.as_cents().saturating_mul(i64::from(qty));
            if player.cash.as_cents() < total {
                continue;
            }
            let id = OrderId::new(npc_order_id(self_id, tick, seq));
            if let Ok(o) = MarketOrder::new_with_ttl(
                id,
                self_id,
                city,
                product,
                OrderSide::Buy,
                qty,
                bid_price,
                tick,
                ttl,
            ) {
                cmds.push(Command::SubmitOrder(o));
            }
        }
        cmds
    }
}

// =============================================================================
// EsnafNpc — saf satıcı (dükkan, devasa stok, sürekli arz)
// =============================================================================

/// Saf satıcı NPC. Seed'de `name` "NPC-Esnaf" prefix'i olan oyuncular bu
/// davranışı alır. Her tick 4 sell emri verir, bias **raw materials**'a
/// (oyuncunun fabrika hammaddesi için talebi karşılar). Kervan/fabrika yok,
/// sadece stoğundan satar. Fiyat market × 102% (hafif markup, dump değil).
#[derive(Debug, Default, Clone, Copy)]
pub struct EsnafNpc;

impl NpcBehavior for EsnafNpc {
    fn decide(
        &self,
        state: &GameState,
        self_id: PlayerId,
        rng: &mut ChaCha8Rng,
        tick: Tick,
    ) -> Vec<Command> {
        let Some(player) = state.players.get(&self_id) else {
            return Vec::new();
        };
        let ttl = state.config.balance.default_order_ttl;

        // Sezon faza göre aktivite — Hasat döneminde piyasada arz çöküşünü
        // önlemek için Esnaflar daha aktif olur:
        // - Bahar/Yaz: %30 sessiz, 2-3 emir (mevcut)
        // - Hasat:     %10 sessiz, 3-5 emir (arz pompalama)
        let progress = state.season_progress();
        let (silence_ratio, max_orders) = if progress.is_late() {
            ((1, 10), 5)
        } else {
            ((3, 10), 3)
        };
        if rng.random_ratio(silence_ratio.0, silence_ratio.1) {
            return Vec::new();
        }

        let order_count = rng.random_range(2u32..=max_orders);
        let mut cmds = Vec::new();
        for seq in 0..order_count {
            let product = if rng.random_ratio(6, 10) {
                let idx = rng.random_range(0usize..ProductKind::RAW_MATERIALS.len());
                ProductKind::RAW_MATERIALS[idx]
            } else {
                let idx = rng.random_range(0usize..ProductKind::FINISHED_GOODS.len());
                ProductKind::FINISHED_GOODS[idx]
            };
            let city = pick_city(rng);
            let have = player.inventory.get(city, product);
            if have == 0 {
                continue;
            }
            let market = market_or_base(state, city, product);
            let ask_price = Money::from_cents((market.as_cents() * 102) / 100);
            let sell_qty = rng.random_range(10u32..=25).min(have);
            let id = OrderId::new(npc_order_id(self_id, tick, seq));
            if let Ok(o) = MarketOrder::new_with_ttl(
                id,
                self_id,
                city,
                product,
                OrderSide::Sell,
                sell_qty,
                ask_price,
                tick,
                ttl,
            ) {
                cmds.push(Command::SubmitOrder(o));
            }
        }
        cmds
    }
}

// =============================================================================
// SpekulatorNpc — market maker (spread doldurur, mallar bekleyici kalmasın)
// =============================================================================

/// Spekülatör NPC — her tick **iki** (city, product) için **hem bid hem ask**
/// emir verir. Bid market × 0.97, ask market × 1.03 → ~%6 spread, eşleşme
/// olmazsa NPC iki uçta da likidite tutuyor; başka oyuncu/NPC ortadan
/// rahatlıkla geçebilir. Net etki: pazar sürekli akar, "mallar bekliyor"
/// hissini öldürür.
///
/// Risk yönetimi: stoğu varsa sat, yoksa sadece bid (yarısı için bile yeterli).
/// Nakit < `bid_total` ise bid'i atla. Bu sayede iflas etmez ama tutarlı likidite verir.
#[derive(Debug, Default, Clone, Copy)]
pub struct SpekulatorNpc;

impl NpcBehavior for SpekulatorNpc {
    fn decide(
        &self,
        state: &GameState,
        self_id: PlayerId,
        rng: &mut ChaCha8Rng,
        tick: Tick,
    ) -> Vec<Command> {
        let Some(player) = state.players.get(&self_id) else {
            return Vec::new();
        };
        let ttl = state.config.balance.default_order_ttl;
        let mut cmds = Vec::new();
        let mut seq: u32 = 0;

        // 2 (city, product) seç ve her biri için bid+ask emir çifti üret.
        for _ in 0..2u32 {
            let city = pick_city(rng);
            let product = pick_product(rng);
            let market = market_or_base(state, city, product);
            let market_cents = market.as_cents();

            // Ask (sat) — stoğum varsa
            let have = player.inventory.get(city, product);
            if have > 0 {
                let ask_cents = (market_cents * 103) / 100;
                // Eski 5-15 birim arzı çok azdı; 10-25 birim ile spread
                // dolulu daha güçlü, sezon-sonu arz çöküşüne dirençli.
                let qty = have.min(rng.random_range(10u32..=25));
                let id = OrderId::new(npc_order_id(self_id, tick, seq));
                seq += 1;
                if let Ok(o) = MarketOrder::new_with_ttl(
                    id,
                    self_id,
                    city,
                    product,
                    OrderSide::Sell,
                    qty,
                    Money::from_cents(ask_cents),
                    tick,
                    ttl,
                ) {
                    cmds.push(Command::SubmitOrder(o));
                }
            }

            // Bid (al) — nakit yetiyorsa
            let bid_cents = (market_cents * 97) / 100;
            let qty = rng.random_range(10u32..=25);
            let total = bid_cents.saturating_mul(i64::from(qty));
            if player.cash.as_cents() >= total {
                let id = OrderId::new(npc_order_id(self_id, tick, seq));
                seq += 1;
                if let Ok(o) = MarketOrder::new_with_ttl(
                    id,
                    self_id,
                    city,
                    product,
                    OrderSide::Buy,
                    qty,
                    Money::from_cents(bid_cents),
                    tick,
                    ttl,
                ) {
                    cmds.push(Command::SubmitOrder(o));
                }
            }
        }

        cmds
    }
}

// =============================================================================
// Dispatcher
// =============================================================================

/// Tüm NPC'ler için bu tick'e ait komut setini, verilen zorluğa göre üret.
/// Pure-buyer ve pure-seller davranışları Difficulty'den bağımsız; diğerleri
/// Difficulty'ye göre `MarketMaker` (Easy) veya `SmartTrader` (Hard).
#[must_use]
pub fn decide_all_npcs(
    state: &GameState,
    rng: &mut ChaCha8Rng,
    tick: Tick,
    difficulty: Difficulty,
) -> Vec<Command> {
    let npc_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter_map(|(id, p)| if p.is_npc { Some(*id) } else { None })
        .collect();
    let mut cmds = Vec::new();
    for pid in npc_ids {
        // Dispatch — player.npc_kind structural ayrımına göre. Set edilmemiş
        // (None) NPC'ler `Difficulty`'ye göre genel davranışa düşer.
        let player = state.players.get(&pid);
        let kind = player.and_then(|p| p.npc_kind);
        let personality = player.and_then(|p| p.personality);
        let next: Vec<Command> = match kind {
            Some(NpcKind::Alici) => AliciNpc.decide(state, pid, rng, tick),
            Some(NpcKind::Esnaf) => EsnafNpc.decide(state, pid, rng, tick),
            Some(NpcKind::Spekulator) => SpekulatorNpc.decide(state, pid, rng, tick),
            // Sanayici / Tüccar → role-aware. Expert'te DSS, Hard'da SmartTrader,
            // Easy'de MarketMaker. Expert için personality lazım (yoksa SmartTrader fallback).
            _ => match (difficulty, personality, player.map(|p| p.role)) {
                (Difficulty::Expert, Some(p), Some(Role::Sanayici)) => {
                    crate::dss::sanayici::decide_sanayici_dss(state, pid, p, rng, tick)
                }
                (Difficulty::Expert, Some(p), Some(Role::Tuccar)) => {
                    crate::dss::tuccar::decide_tuccar_dss(state, pid, p, rng, tick)
                }
                (Difficulty::Hard, _, _) | (Difficulty::Expert, None, _) => {
                    SmartTrader.decide(state, pid, rng, tick)
                }
                _ => MarketMaker.decide(state, pid, rng, tick),
            },
        };
        cmds.extend(next);
    }
    cmds
}

// =============================================================================
// Helpers
// =============================================================================

fn pick_city(rng: &mut ChaCha8Rng) -> CityId {
    CityId::ALL[rng.random_range(0usize..CityId::ALL.len())]
}

/// NPC-Sanayici için en kârlı şehri seçer: o şehrin raw fiyatı ile finished
/// fiyatı arasındaki marj en yüksek olan. Fiyat son clearing → baseline → base
/// fallback sırasıyla okunur.
fn best_factory_city(state: &GameState) -> CityId {
    CityId::ALL
        .iter()
        .copied()
        .max_by_key(|city| {
            let (raw, finished) = city_specialty(state, *city);
            let raw_cents = market_or_base(state, *city, raw).as_cents();
            let fin_cents = market_or_base(state, *city, finished).as_cents();
            fin_cents.saturating_sub(raw_cents)
        })
        .unwrap_or(CityId::Istanbul)
}

/// Şehir uzmanlaşması — bu oyundaki (raw, finished) çifti. Eskiden
/// hard-coded'du; şimdi `state.cheap_raw_for(city)` üstünden state-aware →
/// her oyunda farklı eşleşme. `finished` her zaman raw'ın `finished_output`'u.
fn city_specialty(state: &GameState, city: CityId) -> (ProductKind, ProductKind) {
    let raw = state.cheap_raw_for(city);
    let finished = raw.finished_output().unwrap_or(ProductKind::Kumas);
    (raw, finished)
}

fn pick_product(rng: &mut ChaCha8Rng) -> ProductKind {
    ProductKind::ALL[rng.random_range(0usize..ProductKind::ALL.len())]
}

/// §10 kaba baz fiyatları — iskelet likidite fiyatlandırması.
fn base_price(product: ProductKind) -> Money {
    let lira = if product.is_raw() {
        moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA
    } else {
        moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA
    };
    Money::from_lira(lira).expect("fixed literal fits i64")
}

/// Pazarda son clearing fiyatı varsa onu, yoksa seed anında üretilmiş
/// `price_baseline`'ı (aktif şok varsa o çarpılmış), o da yoksa hardcoded
/// base'i döner. `SmartTrader`'ın "fair value" hesabı için.
///
/// Aktif şok mekanizması: olay motoru `state.active_shocks`'a yazar; NPC
/// fiyat referansı bunu otomatik içerir → kuraklık olduğunda NPC'ler
/// daha pahalıya satıp daha pahalıya alır → piyasa fiyatı yukarı kayar.
fn market_or_base(state: &GameState, city: CityId, product: ProductKind) -> Money {
    if let Some((_, last)) = state
        .price_history
        .get(&(city, product))
        .and_then(|v| v.last())
    {
        return *last;
    }
    if let Some(eff) = state.effective_baseline(city, product) {
        return eff;
    }
    base_price(product)
}

/// NPC `OrderId`: insan oyuncu havuzu ile çakışmasın diye yüksek ofsetli.
/// `seq` aynı tick'te birden çok emir verme imkânı verir (max 1000/oyuncu).
fn npc_order_id(player_id: PlayerId, tick: Tick, seq: u32) -> u64 {
    moneywar_domain::balance::NPC_ORDER_ID_OFFSET
        .saturating_add(u64::from(tick.value()).saturating_mul(100_000))
        .saturating_add((player_id.value() % 1_000).saturating_mul(100))
        .saturating_add(u64::from(seq).min(99))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{Money, Player, PlayerId, Role, RoomConfig, RoomId};
    use rand_chacha::rand_core::SeedableRng;

    fn state_with_npc() -> (GameState, PlayerId) {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut npc = Player::new(
            PlayerId::new(100),
            "NPC1",
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap();
        npc.inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 50)
            .unwrap();
        s.players.insert(npc.id, npc);
        (s, PlayerId::new(100))
    }

    fn fresh_rng() -> ChaCha8Rng {
        ChaCha8Rng::from_seed([42u8; 32])
    }

    #[test]
    fn market_maker_produces_at_most_one_command() {
        let (s, pid) = state_with_npc();
        let mut rng = fresh_rng();
        let cmds = MarketMaker.decide(&s, pid, &mut rng, Tick::new(1));
        assert!(cmds.len() <= 1);
    }

    #[test]
    fn decide_is_deterministic_for_same_rng() {
        let (s, pid) = state_with_npc();
        let mut rng_a = fresh_rng();
        let mut rng_b = fresh_rng();
        let a = MarketMaker.decide(&s, pid, &mut rng_a, Tick::new(1));
        let b = MarketMaker.decide(&s, pid, &mut rng_b, Tick::new(1));
        assert_eq!(a, b);
    }

    #[test]
    fn decide_all_skips_human_players() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let human = Player::new(
            PlayerId::new(1),
            "H",
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            false,
        )
        .unwrap();
        let mut npc1 = Player::new(
            PlayerId::new(2),
            "N1",
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap();
        npc1.inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 30)
            .unwrap();
        s.players.insert(human.id, human);
        s.players.insert(npc1.id, npc1);

        let mut rng = fresh_rng();
        let cmds = decide_all_npcs(&s, &mut rng, Tick::new(1), Difficulty::Easy);
        for c in &cmds {
            assert_ne!(c.requester(), PlayerId::new(1));
        }
    }

    #[test]
    fn npc_order_ids_do_not_collide_within_tick() {
        let mut ids = std::collections::BTreeSet::new();
        for i in 0..10u64 {
            for seq in 0..5u32 {
                let id = npc_order_id(PlayerId::new(i), Tick::new(1), seq);
                assert!(ids.insert(id), "collision at player {i} seq {seq}");
            }
        }
    }

    #[test]
    fn npc_without_stock_or_cash_produces_nothing() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let poor_npc =
            Player::new(PlayerId::new(1), "poor", Role::Tuccar, Money::ZERO, true).unwrap();
        s.players.insert(poor_npc.id, poor_npc);
        let mut rng = fresh_rng();
        let cmds = MarketMaker.decide(&s, PlayerId::new(1), &mut rng, Tick::new(1));
        assert!(cmds.is_empty());
    }

    #[test]
    fn smart_sanayici_builds_factory_when_none() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(1),
            "smartS",
            Role::Sanayici,
            Money::from_lira(50_000).unwrap(),
            true,
        )
        .unwrap();
        s.players.insert(npc.id, npc);
        let mut rng = fresh_rng();
        let cmds = SmartTrader.decide(&s, PlayerId::new(1), &mut rng, Tick::new(1));
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::BuildFactory { .. })),
            "Sanayici fabrika kurmayı denemeli: {cmds:?}"
        );
    }

    #[test]
    fn smart_tuccar_buys_caravan_when_none() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(1),
            "smartT",
            Role::Tuccar,
            Money::from_lira(50_000).unwrap(),
            true,
        )
        .unwrap();
        s.players.insert(npc.id, npc);
        let mut rng = fresh_rng();
        let cmds = SmartTrader.decide(&s, PlayerId::new(1), &mut rng, Tick::new(1));
        assert!(
            cmds.iter().any(|c| matches!(c, Command::BuyCaravan { .. })),
            "Tüccar kervan almayı denemeli: {cmds:?}"
        );
    }

    // -----------------------------------------------------------------------
    // AliciNpc testleri
    // -----------------------------------------------------------------------

    #[test]
    fn alici_produces_three_buy_orders_per_tick() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(1),
            "Selim Bey",
            Role::Tuccar,
            Money::from_lira(100_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        s.players.insert(npc.id, npc);
        let mut rng = fresh_rng();
        let cmds = AliciNpc.decide(&s, PlayerId::new(1), &mut rng, Tick::new(1));
        assert_eq!(cmds.len(), 3);
        for c in &cmds {
            match c {
                Command::SubmitOrder(o) => assert!(matches!(o.side, OrderSide::Buy)),
                other => panic!("alici sadece buy emri vermeli: {other:?}"),
            }
        }
    }

    #[test]
    fn alici_skips_orders_when_cash_insufficient() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let npc = Player::new(
            PlayerId::new(1),
            "Selim Bey",
            Role::Tuccar,
            Money::from_lira(50).unwrap(), // çok düşük — finished goods için yetmez
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        s.players.insert(npc.id, npc);
        let mut rng = fresh_rng();
        let cmds = AliciNpc.decide(&s, PlayerId::new(1), &mut rng, Tick::new(1));
        assert!(
            cmds.len() < 3,
            "düşük nakitle 3 emir verilememeli, {} verildi",
            cmds.len()
        );
    }

    #[test]
    fn dispatcher_routes_alici_kind_to_pure_buyer() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let alici = Player::new(
            PlayerId::new(1),
            "Zeynep Hanım",
            Role::Tuccar,
            Money::from_lira(100_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        s.players.insert(alici.id, alici);
        let mut rng = fresh_rng();
        let cmds = decide_all_npcs(&s, &mut rng, Tick::new(1), Difficulty::Hard);
        // Hard mode'da tüccar olsa kervan alırdı; AliciNpc sadece buy verir.
        assert!(cmds.iter().all(|c| matches!(c, Command::SubmitOrder(_))));
        assert_eq!(cmds.len(), 3);
    }
}
