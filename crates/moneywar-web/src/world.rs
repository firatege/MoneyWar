//! Sezon dünyası kurucu — saf NPC ekonomisi (insan oyuncu yok, spectator).
//!
//! `moneywar-sim`'deki `build_state` mantığını aynalar; tek fark: izleyici
//! modunda insan oyuncu yok, dünya tamamen NPC'lerden oluşur. Baseline +
//! şehir profili + NPC kompozisyonu sim ile aynı tutulur ki ekonomi davranışı
//! sapmasın.

use moneywar_domain::{
    CityId, DemandLevel, GameState, Money, NewsTier, NpcComposition, NpcKind, Personality, Player,
    PlayerId, ProductKind, RoomConfig, RoomId, Role,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Verilen seed'den deterministik, dengeli, saf-NPC bir sezon dünyası kurar.
#[must_use]
pub fn new_season(seed: u64) -> GameState {
    let comp = NpcComposition::default();
    let mut s = GameState::new(RoomId::new(seed), RoomConfig::hizli());
    let mut rng = ChaCha8Rng::seed_from_u64(seed);

    seed_baselines(&mut s);
    seed_profiles(&mut s, &mut rng);
    seed_npcs(&mut s, &mut rng, comp);

    s
}

/// Baseline fiyatları doldur (sim build_state ile birebir):
/// ham → lokal specialty 4₺, off-specialty 7₺; mamul → şehir talebine göre
/// `base × {High:1.2, Normal:1.0, Low:0.8}`.
fn seed_baselines(s: &mut GameState) {
    for city in CityId::ALL {
        let cheap = city.cheap_raw();
        for product in ProductKind::ALL {
            let lira = if product.is_finished() {
                let base = product.base_price_lira();
                match city.demand_for(product) {
                    DemandLevel::High => base * 12 / 10,
                    DemandLevel::Normal => base,
                    DemandLevel::Low => base * 8 / 10,
                }
            } else if product == cheap {
                4
            } else {
                7
            };
            let baseline = Money::from_lira(lira).unwrap_or(Money::ZERO);
            s.price_baseline.insert((city, product), baseline);
            s.price_baseline_initial.insert((city, product), baseline);
        }
    }
}

/// Şehir specialty/secondary/demand profilini kur — sim ile aynı 3-tier
/// rotasyon (ilk 3 şehir shuffle edilmiş ham, Bursa=Pamuk, Konya=Buğday).
fn seed_profiles(s: &mut GameState, rng: &mut ChaCha8Rng) {
    let mut raws = ProductKind::RAW_MATERIALS;
    for i in (1..raws.len()).rev() {
        let j = rng.random_range(0..=i);
        raws.swap(i, j);
    }
    let prime_per_city: [(CityId, ProductKind); 5] = [
        (CityId::Istanbul, raws[0]),
        (CityId::Ankara, raws[1]),
        (CityId::Izmir, raws[2]),
        (CityId::Bursa, ProductKind::Pamuk),
        (CityId::Konya, ProductKind::Bugday),
    ];
    s.seed_city_profiles(prime_per_city);
}

/// NPC kadrosunu kur — Tüccar / Sanayici / Spekülatör / Alıcı / Çiftçi / Banka.
/// Cash ve başlangıç stoğu değerleri sim build_state ile aynı.
fn seed_npcs(s: &mut GameState, rng: &mut ChaCha8Rng, comp: NpcComposition) {
    let mut next_id: u64 = 100;

    for _ in 0..comp.tuccar {
        let pers = pick_personality(rng);
        let mut npc = make_npc(next_id, "Tuccar", Role::Tuccar, 15_000, NpcKind::Tuccar)
            .with_personality(pers);
        distribute_inv(&mut npc, rng, 8_000);
        insert_npc(s, npc, &mut next_id);
    }

    for _ in 0..comp.sanayici {
        let pers = pick_personality(rng);
        let mut npc = make_npc(next_id, "Sanayici", Role::Sanayici, 50_000, NpcKind::Sanayici)
            .with_personality(pers);
        distribute_inv(&mut npc, rng, 5_000);
        insert_npc(s, npc, &mut next_id);
    }

    for _ in 0..comp.spekulator {
        let mut npc = make_npc(next_id, "Spek", Role::Tuccar, 40_000, NpcKind::Spekulator);
        distribute_inv(&mut npc, rng, 2_000);
        insert_npc(s, npc, &mut next_id);
    }

    for _ in 0..comp.alici {
        let npc = make_npc(next_id, "Alici", Role::Tuccar, 150_000, NpcKind::Alici);
        insert_npc(s, npc, &mut next_id);
    }

    for _ in 0..comp.ciftci {
        let mut npc = make_npc(next_id, "Ciftci", Role::Tuccar, 8_000, NpcKind::Ciftci);
        let city = CityId::ALL[(next_id as usize) % CityId::ALL.len()];
        let prime = s
            .city_specialty
            .get(&city)
            .copied()
            .unwrap_or_else(|| city.cheap_raw());
        let _ = npc.inventory.add(city, prime, 200);
        insert_npc(s, npc, &mut next_id);
    }

    for _ in 0..comp.banka {
        let npc = make_npc(next_id, "Banka", Role::Tuccar, 200_000, NpcKind::Banka);
        insert_npc(s, npc, &mut next_id);
    }
}

fn make_npc(id: u64, prefix: &str, role: Role, cash_lira: i64, kind: NpcKind) -> Player {
    Player::new(
        PlayerId::new(id),
        format!("{prefix}-{id}"),
        role,
        Money::from_lira(cash_lira).unwrap_or(Money::ZERO),
        true,
    )
    .expect("npc construction is infallible with valid cash")
    .with_kind(kind)
}

fn insert_npc(s: &mut GameState, npc: Player, next_id: &mut u64) {
    s.news_subscriptions.insert(npc.id, NewsTier::Free);
    s.players.insert(npc.id, npc);
    *next_id += 1;
}

fn pick_personality(rng: &mut ChaCha8Rng) -> Personality {
    Personality::ALL[rng.random_range(0..Personality::ALL.len())]
}

/// Stoğu ağırlıklı rastgele dağıt (sim build_state ile aynı algoritma).
fn distribute_inv(player: &mut Player, rng: &mut ChaCha8Rng, total: u32) {
    let buckets: Vec<(CityId, ProductKind)> = CityId::ALL
        .iter()
        .flat_map(|c| ProductKind::ALL.iter().map(move |p| (*c, *p)))
        .collect();
    let weights: Vec<u32> = (0..buckets.len()).map(|_| rng.random_range(0u32..=10)).collect();
    let total_w: u32 = weights.iter().sum();
    if total_w == 0 {
        return;
    }
    for ((city, product), w) in buckets.iter().zip(weights.iter()) {
        let share =
            u32::try_from(u64::from(total) * u64::from(*w) / u64::from(total_w)).unwrap_or(0);
        if share > 0 {
            let _ = player.inventory.add(*city, *product, share);
        }
    }
}
