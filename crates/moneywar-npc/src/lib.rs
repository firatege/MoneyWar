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

mod error;

pub use error::NpcError;

use moneywar_domain::{
    Caravan, CargoSpec, CityId, Command, Factory, GameState, MarketOrder, Money, OrderId,
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
}

impl Difficulty {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Easy => "Easy (basit likidite)",
            Self::Hard => "Hard (akıllı NPC, rekabetçi)",
        }
    }
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Easy => Self::Hard,
            Self::Hard => Self::Easy,
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
        let city = pick_city(rng);
        let product = pick_product(rng);
        let base = base_price(product);
        let have = player.inventory.get(city, product);
        let order_id = OrderId::new(npc_order_id(self_id, tick, 0));

        if have > 0 {
            let qty = rng.random_range(1u32..=have.min(20));
            let sell_price = Money::from_cents(
                base.as_cents() * moneywar_domain::balance::NPC_SELL_MARKUP_PCT / 100,
            );
            if let Ok(order) = MarketOrder::new(
                order_id,
                self_id,
                city,
                product,
                OrderSide::Sell,
                qty,
                sell_price,
                tick,
            ) {
                return vec![Command::SubmitOrder(order)];
            }
            return Vec::new();
        }

        let min_cash_cents = base.as_cents().saturating_mul(10);
        if player.cash.as_cents() < min_cash_cents {
            return Vec::new();
        }
        let qty: u32 = rng.random_range(1u32..=10);
        let buy_price = Money::from_cents(
            base.as_cents() * moneywar_domain::balance::NPC_BUY_MARKDOWN_PCT / 100,
        );
        MarketOrder::new(
            order_id,
            self_id,
            city,
            product,
            OrderSide::Buy,
            qty,
            buy_price,
            tick,
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
    rng: &mut ChaCha8Rng,
    tick: Tick,
) -> Vec<Command> {
    let mut cmds: Vec<Command> = Vec::new();
    let mut seq: u32 = 0;
    let player = state.players.get(&pid).expect("checked");

    let factory_count = u32::try_from(state.factories.values().filter(|f| f.owner == pid).count())
        .unwrap_or(u32::MAX);

    // 1) Fabrika yoksa kur — ilki bedava. RNG ile şehir/ürün seç (deterministik).
    if factory_count == 0 {
        let city = pick_city(rng);
        let product = match city {
            CityId::Istanbul => ProductKind::Kumas,
            CityId::Ankara => ProductKind::Un,
            CityId::Izmir => ProductKind::Zeytinyagi,
        };
        cmds.push(Command::BuildFactory {
            owner: pid,
            city,
            product,
        });
    } else if factory_count < 3
        && player.cash >= Factory::build_cost(factory_count)
        && rng.random_ratio(1, 5)
    {
        // %20 olasılık ile yeni fabrika kur (eğer cash yeterli ve <3 fabrika varsa).
        let city = pick_city(rng);
        let product = match city {
            CityId::Istanbul => ProductKind::Kumas,
            CityId::Ankara => ProductKind::Un,
            CityId::Izmir => ProductKind::Zeytinyagi,
        };
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
                if let Ok(o) = MarketOrder::new(
                    id,
                    pid,
                    factory.city,
                    raw,
                    OrderSide::Buy,
                    qty,
                    buy_price,
                    tick,
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
        if let Ok(o) = MarketOrder::new(
            id,
            pid,
            city,
            product,
            OrderSide::Sell,
            sell_qty,
            sell_price,
            tick,
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

    // 1) Kervan yoksa al — ilki bedava.
    let caravan_count = u32::try_from(state.caravans.values().filter(|c| c.owner == pid).count())
        .unwrap_or(u32::MAX);
    if caravan_count == 0 {
        cmds.push(Command::BuyCaravan {
            owner: pid,
            starting_city: pick_city(rng),
        });
    } else if caravan_count < 3
        && player.cash >= moneywar_domain::Caravan::buy_cost(Role::Tuccar, caravan_count)
        && rng.random_ratio(1, 6)
    {
        // %16 olasılık ile yeni kervan al.
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
    for caravan in my_idle_caravans.iter().take(2) {
        let here = caravan
            .state
            .current_city()
            .expect("idle caravan has location");
        // En kârlı (product, to_city) çiftini bul.
        let mut best: Option<(ProductKind, CityId, i64)> = None;
        for product in ProductKind::ALL {
            let here_price = market_or_base(state, here, product).as_cents();
            for to in CityId::ALL {
                if to == here {
                    continue;
                }
                let there_price = market_or_base(state, to, product).as_cents();
                let profit = there_price - here_price;
                if profit > best.map_or(0, |(_, _, p)| p) && profit > 50 {
                    // 0.50₺/birim üstü kâr beklenirse aday.
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
                if let Ok(o) = MarketOrder::new(
                    id,
                    pid,
                    here,
                    product,
                    OrderSide::Buy,
                    qty_target,
                    buy_price,
                    tick,
                ) {
                    cmds.push(Command::SubmitOrder(o));
                }
            }
        }
    }

    // 3) Stoğu olan ürünleri bulundukları şehirde sat (kervan yoksa veya
    //    arbitraj fırsatı yoksa likidite ver).
    let mut entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, _, q)| *q > 0)
        .collect();
    entries.sort_by_key(|(_, _, q)| std::cmp::Reverse(*q));
    for (city, product, qty) in entries.into_iter().take(2) {
        let target = market_or_base(state, city, product);
        let sell_price = Money::from_cents((target.as_cents() * 105) / 100);
        let sell_qty = qty.min(15);
        let id = OrderId::new(npc_order_id(pid, tick, seq));
        seq += 1;
        if let Ok(o) = MarketOrder::new(
            id,
            pid,
            city,
            product,
            OrderSide::Sell,
            sell_qty,
            sell_price,
            tick,
        ) {
            cmds.push(Command::SubmitOrder(o));
        }
    }

    cmds.truncate(4);
    cmds
}

// =============================================================================
// Dispatcher
// =============================================================================

/// Tüm NPC'ler için bu tick'e ait komut setini, verilen zorluğa göre üret.
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
        let next: Vec<Command> = match difficulty {
            Difficulty::Easy => MarketMaker.decide(state, pid, rng, tick),
            Difficulty::Hard => SmartTrader.decide(state, pid, rng, tick),
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

/// Pazarda son clearing fiyatı varsa onu, yoksa baz fiyatı döner.
/// `SmartTrader`'ın "fair value" hesabı için.
fn market_or_base(state: &GameState, city: CityId, product: ProductKind) -> Money {
    state
        .price_history
        .get(&(city, product))
        .and_then(|v| v.last())
        .map_or_else(|| base_price(product), |(_, p)| *p)
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
}
