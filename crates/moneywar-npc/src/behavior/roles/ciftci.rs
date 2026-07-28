//! Çiftçi rol davranışı — hammadde üreticisi, sell-only.
//!
//! Çiftçi her `HARVEST_PERIOD` (8) tick'te `harvest_ciftci_stock` ile envantere
//! mahsul alır. Davranışı sade: stoğunu pazara satar, "ne zaman ne kadar?"
//! kararı utility skor üzerinden.
//!
//! # Aday üretim kuralı
//!
//! Envanterindeki her `(city, raw_product, qty)` için bir Sell adayı:
//! - quantity = stoğun yarısı (max 100, min 1)
//! - `unit_price` = `effective_baseline(city, product)`
//! - skor → orchestrator hesaplar (Çiftçi `Weights`'i ile)
//!
//! # Çiftçi `Weights` mantığı (`personality.rs`'te)
//!
//! - `stock` +1.0 → stok varsa sat
//! - `urgency` +0.5 → sezon sonu agresifleş
//! - `local_raw_advantage` +0.4 → uzmanlık şehrini önceliklendir
//! - `price_rel_avg` +0.3 → pahalıyken sat
//! - `competition` -0.2 → rakip baskısı varsa bekle
//! - `cash` -0.3 → düşük cash → motive et

use moneywar_domain::{GameState, Money, OrderSide, Player, ProductKind};

use crate::behavior::candidates::ActionCandidate;
use crate::behavior::pricing::{CrossPolicy, marketable_ask};

/// Çiftçi'nin bu tick için olası satış adayları.
/// v8.20: Order-book aware pricing — `marketable_ask` üzerinden geçer.
///
/// Rol kimliği: **üretici** → mahsul fire riski → satış zorunluluğu.
/// Stok>500 olduğunda CROSS — `best_bid`'a iner (taze stoğu eritir).
/// Stok<500 ise PASSIVE — kâr maks (taze ürün, beklemeye değer).
///
/// Floor (asla altına satmama eşiği) stok'a göre:
/// - 0-199 → %100 baz (taze, kâr maks)
/// - 200-499 → %90 (hafif basınç)
/// - 500-999 → %80 (orta basınç)
/// - 1000+ → %65 (kriz)
///
/// `marketable_ask` `urgency_pct` uygular → hatta cross olmadan bile floor
/// patience erosion + season drift ile %30 düşer → match garantisi.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_raw() || qty == 0 {
            continue;
        }
        // Minimum 10 birim olmadan satma — küçük stok biriktirsin.
        // Böylece 1-5 birimlik mikro fill'ler ortadan kalkar.
        if qty < 10 {
            continue;
        }
        let quantity = (qty / 2).min(30);
        // **Çapa `effective_baseline`, yürüyen ortalama değil.**
        //
        // `reference_price` son 5 işlemin ortalamasıdır; ona göre fiyatlamak
        // kendi kendini besleyen bir sarmal kurar — yüksek fill → yüksek
        // ortalama → yüksek taban → daha yüksek teklif. Ölçümde sezon boyunca
        // fiyatlar 3,74 katına çıkıyordu; para arzı ise yalnız +%1,5. Yani
        // enflasyon parasal değil, fiyat kuralının artefaktıydı.
        //
        // `effective_baseline` sezon başı çapasının [%60, %160] aralığına
        // clamp'li. Çiftçi zincirin **tabanındaki** fiyatı koyduğu için bu
        // çapa yukarı doğru tüm katmanlara taşınır.
        let anchor = state.effective_baseline(city, product).unwrap_or_else(|| {
            // Baseline yoksa fallback — sim her zaman init eder, prod CLI de.
            Money::from_lira(default_raw_price(product)).unwrap_or(Money::ZERO)
        });

        // **Taban maliyetin altına inemez.**
        //
        // Çapa tek başına yetmiyordu: clamp'li olduğu için ölçümde 60 kovanın
        // 48'i bir sınıra dayalıydı — çapa fiyat sinyali değil sabit sayıydı.
        // Ücret hayat pahalılığına endekslenince maliyet tırmandı, fiyat
        // çakılı kaldı, marj sıfıra indi ve Çiftçi batmaya başladı.
        //
        // Gerçek hayatta hiçbir üretici maliyetinin altına satmaz.
        // Sanayici'de bu zaten böyle (`derived_input_ceiling` tarif
        // maliyetinden hesaplıyor); Çiftçi'de eksikti.
        // Taban piyasadan tamamen kopamaz. Maliyet çapanın çok üstündeyse
        // ısrar etmek satmamak demek: tarlası olmayan Çiftçi'nin birim
        // maliyeti yüksek kalıyor ve sınırsız taban onu pazardan tamamen
        // çıkarıyordu (ölçüm: Çiftçi iflas 2,0, Spekülatör fill 0,47 → 0,15).
        // Gerçek üretici de maliyetinin üstünde ısrar edip hiç satmaz —
        // zararına satar, sonra küçülür ya da çıkar.
        let raw_floor = farmer_unit_cost_cents(state, player)
            .saturating_mul(100 + FARMER_MARGIN_PCT)
            / 100;
        let ceiling = anchor
            .as_cents()
            .saturating_mul(COST_FLOOR_MAX_PCT)
            / 100;
        let cost_floor = Money::from_cents(raw_floor.min(ceiling));
        let reference = if cost_floor > anchor { cost_floor } else { anchor };
        // Stok-baskısı indirim: Çiftçi'nin bu (city, raw)'da stok'u büyükse
        // SELL fiyatı agresif düşür — pazar onu emsin. Aksi takdirde prime
        // şehir over-supply pattern'inde stok kilitlenir.
        let stock_floor_pct: i64 = match qty {
            0..=199 => 100,  // taze stok → kâr maks
            200..=499 => 90, // hafif basınç
            500..=999 => 80, // orta basınç
            _ => 65,         // 1000+ birim → kriz
        };
        let stock_floor = Money::from_cents(
            reference
                .as_cents()
                .saturating_mul(stock_floor_pct)
                .saturating_div(100),
        );
        // Stok>500 → CROSS (eritmek için best_bid'a in). Aksi halde PASSIVE.
        let policy = if qty >= 500 {
            CrossPolicy::Cross
        } else {
            CrossPolicy::Passive
        };
        let Some(unit_price) = marketable_ask(
            state,
            player.id,
            city,
            product,
            stock_floor,
            policy,
            state.current_tick,
        ) else {
            continue;
        };
        out.push(ActionCandidate::SubmitOrder {
            side: OrderSide::Sell,
            city,
            product,
            quantity,
            unit_price,
            ttl_override: None,
        });
    }
    out
}

/// Çiftçi'nin maliyetin üstüne koyduğu marj (yüzde). Üretici kâr etmeden
/// tohum alamaz; Sanayici'nin `FACTORY_TARGET_MARGIN_PCT`'i ile aynı fikir.
const FARMER_MARGIN_PCT: i64 = 25;

/// Maliyet tabanı çapanın en fazla bu kadarına çıkabilir (yüzde). Üstü
/// "maliyetim yüksek" diye pazardan çıkmak olur; fiyat sinyali kalkar.
const COST_FLOOR_MAX_PCT: i64 = 140;

/// Çiftçi'nin birim mahsul maliyeti (kuruş) — **kendi kapasitesine göre**.
///
/// İki gider var: tohum (birim başına sabit) ve işçilik (kadro × endeksli
/// ücret). İşçilik döngü başına üretilen birime bölünür.
///
/// Kritik nokta bölen: üretim yalnız sabit hasat değil, **tarla çıktısı
/// dahil**. Tarla kuran Çiftçi'nin işçi başına üretimi arttığı için birim
/// maliyeti düşer ve fiyatı rekabetçi kalır. Endeksli ücret ancak böyle
/// sürdürülebilir — verimlilik artmadan ücret artışı üreticiyi öldürür.
fn farmer_unit_cost_cents(state: &GameState, player: &Player) -> i64 {
    use moneywar_domain::balance as b;

    let cpi = state.cost_of_living_index().max(100);
    let wage_per_head_cents = b::wage_per_employee_lira().saturating_mul(cpi);

    let farm_heads: i64 = state
        .private_farms
        .values()
        .filter(|f| f.owner == player.id)
        .map(|f| i64::from(f.employees))
        .sum();
    let heads = b::CREW_PER_FARMER.saturating_add(farm_heads);

    // Bir hasat döngüsünde kesilen bordro sayısı.
    let wage_periods_x100 =
        i64::from(b::HARVEST_PERIOD_TICKS) * 100 / i64::from(b::WAGE_PERIOD_TICKS).max(1);
    let labor_cents = heads
        .saturating_mul(wage_per_head_cents)
        .saturating_mul(wage_periods_x100)
        / 100;

    // Döngü başına üretim: sabit hasat + tarlaların o süredeki çıktısı.
    let harvest = i64::from(b::HARVEST_QTY_MIN + b::HARVEST_QTY_MAX) / 2;
    let farm_output: i64 = state
        .private_farms
        .values()
        .filter(|f| f.owner == player.id)
        .map(|f| i64::from(f.output_per_tick()))
        .sum::<i64>()
        .saturating_mul(i64::from(b::HARVEST_PERIOD_TICKS));
    let units = harvest.saturating_add(farm_output).max(1);

    let seed_cents = b::SEED_COST_PER_RAW_LIRA.saturating_mul(100);
    seed_cents.saturating_add(labor_cents / units)
}

const fn default_raw_price(_product: ProductKind) -> i64 {
    moneywar_domain::balance::npc_base_price_raw_lira()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{CityId, NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId};

    fn ciftci_with_stock(stock: u32) -> (GameState, Player) {
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "ciftci",
            Role::Tuccar,
            Money::from_lira(8_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Ciftci);
        if stock > 0 {
            p.inventory
                .add(CityId::Istanbul, ProductKind::Pamuk, stock)
                .unwrap();
        }
        (s, p)
    }

    #[test]
    fn empty_inventory_yields_no_candidates() {
        let (s, p) = ciftci_with_stock(0);
        assert!(enumerate(&s, &p).is_empty());
    }

    #[test]
    fn raw_stock_yields_sell_at_half_qty() {
        let (s, p) = ciftci_with_stock(200);
        let cands = enumerate(&s, &p);
        assert_eq!(cands.len(), 1);
        let ActionCandidate::SubmitOrder { side, quantity, .. } = &cands[0] else {
            panic!("expected SubmitOrder");
        };
        assert_eq!(*side, OrderSide::Sell);
        assert_eq!(*quantity, 30); // 200/2=100, ama cap 30
    }

    #[test]
    fn finished_stock_skipped() {
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "ciftci",
            Role::Tuccar,
            Money::from_lira(8_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Ciftci);
        // Çiftçi'nin elinde mamul olamaz normalde, ama enumerate skip etmeli.
        p.inventory
            .add(CityId::Istanbul, ProductKind::Kumas, 50)
            .unwrap();
        let cands = enumerate(&s, &p);
        assert!(cands.is_empty(), "Çiftçi mamul satmamalı");
    }

    #[test]
    fn quantity_caps_at_30() {
        let (s, p) = ciftci_with_stock(500);
        let cands = enumerate(&s, &p);
        let ActionCandidate::SubmitOrder { quantity, .. } = &cands[0] else {
            panic!()
        };
        // 500/2 = 250, ama cap 30.
        assert_eq!(*quantity, 30);
    }

    #[test]
    fn tiny_stock_below_threshold_yields_no_sell() {
        // 10 birim altı — biriktirsin, satmasın.
        let (s, p) = ciftci_with_stock(5);
        assert!(enumerate(&s, &p).is_empty(), "küçük stok satılmamalı");
    }

    #[test]
    fn exactly_threshold_yields_sell() {
        let (s, p) = ciftci_with_stock(10);
        let cands = enumerate(&s, &p);
        assert!(!cands.is_empty(), "eşikte satış başlamalı");
    }
}

/// Çiftçi'nin tarla kurma/büyütme adayları — **arzın fiyata tepkisi**.
///
/// Çiftçi'nin hasadı yatırımdan bağımsız sabitti: buğday yedi kat pahalansa da
/// bir gram fazla üretilmiyordu. Piyasanın tek tarafı fiyata tepki
/// verebildiği için ham madde tarafı tıkanıyordu (ham 5-7×, mamul 1,3-3,9×).
///
/// Tarlanın ikinci ve daha kritik işi: **işçi başına üretimi artırmak.** Ücret
/// hayat pahalılığına endeksli olduğu için sabit kapasiteyle birim maliyet
/// ücretle birlikte patlıyor ve ham madde üretimi kârsızlaşıyor. Tarla
/// seviyesi (1× / 1,75× / 2,75×) bunu telafi eder — `farmer_unit_cost_cents`
/// bölenine doğrudan girer.
///
/// Karar: sezon başı çapasına göre **en çok pahalanmış** ham madde, yani
/// piyasanın "bundan daha çok istiyorum" dediği mal. Elde tarla varsa önce
/// yükseltme — yeni tarla kurmaktan ucuz ve kadro şartı aynı.
#[must_use]
pub fn enumerate_farm(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    use moneywar_domain::CityId;
    use moneywar_domain::balance::PRIVATE_FARM_BUILD_COOLDOWN;

    /// Nakdin **yüzde kaçı** işletme sermayesi olarak yatırıma kapalı.
    /// Tarlaya yatırıp tohum/bordro parasız kalan Çiftçi krediye düşüp
    /// temerrütle batıyordu; yatırım yalnız bu payın üstündeki paradan.
    const RESERVE_PCT: i64 = 65;
    /// Bu kadar pahalanmadan tarla kurmaya değmez (sezon başının yüzdesi).
    const BUILD_SIGNAL_PCT: i64 = 130;

    let reserve = player.cash.as_cents().saturating_mul(RESERVE_PCT) / 100;

    // Önce yükseltme: mevcut tarlayı büyütmek yeni tarla kurmaktan ucuz ve
    // işçi başına üretimi doğrudan artırır.
    for farm in state.private_farms.values().filter(|f| f.owner == player.id) {
        if farm.level >= moneywar_domain::PrivateFarm::FARM_MAX_LEVEL {
            continue;
        }
        let Some(cost) = moneywar_domain::PrivateFarm::upgrade_cost(farm.level) else {
            continue;
        };
        if player.cash.as_cents().saturating_sub(cost.as_cents()) >= reserve {
            return vec![ActionCandidate::UpgradeFarm { farm_id: farm.id }];
        }
    }

    // Kurulum beklemesi — motor da aynı kuralı uygular; burada bakmak
    // reddedilecek komut üretmemek için.
    let last_built = state
        .private_farms
        .values()
        .filter(|f| f.owner == player.id)
        .map(|f| f.built_at)
        .max();
    if let Some(last) = last_built
        && state.current_tick.value().saturating_sub(last) < PRIVATE_FARM_BUILD_COOLDOWN
    {
        return Vec::new();
    }

    // Fiyat sinyali: sezon başı çapasına göre en çok pahalanan ham madde.
    // Uzmanlık şartı yok — `output_per_tick` yalnız seviyeye ve kadroya bakar.
    let mut best: Option<(i64, CityId, ProductKind)> = None;
    for city in CityId::ALL {
        for &raw in &ProductKind::RAW_MATERIALS {
            if state
                .private_farms
                .values()
                .any(|f| f.owner == player.id && f.city == city && f.product == raw)
            {
                continue;
            }
            let Some(anchor) = state.price_baseline_initial.get(&(city, raw)) else {
                continue;
            };
            if anchor.as_cents() <= 0 {
                continue;
            }
            let Some(now) = state.reference_price(city, raw) else {
                continue;
            };
            let climb = now.as_cents() * 100 / anchor.as_cents();
            if best.is_none_or(|(b, _, _)| climb > b) {
                best = Some((climb, city, raw));
            }
        }
    }

    match best {
        Some((climb, city, product)) if climb >= BUILD_SIGNAL_PCT => {
            // Maliyet sahip olunan tarla ve slot kalabalığıyla büyüyor.
            let owned = state
                .private_farms
                .values()
                .filter(|f| f.owner == player.id)
                .count();
            let slot_taken = state
                .private_farms
                .values()
                .filter(|f| f.city == city && f.product == product)
                .count();
            let Some(cost) = moneywar_domain::PrivateFarm::build_cost(owned, slot_taken) else {
                return Vec::new();
            };
            if player.cash.as_cents().saturating_sub(cost.as_cents()) < reserve {
                return Vec::new();
            }
            vec![ActionCandidate::BuildPrivateFarm { city, product }]
        }
        _ => Vec::new(),
    }
}
