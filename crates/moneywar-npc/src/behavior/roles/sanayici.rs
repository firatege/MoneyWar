//! Sanayici rol davranışı — fabrika kuran üretici.
//!
//! Sanayici 3 tür aksiyon yapar:
//! 1. **Fabrika kur** (cash varsa, fab sayısı az ise) — şehir × mamul seçer
//! 2. **Ham madde AL** — production için raw input (her şehir × ham mal)
//! 3. **Mamul SAT** — fabrika çıktısı stoktan satar
//!
//! Production zinciri Pamuk→Kumas, Buğday→Un, Zeytin→Zeytinyağı (otomatik
//! engine `step_factory` ile). Sanayici sadece input/output pazarlamasını
//! yönetir.
//!
//! # `Weights` mantığı (`personality.rs`'te)
//!
//! - `cash +0.4` — cash varsa hareket (BUY raw / Build)
//! - `urgency +0.3` — sezon ilerledikçe agresifleş
//! - `price_rel_avg +0.2` — fiyat fırsatlarını yakala
//! - `arbitrage +0.3` — şehirler arası fark
//! - `competition -0.2` — rakip baskı varsa bekle

use moneywar_domain::{
    CaravanState, Cargo, CityId, Factory, GameState, Money, OrderSide, Player, ProductKind,
    balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;
use crate::behavior::pricing::{CrossPolicy, marketable_ask, marketable_bid};

/// Yeni fabrika kurma eşiği. Faz 1: 4→8 (monopolleşme + rekabet dinamiği).
/// 5 şehir × 3 mamul = 15 slot, 3 Sanayici × 8 = 24 hedef > 15 slot →
/// sanayiciler slotlar için rekabet eder; biri sıyrılırsa monopol oluşabilir.
/// Maliyet kontrolü: build_cost tablosu 8 fab'a kadar genişletildi (max 90K),
/// 50K başlangıç nakdi ilk 4-5 fabrikayı karşılar, sonrası kârdan finanse edilir.
const TARGET_FACTORIES: usize = 8;

/// Sanayici'nin bu tick için aday listesi.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();

    // 1) Fabrika kurma: hedef sayıdan azsa + 1 fab kuruluş maliyeti
    //    karşılanabiliyorsa.
    let owned = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .count();
    if owned < TARGET_FACTORIES {
        let next_cost = moneywar_domain::Factory::build_cost(u32::try_from(owned).unwrap_or(0));
        if player.cash >= next_cost {
            // Önce mevcut fabrikaların kapsamadığı (city, mamul) seç.
            if let Some((city, product)) = pick_factory_target(state, player) {
                out.push(ActionCandidate::BuildFactory { city, product });
            }
        }
    }

    // 2) Ham madde AL — fab-bazlı talep (gerçek tedarik zinciri).
    //    Her fab'ın raw_input'unu hesapla. Sanayici Ist'te Kumaş fab kurmuşsa
    //    Pamuk her 3 şehirde de arar (Tüccar Ist'ten Ank'a getirebilir).
    //    Fab yoksa fallback: şehir specialty raw'ı.
    let needed_raws: std::collections::BTreeSet<ProductKind> = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .filter_map(|f| f.product.raw_input())
        .collect();
    // v8.20: Cross policy = fab var ise CROSS (ham açlığı, agresif al).
    // Fab yoksa PASSIVE (gelecek fab planı için seyrek alım, kâr odaklı).
    let buy_policy = if needed_raws.is_empty() {
        CrossPolicy::Passive
    } else {
        CrossPolicy::Cross
    };
    if needed_raws.is_empty() {
        // Fab yok → fallback: her şehir kendi specialty raw'ı (3 BUY).
        let bucket_cash = Money::from_cents((player.cash.as_cents() / 6).max(0));
        for city in CityId::ALL {
            let product = city.cheap_raw();
            let reference = state.reference_price(city, product).unwrap_or_else(|| {
                Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA)
                    .unwrap_or(Money::ZERO)
            });
            // Pasif tavan: baseline × 1.05 (Çiftçi'nin baz fiyatına yakın).
            let cash_ceiling = scale_pct(reference, 105);
            let Some(unit_price) = marketable_bid(
                state,
                player.id,
                city,
                product,
                cash_ceiling,
                buy_policy,
                state.current_tick,
            ) else {
                continue;
            };
            let quantity = affordable_qty(bucket_cash, unit_price, 20);
            if quantity > 0 {
                out.push(ActionCandidate::SubmitOrder {
                    side: OrderSide::Buy,
                    city,
                    product,
                    quantity,
                    unit_price,
                    ttl_override: None,
                });
            }
        }
    } else {
        // Fix 1: her şehirde SADECE o şehrin fabrikalarının ham maddesi.
        // Global needed_raws × fab_cities cross-product yanlıştı: Ankara'da
        // Zeytinyağı fab varsa Buğday (Un fab İzmir'de) için BUY yapmamalı.
        let fab_cities: std::collections::BTreeSet<CityId> = state
            .factories
            .values()
            .filter(|f| f.owner == player.id)
            .map(|f| f.city)
            .collect();
        // Bütçe: gerçek (city, raw) çifti sayısına böl.
        let pair_count = state
            .factories
            .values()
            .filter(|f| f.owner == player.id && f.product.raw_input().is_some())
            .count()
            .max(1) as i64;
        let bucket_cash = Money::from_cents(player.cash.as_cents() / 2 / pair_count);
        for &city in &fab_cities {
            // Fix 1: bu şehirdeki fabrikaların ham maddeleri.
            let city_raws: std::collections::BTreeSet<ProductKind> = state
                .factories
                .values()
                .filter(|f| f.owner == player.id && f.city == city)
                .filter_map(|f| f.product.raw_input())
                .collect();
            for &product in &city_raws {
                // Fix 5: stok eksiği kadar iste — talep sinyali + gerçek ihtiyaç.
                // Stok doluysa BUY yapma (gereksiz sinyal).
                let have = player.inventory.get(city, product);
                if have >= Factory::BATCH_SIZE {
                    continue;
                }
                let shortage = Factory::BATCH_SIZE - have;
                let want = shortage.min(50);

                // effective_baseline: Walras clamp'lı referans — rolling_avg
                // spiral'ini kırar (her yüksek fill bir sonraki ceiling'i daha
                // da yükseltmez). initial × [%60, %160] aralığında kalır.
                let baseline = state
                    .effective_baseline(city, product)
                    .unwrap_or_else(|| {
                        Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA)
                            .unwrap_or(Money::ZERO)
                    });
                // Kademeli premium: stok ne kadar azsa o kadar yüksek teklif.
                // Kitapta hep tavana gitmek yerine organik fiyat keşfi.
                let premium_pct: i64 = match shortage {
                    0..=25  => 105, // az eksik → fazla ödeme
                    26..=60 => 115, // orta eksik
                    61..=99 => 125, // çok eksik
                    _       => 130, // sıfır stok → max (Tüccar sinyali)
                };
                let cash_ceiling = scale_pct(baseline, premium_pct);
                let Some(unit_price) = marketable_bid(
                    state,
                    player.id,
                    city,
                    product,
                    cash_ceiling,
                    buy_policy,
                    state.current_tick,
                ) else {
                    continue;
                };
                let quantity = affordable_qty(bucket_cash, unit_price, want);
                if quantity > 0 {
                    out.push(ActionCandidate::SubmitOrder {
                        side: OrderSide::Buy,
                        city,
                        product,
                        quantity,
                        unit_price,
                        ttl_override: Some(6),
                    });
                }
            }
        }
    }

    // 3) Mamul SAT — stok-baskılı pricing (donmuş mamul fiyatı sorununun fix'i).
    //    Eski `baseline × 0.95 sabit` → fiyat 80+ tick boyunca kıpırdamıyordu
    //    (rolling avg self-reinforcing). Yeni tier: stok birikince ASK düşer,
    //    fiyat keşfi açılır.
    //    PASSIVE policy korunur: Sanayici cross etmez (Alıcı CROSS yetişir).
    //    Stok>150 → CROSS (mamul çürümez ama cash kilitlenir, hızlı erit).
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_finished() || qty == 0 {
            continue;
        }
        let quantity = (qty / 2).max(1).min(50);
        // v0.5.1: rolling_avg self-reinforcing loop (eski yüksek clearing'ler
        // NPC fiyat tavanını yukarı tutuyordu, Alıcı bütçesi yetmiyordu) için
        // Sanayici reference = effective_baseline (Walras-shifted). Rolling avg
        // değil. Walras imbalance'a göre kayan baseline gerçek piyasa dengesi.
        let reference = state.effective_baseline(city, product).unwrap_or_else(|| {
            Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA)
                .unwrap_or(Money::ZERO)
        });
        // Nakit kritikse taban düşür — ücret ödemek için satmak zorunda.
        let cash_lira = player.cash.as_cents() / 100;
        let stock_floor_pct: i64 = if cash_lira < 5_000 {
            78  // kritik nakit → ne olursa satalım
        } else {
            match qty {
                0..=49 => 95,  // taze mamul
                50..=99 => 90, // orta stok
                _ => 85,       // yüksek stok → kervan da dispatch edecek
            }
        };
        let stock_floor = scale_pct(reference, stock_floor_pct);
        let policy = CrossPolicy::Passive;
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

    // v8.23: Açık Tüccar kontratlarını tara, fab ihtiyacına uyanı kabul et.
    // Cap: Sanayici aynı anda max 1 aktif buyer kontratı.
    out.extend(enumerate_contract_accepts(state, player, &needed_raws));

    // Mamul kervan dispatch — stok yüksekse en pahalı şehre gönder.
    out.extend(enumerate_mamul_dispatch(state, player));

    out
}

/// Sanayici'nin mamul kervan dispatch adayları.
/// Stok BATCH_SIZE × 2 birimi aşarsa, en yüksek referans fiyatlı şehre gönder.
fn enumerate_mamul_dispatch(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    let dispatch_threshold = moneywar_domain::balance::FACTORY_BATCH_SIZE * 2;

    for caravan in state.caravans.values() {
        if caravan.owner != player.id {
            continue;
        }
        let CaravanState::Idle { location: cur_city } = caravan.state else {
            continue;
        };
        // En yüksek stoklu mamul ve en kârlı hedef şehri bul.
        let mut best: Option<(ProductKind, CityId, u32, i64)> = None;
        for product in ProductKind::FINISHED_GOODS {
            let stock = player.inventory.get(cur_city, product);
            if stock < dispatch_threshold {
                continue;
            }
            let local_price = state
                .reference_price(cur_city, product)
                .map(|m| m.as_cents())
                .unwrap_or(0);
            for to_city in CityId::ALL {
                if to_city == cur_city {
                    continue;
                }
                let to_price = state
                    .best_bid(to_city, product)
                    .map(|(p, _)| p.as_cents())
                    .or_else(|| state.reference_price(to_city, product).map(|m| m.as_cents()))
                    .unwrap_or(0);
                let profit = to_price - local_price;
                if profit <= 0 {
                    continue;
                }
                if best.is_none_or(|(_, _, _, p)| profit > p) {
                    best = Some((product, to_city, stock.min(caravan.capacity), profit));
                }
            }
        }
        if let Some((product, to_city, qty, _)) = best {
            let mut cargo = Cargo::new();
            if cargo.add(product, qty).is_ok() {
                out.push(ActionCandidate::DispatchCaravan {
                    caravan_id: caravan.id,
                    from: cur_city,
                    to: to_city,
                    cargo,
                });
            }
        }
    }
    out
}

/// Sanayici'nin mamul satış kontratı önerileri. Stok mamul varsa Public
/// propose. Tüccar accept ederse 5 tick sonra teslim, kervan ile dağıtım.
/// v8.26: Şu an çağrılmıyor (NPC propose kapalı). İleride stok escrow
/// eklenirse tekrar açılabilir.
#[allow(dead_code)]
fn enumerate_contract_proposals(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    use moneywar_domain::{ContractProposal, ContractState, ListingKind};

    // Cap: aynı anda max 1 aktif kontrat (seller olarak)
    let active = state
        .contracts
        .values()
        .filter(|c| c.seller == player.id)
        .filter(|c| matches!(c.state, ContractState::Proposed | ContractState::Active))
        .count();
    if active >= 1 {
        return Vec::new();
    }

    // En çok stok mamul nerede? (city, product, qty) bul.
    // v8.25 fix2: Stok ≥ qty × 3 (90 birim) — qty × 2 hâlâ %57 cay
    // veriyordu. 8 tick içinde 60 stok satılabiliyor (Sanayici Cross
    // policy + Alıcı'nın ısrarlı BUY). 90 birim güvenli buffer.
    let mut best_stock: Option<(CityId, ProductKind, u32)> = None;
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_finished() || qty < 90 {
            continue;
        }
        if best_stock.is_none_or(|(_, _, q)| qty > q) {
            best_stock = Some((city, product, qty));
        }
    }
    let Some((city, product, _qty)) = best_stock else {
        return Vec::new();
    };

    // Fiyat: bu şehrin mamul baseline × 1.05 (Sanayici margin).
    // Tüccar zaten %95 markdown ile başka şehre satar, kâr fırsatı.
    let Some(reference) = state.reference_price(city, product) else {
        return Vec::new();
    };
    let unit_price_cents = reference.as_cents().saturating_mul(105) / 100;
    if unit_price_cents <= 0 {
        return Vec::new();
    }
    let unit_price = Money::from_cents(unit_price_cents);
    let quantity = 30u32;
    let total = unit_price_cents.saturating_mul(i64::from(quantity));
    let seller_deposit = Money::from_cents(total / 20); // %5
    let buyer_deposit = Money::from_cents(total / 20);
    if player.cash.as_cents() < seller_deposit.as_cents() {
        return Vec::new();
    }
    // v8.25: 5 → 8 tick. Sanayici 5 tick içinde satılan mamulü teslim
    // edemiyordu (breach). 8 tick buffer + stok≥60 kontrolü ile breach %67
    // azalmalı.
    let delivery_tick = match state.current_tick.checked_add(8) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let proposal = ContractProposal {
        seller: player.id,
        listing: ListingKind::Public,
        product,
        quantity,
        unit_price,
        delivery_city: city, // Sanayici kendi fab şehrinde teslim eder
        delivery_tick,
        seller_deposit,
        buyer_deposit,
    };
    vec![ActionCandidate::ProposeContract(proposal)]
}

/// Sanayici'nin Tüccar tarafından önerilen kontratları kabul etme adayları.
fn enumerate_contract_accepts(
    state: &GameState,
    player: &Player,
    needed_raws: &std::collections::BTreeSet<ProductKind>,
) -> Vec<ActionCandidate> {
    use moneywar_domain::ContractState;

    if needed_raws.is_empty() {
        return Vec::new();
    }
    // Cap: aktif kontrat varsa pas
    let active = state
        .contracts
        .values()
        .filter(|c| c.accepted_by == Some(player.id))
        .filter(|c| matches!(c.state, ContractState::Active))
        .count();
    if active >= 1 {
        return Vec::new();
    }

    // v8.24: Fab şehri kısıtlaması GEVŞETİLDİ. Önceki sürüm
    // delivery_city == fab_city istiyordu — Tüccar genelde "richest_city"
    // seçer, Sanayici fab şehri farklı olabilir. Sonuçta 5 propose / 0
    // accept. Yeni: fab varsa **herhangi bir şehirde** ham gelsin yeter,
    // Sanayici sonra kervan ile lojistik yapar (Sanayici kervan kapasitesi
    // 500'e çıktı v8.24).
    let has_any_fab = state.factories.values().any(|f| f.owner == player.id);
    if !has_any_fab {
        return Vec::new();
    }

    // İlk uyumlu Public/Personal kontratı kabul et
    for contract in state.contracts.values() {
        if contract.state != ContractState::Proposed {
            continue;
        }
        if contract.seller == player.id {
            continue;
        }
        // Personal ise kendisine olmalı
        if let moneywar_domain::ListingKind::Personal { target } = contract.listing {
            if target != player.id {
                continue;
            }
        }
        if !needed_raws.contains(&contract.product) {
            continue;
        }
        // Buyer deposit affordable mı
        if player.cash.as_cents() < contract.buyer_deposit.as_cents() {
            continue;
        }
        return vec![ActionCandidate::AcceptContract {
            contract_id: contract.id,
        }];
    }
    Vec::new()
}

fn scale_pct(price: Money, pct: i64) -> Money {
    Money::from_cents(price.as_cents().saturating_mul(pct) / 100)
}

/// Kuracak fab hedefini seç: **dünyada** henüz fab kurulmamış (city, mamul)
/// çiftlerinden birini deterministik döner. Yoksa kendi fab'larının olmadığı
/// kombinasyonu, en son fallback olarak `None`.
///
/// Önceki sürüm sadece `f.owner == player.id` filtresine bakıyordu → 5
/// Sanayici NPC'si **birbirinden habersiz** hepsi Istanbul-Kumas'a yığılıyordu
/// → Ankara/İzmir'de fab yok → off-specialty ham talebi olmuyordu.
/// Yeni: dünyada hangi (city, product) boş, onu seç. Sanayici'ler doğal
/// olarak farklı şehirlere yayılır.
/// Sanayici fab kuruluş motivasyonu — iki aşamalı:
///
/// 1. **İlk fab**: player_id ile deterministic dağılım. 5 NPC aynı tick'te
///    karar verince hepsi "Ist-Kumas boş" görüyordu → yığılırdı. Şimdi NPC
///    kendi id modulo aday sayısı ile farklı (city, product) seçer →
///    Sanayici'ler doğal yayılır.
///
/// 2. **Sonraki fab**: en yüksek **profit margin** (mamul_price - raw_price).
///    Lüks talep şehirleri (Ist-Kumas 36₺, Ank-Un 36₺) çekici çünkü mamul
///    pahalı + ham aynı baseline. Sezgisel kârlı yatırım kararı.
fn pick_factory_target(state: &GameState, player: &Player) -> Option<(CityId, ProductKind)> {
    let world_taken: std::collections::BTreeSet<(CityId, ProductKind)> = state
        .factories
        .values()
        .map(|f| (f.city, f.product))
        .collect();

    // Boş aday listesi. v8.6'da denenen "demand-bucket'a fab kurma" filtresi
    // (B1) talep tarafını da boğuyordu — fab yasaklanan şehirde mamul SELL
    // emri olmuyor → o (city, mamul) bucket'ta 1500+ BUY 0 SELL ölü pazar.
    // v8.7: filtre kaldırıldı. Çiftçi demand qty/8 üretir → fab kısmi ham
    // bulur (~%27 zaman üretim). Geri kalan FactoryIdle, ama mamul SELL
    // emirleri çıkar → mamul bucket aktif kalır.
    let candidates: Vec<(CityId, ProductKind)> = CityId::ALL
        .iter()
        .flat_map(|c| ProductKind::FINISHED_GOODS.iter().map(move |p| (*c, *p)))
        .filter(|cp| !world_taken.contains(cp))
        .collect();

    if candidates.is_empty() {
        // Tüm 9 dolmuş — kendi sahibi olmadığı bir kombinasyon (overlap)
        let own_taken: std::collections::BTreeSet<(CityId, ProductKind)> = state
            .factories
            .values()
            .filter(|f| f.owner == player.id)
            .map(|f| (f.city, f.product))
            .collect();
        return CityId::ALL
            .iter()
            .flat_map(|c| ProductKind::FINISHED_GOODS.iter().map(move |p| (*c, *p)))
            .find(|cp| !own_taken.contains(cp));
    }

    let own_count = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .count();

    if own_count == 0 {
        // İlk fab — player_id ile deterministic farklı yer
        let idx = (player.id.value() as usize) % candidates.len();
        return Some(candidates[idx]);
    }

    // Sonraki fab — multi-faktör skorlama + player_id jitter.
    //   1. Margin (mamul - raw fiyatı)         → ağırlık +
    //   2. Rakip fab sayısı                     → ağırlık -
    //   3. Kendi fab sayısı (aynı çiftte)       → ağırlık -
    //   4. Player-id jitter                     → tick içi çakışma kırma
    //
    // Tick içinde state immutable — 5 NPC aynı anda aynı "en kârlı" seçeneği
    // görüyordu → yığılıyordu. Her NPC kendi player_id × tick hash'i ile
    // küçük rastgele jitter alır → farklı NPC'ler farklı seçer.
    let current_tick = state.current_tick.value();
    candidates.into_iter().max_by_key(|(city, product)| {
        let mamul_cents = state
            .reference_price(*city, *product)
            .map_or(0, |m| m.as_cents());
        let raw_cents = product
            .raw_input()
            .and_then(|raw| state.reference_price(*city, raw))
            .map_or(0, |m| m.as_cents());
        let margin = (mamul_cents - raw_cents).max(0);

        let rival_count = state
            .factories
            .values()
            .filter(|f| f.city == *city && f.product == *product && f.owner != player.id)
            .count() as i64;
        let own_count = state
            .factories
            .values()
            .filter(|f| f.city == *city && f.product == *product && f.owner == player.id)
            .count() as i64;
        // Aynı mamulün tüm sahipler/tüm şehirlerdeki toplam fab sayısı.
        // (city, product) bazlı own/rival, farklı şehirlerde aynı mamulü
        // istiflemeyi engellemiyordu — Zeytinyağı margin (60) Un (17) ve
        // Kumaş'ı (30) her zaman yenip 5 şehre Zeytinyağı dağıtıyordu, Un
        // hiç kurulmuyordu → Buğday talep tarafı sıfıra düşüyordu.
        // Global product penalty ürün çeşitliliği için pressure ekler.
        // v0.6.0 Faz 4 (Bugday arz fazla): 2× → 3×. Math:
        //   1. fab Zeytinyağı: 60/1=60 → kurulur
        //   2. fab Zeytinyağı: 60/(1+3)=15 ← Kumaş 30/1=30 kazanır
        //   3. fab Un: 17/1=17 ← Zeytinyağı 60/(1+6)=8.5, Kumaş 30/(1+3)=7.5
        // 3 fab dağılımı garantili: Zeytinyağı, Kumaş, Un. Un fab → Bugday
        // talebi 3× → 4 Bugday Çiftçi'si mal birikmesi azalır.
        let same_product_global = state
            .factories
            .values()
            .filter(|f| f.product == *product)
            .count() as i64;
        // Zeytinyağı için ek global penalty — Zeytin kıt, fazla Zeyt.yağı fab
        // kurulmasın. Global same_product yerine ürün bazlı ek ağırlık.
        let product_bias = if *product == ProductKind::Zeytinyagi { 2 } else { 0 };
        let competition_factor = 1 + 2 * rival_count + 3 * own_count + 3 * same_product_global + product_bias;
        let base_score = margin / competition_factor;

        // Specialty bonus: şehrin prime hammaddesi bu ürünün girdisiyle eşleşirse
        // +%50 bonus. Sanayici hammadde bol olan şehre fabrika kurmayı tercih eder
        // → idle azalır, üretim artar.
        let city_prime = state.city_specialty.get(city).copied();
        let product_raw = product.raw_input();
        // Specialty bonus sadece hammadde gerçekten bol ve ucuzsa geçerli.
        // Zeytin fiyatı 2× baseline'ı geçtiyse (kıtlık) bonus sıfır —
        // Sanayici o şehre daha fazla Zeytinyağı fabrikası kurmamalı.
        let raw_supply_ok = product_raw.map_or(true, |raw| {
            let ref_price = state.reference_price(*city, raw)
                .map(|m| m.as_cents()).unwrap_or(0);
            let baseline = state.price_baseline.get(&(*city, raw))
                .map(|m| m.as_cents()).unwrap_or(1).max(1);
            ref_price > 0 && ref_price < baseline * 2
        });
        let specialty_bonus = if city_prime.is_some() && city_prime == product_raw && raw_supply_ok {
            base_score / 2 / (1 + same_product_global / 2)
        } else {
            0
        };

        // Jitter: NPC × tick × (city, product) hash'i ile. Marjın %20'si
        // kadar varyans → kararı sallar ama yön kaybetmez.
        let hash_seed = player
            .id
            .value()
            .wrapping_mul(31)
            .wrapping_add(u64::from(current_tick))
            .wrapping_mul(17)
            .wrapping_add(*city as u64)
            .wrapping_mul(7)
            .wrapping_add(*product as u64);
        let jitter = ((hash_seed % 100) as i64) * margin.max(1) / 500;
        base_score + specialty_bonus + jitter
    })
}

/// Tax-aware satın alma qty.
fn affordable_qty(cash: Money, unit_price: Money, want: u32) -> u32 {
    let unit_with_tax = unit_price
        .as_cents()
        .saturating_mul(100 + TRANSACTION_TAX_PCT)
        / 100;
    if unit_with_tax <= 0 {
        return 0;
    }
    let max_qty_i64 = cash.as_cents() / unit_with_tax;
    let max_qty = u32::try_from(max_qty_i64).unwrap_or(u32::MAX);
    max_qty.min(want)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        Factory, FactoryId, NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn fresh() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn sanayici(cash_lira: i64) -> Player {
        Player::new(
            PlayerId::new(104),
            "san",
            Role::Sanayici,
            Money::from_lira(cash_lira).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Sanayici)
    }

    #[test]
    fn no_factory_emits_build_candidate() {
        let s = fresh();
        let p = sanayici(50_000);
        let cands = enumerate(&s, &p);
        let has_build = cands
            .iter()
            .any(|c| matches!(c, ActionCandidate::BuildFactory { .. }));
        assert!(has_build, "fab yoksa BuildFactory emit etmeli");
    }

    #[test]
    fn target_factories_reached_no_build() {
        let mut s = fresh();
        // TARGET_FACTORIES = 8, yüksek başlangıç nakdiyle tavan sanayici.
        let p = sanayici(500_000);
        // 5 şehir × 3 mamul = 15 slot ama TARGET=8; tam 8 fab kuruyoruz.
        let combos = [(CityId::Istanbul, ProductKind::Kumas),
                      (CityId::Istanbul, ProductKind::Un),
                      (CityId::Istanbul, ProductKind::Zeytinyagi),
                      (CityId::Ankara, ProductKind::Kumas),
                      (CityId::Ankara, ProductKind::Un),
                      (CityId::Ankara, ProductKind::Zeytinyagi),
                      (CityId::Izmir, ProductKind::Kumas),
                      (CityId::Izmir, ProductKind::Un)];
        for (i, (city, product)) in combos.iter().enumerate() {
            let fid = FactoryId::new(i as u64 + 1);
            let f = Factory::new(fid, p.id, *city, *product).unwrap();
            s.factories.insert(fid, f);
        }
        s.players.insert(p.id, p.clone());
        let cands = enumerate(&s, &p);
        let has_build = cands
            .iter()
            .any(|c| matches!(c, ActionCandidate::BuildFactory { .. }));
        assert!(!has_build, "TARGET_FACTORIES sayıda fab varsa Build durur");
    }

    #[test]
    fn no_factory_falls_back_to_specialty_raw() {
        // v0.6.0: 5 şehir × specialty = 5 BUY (Ist=Pamuk, Ank=Bug, Izm=Zey,
        // Bursa=Pamuk, Konya=Bug).
        let s = fresh();
        let p = sanayici(50_000);
        let cands = enumerate(&s, &p);
        let buy_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Buy, product, .. } if product.is_raw()))
            .count();
        assert_eq!(buy_count, 5, "fab yok → fallback specialty (5 BUY)");
        for c in &cands {
            if let ActionCandidate::SubmitOrder {
                side: OrderSide::Buy,
                city,
                product,
                ..
            } = c
            {
                assert_eq!(
                    *product,
                    city.cheap_raw(),
                    "fab yok → BUY {city:?}'in specialty'si"
                );
            }
        }
    }

    #[test]
    fn factory_drives_raw_demand_only_in_fab_city() {
        // v0.6.0 Faz 2: Sanayici sadece kendi fab şehrinde raw alır. Off-fab
        // şehirlerde likidite Spekülatör market maker tarafından sağlanır.
        // Ist'te Kumaş fab → Pamuk SADECE Ist'te BUY.
        let mut s = fresh();
        let p = sanayici(50_000);
        let fid = FactoryId::new(1);
        let f = Factory::new(fid, p.id, CityId::Istanbul, ProductKind::Kumas).unwrap();
        s.factories.insert(fid, f);
        s.players.insert(p.id, p.clone());
        let cands = enumerate(&s, &p);
        let pamuk_buys: Vec<_> = cands
            .iter()
            .filter_map(|c| match c {
                ActionCandidate::SubmitOrder {
                    side: OrderSide::Buy,
                    city,
                    product: ProductKind::Pamuk,
                    ..
                } => Some(*city),
                _ => None,
            })
            .collect();
        assert_eq!(pamuk_buys.len(), 1, "Pamuk talebi sadece fab şehrinde");
        assert_eq!(pamuk_buys[0], CityId::Istanbul, "fab şehri Istanbul");
    }

    #[test]
    fn no_cash_no_buy_candidates() {
        let s = fresh();
        let p = sanayici(0);
        let cands = enumerate(&s, &p);
        let buy_count = cands
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    ActionCandidate::SubmitOrder {
                        side: OrderSide::Buy,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(buy_count, 0);
    }

    #[test]
    fn finished_stock_yields_sell_candidates() {
        let s = fresh();
        let mut p = sanayici(50_000);
        p.inventory
            .add(CityId::Istanbul, ProductKind::Kumas, 100)
            .unwrap();
        let cands = enumerate(&s, &p);
        let sell_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, product, .. } if product.is_finished()))
            .count();
        assert!(sell_count >= 1, "mamul stok varsa SELL emit");
    }

    #[test]
    fn raw_stock_does_not_yield_sell() {
        let s = fresh();
        let mut p = sanayici(50_000);
        // Sanayici raw'ı satmaz (sadece mamul SAT).
        p.inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100)
            .unwrap();
        let cands = enumerate(&s, &p);
        let sell_raw = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, product, .. } if product.is_raw()))
            .count();
        assert_eq!(sell_raw, 0);
    }

    #[test]
    fn deterministic_no_rng() {
        let s = fresh();
        let p = sanayici(50_000);
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }
}
