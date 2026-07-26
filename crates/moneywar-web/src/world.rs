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

/// Baseline fiyatları doldur (sim `build_state` ile birebir):
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
    // Faz 2: 5 ham madde, 5 şehir → her şehir bir hammaddenin ana kaynağı.
    // Boya ve Üzüm tek şehirden çıkar; Elbise/Ziyafet üretmek isteyen o
    // şehrin arzına bağımlı olur — tedarik boğmanın (SupplyChoke) zemini.
    // Hangi şehrin neyi ürettiği her seed'de değişir: ezbere strateji yok.
    let all_cities = [
        CityId::Istanbul, CityId::Ankara, CityId::Izmir,
        CityId::Bursa, CityId::Konya,
    ];
    let mut raws = ProductKind::RAW_MATERIALS;
    for i in (1..raws.len()).rev() {
        let j = rng.random_range(0..=i);
        raws.swap(i, j);
    }
    let prime_per_city: [(CityId, ProductKind); 5] = [
        (all_cities[0], raws[0]),
        (all_cities[1], raws[1]),
        (all_cities[2], raws[2]),
        (all_cities[3], raws[3]),
        (all_cities[4], raws[4]),
    ];
    s.seed_city_profiles(prime_per_city);
}

/// NPC kadrosunu kur — Tüccar / Sanayici / Spekülatör / Alıcı / Çiftçi / Banka.
/// Cash ve başlangıç stoğu değerleri sim `build_state` ile aynı.
fn seed_npcs(s: &mut GameState, rng: &mut ChaCha8Rng, comp: NpcComposition) {
    let mut next_id: u64 = 100;

    for idx in 0..comp.tuccar as usize {
        let pers = pick_personality(rng);
        let mut npc = make_npc(next_id, idx, "Tuccar", Role::Tuccar, 15_000, NpcKind::Tuccar)
            .with_personality(pers);
        // Başlangıç malı 8.000'de kaldı. Bir ara 4.000'e indirilmişti çünkü
        // Tüccar'ın PnL üstünlüğünün kaynağı sanılmıştı; asıl sebep skorlama
        // hatasıymış — başlangıç stoğu PnL referansına dahil değildi, yani
        // satılınca saf kâr yazılıyordu. Referans düzeltilince dünya
        // kurulumuna dokunmaya gerek kalmadı.
        distribute_inv(&mut npc, rng, 8_000);
        insert_npc(s, npc, &mut next_id);
    }

    for idx in 0..comp.sanayici as usize {
        let pers = pick_personality(rng);
        let mut npc = make_npc(next_id, idx, "Sanayici", Role::Sanayici, 50_000, NpcKind::Sanayici)
            .with_personality(pers);
        distribute_raw_inv(&mut npc, rng, 5_000); // sadece ham madde
        insert_npc(s, npc, &mut next_id);
    }

    for idx in 0..comp.spekulator as usize {
        let mut npc = make_npc(next_id, idx, "Spek", Role::Tuccar, 40_000, NpcKind::Spekulator);
        distribute_inv(&mut npc, rng, 2_000);
        insert_npc(s, npc, &mut next_id);
    }

    for idx in 0..comp.alici as usize {
        let npc = make_npc(next_id, idx, "Alici", Role::Tuccar, 150_000, NpcKind::Alici);
        insert_npc(s, npc, &mut next_id);
    }

    for idx in 0..comp.ciftci as usize {
        let mut npc = make_npc(next_id, idx, "Ciftci", Role::Tuccar, 8_000, NpcKind::Ciftci);
        let city = CityId::ALL[(next_id as usize) % CityId::ALL.len()];
        let prime = s
            .city_specialty
            .get(&city)
            .copied()
            .unwrap_or_else(|| city.cheap_raw());
        let _ = npc.inventory.add(city, prime, 200);
        insert_npc(s, npc, &mut next_id);
    }

    for idx in 0..comp.banka as usize {
        let npc = make_npc(next_id, idx, "Banka", Role::Tuccar, 200_000, NpcKind::Banka);
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
    "Ege Holding",
    "Kuzey Sanayi",
    "Selçuk Grubu",
    "Yıldız Holding",
    "Meriç Sanayi",
    "Sakarya Grubu",
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

/// Rol'e göre firma ismi. `idx` o rolün kendi içindeki sırasıdır.
///
/// Eskiden indeks `id`'den sabit ofsetlerle türetiliyordu (ör. "Çiftçi'ler
/// 100+20'den başlar") ve bu ofsetler kadro sayılarını varsayıyordu; Sanayici
/// 3'ten 10'a çıkınca Çiftçi isimleri kaydı ve "Çiftlik-127" gibi yedeklere
/// düştü. Artık sıra doğrudan geçiliyor, kadro değişse de isimler yerinde kalır.
///
/// Alıcı, Spekülatör, Banka isimsiz (event'lerde rol etiketi gösterilir).
fn npc_name(id: u64, idx: usize, prefix: &str, kind: NpcKind) -> String {
    match kind {
        NpcKind::Tuccar => TUCCAR_NAMES
            .get(idx)
            .map_or_else(|| format!("Lojistik-{id}"), |n| (*n).to_string()),
        NpcKind::Sanayici => SANAYICI_NAMES
            .get(idx)
            .map_or_else(|| format!("Sanayi-{id}"), |n| (*n).to_string()),
        NpcKind::Ciftci => CIFTCI_NAMES
            .get(idx)
            .map_or_else(|| format!("Çiftlik-{id}"), |n| (*n).to_string()),
        _ => format!("{prefix}-{id}"),
    }
}

fn make_npc(id: u64, idx: usize, prefix: &str, role: Role, cash_lira: i64, kind: NpcKind) -> Player {
    Player::new(
        PlayerId::new(id),
        npc_name(id, idx, prefix, kind),
        role,
        Money::from_lira(cash_lira).unwrap_or(Money::ZERO),
        true,
    )
    .expect("npc construction is infallible with valid cash")
    .with_kind(kind)
}

/// NPC'yi dünyaya ekler ve **başlangıç mal değerini** damgalar.
///
/// `PnL` referansı nakit + mal olduğu için stok değeri burada, sezon başı
/// baseline fiyatlarından hesaplanıp yazılır. Yazılmazsa başlangıç stoğu
/// saf kâr sayılır (bkz. `Player::starting_stock_value`).
fn insert_npc(s: &mut GameState, mut npc: Player, next_id: &mut u64) {
    let stock_cents: i64 = npc
        .inventory
        .entries()
        .map(|(city, product, qty)| {
            let unit = s
                .price_baseline
                .get(&(city, product))
                .map_or(0i64, |m| m.as_cents());
            unit.saturating_mul(i64::from(qty))
        })
        .sum();
    npc.starting_stock_value = moneywar_domain::Money::from_cents(stock_cents);

    s.news_subscriptions.insert(npc.id, NewsTier::Free);
    s.players.insert(npc.id, npc);
    *next_id += 1;
}

fn pick_personality(rng: &mut ChaCha8Rng) -> Personality {
    Personality::ALL[rng.random_range(0..Personality::ALL.len())]
}

/// Stoğu ağırlıklı rastgele dağıt (sim `build_state` ile aynı algoritma).
/// Sanayici başlangıç stoğu — sadece ham madde.
/// Mamul stoğuyla başlayınca t1'de haksız satış yapıyorlardı.
fn distribute_raw_inv(player: &mut Player, rng: &mut ChaCha8Rng, total: u32) {
    let buckets: Vec<(CityId, ProductKind)> = CityId::ALL
        .iter()
        .flat_map(|c| ProductKind::RAW_MATERIALS.iter().map(move |p| (*c, *p)))
        .collect();
    let weights: Vec<u32> = (0..buckets.len()).map(|_| rng.random_range(0u32..=10)).collect();
    let total_w: u32 = weights.iter().sum();
    if total_w == 0 { return; }
    for ((city, product), w) in buckets.iter().zip(weights.iter()) {
        let share = u32::try_from(u64::from(total) * u64::from(*w) / u64::from(total_w)).unwrap_or(0);
        if share > 0 { let _ = player.inventory.add(*city, *product, share); }
    }
}

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
