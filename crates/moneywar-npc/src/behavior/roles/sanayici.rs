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
    CaravanState, Cargo, CityId, Factory, GameState, Money, OrderSide, Player, PlayerId,
    ProductKind, balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;
use crate::behavior::pricing::{CrossPolicy, derived_input_ceiling, marketable_ask, marketable_bid};

/// Fabrika sayısı üst sınırı — **yok**. Firma nakit yettiği sürece kurar.
///
/// Tek fren ekonomik: her fabrika için `build_cost` (kademeli artan) + fabrika
/// başına 12K işletme rezervi, üstüne periyodik maintenance gideri. Sabit bir
/// tavan yerine bu tercih edildi ki tekelleşme emergent kalsın.
///
/// Denendi ve geri alındı (2026-07-25): "aç fabrikan varken yenisini kurma"
/// freni Sanayici'nin kişi başı `PnL`'ini 57K → 30K'ya düşürdü. Darboğaz fabrika
/// sayısı değil, hammaddeyi piyasada kaybetmekti; çözüm türev talep tavanı
/// oldu (bkz. `pricing::derived_input_ceiling`).
const TARGET_FACTORIES: usize = usize::MAX;

/// Tekelci sömürü primi (puan). Dedektör tekeli onayladığında fiyat tabanına
/// eklenir — tekel kârlıdır, ama şişen fiyat rakipleri pazara çeker.
const MONOPOLY_PREMIUM_PCT: i64 = 25;

/// Fiyat savaşında hedefin ask'inin bu yüzdesine inilir (%95 = %5 altına gir).
/// Dedektörün undercut eşiği %98 olduğundan bu kırma kesin olarak sayılır.
const WAR_UNDERCUT_PCT: i64 = 95;
/// Savaşta bile inilmeyen taban: referans fiyatın bu yüzdesi. Savaş var diye
/// firma kendini iflasa sürüklemesin.
const WAR_PRICE_FLOOR_PCT: i64 = 60;

/// `target` firmasının bu pazardaki en düşük SELL fiyatı (order book'ta açık
/// emir varsa). Fiyat savaşının "kimin fiyatını kıracağım" sorusunu cevaplar.
fn lowest_ask_of(
    state: &GameState,
    target: PlayerId,
    city: CityId,
    product: ProductKind,
) -> Option<Money> {
    state
        .order_book
        .get(&(city, product))?
        .iter()
        .filter(|o| o.side == OrderSide::Sell && o.player == target)
        .map(|o| o.unit_price)
        .min()
}


/// Brain ile birlikte enumerate — Goal-bilinçli fabrika seçimi.
#[must_use]
pub fn enumerate_with_brain(state: &GameState, player: &Player, brain: Option<&crate::behavior::brain::AgentBrain>) -> Vec<ActionCandidate> {
    enumerate_inner(state, player, brain)
}

/// Geriye uyumluluk için brain'siz versiyon.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    enumerate_inner(state, player, None)
}

fn enumerate_inner(state: &GameState, player: &Player, brain: Option<&crate::behavior::brain::AgentBrain>) -> Vec<ActionCandidate> {
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
        // Büyüme freni: ilk 3 fab hızlı, 4-8 fab kademeli, 8+ serbest.
        // owned 4-7 arasında "orta faz" → 3 fab başlangıç fazı bitti, rekabetçi büyüme.
        // Tick bazlı rate limit: owned fabrika sayısına göre minimum tick eşiği.
        // owned fab başına X tick minimum geçmiş olmalı (büyüme doğrusal).
        // Reserve: her fabrika başına 12K operasyon nakiti gerekli.
        // owned=0: 12K → needed=12K < 50K ✓ (1. fab kurulur)
        // owned=1: 24K → needed=32K < 50K ✓ (2. fab)
        // owned=2: 36K → needed=44K < 50K ✓ (3. fab)
        // owned=3: 48K → needed=56K > 50K ✗ (BLOKE — kâr bekle)
        // Shadow doğru cash'i yansıtıyor, 4. fabrika için kâr gerekiyor.
        let reserve_per_owned = 12_000i64;
        let min_reserve_cents = (owned as i64 + 1) * reserve_per_owned * 100;
        let reserve_ok = player.cash.as_cents() >= next_cost.as_cents() + min_reserve_cents;

        // NOT — buraya "kadro bulabilecek misin" kapısı koymak denendi ve
        // **zarar verdi**. Gerekçe makuldü: işgücü havuzu t210'dan sonra
        // %100 dolu, t350'de 89 fabrikanın 33'ü tek işçisiz duruyor
        // (`moneywar-web/tests/labor_probe.rs`) ve komut tarafında 16.672
        // "no free labor" reddi birikiyor. Kadrosuz fabrika parası ödenmiş,
        // bakımı kesiliyor, ürettiği sıfır.
        //
        // Ölçüldü (20 oyun × 350 tick):
        //
        //   kapı                          batch  girdi-yok  Tüccar  makas  iflas
        //   yok (mevcut)                   3757       %52  369.306   2,4×    0,6
        //   havuzda tam kadro varsa        3122       %56  304.675   2,4×    0,8
        //   havuzda yarım kadro varsa      3278       %55  304.314   2,3×    0,8
        //   + kendi kadrosuzun varsa dur   2414       %63  301.461   2,2×    1,1
        //
        // Üçü de üretimi kısıyor ve Tüccar'ı %18'e varan oranda vuruyor.
        // Sebep: kadro fabrikalar arasında dönüyor — boş bina israf değil,
        // kapasite yedeği. Talep kaydığında hazır fabrika olan üretiyor,
        // kapı konunca o esneklik gidiyor. Makastaki 0,1-0,2 iyileşme
        // gürültü (n=20'de metrik 2,2-2,6 arası salınıyor).
        if reserve_ok
            && let Some((city, product)) = pick_factory_target(state, player, brain) {
                out.push(ActionCandidate::BuildFactory { city, product });
            }
    }

    // 1b) Devralma: zorda kalmış rakibin fabrikasını kap.
    //
    // Kurmaktan ucuz (bedelin %60'ı) ve rakibi zayıflatıyor. Kapıları motor
    // doğruluyor; burada yalnız gerçekten uygun hedef varsa aday üretiyoruz
    // ki reddedilecek komutla tur harcanmasın.
    //
    // Hedef seçimi: kendi ürettiğim ürünlerden başla — aynı pazarda ikinci
    // fabrika yoğunlaşma demek. Yoksa herhangi bir uygun hedef.
    out.extend(enumerate_acquisition(state, player));

    // 2) Ham madde AL — fab-bazlı talep (gerçek tedarik zinciri).
    //    Her fab'ın raw_input'unu hesapla. Sanayici Ist'te Kumaş fab kurmuşsa
    //    Pamuk her 3 şehirde de arar (Tüccar Ist'ten Ank'a getirebilir).
    //    Fab yoksa fallback: şehir specialty raw'ı.
    // Faz 2: tarifin TAMAMI alınır. Ana girdiyi alıp ek girdiyi unutmak
    // fabrikayı aç bırakır — çok parçalı üründe her parça ayrı tedariktir.
    let needed_raws: std::collections::BTreeSet<ProductKind> = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .flat_map(|f| f.product.recipe())
        .map(|(input, _)| input)
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
                Money::from_lira(moneywar_domain::balance::npc_base_price_raw_lira())
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
                .flat_map(|f| f.product.recipe())
                .map(|(input, _)| input)
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

                // Ödeme gücü **türev talepten** gelir: bu girdiyi çevirdiğim
                // mamulün fiyatı tavanı belirler, girdinin kendi baseline'ı
                // değil. Girdi baseline'ına bağlıyken fabrika sahibi aynı
                // hammaddeyi çevirip satan Tüccar/Spekülatör'den daha çekingen
                // teklif veriyordu ve kendi girdisini kaybediyordu.
                //
                // Bir girdi birden çok fabrikamı besliyorsa (aynı şehirde hem
                // Un hem Ekmek fabrikası Buğday/Un ister) en yüksek tavan
                // geçerli: girdiyi en değerli kullanan hat fiyatı belirler.
                let derived = state
                    .factories
                    .values()
                    .filter(|f| f.owner == player.id && f.city == city)
                    .filter(|f| f.product.recipe().iter().any(|(i, _)| *i == product))
                    .filter_map(|f| {
                        derived_input_ceiling(state, city, f.product, product, f.batch_size())
                    })
                    .max();

                // Türev tavan hesaplanamazsa (mamul fiyatı bilinmiyor) eski
                // baseline tabanlı davranışa düş.
                let fallback = state.effective_baseline(city, product).unwrap_or_else(|| {
                    // Ek girdiler mamul olabilir — ürünün kendi baz fiyatı.
                    Money::from_lira(product.base_price_lira()).unwrap_or(Money::ZERO)
                });
                let anchor = derived.unwrap_or(fallback);

                // Kademeli agresiflik: stok ne kadar azsa tavana o kadar yakın
                // teklif. Hep tavana gitmek fiyat keşfini öldürür.
                let premium_pct: i64 = match shortage {
                    0..=25  => 75,  // az eksik → temkinli
                    26..=60 => 85,  // orta eksik
                    61..=99 => 95,  // çok eksik
                    _       => 100, // sıfır stok → tavana kadar
                };
                let cash_ceiling = scale_pct(anchor, premium_pct);
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

    // 3) Mamul SAT — stok-baskılı pricing.
    //    Faz 4: brain.goal → fiyat (premium/undercut) + hacim kararını etkiler.
    //    Corner(c,p): tam baskın → monopol premium + daha büyük hacim flood.
    //    PriceWar(c,p,target): o firmayı ez → fiyatının altına gir (kişisel).
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_finished() || qty == 0 {
            continue;
        }
        // Faz 4: goal'e göre hacim ve fiyat stratejisi.
        // Flood fazla agresif olmasın — volume × 2 yeterli; fiyatı çok düşürme.
        // Faz 1 (entrika): savaşta hedefin fiyatı doğrudan kırılır.
        let mut war_price_cap: Option<Money> = None;
        let (goal_vol_mult, goal_price_adj, goal_force_cross): (u32, i64, bool) =
            if let Some(b) = brain {
                use crate::behavior::brain::Goal;
                match &b.goal {
                    Goal::Corner { city: gc, product: gp } if *gc == city && *gp == product => {
                        let ownership = b.ownership_of(city, product);
                        if ownership > 0.55 {
                            // Baskın: monopol premium, normal hacim
                            (1, 18, false)
                        } else {
                            // Köşeleme: daha fazla hacim, hafif indirim → piyasayı doldur
                            (2, -3, false)
                        }
                    }
                    Goal::PriceWar { city: gc, product: gp, target }
                        if *gc == city && *gp == product =>
                    {
                        // Fiyat savaşı kişiseldir: hedefin bu pazardaki en düşük
                        // ask'ini bul, altına gir. Hedef bu tick fiyat vermediyse
                        // yüzde bazlı undercut'a düşülür.
                        war_price_cap = lowest_ask_of(state, *target, city, product)
                            .map(|ask| scale_pct(ask, WAR_UNDERCUT_PCT));
                        (2, -10, true)
                    }
                    _ => (1, 0, false),
                }
            } else {
                (1, 0, false)
            };

        let base_qty = (qty / 2).clamp(1, 50);
        let quantity = base_qty.saturating_mul(goal_vol_mult).min(200);

        let reference = state.effective_baseline(city, product).unwrap_or_else(|| {
            Money::from_lira(moneywar_domain::balance::npc_base_price_finished_lira())
                .unwrap_or(Money::ZERO)
        });
        let cash_lira = player.cash.as_cents() / 100;
        let rival_fab_count = state.factories.values()
            .filter(|f| f.city == city && f.product == product && f.owner != player.id)
            .count();
        let own_fab_count = state.factories.values()
            .filter(|f| f.city == city && f.product == product && f.owner == player.id)
            .count();
        let trust_discount = {
            let trust = state.max_trust_in_bucket(player.id, city, product);
            if trust > 0.5 { 3i64 } else { 0i64 }
        };
        // Baz fiyat katmanı — rakip/stok durumuna göre.
        let base_floor_pct: i64 = if rival_fab_count == 0 && own_fab_count > 0 {
            120 - trust_discount
        } else if rival_fab_count == 1 && own_fab_count >= rival_fab_count {
            108 - trust_discount
        } else if cash_lira < 5_000 {
            78
        } else {
            match qty { 0..=49 => 95, 50..=99 => 90, _ => 85 }
        } - trust_discount;
        // Faz 1 (entrika): dedektörün onayladığı gerçek tekelse sömürü primi.
        // Bu prim aynı zamanda rakiplere "bu pazar şişti, gir" sinyalidir —
        // tekel-kırma dinamiğini besleyen şey tekelcinin kendi açgözlülüğüdür.
        let monopoly_premium: i64 = if state.intrigue.is_monopolist(player.id, city, product) {
            MONOPOLY_PREMIUM_PCT
        } else {
            0
        };
        // Faz 4: goal price adj uygula.
        let stock_floor_pct = (base_floor_pct + goal_price_adj + monopoly_premium).max(70);
        let pct_floor = scale_pct(reference, stock_floor_pct);
        // Faz 1: savaş fiyatı yüzde tabanını ezer — ama zarar tabanının
        // (referansın %60'ı) altına inilmez, intihar değil savaş.
        let stock_floor = match war_price_cap {
            Some(cap) => cap.max(scale_pct(reference, WAR_PRICE_FLOOR_PCT)).min(pct_floor),
            None => pct_floor,
        };

        let rival_sell: u32 = state
            .order_book
            .get(&(city, product))
            .map_or(0, |orders| {
                orders.iter()
                    .filter(|o| o.side == OrderSide::Sell && o.player != player.id)
                    .map(|o| o.quantity)
                    .sum()
            });
        let my_sell: u32 = state
            .order_book
            .get(&(city, product))
            .map_or(0, |orders| {
                orders.iter()
                    .filter(|o| o.side == OrderSide::Sell && o.player == player.id)
                    .map(|o| o.quantity)
                    .sum()
            });
        let policy = if goal_force_cross || rival_sell > my_sell.saturating_add(5) {
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

    // 4c) Özel çiftlik — yeterli fabrika VE minimum tick.
    // state.config.season_ticks hizli=90 olduğundan season_pct güvenilmez,
    // bu yüzden mutlak tick eşiği: t60 sonrası (350 tick'in ~%17'si).
    //
    // Fabrika eşiği 8'di ve **hiç kurulmuyordu**: ölçümde 350 tick boyunca
    // sıfır tarla, sıfır reddedilen komut — yani aday listeye hiç girmiyordu.
    // Sebep eşiğin ulaşılamaz olması; Sanayici'lerin fabrika dağılımı 2–5
    // arasında, en çoğu 5. Eşik muhtemelen fabrika sayıları daha yüksekken
    // yazılmıştı, denge çalışması yayılmayı kısınca erişilmez kaldı.
    //
    // 3'e çekildi. Kapı hâlâ dar: aday üretmek için ayrıca t60, nakit
    // tamponu (maliyet × 1.5) ve **gerçek** bir hammadde açığı gerekiyor.
    const MIN_FACTORIES_FOR_FARM: usize = 5;
    let current_tick = state.current_tick.value();
    if owned >= MIN_FACTORIES_FOR_FARM && current_tick >= 60 {
        out.extend(enumerate_private_farm(state, player));
    }

    // 4a-i) Tarla yükseltme — aktif tarla + nakit varsa lv artır (önce tarla yükselt)
    out.extend(enumerate_upgrade_farm(state, player));

    // 4a-ii) Fabrika yükseltme — bol nakit + aktif fab + makul seviye varsa güçlendir.
    out.extend(enumerate_upgrade(state, player));

    // 4b) Fabrika kapatma — nakit kritik + uzun atıl → kapat, sermaye kurtar.
    out.extend(enumerate_demolish(state, player));

    // v8.23: Açık Tüccar kontratlarını tara, fab ihtiyacına uyanı kabul et.
    // Cap: Sanayici aynı anda max 1 aktif buyer kontratı.
    out.extend(enumerate_contract_accepts(state, player, &needed_raws));

    // Kadro kararı — emek kıt, doğru fabrikaya koy.
    out.extend(enumerate_staffing(state, player));

    // Mamul kervan dispatch — stok yüksekse en pahalı şehre gönder.
    out.extend(enumerate_mamul_dispatch(state, player));

    // Sanayici propose'u **kasten kapalı** (fonksiyon duruyor, çağrılmıyor).
    //
    // Stok escrow'u geldikten sonra denendi: kontrat hacmi 19'dan 137/oyuna
    // fırladı ve breach %1.4'te kaldı, yani mekanik sağlam çalışıyor. Ama
    // denge bozuldu — adalet makası 3.3× → 4.6×, Sanayici kişi başı PnL
    // 102K → 94K, Tüccar 267K → 375K. Sebep: üretici mamulünü vadeli sabit
    // fiyattan bağlayınca artığı alan Tüccar'a devrediyor; kontrat fiyatı
    // spot beklentisini yansıtmıyor.
    //
    // Açmadan önce kontrat fiyatlamasının düzelmesi gerekir (vade primi /
    // beklenen spot). Tüccar tarafı açık ve orada net kazanç var.
    // out.extend(enumerate_contract_proposals(state, player));
    let _ = enumerate_contract_proposals as fn(&GameState, &Player) -> Vec<ActionCandidate>;

    out
}

/// Sanayici'nin mamul kervan dispatch adayları.
/// Stok `BATCH_SIZE` × 2 birimi aşarsa, en yüksek referans fiyatlı şehre gönder.
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
                .map_or(0, moneywar_domain::Money::as_cents);
            for to_city in CityId::ALL {
                if to_city == cur_city {
                    continue;
                }
                let to_price = state
                    .best_bid(to_city, product)
                    .map(|(p, _)| p.as_cents())
                    .or_else(|| state.reference_price(to_city, product).map(moneywar_domain::Money::as_cents))
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
        if let moneywar_domain::ListingKind::Personal { target } = contract.listing
            && target != player.id {
                continue;
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
/// 1. **İlk fab**: `player_id` ile deterministic dağılım. 5 NPC aynı tick'te
///    karar verince hepsi "Ist-Kumas boş" görüyordu → yığılırdı. Şimdi NPC
///    kendi id modulo aday sayısı ile farklı (city, product) seçer →
///    Sanayici'ler doğal yayılır.
///
/// 2. **Sonraki fab**: en yüksek **profit margin** (`mamul_price` - `raw_price`).
///    Lüks talep şehirleri (Ist-Kumas 36₺, Ank-Un 36₺) çekici çünkü mamul
///    pahalı + ham aynı baseline. Sezgisel kârlı yatırım kararı.
fn pick_factory_target(state: &GameState, player: &Player, brain: Option<&crate::behavior::brain::AgentBrain>) -> Option<(CityId, ProductKind)> {
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
        // ── BİLİNEN SORUN: fabrika dağılımı = ürün sıralaması ─────────────
        //
        // Aşağıdaki `find()` "enum sırasındaki ilk boşluk"u seçiyor; marja
        // da ihtiyaca da bakmıyor. Dünyadaki tüm (şehir, ürün) slotları
        // dolduktan sonra kurulan **her** fabrika bu yoldan geçtiği için
        // dağılım listenin kendisi oluyor:
        //
        //   gözlenen  Kumaş 13 · Un 12 · Zeytinyağı 8 · Şarap 7 · Elbise 6 · Ekmek 5 · Ziyafet 5
        //   liste     [Kumaş, Un, Zeytinyağı, Şarap, Elbise, Ekmek, Ziyafet]
        //
        // Zincirin tepesi bu yüzden hiç büyümüyor — Ziyafet listenin sonunda.
        //
        // Düzeltmesi denendi: yedek yol da ilk fabrikayla aynı skorlamadan
        // geçirildi. Dağılım gerçekten düzeldi (Kumaş 13→5, Un 12→5,
        // Ekmek 5→9, Ziyafet 5→14) ama **açlık tepeden dibe taşındı**:
        // 14 Ziyafet fabrikası 3.972 Ekmek için kapışır oldu, taban çöktü.
        //
        //   30 oyun × 350 tick     açlık  makas  Sanayici   Alıcı
        //   şimdiki (find)           %54   3.4x     85015   +8952
        //   skorlamalı, açık ağ. 0   %59   3.8x    116440   -9112
        //   skorlamalı, açık ağ. 10  %64   3.7x     85285  -12853
        //
        // Marj tek başına yanlış sinyal (ham ucuz olduğu için dibi ödüllendirir),
        // kıtlık sinyali de tek başına yanlış (tepeye yığar). Doğrusu **akış
        // dengesi**: bir Un fabrikası ~1.5 Ekmek fabrikasını, bir Ekmek
        // fabrikası ~1.4 Ziyafet fabrikasını besler. Skorun bu oranı hedeflemesi
        // gerekiyor — girdi bulunabilirliğine bakan gerçek bir tasarım işi,
        // tek sabitle çözülmüyor.
        //
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
        // İlk fab: player_id % 3 → ürün uzmanlığı belirle.
        // Her sanayici farklı mamul ürünüyle başlar → doğal uzmanlaşma temeli.
        // ID 104→Kumaş, 105→Un, 106→Zeytinyağı (mod değil ID-100 mod 3)
        let preferred_product = match (player.id.value().saturating_sub(100)) % 3 {
            0 => ProductKind::Kumas,
            1 => ProductKind::Un,
            _ => ProductKind::Zeytinyagi,
        };
        // Tercih edilen ürün için en iyi şehri seç (specialty varsa orası)
        let preferred = candidates.iter()
            .filter(|(_, p)| *p == preferred_product)
            .min_by_key(|(c, _)| {
                // Specialty şehiri tercih et (daha ucuz hammadde)
                let is_specialty = state.city_specialty.get(c)
                    .and_then(|raw| preferred_product.raw_input().map(|r| *raw == r))
                    .unwrap_or(false);
                usize::from(!is_specialty)
            })
            .copied();
        if let Some(target) = preferred {
            return Some(target);
        }
        // Fallback: id-bazlı genel seçim
        let idx = (player.id.value() as usize) % candidates.len();
        return Some(candidates[idx]);
    }

    // # Zincir farkındalığı denendi ve geri alındı
    //
    // Skor marja bakıyor, zincire bakmıyor. Canlıda t49'da sonucu şuydu:
    // **5 Ziyafet fabrikası, 0 Şarap fabrikası**. Ziyafet'in tarifi Şarap
    // istiyor; kimse üretmediği için beş fabrika da doğuştan ölü. Gerçek
    // bir firma önce "girdim nereden gelecek" diye sorar.
    //
    // Girdi tedarik edilebilirliği dört ayrı biçimde skora bağlandı
    // (20 oyun × 350 tick, taban = şimdiki hâli):
    //
    //   varyant                        makas  Sanayici   Alıcı  Ziyafet  Ekmek
    //   taban (marj + jitter)           3.8x     90668  -18313     %84   %138
    //   orantılı çarpan (ham dahil)     4.0x     82725  -30774     %73   %121
    //   orantılı çarpan (yalnız mamul)  4.4x     83476  -25992     %47   %102
    //   sert kapı (hepsi tedariksizse)  3.8x     90668  -18313     %84   %138  ← hiç ateşlenmedi
    //   sert kapı (biri tedariksizse)   4.5x     97022  -28930     %75   %112
    //
    // Dördü de tabanın altında. Sebep: **erken kurmak, doğru kurmaktan
    // değerli**. Ölü fabrikanın maliyeti 8.000₺ ve o para hane halkına
    // dönüyor (capex dağıtımı); zinciri bekleyip geç kurmanın maliyeti ise
    // kaybedilen üretim tick'leri. Açgözlü kurucu, ölü fabrika kursa bile
    // toplamda kazanıyor.
    //
    // Kurulamayan zincirin gerçek çözümü kurma kararında değil, girdinin
    // bulunabilirliğinde — fabrika girdisini bulabilseydi ölmezdi.
    //
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
            .map_or(0, moneywar_domain::Money::as_cents);
        // Gerçek kâr: (mamul_fiyat × çıktı_adedi) - (ham_fiyat × batch_size)
        // Çıktı oranı: Kumaş %80, Un %90, Zeytinyağı %100
        let output_pct = i64::from(product.output_ratio_pct());
        let batch = i64::from(moneywar_domain::balance::FACTORY_BATCH_SIZE);
        let gross_revenue = mamul_cents * (batch * output_pct / 100);
        let raw_cents = product
            .raw_input()
            .and_then(|raw| state.reference_price(*city, raw))
            .map_or(0, moneywar_domain::Money::as_cents);
        let raw_cost = raw_cents * batch;
        let margin = (gross_revenue - raw_cost).max(0) / batch; // normalize

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
        let product_bias = 0; // Zeytinyagi bias kaldırıldı — eşit fırsat
        // Rakip ağırlığını artır: kendi slot'una girmek çok pahalı → uzmanlaşma zorlar.
        // Sadece slot bazlı rekabet cezası — global ürün sayısı kaldırıldı.
        // Zeytinyağı yüksek marjıyla doğal seçilsin, yapay kısıtlama olmasın.
        let competition_factor = 1 + 10 * rival_count + 3 * own_count + product_bias;
        let base_score = margin / competition_factor;

        // Specialty bonus: şehrin prime hammaddesi bu ürünün girdisiyle eşleşirse
        // +%50 bonus. Sanayici hammadde bol olan şehre fabrika kurmayı tercih eder
        // → idle azalır, üretim artar.
        let city_prime = state.city_specialty.get(city).copied();
        let product_raw = product.raw_input();
        // Specialty bonus sadece hammadde gerçekten bol ve ucuzsa geçerli.
        // Zeytin fiyatı 2× baseline'ı geçtiyse (kıtlık) bonus sıfır —
        // Sanayici o şehre daha fazla Zeytinyağı fabrikası kurmamalı.
        let raw_supply_ok = product_raw.is_none_or(|raw| {
            let ref_price = state.reference_price(*city, raw)
                .map_or(0, moneywar_domain::Money::as_cents);
            let baseline = state.price_baseline.get(&(*city, raw))
                .map_or(1, |m| m.as_cents()).max(1);
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

        // Goal bonus: Corner modunda hedef ürüne büyük bonus → o ürüne odaklan.
        let corner_bonus: i64 = if let Some(b) = brain {
            match &b.goal {
                crate::behavior::brain::Goal::Corner { product: target_prod, .. }
                | crate::behavior::brain::Goal::PriceWar { product: target_prod, .. }
                    if target_prod == product => base_score * 10, // 10× → güçlü odak
                _ => 0,
            }
        } else { 0 };

        base_score + specialty_bonus + jitter + corner_bonus
    })
}

/// Kadro adayları — hangi fabrikaya işçi konacak, hangisinden çekilecek.
///
/// Emek dünyada kıt ([`LABOR_POOL_SIZE`]) ve ücret çalışan başına ödeniyor,
/// yani boş duran fabrikanın kadrosu saf zarar. Karar iki yönlü:
///
/// - **Çıkar:** fabrika uzun süredir üretmiyorsa (girdisi yok) kadroyu
///   sıfırla. Hem ücret yükünden kurtulur hem işçiyi havuza iade eder —
///   başka firma ya da kendi çalışan fabrikası alabilsin.
/// - **Al:** üretimi süren ama eksik kadrolu fabrikayı tam kadroya çıkar.
///
/// Tick başına en fazla bir hamle; kadro sürekli oynamasın.
///
/// [`LABOR_POOL_SIZE`]: moneywar_domain::balance::LABOR_POOL_SIZE
fn enumerate_staffing(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let threshold = moneywar_domain::balance::IDLE_FACTORY_THRESHOLD;
    let mine = || state.factories.values().filter(|f| f.owner == player.id);

    // Önce boşa ücret ödenen fabrikayı boşalt — nakit koruması öncelikli.
    if let Some(f) = mine()
        .filter(|f| f.employees > 0)
        .find(|f| f.is_atil(state.current_tick, threshold))
    {
        return vec![ActionCandidate::SetStaff {
            factory_id: f.id,
            employees: 0,
        }];
    }

    // Sonra çalışan ama eksik kadrolu fabrikayı doldur.
    //
    // # Bilinen kilitlenme — ve düzeltmesinin neden geri alındığı
    //
    // Buradaki "atıl **olmayan**" koşulu bir kilit yaratıyor:
    //
    //   1. fabrika tam kadroyla açılır
    //   2. girdi bulamaz, `IDLE_FACTORY_THRESHOLD` tick sonra atıl sayılır
    //   3. yukarıdaki kural "boşa ücret ödeme" deyip kadroyu sıfırlar
    //   4. bu kural atıl fabrikaya işçi vermez
    //   5. kadrosuz fabrika üretemez → atıl kalır → bir daha açılamaz
    //
    // Canlıda t49'da Ziyafet'in beş fabrikasının beşi de böyle kadrosuz
    // duruyordu, üstelik havuzda 63 boş işçi varken.
    //
    // Koşulu "girdisi varsa yeniden kadrola" diye gevşetmek **denendi ve
    // toplam üretimi düşürdü** (20 oyun × 350 tick):
    //
    //   ürün       şimdiki   koşulsuz aç   havuz boşken aç
    //   Ziyafet    2966 %84     1936 %57        2450 %77
    //   Ekmek     11813 %138   10095 %113      10876 %123
    //   makas         3.8x        4.4x            4.4x
    //   Alıcı      -18313      -26586          -29331
    //
    // Sebep: işçi kıt. Duran fabrikayı açmak, tam batch çalışabilecek
    // fabrikadan işçi alıp ancak çeyrek batch çalışacak olana vermek
    // demek. "Aç fabrikayı terk et" davranışı çirkin görünüyor ama kıt
    // emeği yoğunlaştırdığı için verimli.
    //
    // Doğru çözüm kadro tarafında değil: fabrika **zaten** girdisini
    // bulabilseydi kilit hiç oluşmazdı.
    // Atıl fabrikayı atlama kuralı **emek kıtken** doğru: duran fabrikayı
    // açmak, tam batch çalışabilecek olandan işçi çalmak demek. Ama havuzda
    // boşluk varken aynı kural fabrikaları boşuna boş bırakıyor — ölçümde
    // havuz %65 doluyken 106 fabrikanın 54'ü kadrosuzdu, yani işçi vardı,
    // işe alan yoktu. Yeni fabrika hiç üretmediği için "atıl" sayılıyor ve
    // bu yüzden hiç kadro alamıyordu: kilit.
    let employed: u32 = state
        .factories
        .values()
        .map(|f| f.employees)
        .sum::<u32>()
        + state.private_farms.values().map(|f| f.employees).sum::<u32>();
    let pool_slack = moneywar_domain::balance::labor_pool_at(state.current_tick.value())
        .saturating_sub(employed);
    let labor_is_scarce = pool_slack < moneywar_domain::balance::EMPLOYEES_PER_FACTORY_L1;

    if let Some(f) = mine()
        .filter(|f| !labor_is_scarce || !f.is_atil(state.current_tick, threshold))
        .find(|f| f.employees < f.required_employees())
    {
        return vec![ActionCandidate::SetStaff {
            factory_id: f.id,
            employees: f.required_employees(),
        }];
    }

    Vec::new()
}

/// Devralınabilir fabrika adayları.
///
/// Motor üç kapı uyguluyor: hedef en az `ACQUISITION_IDLE_TICKS` atıl,
/// sahibi `ACQUISITION_DISTRESS_CASH_LIRA` altında nakitte, alıcı bedeli
/// ödeyebiliyor. Burada aynı kapılara bakıyoruz — reddedilen komut, o tur
/// başka bir şey yapılamaması demek.
///
/// Sıralama: önce **kendi ürettiğim** ürünün fabrikası (aynı pazarda ikinci
/// tesis = yoğunlaşma), sonra en ucuz kapasite.
fn enumerate_acquisition(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    use moneywar_domain::balance::{
        ACQUISITION_DISTRESS_CASH_LIRA, ACQUISITION_IDLE_TICKS, ACQUISITION_PRICE_PCT,
    };

    let owned = u32::try_from(
        state
            .factories
            .values()
            .filter(|f| f.owner == player.id)
            .count(),
    )
    .unwrap_or(u32::MAX);
    // Motorla aynı formül: bedel sahiplik sayısıyla ağırlaşıyor.
    let escalation = 100
        + i64::from(owned).saturating_mul(moneywar_domain::balance::ACQUISITION_ESCALATION_PCT);
    let price_cents = moneywar_domain::Factory::build_cost(owned).as_cents()
        * ACQUISITION_PRICE_PCT
        / 100
        * escalation
        / 100;
    // Devralıp meteliksiz kalma: bedelin 1.5 katı nakit iste.
    if player.cash.as_cents() < price_cents.saturating_mul(3) / 2 {
        return Vec::new();
    }

    let my_products: std::collections::BTreeSet<_> = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .map(|f| f.product)
        .collect();

    let mut best: Option<(bool, moneywar_domain::FactoryId)> = None;
    for f in state.factories.values() {
        if f.owner == player.id || !f.is_atil(state.current_tick, ACQUISITION_IDLE_TICKS) {
            continue;
        }
        // Motorla aynı kapı: kadrosuz tesis ya da nakitsiz sahip.
        let seller_broke = state.players.get(&f.owner).is_some_and(|p| {
            p.cash.as_cents() <= ACQUISITION_DISTRESS_CASH_LIRA.saturating_mul(100)
        });
        if !seller_broke && f.employees > 0 {
            continue;
        }
        let synergy = my_products.contains(&f.product);
        // Sinerjili hedef önce; eşitlikte küçük id (deterministik).
        let better = match best {
            None => true,
            Some((s, id)) => (synergy, std::cmp::Reverse(f.id)) > (s, std::cmp::Reverse(id)),
        };
        if better {
            best = Some((synergy, f.id));
        }
    }

    best.map(|(_, factory_id)| vec![ActionCandidate::AcquireFactory { factory_id }])
        .unwrap_or_default()
}

/// Özel çiftlik kurma adayını listele — dikey entegrasyon hamlesi.
///
/// Aday, firmanın **en çok aç kaldığı** (şehir, ham madde) çiftidir: o
/// şehirdeki fabrikalarımın bir batch için istediği ham madde ile elimdeki
/// stok arasındaki fark. En büyük açık kazanır.
///
/// İki şey özellikle önemli:
/// - **Yalnız ham madde.** Tarla ham madde üretir; tier-2/3 fabrikanın
///   girdisi mamuldür (Ekmek → Un, Ziyafet → Ekmek). Bunlar elenmezse motor
///   komutu "`PrivateFarm` only produces raw materials" diye reddediyordu.
/// - **Kıtlığa göre sıra.** Eskiden aday `BTreeSet`'ten `find()` ile, yani
///   ürün sıralamasındaki ilk eleman olarak alınıyordu; firma zaten bol olan
///   hammaddeye tarla kurup asıl darboğazını beslemeden kalıyordu.
///
/// Koşullar: tarla kotası dolmamış + nakit (maliyet × 1.5) + gerçek açık.
fn enumerate_private_farm(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    use moneywar_domain::balance::PRIVATE_FARM_BUILD_COOLDOWN;
    use moneywar_domain::{CityId, ProductKind};

    let owned_farms = state
        .private_farms
        .values()
        .filter(|f| f.owner == player.id)
        .count();

    // Kurulum beklemesi — motor da aynı kuralı uyguluyor; burada da bakmak
    // reddedilecek komut üretmemek için (red = boşa geçen aday slotu).
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

    // Nakit kontrolü hedef seçildikten **sonra** yapılır: maliyet hem sahip
    // olunan tarla sayısıyla hem de hedef slottaki kalabalıkla büyüyor.
    // Önceden sabit maliyete bakılıyordu ve motor komutu "insufficient
    // funds" diye reddediyordu — ölçümde sezonda 229 boşa giden aday.

    // (şehir, ham) → bir batch'lik açık. Aynı şehirde aynı hammaddeye dayanan
    // birden çok fabrika varsa açıklar toplanır.
    let mut shortage: std::collections::BTreeMap<(CityId, ProductKind), i64> =
        std::collections::BTreeMap::new();
    for f in state.factories.values().filter(|f| f.owner == player.id) {
        let Some(raw) = f.product.raw_input() else { continue };
        if !raw.is_raw() {
            continue; // tarla mamul üretemez
        }
        // Motor ana girdiyi batch boyutuyla 1:1 tüketir (bkz. production.rs).
        let need = i64::from(f.batch_size());
        let have = i64::from(player.inventory.get(f.city, raw));
        *shortage.entry((f.city, raw)).or_default() += need - have;
    }

    // Zaten tarlam olan çiftleri çıkar — ikinci tarla aynı yere kurulmasın.
    for f in state.private_farms.values().filter(|f| f.owner == player.id) {
        shortage.remove(&(f.city, f.product));
    }

    // En büyük açık. Eşitlikte `max_by_key` sonuncuyu seçer; `BTreeMap`
    // sırası deterministik olduğu için sonuç da deterministik.
    let Some(((city, product), _)) = shortage
        .into_iter()
        .filter(|(_, gap)| *gap > 0)
        .max_by_key(|(_, gap)| *gap)
    else {
        return Vec::new();
    };

    let slot_taken = state
        .private_farms
        .values()
        .filter(|f| f.city == city && f.product == product)
        .count();
    let build_cost = moneywar_domain::PrivateFarm::build_cost(owned_farms, slot_taken)
        .unwrap_or(moneywar_domain::Money::ZERO);
    // Maliyetin 1.5 katı: tarlayı kurup meteliksiz kalmasın.
    let needed_cash =
        moneywar_domain::Money::from_cents(build_cost.as_cents().saturating_mul(3) / 2);
    if player.cash < needed_cash {
        return Vec::new();
    }

    vec![ActionCandidate::BuildPrivateFarm { city, product }]
}

/// Yükseltmeye uygun özel tarlalar — aktif tarla + nakit varsa.
fn enumerate_upgrade_farm(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    state.private_farms.values()
        .filter(|f| f.owner == player.id)
        .filter(|f| f.level < moneywar_domain::PrivateFarm::FARM_MAX_LEVEL)
        .filter_map(|f| {
            let cost = moneywar_domain::PrivateFarm::upgrade_cost(f.level)?;
            // 1.5× maliyet buffer
            let needed = moneywar_domain::Money::from_cents(cost.as_cents() * 3 / 2);
            if player.cash >= needed { Some(ActionCandidate::UpgradeFarm { farm_id: f.id }) }
            else { None }
        })
        .take(1) // tick başına max 1
        .collect()
}

/// Yükseltmeye uygun fabrikaları listele.
///
/// Yükseltme koşulları:
/// 1. Fabrika maksimum seviyenin altında (< 3).
/// 2. Yükseltme maliyetini karşılayacak nakit var (maliyet × 1.5 güvenlik buffer).
/// 3. Fabrika aktif — son `IDLE_FACTORY_THRESHOLD` tick içinde üretim yapmış.
///
/// Tick başına en fazla 1 yükseltme (kaynak dağılımı kontrolü).
fn enumerate_upgrade(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    use moneywar_domain::balance::FACTORY_MAX_LEVEL;

    let mut upgradeable: Vec<moneywar_domain::FactoryId> = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .filter(|f| f.level < FACTORY_MAX_LEVEL)
        .filter(|f| {
            // Aktif fabrika — atıl değil.
            !f.is_atil(state.current_tick, moneywar_domain::balance::IDLE_FACTORY_THRESHOLD)
        })
        .filter(|f| {
            // Nakit yeterli mi (maliyet × 1.5 buffer)?
            if let Some(cost) = moneywar_domain::Factory::upgrade_cost(f.level) {
                let needed = moneywar_domain::Money::from_cents(
                    cost.as_cents().saturating_mul(3) / 2
                );
                player.cash >= needed
            } else {
                false
            }
        })
        .map(|f| f.id)
        .collect();

    // Deterministik sıra — en düşük ID'li fab önce (en eski, en köklü).
    upgradeable.sort_unstable();

    upgradeable
        .into_iter()
        .take(1)
        .map(|factory_id| ActionCandidate::UpgradeFactory { factory_id })
        .collect()
}

/// Kapatılabilecek fabrikaları listele.
///
/// Kapatma koşulları (her ikisi de gerekli):
/// 1. Fabrika uzun süredir atıl (`IDLE_FACTORY_THRESHOLD` × 2 = 20 tick) — ham
///    madde yetersizliği veya mamul çok ucuz, sürdürülemez üretim.
/// 2. Nakit kritik (<8K) VEYA `PnL` son birkaç tickte negatif yönelimli.
///    (`PnL` sinyal brain'den gelir ama burada cash vekil olarak kullanılır.)
///
/// Tick başına en fazla 1 fabrika kapatılır (fazla çığ etkisi önlenir).
fn enumerate_demolish(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    const IDLE_CLOSE_TICKS: u32 = moneywar_domain::balance::IDLE_FACTORY_THRESHOLD * 2;
    const CASH_CRITICAL_LIRA: i64 = 8_000;

    let cash_lira = player.cash.as_cents() / 100;
    let cash_critical = cash_lira < CASH_CRITICAL_LIRA;

    // Atıl fabrikaları sırala: en uzun atıl → en önce kapat.
    let mut candidates: Vec<moneywar_domain::FactoryId> = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .filter(|f| {
            let idle = f.is_atil(state.current_tick, IDLE_CLOSE_TICKS);
            idle && cash_critical
        })
        .map(|f| f.id)
        .collect();

    // Deterministik sıra (FactoryId azalan — en eski fab önce).
    candidates.sort_unstable();

    // Tick başına en fazla 1 kapatma önerisi.
    candidates
        .into_iter()
        .take(1)
        .map(|factory_id| ActionCandidate::DemolishFactory { factory_id })
        .collect()
}

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

    // TARGET_FACTORIES sınırsız olduğu için bu test kaldırıldı.

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
