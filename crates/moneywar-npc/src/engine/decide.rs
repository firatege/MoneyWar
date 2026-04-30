//! Fuzzy NPC karar motoru — orchestrator.
//!
//! Akış:
//! 1. Her (city, product) için `compute_inputs` ile sinyaller hesapla.
//! 2. Rol-spesifik `engine_for(role, kind)` → fuzzy çıkışlar (buy_score,
//!    sell_score, bid_aggressiveness, vb.) per-bucket.
//! 3. Tüm bucket'lardan en yüksek skorlu action adaylarını topla.
//! 4. Difficulty modulator: threshold + max_actions filter.
//! 5. Personality bias: utility multiplier (Faz 7'de detaylanır).
//! 6. Command emit.
//!
//! Output skorları (fuzzy):
//! - `buy_score`, `sell_score` → SubmitOrder
//! - `bid_aggressiveness`, `ask_aggressiveness` → fiyat hesabı
//! - `build_factory_score` → BuildFactory (Sanayici)
//! - `buy_caravan_score` → BuyCaravan (Tüccar)
//! - `dispatch_score` → DispatchCaravan
//! - `contract_score` → ProposeContract
//!
//! Modulator parametreleri (`Easy/Medium/Hard`):
//! - `max_actions_per_tick`: top-K filter
//! - `silence_ratio_per10`: rng skip
//! - `aggressiveness`: bid/ask fiyat scale
//! - `min_score_threshold`: utility kabul eşiği

use moneywar_domain::{
    CargoSpec, CityId, Command, ContractProposal, GameState, ListingKind, MarketOrder, Money,
    NpcKind, OrderId, OrderSide, Personality, Player, PlayerId, ProductKind, Role, Tick,
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::DifficultyModulator;
use crate::engine::inputs::compute_inputs;
use crate::engine::rules::engine_for;
use crate::fuzzy::Outputs;

/// Personality başına fuzzy output multiplier'ları. NPC'nin kişiliği fuzzy
/// kararlardan sonra çıkışları kendine göre eğer:
/// - Aggressive risk almayı sever, bid/ask agresif.
/// - Hoarder satmaz, biriktirir.
/// - Arbitrageur kervan/dispatch öncelikli.
/// - EventTrader olay-reaktif aksiyonlarda agresif.
/// - MeanReverter sakin, fiyat dalgalanmalarına az tepki.
/// - TrendFollower momentum sinyali zaten kuralda — bias düz.
/// - Cartel kontrat odaklı, uzun vadeli.
struct PersonalityBias {
    buy_score: f64,
    sell_score: f64,
    bid_aggressiveness: f64,
    ask_aggressiveness: f64,
    build_factory_score: f64,
    buy_caravan_score: f64,
    contract_score: f64,
}

impl PersonalityBias {
    const NEUTRAL: Self = Self {
        buy_score: 1.0,
        sell_score: 1.0,
        bid_aggressiveness: 1.0,
        ask_aggressiveness: 1.0,
        build_factory_score: 1.0,
        buy_caravan_score: 1.0,
        contract_score: 1.0,
    };

    fn for_personality(p: Personality) -> Self {
        match p {
            Personality::Aggressive => Self {
                buy_score: 1.2,
                sell_score: 1.0,
                bid_aggressiveness: 1.3,
                ask_aggressiveness: 1.3,
                build_factory_score: 1.2,
                buy_caravan_score: 1.2,
                contract_score: 1.0,
            },
            Personality::TrendFollower => Self::NEUTRAL,
            Personality::MeanReverter => Self {
                buy_score: 1.0,
                sell_score: 1.0,
                bid_aggressiveness: 0.9,
                ask_aggressiveness: 0.9,
                ..Self::NEUTRAL
            },
            Personality::Arbitrageur => Self {
                buy_score: 1.1,
                sell_score: 1.0,
                bid_aggressiveness: 1.0,
                ask_aggressiveness: 1.0,
                build_factory_score: 0.9,
                buy_caravan_score: 1.3,
                contract_score: 1.2,
            },
            Personality::EventTrader => Self {
                buy_score: 1.1,
                sell_score: 1.1,
                bid_aggressiveness: 1.2,
                ask_aggressiveness: 1.2,
                ..Self::NEUTRAL
            },
            Personality::Hoarder => Self {
                buy_score: 1.0,
                sell_score: 0.7,
                bid_aggressiveness: 0.8,
                ask_aggressiveness: 0.7,
                build_factory_score: 1.0,
                buy_caravan_score: 0.8,
                contract_score: 1.1,
            },
            Personality::Cartel => Self {
                buy_score: 1.1,
                sell_score: 1.0,
                bid_aggressiveness: 1.1,
                ask_aggressiveness: 1.1,
                build_factory_score: 1.0,
                buy_caravan_score: 1.0,
                contract_score: 1.3,
            },
        }
    }
}

fn apply_personality_bias(outputs: &mut Outputs, personality: Option<Personality>) {
    let Some(p) = personality else { return };
    let bias = PersonalityBias::for_personality(p);
    if let Some(v) = outputs.get_mut("buy_score") {
        *v = (*v * bias.buy_score).clamp(0.0, 1.5);
    }
    if let Some(v) = outputs.get_mut("sell_score") {
        *v = (*v * bias.sell_score).clamp(0.0, 1.5);
    }
    if let Some(v) = outputs.get_mut("bid_aggressiveness") {
        *v = (*v * bias.bid_aggressiveness).clamp(0.0, 1.5);
    }
    if let Some(v) = outputs.get_mut("ask_aggressiveness") {
        *v = (*v * bias.ask_aggressiveness).clamp(0.0, 1.5);
    }
    if let Some(v) = outputs.get_mut("build_factory_score") {
        *v = (*v * bias.build_factory_score).clamp(0.0, 1.5);
    }
    if let Some(v) = outputs.get_mut("buy_caravan_score") {
        *v = (*v * bias.buy_caravan_score).clamp(0.0, 1.5);
    }
    if let Some(v) = outputs.get_mut("contract_score") {
        *v = (*v * bias.contract_score).clamp(0.0, 1.5);
    }
}

/// Plan v5 Faz 12 — NPC tipi başına pulse/cadence (action periyodu).
///
/// Farklı rolllerin doğal ritmi farklı: Çiftçi yavaş üretir, Spekülatör
/// sürekli market maker, Banka çok seyrek. Bu fonksiyon `(tick % period == 0)`
/// kontrolü için periyodu döner — `decide_npc_fuzzy` skip eder.
///
/// Periyod 1 = her tick aktif. Bigger = daha seyrek.
fn pulse_period(npc_kind: Option<NpcKind>) -> u32 {
    match npc_kind {
        Some(NpcKind::Spekulator) => 2, // market maker: 2 tick'te 1 (emir spam'i azalsın, match oranı artsın)
        Some(NpcKind::Tuccar) => 1,     // tüccar: her tick (lojistik scanner — BUY+dispatch hızlı tetiklensin)
        Some(NpcKind::Sanayici) => 2,   // sanayici: 2 tick'te 1 (cadence 1 ekstra BUY → ham fiyatı yükseliyordu)
        Some(NpcKind::Esnaf) => 1,      // toptancı: her tick (pazar dolaşımı için)
        Some(NpcKind::Alici) => 3,      // alıcı: 3 tick'te 1 (tüketim ritmi)
        Some(NpcKind::Ciftci) => 1,     // çiftçi: her tick (eski 2 → ham arzı +%100, Sanayici 15 fabrikayı doyurmak için)
        Some(NpcKind::Banka) => 12,     // banka: çok seyrek (bank.rs aynısını kullanır)
        None => 1,                       // insan oyuncu (etki yok, NPC değil)
    }
}

/// Plan v4 Faz 2 — NPC tipi başına izin verilen aksiyonlar (single-task gate).
///
/// Her NPC kendi şeridinde kalsın diye:
/// - `Ciftci` → sadece SELL (ham madde)
/// - `Esnaf` (Toptancı) → BUY ham, SELL ham/mamul (aracı)
/// - `Sanayici` → BUY sadece kendi fabrikalarının `raw_input`'u, SELL sadece
///                kendi fabrikalarının ürünü
/// - `Tuccar` → BUY/SELL serbest ama ileride şehir-arbitraj gate eklenir
/// - `Alici` → BUY sadece mamul; SELL likidite kolu (her şey)
/// - `Spekulator` → market maker (her ikisi de serbest)
/// - `Banka` → normal market aksiyonu yok (özel akış)
fn allow_buy(player: &Player, state: &GameState, product: ProductKind) -> bool {
    match player.npc_kind {
        Some(NpcKind::Banka) | Some(NpcKind::Ciftci) => false,
        Some(NpcKind::Alici) => product.is_finished(),
        Some(NpcKind::Esnaf) => product.is_raw(),
        Some(NpcKind::Sanayici) => {
            if !product.is_raw() {
                return false;
            }
            state
                .factories
                .values()
                .any(|f| f.owner == player.id && f.product.raw_input() == Some(product))
        }
        Some(NpcKind::Tuccar) | Some(NpcKind::Spekulator) | None => true,
    }
}

fn allow_sell(player: &Player, state: &GameState, product: ProductKind) -> bool {
    match player.npc_kind {
        Some(NpcKind::Banka) => false,
        Some(NpcKind::Ciftci) => product.is_raw(),
        Some(NpcKind::Sanayici) => {
            if !product.is_finished() {
                return false;
            }
            state
                .factories
                .values()
                .any(|f| f.owner == player.id && f.product == product)
        }
        // Alıcı: TASARIMSAL TÜKETİCİ — SELL kapalı. Eski "likidite kolu" kuralı
        // (cash low → mamul sat) Alıcı'yı arbitraj yapan oyuncu yapıyordu.
        // PnL +45K (tek kazanan, Sanayici -33K kaybederken) bu yüzden.
        // Alıcı sadece BUY mamul, SELL yok. Cash krizi → maaş periyodik çözer.
        Some(NpcKind::Alici) => false,
        Some(NpcKind::Esnaf)
        | Some(NpcKind::Tuccar)
        | Some(NpcKind::Spekulator)
        | None => true,
    }
}

fn allow_build_factory(player: &Player) -> bool {
    !matches!(player.npc_kind, Some(NpcKind::Banka))
}

fn allow_contract(player: &Player) -> bool {
    !matches!(
        player.npc_kind,
        Some(NpcKind::Banka) | Some(NpcKind::Alici) | Some(NpcKind::Ciftci)
    )
}

fn allow_caravan(player: &Player) -> bool {
    // Çiftçi/Banka/Alici kervan kullanmaz — Tüccar/insan oyuncu serbest.
    matches!(player.npc_kind, Some(NpcKind::Tuccar) | None)
}

/// Plan v4 Faz 6 — Tüccar arbitraj gate.
///
/// Tüccar aynı şehirde BUY+SELL yapmasın diye:
/// - BUY izni: bu şehir, başka şehir ortalamasından **daha ucuz** (≥ %3 spread)
/// - SELL izni: bu şehir, başka şehir ortalamasından **daha pahalı** (≥ %3 spread)
///
/// Spread eşiği `ARBITRAGE_SPREAD_PCT` = 3 — kervan maliyeti ~%2-3 ile uyumlu.
/// İlk audit'te %5 hiç tetiklenmedi (90 tick); %3 daha gerçekçi.
const ARBITRAGE_SPREAD_PCT: i64 = 3;
const ARBITRAGE_WINDOW: usize = 5;

/// Tüccar arbitraj gate için fiyat — rolling avg yoksa baseline'a düş.
/// Sezon başında price_history boş, gate hiç tetiklenmiyordu (Tuning v6.5 fix).
fn arbitrage_price_cents(state: &GameState, city: CityId, product: ProductKind) -> Option<i64> {
    state
        .rolling_avg_price(city, product, ARBITRAGE_WINDOW)
        .or_else(|| state.effective_baseline(city, product))
        .map(|p| p.as_cents())
}

fn tuccar_buy_arbitrage_ok(state: &GameState, city: CityId, product: ProductKind) -> bool {
    let here = match arbitrage_price_cents(state, city, product) {
        Some(p) if p > 0 => p,
        _ => return false,
    };
    CityId::ALL.iter().filter(|&&c| c != city).any(|&c| {
        if let Some(there) = arbitrage_price_cents(state, c, product) {
            // diğer şehir burdan en az %3 pahalı olmalı (burası ucuz)
            (there - here) * 100 >= here * ARBITRAGE_SPREAD_PCT
        } else {
            false
        }
    })
}

fn tuccar_sell_arbitrage_ok(state: &GameState, city: CityId, product: ProductKind) -> bool {
    let here = match arbitrage_price_cents(state, city, product) {
        Some(p) if p > 0 => p,
        _ => return false,
    };
    CityId::ALL.iter().filter(|&&c| c != city).any(|&c| {
        if let Some(there) = arbitrage_price_cents(state, c, product) {
            // diğer şehir burdan en az %3 ucuz olmalı (burası pahalı)
            (here - there) * 100 >= here * ARBITRAGE_SPREAD_PCT
        } else {
            false
        }
    })
}

/// Fuzzy karar motoru — tüm (city, product) bucket'larını değerlendirir,
/// modulator filter sonrası `Command` listesi döner.
#[must_use]
pub fn decide_npc_fuzzy(
    state: &GameState,
    npc_id: PlayerId,
    modulator: DifficultyModulator,
    rng: &mut ChaCha8Rng,
) -> Vec<Command> {
    // Silence — Easy'de yarı tick pasif.
    if modulator.silence_ratio_per10 > 0
        && rng.random_ratio(modulator.silence_ratio_per10, 10)
    {
        return Vec::new();
    }

    let Some(player) = state.players.get(&npc_id) else {
        return Vec::new();
    };
    let role = player.role;
    let npc_kind = player.npc_kind;
    let personality = player.personality;

    // Plan v5 — NPC tipi pulse/cadence: bu tick aktif değilse skip.
    let period = pulse_period(npc_kind);
    if period > 1 {
        let next_t = state.current_tick.next().value();
        if next_t % period != 0 {
            return Vec::new();
        }
    }
    let engine = engine_for(role, npc_kind);
    let tick = state.current_tick.next();
    let ttl = state.config.balance.default_order_ttl;

    let mut candidates: Vec<(Command, f64)> = Vec::new();
    let mut seq: u32 = 0;

    // Her (city, product) için fuzzy değerlendir.
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let inputs = compute_inputs(state, npc_id, city, product);
            let mut outputs = engine.evaluate(&inputs);
            apply_personality_bias(&mut outputs, personality);

            let buy_score = outputs.get("buy_score").copied().unwrap_or(0.0);
            let sell_score = outputs.get("sell_score").copied().unwrap_or(0.0);
            let bid_aggro = outputs.get("bid_aggressiveness").copied().unwrap_or(0.5);
            let ask_aggro = outputs.get("ask_aggressiveness").copied().unwrap_or(0.5);

            // Tüccar için ek arbitraj gate — sadece spread varsa BUY/SELL.
            let tuccar_buy_ok = !matches!(player.npc_kind, Some(NpcKind::Tuccar))
                || tuccar_buy_arbitrage_ok(state, city, product);
            let tuccar_sell_ok = !matches!(player.npc_kind, Some(NpcKind::Tuccar))
                || tuccar_sell_arbitrage_ok(state, city, product);

            // Buy emir adayı — NPC tipi gate + (Tüccar) arbitraj gate.
            if allow_buy(player, state, product)
                && tuccar_buy_ok
                && buy_score >= modulator.min_score_threshold
                && buy_score > 0.3
            {
                if let Some(cmd) = build_buy_order(
                    state, npc_id, city, product, bid_aggro, modulator, tick, ttl, &mut seq,
                ) {
                    candidates.push((cmd, buy_score));
                }
            }

            // Sell emir adayı — NPC tipi gate + (Tüccar) arbitraj gate.
            if allow_sell(player, state, product)
                && tuccar_sell_ok
                && sell_score >= modulator.min_score_threshold
                && sell_score > 0.3
            {
                if let Some(cmd) = build_sell_order(
                    state, npc_id, city, product, ask_aggro, modulator, tick, ttl, &mut seq,
                ) {
                    candidates.push((cmd, sell_score));
                }
            }

            // BuildFactory adayı (Sanayici + ekonomik gate).
            //
            // Duplicate guard: aynı (city, product) için NPC'nin fabrikası
            // zaten varsa aday olarak ekleme.
            //
            // Şehir çeşitlendirmesi — NPC ID hash'iyle home_city ata
            // (NPC.id % 3 → Istanbul/Ankara/Izmir). Home_city + lokal mamul'a
            // büyük bonus → tüm Sanayici NPC'leri aynı 3 yere yığılmasın.
            //
            // EKONOMİK GATE (kullanıcı isteği) — fabrika kurarken Sanayici 3 koşulu
            // değerlendirsin:
            // 1. CASH BUFFER: build_cost + 10 batch'lık ham bütçesi <= cash
            //    (kuruluş sonrası fabrikayı besleyecek paran kalsın)
            // 2. HAM ARZ: o (city, raw_input) için ya rolling_avg ya da baseline
            //    var (piyasada potansiyel arz gözüküyor — Çiftçi/Tüccar erişebilir)
            // 3. TALEP: o (city, finished) için baseline var (Alıcı talebi olabilir)
            //
            // Hammaddenin ucuz olduğu şehir (cheap_raw match) varsa local market'tan
            // direkt alabilir; değilse Tüccar dispatch'i bekler. Ham erişimi yok ise
            // fabrikayı boş bırakır → cash sink. Bu gate önler.
            if matches!(role, Role::Sanayici) && allow_build_factory(player) {
                let already_has_factory = state
                    .factories
                    .values()
                    .any(|f| f.owner == npc_id && f.city == city && f.product == product);
                if !already_has_factory && product.is_finished() {
                    let build_score = outputs
                        .get("build_factory_score")
                        .copied()
                        .unwrap_or(0.0);
                    if build_score >= modulator.min_score_threshold
                        && build_score > 0.5
                    {
                        // Gate 1: cash buffer hesapla.
                        let owned_count = u32::try_from(
                            state.factories.values().filter(|f| f.owner == npc_id).count(),
                        )
                        .unwrap_or(u32::MAX);
                        let build_cost = moneywar_domain::Factory::build_cost(owned_count);
                        // 10 batch × 100 ham × ortalama baseline ~7₺ = 7K buffer.
                        let raw_buffer_cents: i64 = 10
                            * i64::from(moneywar_domain::Factory::BATCH_SIZE)
                            * 700; // 7₺ = 700 cent
                        let needed_cents = build_cost.as_cents() + raw_buffer_cents;
                        let cash_ok = player.cash.as_cents() >= needed_cents;

                        // Gate 2: ham arz erişimi — bu (city, raw) için piyasa fiyatı
                        // var mı (rolling_avg veya baseline)?
                        let raw_input = match product.raw_input() {
                            Some(r) => r,
                            None => continue,
                        };
                        let raw_supply_ok = state
                            .rolling_avg_price(city, raw_input, 5)
                            .or_else(|| state.effective_baseline(city, raw_input))
                            .is_some();

                        // Gate 3: mamul talep erişimi — bu (city, finished) için
                        // piyasa fiyatı var mı?
                        let demand_ok = state
                            .rolling_avg_price(city, product, 5)
                            .or_else(|| state.effective_baseline(city, product))
                            .is_some();

                        if cash_ok && raw_supply_ok && demand_ok {
                            let home_city = match npc_id.value() % 3 {
                                0 => CityId::Istanbul,
                                1 => CityId::Ankara,
                                _ => CityId::Izmir,
                            };
                            let home_bonus = if city == home_city { 0.15 } else { 0.0 };
                            candidates.push((
                                Command::BuildFactory {
                                    owner: npc_id,
                                    city,
                                    product,
                                },
                                build_score + home_bonus,
                            ));
                        }
                    }
                }
            }

            // Contract aday (Sanayici/Tüccar + gate).
            if matches!(role, Role::Sanayici | Role::Tuccar) && allow_contract(player) {
                let contract_score = outputs
                    .get("contract_score")
                    .copied()
                    .unwrap_or(0.0);
                if contract_score >= modulator.min_score_threshold && contract_score > 0.5 {
                    if let Some(cmd) = build_contract_proposal(
                        state, npc_id, city, product, ask_aggro, tick,
                    ) {
                        candidates.push((cmd, contract_score));
                    }
                }
            }
        }
    }

    // BuyCaravan adayı (Tüccar only) — global, role-spesifik bir tek (city,product) skoru.
    if matches!(role, Role::Tuccar) && allow_caravan(player) {
        // Pamuk + İstanbul örnek olarak kullan; aslında en yüksek arbitraj şehri.
        let inputs = compute_inputs(state, npc_id, CityId::Istanbul, ProductKind::Pamuk);
        let mut outputs = engine.evaluate(&inputs);
        apply_personality_bias(&mut outputs, personality);
        let caravan_score = outputs
            .get("buy_caravan_score")
            .copied()
            .unwrap_or(0.0);
        if caravan_score >= modulator.min_score_threshold && caravan_score > 0.4 {
            // İlk şehirden başlat (en arbitraj fırsatlısı orchestrator'da seçilebilir).
            candidates.push((
                Command::BuyCaravan {
                    owner: npc_id,
                    starting_city: CityId::Istanbul,
                },
                caravan_score,
            ));
        }
    }

    // Dispatch adayları (Tüccar): kervan idle + stok varsa, en yüksek skorlu (from→to).
    // Skor 1.5 → orchestrator top-K filter'da BUY/SELL/BuyCaravan adaylarının üstüne çıksın.
    // Aksi halde 18 bucket × 2 = 36 BUY/SELL adayı dispatch'ı top-5'ten dışarı atar.
    if matches!(role, Role::Tuccar) && allow_caravan(player) {
        if let Some(cmd) =
            build_dispatch_command(state, npc_id, &engine, modulator, tick, &mut seq)
        {
            candidates.push((cmd, 1.5));
        }
    }

    // Skora göre sırala (desc), threshold + top-K filter.
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });

    candidates
        .into_iter()
        .filter(|(_, s)| *s >= modulator.min_score_threshold)
        .take(modulator.max_actions_per_tick as usize)
        .map(|(c, _)| c)
        .collect()
}

/// BUY emri oluştur — fiyat market × bid_aggressiveness oranıyla.
#[allow(clippy::too_many_arguments)]
fn build_buy_order(
    state: &GameState,
    npc_id: PlayerId,
    city: CityId,
    product: ProductKind,
    bid_aggro: f64,
    modulator: DifficultyModulator,
    tick: Tick,
    ttl: u32,
    seq: &mut u32,
) -> Option<Command> {
    let player = state.players.get(&npc_id)?;
    let market = market_or_base(state, city, product);
    // Bid agresif: market × (1 + (bid_aggro - 0.5) × 0.5 × modulator.aggressiveness)
    // bid_aggro 0.5 = market, 1.0 = +%25 üstü → ask'larla daha çok karşılaşır.
    // Tune v12: scale 0.5 → 0.7 — fiyatlar daha agresif çapraz olur
    let aggro_factor = 1.0 + (bid_aggro - 0.5) * 0.7 * modulator.aggressiveness;
    let bid_cents = (market.as_cents() as f64 * aggro_factor) as i64;
    if bid_cents <= 0 {
        return None;
    }
    // Sanayici hammadde alımı: fabrikası varsa büyük al (üretim besler).
    // Diğerleri default qty cap.
    let is_sanayici_raw = matches!(player.role, Role::Sanayici) && product.is_raw();
    let factory_count = state
        .factories
        .values()
        .filter(|f| f.owner == npc_id)
        .count();
    let qty = if is_sanayici_raw && factory_count > 0 {
        pick_buy_qty_sanayici(player.cash.as_cents(), bid_cents, factory_count as u32)
    } else {
        pick_buy_qty(player.cash.as_cents(), bid_cents)
    };
    if qty == 0 {
        return None;
    }
    let id = OrderId::new(npc_decide_order_id(npc_id, tick, *seq));
    *seq += 1;
    MarketOrder::new_with_ttl(
        id,
        npc_id,
        city,
        product,
        OrderSide::Buy,
        qty,
        Money::from_cents(bid_cents),
        tick,
        ttl,
    )
    .ok()
    .map(Command::SubmitOrder)
}

/// SELL emri oluştur — fiyat market × ask_aggressiveness'a göre.
#[allow(clippy::too_many_arguments)]
fn build_sell_order(
    state: &GameState,
    npc_id: PlayerId,
    city: CityId,
    product: ProductKind,
    ask_aggro: f64,
    modulator: DifficultyModulator,
    tick: Tick,
    ttl: u32,
    seq: &mut u32,
) -> Option<Command> {
    let player = state.players.get(&npc_id)?;
    let stock = player.inventory.get(city, product);
    if stock == 0 {
        return None;
    }
    let market = market_or_base(state, city, product);
    // Ask agresif: market × (1 - (ask_aggro - 0.5) × 0.5 × modulator.aggressiveness)
    // ask_aggro 1.0 = -%25 (agresif satış), 0.5 = market.
    let aggro_factor = 1.0 - (ask_aggro - 0.5) * 0.7 * modulator.aggressiveness;
    let ask_cents = (market.as_cents() as f64 * aggro_factor) as i64;
    if ask_cents <= 0 {
        return None;
    }
    // Sanayici mamul satışı agresif — fabrikası varsa stok eritir.
    let is_sanayici_finished =
        matches!(player.role, Role::Sanayici) && product.is_finished();
    let factory_count = state
        .factories
        .values()
        .filter(|f| f.owner == npc_id)
        .count();
    let qty = if is_sanayici_finished && factory_count > 0 {
        pick_sell_qty_sanayici(stock)
    } else {
        stock.min(pick_sell_qty(stock))
    };
    if qty == 0 {
        return None;
    }
    let id = OrderId::new(npc_decide_order_id(npc_id, tick, *seq));
    *seq += 1;
    MarketOrder::new_with_ttl(
        id,
        npc_id,
        city,
        product,
        OrderSide::Sell,
        qty,
        Money::from_cents(ask_cents),
        tick,
        ttl,
    )
    .ok()
    .map(Command::SubmitOrder)
}

/// Tüccar dispatch — idle kervan varsa, en kârlı (from→to) çiftine.
fn build_dispatch_command(
    state: &GameState,
    npc_id: PlayerId,
    _engine: &crate::fuzzy::Engine,
    _modulator: DifficultyModulator,
    _tick: Tick,
    _seq: &mut u32,
) -> Option<Command> {
    let player = state.players.get(&npc_id)?;
    // Idle kervan
    let idle_caravan = state
        .caravans
        .values()
        .find(|c| c.owner == npc_id && c.is_idle())?;
    let from = idle_caravan.state.current_city()?;

    // En arbitraj fırsatlı (from, to, product) seç.
    // Sezon başında rolling_avg yok → effective_baseline'a fallback (arbitrage gate ile aynı pattern).
    let mut best: Option<(CityId, ProductKind, i64)> = None;
    for product in ProductKind::ALL {
        let stock = player.inventory.get(from, product);
        if stock < 10 {
            continue;
        }
        let Some(here) = state
            .rolling_avg_price(from, product, 5)
            .or_else(|| state.effective_baseline(from, product))
            .map(|p| p.as_cents())
        else {
            continue;
        };
        for to in CityId::ALL {
            if to == from {
                continue;
            }
            let Some(there_cents) = state
                .rolling_avg_price(to, product, 5)
                .or_else(|| state.effective_baseline(to, product))
                .map(|p| p.as_cents())
            else {
                continue;
            };
            let profit = there_cents - here;
            if profit > 25 && best.map_or(true, |(_, _, p)| profit > p) {
                best = Some((to, product, profit));
            }
        }
    }
    let (to, product, _profit) = best?;
    let qty = player.inventory.get(from, product).min(idle_caravan.capacity);
    if qty == 0 {
        return None;
    }
    let mut cargo = CargoSpec::default();
    cargo.add(product, qty).ok()?;
    Some(Command::DispatchCaravan {
        caravan_id: idle_caravan.id,
        from,
        to,
        cargo,
    })
}

/// Sanayici/Tüccar için kontrat öneri oluştur — public listing, mamul satışı.
/// Stoğu varsa SAT kontratı (gelecek tick'te teslim).
fn build_contract_proposal(
    state: &GameState,
    npc_id: PlayerId,
    city: CityId,
    product: ProductKind,
    ask_aggro: f64,
    tick: Tick,
) -> Option<Command> {
    let player = state.players.get(&npc_id)?;
    let stock = player.inventory.get(city, product);
    if stock < 30 {
        return None;
    }
    let market = market_or_base(state, city, product);
    // Kontrat fiyatı market × (1 + ask_aggro × 0.05) — küçük markup.
    let unit_cents = (market.as_cents() as f64 * (1.0 + (ask_aggro - 0.5) * 0.1)) as i64;
    if unit_cents <= 0 {
        return None;
    }
    let qty = stock.min(80).max(30);
    // Deposit küçük: NPC için kontrat zorunlu değil.
    let deposit_cents = (unit_cents.saturating_mul(i64::from(qty))) / 20; // %5
    let deposit = Money::from_cents(deposit_cents.max(100));
    // Cash kontrol — yetmezse atla.
    if player.cash.as_cents() < deposit_cents * 2 {
        return None;
    }
    let delivery = tick.checked_add(8).unwrap_or(tick); // 8 tick sonra teslim
    let proposal = ContractProposal {
        seller: npc_id,
        listing: ListingKind::Public,
        product,
        quantity: qty,
        unit_price: Money::from_cents(unit_cents),
        delivery_city: city,
        delivery_tick: delivery,
        seller_deposit: deposit,
        buyer_deposit: deposit,
    };
    Some(Command::ProposeContract(proposal))
}

fn market_or_base(state: &GameState, city: CityId, product: ProductKind) -> Money {
    state
        .rolling_avg_price(city, product, 5)
        .or_else(|| state.effective_baseline(city, product))
        .unwrap_or_else(|| {
            // fallback: ham 6, mamul 18
            Money::from_lira(if product.is_finished() { 18 } else { 6 }).unwrap()
        })
}

/// Cash kapasitesine göre alım miktarı (cash'in %15'i, cap 150).
/// Eski %25/cap 400 → büyük emirler match verimi düşürüyordu (matched_qty
/// orantısı: küçük emirler tam dolma şansı yüksek). Hedef match verim %5+.
fn pick_buy_qty(cash_cents: i64, bid_cents: i64) -> u32 {
    if bid_cents <= 0 {
        return 0;
    }
    let budget = cash_cents * 12 / 100; // %12
    let max_qty = budget / bid_cents;
    let capped = max_qty.clamp(0, 120);
    u32::try_from(capped).unwrap_or(0)
}

/// Sanayici hammadde alımı — fabrika sayısı × 100 birim hedef. Cash sınırlı.
/// 1 fabrika 30 batch × 2 ham = 60 ham/tick tüketir; 100 birim 2-3 tickte tüketilir.
fn pick_buy_qty_sanayici(cash_cents: i64, bid_cents: i64, factory_count: u32) -> u32 {
    if bid_cents <= 0 || factory_count == 0 {
        return 0;
    }
    let target = factory_count.saturating_mul(100); // hedef qty
    // Cash bütçesi %35 — Sanayici fabrika besleme önceliği.
    let budget = cash_cents * 35 / 100;
    let cash_capped = budget / bid_cents;
    let max_qty =
        u32::try_from(cash_capped.clamp(0, i64::from(u32::MAX))).unwrap_or(0);
    target.min(max_qty)
}

/// Stok'a göre satım miktarı (stoğun %20'si, cap 80).
/// Eski %40/cap 250 → küçük emirlere geçiş match verim hedefi için (%5+).
fn pick_sell_qty(stock: u32) -> u32 {
    let q = stock / 5;
    q.clamp(10, 80).min(stock)
}

/// Sanayici mamul satışı — fabrikalar üretim biriktirir, stoğun %40'ını sat,
/// cap 200. Cash sink önler (mamul birikmiyor). Eski cap 400 → 200 küçültüldü
/// ki match verim hedefi yakalansın.
fn pick_sell_qty_sanayici(stock: u32) -> u32 {
    let q = (stock * 2) / 5;
    q.clamp(40, 200).min(stock)
}

/// Fuzzy NPC order ID (DSS NPC_ORDER_ID_OFFSET ile uyumsuz tutmayalım — distinct prefix).
fn npc_decide_order_id(player_id: PlayerId, tick: Tick, seq: u32) -> u64 {
    moneywar_domain::balance::NPC_ORDER_ID_OFFSET
        .saturating_add(u64::from(tick.value()).saturating_mul(100_000))
        .saturating_add((player_id.value() % 1_000).saturating_mul(100))
        .saturating_add(u64::from(seq).min(99))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Difficulty;
    use moneywar_domain::{Money, NpcKind, Player, PlayerId, Role, RoomConfig, RoomId};
    use rand_chacha::rand_core::SeedableRng;

    fn fresh_state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn add_npc(state: &mut GameState, id: u64, role: Role, cash: i64, kind: NpcKind) -> PlayerId {
        let pid = PlayerId::new(id);
        let p = Player::new(
            pid,
            format!("NPC-{id}"),
            role,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(kind);
        state.players.insert(pid, p);
        pid
    }

    #[test]
    fn rich_sanayici_emits_actions_at_medium() {
        let mut s = fresh_state();
        // Plan v5 pulse: Sanayici 3 tick'te 1 → next_tick % 3 == 0 olsun.
        s.current_tick = moneywar_domain::Tick::new(2);
        let pid = add_npc(&mut s, 100, Role::Sanayici, 40_000, NpcKind::Sanayici);
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_npc_fuzzy(&s, pid, Difficulty::Hard.modulator(), &mut rng);
        // Hard'da silence az, threshold negatif → en az 1 action emit etmeli
        assert!(!cmds.is_empty(), "rich sanayici Hard'da action emit etmeli");
    }

    #[test]
    fn easy_modulator_caps_at_one_action() {
        let mut s = fresh_state();
        let pid = add_npc(&mut s, 100, Role::Sanayici, 40_000, NpcKind::Sanayici);
        let mut rng = ChaCha8Rng::from_seed([42u8; 32]);
        let cmds = decide_npc_fuzzy(&s, pid, Difficulty::Easy.modulator(), &mut rng);
        // Easy'de max 1 action (silence atlasa 0)
        assert!(cmds.len() <= 1, "Easy modulator max 1 action (got {})", cmds.len());
    }

    #[test]
    fn missing_npc_returns_empty() {
        let s = fresh_state();
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_npc_fuzzy(&s, PlayerId::new(999), Difficulty::Hard.modulator(), &mut rng);
        assert!(cmds.is_empty());
    }

    #[test]
    fn deterministic_for_same_seed() {
        let mut s = fresh_state();
        let pid = add_npc(&mut s, 100, Role::Tuccar, 30_000, NpcKind::Tuccar);
        let mut r1 = ChaCha8Rng::from_seed([42u8; 32]);
        let mut r2 = ChaCha8Rng::from_seed([42u8; 32]);
        let c1 = decide_npc_fuzzy(&s, pid, Difficulty::Hard.modulator(), &mut r1);
        let c2 = decide_npc_fuzzy(&s, pid, Difficulty::Hard.modulator(), &mut r2);
        assert_eq!(c1, c2);
    }

    #[test]
    fn poor_npc_does_not_emit_buys() {
        let mut s = fresh_state();
        let pid = add_npc(&mut s, 100, Role::Tuccar, 200, NpcKind::Tuccar);
        let mut rng = ChaCha8Rng::from_seed([0u8; 32]);
        let cmds = decide_npc_fuzzy(&s, pid, Difficulty::Hard.modulator(), &mut rng);
        // Fakir NPC buy emri vermez (bankruptcy_risk yüksek)
        let buys = cmds.iter().filter(|c| matches!(c, Command::SubmitOrder(o) if matches!(o.side, OrderSide::Buy))).count();
        assert_eq!(buys, 0, "fakir tüccar buy emir vermemeli");
    }
}
