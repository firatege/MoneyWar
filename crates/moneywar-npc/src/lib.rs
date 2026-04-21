//! NPC davranış implementasyonları — Faz 8 iskelet.
//!
//! # Mimari
//!
//! - [`NpcBehavior`] trait: tek giriş noktası `decide(state, pid, rng, tick)`.
//! - [`MarketMaker`] (v1 iskelet): basit likidite — stok varsa sat, nakit
//!   varsa al. Rol / kişilik yok.
//! - v1.1'de `SanayiciNpc` / `TuccarNpc` (rol-native davranışlar).
//! - v2'de `AggressiveSpeculator` / `Hoarder` / `KartelLider` (persona).
//!
//! Persona atama şu an sabit `MarketMaker`; ileride `GameState.npc_personas`
//! benzeri bir map ile oyuncu-bazlı seçilebilir.
//!
//! # Determinism
//!
//! `decide` RNG alır; aynı (state, rng) → aynı komut seti. Motor, NPC'leri
//! `decide_all_npcs` üzerinden sıralı işler (`BTreeMap` `player_id` ASC).

mod error;

pub use error::NpcError;

use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, OrderId, OrderSide, PlayerId, ProductKind, Tick,
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// Bir NPC'nin bu tick için komut üreten davranışı. Trait-based; persona /
/// rol bazlı varyantlar aynı trait'i uygular.
pub trait NpcBehavior {
    /// Bu tick için NPC'nin atacağı komutlar. Bu NPC'nin `is_npc == true`
    /// olduğu arayan tarafından garantilenir.
    fn decide(
        &self,
        state: &GameState,
        self_id: PlayerId,
        rng: &mut ChaCha8Rng,
        tick: Tick,
    ) -> Vec<Command>;
}

/// v1 iskelet: piyasa likiditesi sağlar. Stok varsa base × 1.1'de Sell,
/// yoksa yeterli cash varsa base × 0.9'da Buy.
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
        let order_id = OrderId::new(npc_order_id(self_id, tick));

        if have > 0 {
            let qty = rng.random_range(1u32..=have.min(20));
            let sell_price = Money::from_cents(base.as_cents() * 110 / 100);
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
        let buy_price = Money::from_cents(base.as_cents() * 90 / 100);
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

/// Tüm NPC'ler için bu tick'e ait komut setini üret. Persona atama şu an
/// sabit `MarketMaker`; ileride `PlayerId → Box<dyn NpcBehavior>` map'i.
#[must_use]
pub fn decide_all_npcs(state: &GameState, rng: &mut ChaCha8Rng, tick: Tick) -> Vec<Command> {
    let behavior = MarketMaker;
    let npc_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter_map(|(id, p)| if p.is_npc { Some(*id) } else { None })
        .collect();
    let mut cmds = Vec::new();
    for pid in npc_ids {
        cmds.extend(behavior.decide(state, pid, rng, tick));
    }
    cmds
}

fn pick_city(rng: &mut ChaCha8Rng) -> CityId {
    CityId::ALL[rng.random_range(0usize..CityId::ALL.len())]
}

fn pick_product(rng: &mut ChaCha8Rng) -> ProductKind {
    ProductKind::ALL[rng.random_range(0usize..ProductKind::ALL.len())]
}

/// §10 kaba baz fiyatları — iskelet likidite fiyatlandırması.
fn base_price(product: ProductKind) -> Money {
    let lira = match product {
        ProductKind::Pamuk | ProductKind::Bugday | ProductKind::Zeytin => 6,
        ProductKind::Kumas | ProductKind::Un | ProductKind::Zeytinyagi => 15,
    };
    Money::from_lira(lira).expect("fixed literal fits i64")
}

/// NPC `OrderId`: insan oyuncu havuzu ile çakışmasın diye yüksek ofsetli,
/// `(tick, player_id)`'den deterministik türetilir.
fn npc_order_id(player_id: PlayerId, tick: Tick) -> u64 {
    const NPC_ID_OFFSET: u64 = 10_000_000_000;
    NPC_ID_OFFSET
        .saturating_add(u64::from(tick.value()).saturating_mul(1_000))
        .saturating_add(player_id.value() % 1_000)
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
        let cmds = decide_all_npcs(&s, &mut rng, Tick::new(1));
        for c in &cmds {
            assert_ne!(c.requester(), PlayerId::new(1));
        }
    }

    #[test]
    fn npc_order_ids_do_not_collide_within_tick() {
        let mut ids = std::collections::BTreeSet::new();
        for i in 0..10u64 {
            let id = npc_order_id(PlayerId::new(i), Tick::new(1));
            assert!(ids.insert(id), "collision at {i}");
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
}
