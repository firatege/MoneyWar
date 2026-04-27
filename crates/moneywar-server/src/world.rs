//! Server-side initial `GameState` üretici.
//!
//! Sprint 2: lobby'deki insan oyuncuları + 8-10 NPC ile ilk state'i kurar.
//! `RoomConfig` Hızlı preset, `RoomId` lobby'den. NPC kompozisyonu human
//! sayısına göre uyarlanır (toplam 12 oyuncu hedefi: 4 insan → 8 NPC,
//! 2 insan → 10 NPC).
//!
//! Bu modül CLI'daki `seed_world` mantığının küçük bir paralelidir.
//! Sprint 3+'da CLI ve server tek bir `moneywar-domain::seed_world`
//! fonksiyonuna refactor edilebilir; şimdilik kopya kabul edilebilir
//! çünkü server side seed'i deterministik kontrol etmek isteyeceğiz.

#![allow(clippy::cast_possible_truncation)]

use moneywar_domain::{
    CityId, GameState, Money, NpcKind, Personality, Player, ProductKind, Role, RoomConfig, RoomId,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::lobby::Lobby;

/// Lobi durumundan deterministik initial state üret.
pub fn build_initial_state(lobby: &Lobby) -> GameState {
    let room_id = lobby.room_id;
    let config = RoomConfig::hizli();
    let mut state = GameState::new(room_id, config);

    // RNG seed — room_id'den. Aynı room_id → aynı NPC isimleri, aynı stoklar.
    let mut rng = rng_from_room(room_id);

    // 1. İnsan oyuncuları lobi sırasıyla ekle.
    for (player_id, slot) in &lobby.slots {
        let role = slot.role.unwrap_or(Role::Tuccar);
        let starting_cash = match role {
            Role::Sanayici => 25_000_i64,
            Role::Tuccar => 40_000_i64,
        };
        let mut human = Player::new(
            *player_id,
            slot.name.clone(),
            role,
            Money::from_lira(starting_cash).unwrap(),
            false,
        )
        .expect("valid player");
        // Sanayici için starter raw (CLI ile uyumlu, 10× ölçekli).
        if matches!(role, Role::Sanayici) {
            let city_idx = rng.random_range(0..CityId::ALL.len());
            let starter_city = CityId::ALL[city_idx];
            let starter_raw = state.cheap_raw_for(starter_city);
            let starter_qty: u32 = rng.random_range(700..=1_300);
            let _ = human.inventory.add(starter_city, starter_raw, starter_qty);
        }
        state.players.insert(*player_id, human);
    }

    // 2. NPC kompozisyonu: 12 - human_count
    let human_count = lobby.slots.len() as u32;
    let total_npc = 12u32.saturating_sub(human_count).max(8);
    // Dağılım: 30% Tüccar, 25% Sanayici, 25% Esnaf, 15% Alıcı, 5% Spekülatör (kabaca)
    let tuccar = (total_npc * 3 / 10).max(1);
    let sanayici = (total_npc / 4).max(1);
    let esnaf = (total_npc / 4).max(1);
    let alici = (total_npc * 3 / 20).max(1);
    let spekulator = total_npc.saturating_sub(tuccar + sanayici + esnaf + alici);

    let mut next_id = lobby.slots.keys().map(|p| p.value()).max().unwrap_or(0) + 1;

    // Tüccar
    for i in 0..tuccar {
        let pid = moneywar_domain::PlayerId::new(next_id);
        next_id += 1;
        let personality = Personality::ALL[i as usize % Personality::ALL.len()];
        let mut npc = Player::new(
            pid,
            format!("Tüccar-{}", i + 1),
            Role::Tuccar,
            Money::from_lira(15_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Tuccar)
        .with_personality(personality);
        seed_distribute(&mut npc, &mut rng, 5_000);
        state.players.insert(pid, npc);
    }

    // Sanayici
    for i in 0..sanayici {
        let pid = moneywar_domain::PlayerId::new(next_id);
        next_id += 1;
        let personality = Personality::ALL[i as usize % Personality::ALL.len()];
        let mut npc = Player::new(
            pid,
            format!("Sanayici-{}", i + 1),
            Role::Sanayici,
            Money::from_lira(30_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Sanayici)
        .with_personality(personality);
        let city_idx = rng.random_range(0..CityId::ALL.len());
        let starter_city = CityId::ALL[city_idx];
        let starter_raw = state.cheap_raw_for(starter_city);
        let raw_qty: u32 = rng.random_range(300..=500);
        let _ = npc.inventory.add(starter_city, starter_raw, raw_qty);
        let fin_qty: u32 = rng.random_range(100..=200);
        let finished_idx = rng.random_range(0..ProductKind::FINISHED_GOODS.len());
        let _ = npc.inventory.add(
            starter_city,
            ProductKind::FINISHED_GOODS[finished_idx],
            fin_qty,
        );
        state.players.insert(pid, npc);
    }

    // Alıcı
    for i in 0..alici {
        let pid = moneywar_domain::PlayerId::new(next_id);
        next_id += 1;
        let npc = Player::new(
            pid,
            format!("Alıcı-{}", i + 1),
            Role::Tuccar,
            Money::from_lira(100_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        state.players.insert(pid, npc);
    }

    // Esnaf
    for i in 0..esnaf {
        let pid = moneywar_domain::PlayerId::new(next_id);
        next_id += 1;
        let mut npc = Player::new(
            pid,
            format!("Esnaf-{}", i + 1),
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Esnaf);
        seed_distribute(&mut npc, &mut rng, 50_000);
        state.players.insert(pid, npc);
    }

    // Spekülatör
    for i in 0..spekulator {
        let pid = moneywar_domain::PlayerId::new(next_id);
        next_id += 1;
        let mut npc = Player::new(
            pid,
            format!("Spekülatör-{}", i + 1),
            Role::Tuccar,
            Money::from_lira(40_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Spekulator);
        seed_distribute(&mut npc, &mut rng, 8_000);
        state.players.insert(pid, npc);
    }

    state
}

fn rng_from_room(room_id: RoomId) -> ChaCha8Rng {
    let mut seed = [0u8; 32];
    let bytes = room_id.value().to_le_bytes();
    seed[..8].copy_from_slice(&bytes);
    ChaCha8Rng::from_seed(seed)
}

/// CLI'daki `distribute_inventory` ile aynı: weighted random (city × product) dağıtımı.
fn seed_distribute(player: &mut Player, rng: &mut ChaCha8Rng, total_budget: u32) {
    let buckets: Vec<(CityId, ProductKind)> = CityId::ALL
        .iter()
        .flat_map(|c| ProductKind::ALL.iter().map(move |p| (*c, *p)))
        .collect();
    let weights: Vec<u32> = (0..buckets.len())
        .map(|_| rng.random_range(0u32..=10))
        .collect();
    let total_weight: u32 = weights.iter().sum();
    if total_weight == 0 {
        return;
    }
    for ((city, product), w) in buckets.iter().zip(weights.iter()) {
        let share =
            u32::try_from(u64::from(total_budget) * u64::from(*w) / u64::from(total_weight))
                .unwrap_or(0);
        if share > 0 {
            let _ = player.inventory.add(*city, *product, share);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::PlayerId;
    use moneywar_net::ServerMessage;
    use tokio::sync::mpsc;

    fn dummy_lobby_with(humans: &[(u64, &str, Role)]) -> Lobby {
        let mut lobby = Lobby::new(RoomId::new(42));
        for (id, name, role) in humans {
            let (tx, _rx) = mpsc::channel::<ServerMessage>(8);
            lobby
                .add_player(PlayerId::new(*id), (*name).to_string(), tx)
                .unwrap();
            lobby.select_role(PlayerId::new(*id), *role);
            lobby.set_ready(PlayerId::new(*id), true);
        }
        lobby
    }

    #[test]
    fn deterministic_for_same_room() {
        let lobby_a = dummy_lobby_with(&[(1, "Ali", Role::Tuccar), (2, "Veli", Role::Sanayici)]);
        let lobby_b = dummy_lobby_with(&[(1, "Ali", Role::Tuccar), (2, "Veli", Role::Sanayici)]);
        let s1 = build_initial_state(&lobby_a);
        let s2 = build_initial_state(&lobby_b);
        assert_eq!(s1.players.len(), s2.players.len());
        // Aynı room_id + aynı slot order → aynı NPC sayıları
        for (id, p) in &s1.players {
            assert_eq!(p.cash, s2.players[id].cash);
            assert_eq!(p.role, s2.players[id].role);
            assert_eq!(p.is_npc, s2.players[id].is_npc);
        }
    }

    #[test]
    fn human_count_drives_npc_total() {
        let lobby_2 = dummy_lobby_with(&[(1, "A", Role::Tuccar), (2, "B", Role::Sanayici)]);
        let s2 = build_initial_state(&lobby_2);
        let humans = s2.players.values().filter(|p| !p.is_npc).count();
        let npcs = s2.players.values().filter(|p| p.is_npc).count();
        assert_eq!(humans, 2);
        assert!(npcs >= 8, "min 8 NPC, found {npcs}");
    }
}
