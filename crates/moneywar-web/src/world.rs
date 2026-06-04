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
                4 // Specialty hammadde (Konya=Buğday, Bursa=Pamuk, vb.)
            } else if product == ProductKind::Bugday {
                // Buğday non-specialty şehirlerde daha pahalı → Un cazibesini dengele.
                9
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
    // 5 şehrin tamamı shuffle — her ham madde 1-2 şehirde specialty.
    // Zeytin artık her seed'de mutlaka 1-2 şehirde → daha bol Zeytin arzı.
    let all_cities = [
        CityId::Istanbul, CityId::Ankara, CityId::Izmir,
        CityId::Bursa, CityId::Konya,
    ];
    let mut raws_repeated = [
        ProductKind::Pamuk, ProductKind::Pamuk,
        ProductKind::Bugday, ProductKind::Bugday,
        ProductKind::Zeytin, ProductKind::Zeytin,
    ];
    // Shuffle'dan 5 seç (tekrarlı 6'dan 5 şehir için dengeli dağılım)
    for i in (1..raws_repeated.len()).rev() {
        let j = rng.random_range(0..=i);
        raws_repeated.swap(i, j);
    }
    let prime_per_city: [(CityId, ProductKind); 5] = [
        (all_cities[0], raws_repeated[0]),
        (all_cities[1], raws_repeated[1]),
        (all_cities[2], raws_repeated[2]),
        (all_cities[3], raws_repeated[3]),
        (all_cities[4], raws_repeated[4]),
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

/// Tüccar → lojistik şirket isimleri (ID 100–103).
const TUCCAR_NAMES: &[&str] = &[
    "Demir Lojistik",
    "Kaya Taşımacılık",
    "Aslan Kargo",
    "Boğaz Nakliyat",
    "Kervan Lojistik",
    "Liman Taşıma",
    "Mavi Kargo",
    "Nehir Nakliyat",
];

/// Sanayici → sanayi grubu isimleri (ID 104–106).
const SANAYICI_NAMES: &[&str] = &[
    "Çelik Grubu",
    "Deniz Sanayi",
    "Ova Holding",
    "Toros Grubu",
    "Fırat Sanayi",
    "Dicle Holding",
    "Anadolu Grubu",
    "Marmara Sanayi",
];

/// Çiftçi → tarım işletmesi isimleri (ID 120–128).
const CIFTCI_NAMES: &[&str] = &[
    "Bereket Tarım",
    "Ova Çiftliği",
    "Güneş Tarım",
    "Dağ Çiftliği",
    "Verimli Tarla",
    "Anadolu Çiftliği",
    "Yeşil Tarım",
    "Toprak Tarım",
    "Hasad Çiftliği",
    "Toprakseven",
    "Bağ & Bahçe",
    "Köy Tarım",
];

/// Rol'e göre firma ismi. Alıcı, Spekülatör, Banka isimsiz (role kodu görünür).
fn npc_name(id: u64, prefix: &str, kind: NpcKind) -> String {
    match kind {
        NpcKind::Tuccar => {
            let idx = id.saturating_sub(100) as usize;
            TUCCAR_NAMES.get(idx)
                .map_or_else(|| format!("Lojistik-{id}"), |n| (*n).to_string())
        }
        NpcKind::Sanayici => {
            // Sanayici ID'leri Tüccar'dan sonra başlar (comp.tuccar sonra sanayici)
            let idx = id.saturating_sub(100 + 4) as usize; // 4 = default tuccar sayısı
            SANAYICI_NAMES.get(idx)
                .map_or_else(|| format!("Sanayi-{id}"), |n| (*n).to_string())
        }
        NpcKind::Ciftci => {
            // Çiftçi ID'leri: tuccar(4)+sanayici(3)+spekulator(3)+alici(10) = 20 sonra
            let idx = id.saturating_sub(100 + 20) as usize;
            CIFTCI_NAMES.get(idx)
                .map_or_else(|| format!("Çiftlik-{id}"), |n| (*n).to_string())
        }
        // Alıcı, Spekülatör, Banka → prefix fallback (event'lerde rol etiketi gösterilir)
        _ => format!("{prefix}-{id}"),
    }
}

fn make_npc(id: u64, prefix: &str, role: Role, cash_lira: i64, kind: NpcKind) -> Player {
    Player::new(
        PlayerId::new(id),
        npc_name(id, prefix, kind),
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
