//! Ekonomi motoru — periyodik para akışı (wages, vergi, mahsul, maintenance).
//!
//! v7 closed-loop tasarımı (Vic3 ilhamı):
//! - **Wages:** Sanayici fabrikaları her 10 tick'te ücret öder → Alıcı'lara
//!   eşit dağıtılır. Fab sayısına bağlı değişken akış. **Bu ana gelir kaynağı.**
//! - **Mahsul:** Çiftçi NPC'lere her 8 tick ham madde refill (üretim)
//! - **Vergi:** Çiftçi/Toptancı/Sanayici cash'inden %2 her 10 tick → Alıcı'lara
//! - **Maintenance:** Her fab her 10 tick maintenance ücreti (sistem dışı, "amortisman")
//!
//! Eski sabit maaş (devlet sıfırdan para basma) kaldırıldı. Artık Alıcı geliri
//! tamamen Sanayici cebinden çıkıyor (closed loop).

use moneywar_domain::{
    CityId, GameState, MarketOrder, Money, NpcKind, OrderId, OrderSide, PlayerId, ProductKind,
    Tick,
    balance::{
        npc_base_price_finished_lira, SEED_COST_PER_RAW_LIRA, WORLD_FAB_PERIOD,
        WORLD_FAB_QTY_PER_PERIOD, WORLD_FAB_SELL_TTL, WORLD_PLAYER_ID_VALUE,
    },
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::report::{LogEntry, TickReport};

/// Wages (ücret) periyodu — her N tick'te aktif fab başına ücret kesilir.
/// v0.6.0 Faz 1 (talep cliff): 10 → 5. Toplam transfer sabit kalsın diye
/// `WAGE_PER_FACTORY_LIRA` da yarıya indi. Akışın sıklığı 2× → Alıcı cash'i
/// daha pürüzsüz akıyor → t40 cliff yumuşaması.
const WAGE_PERIOD: u32 = 5;

/// Bir kervanın mürettebatı. Tüccar da işçi çalıştırır — kervanı kimse
/// bedava sürmüyor.
const CREW_PER_CARAVAN: i64 = 1;

/// Bir Çiftçi'nin tarla ırgadı. Hasat emeksiz olmuyor.
const CREW_PER_FARMER: i64 = 3;

/// Depolama maliyeti periyodu — her N tick'te stok ücreti kesilir.
const STORAGE_PERIOD: u32 = 10;
/// Ücretsiz stok eşiği (birim) — bu miktarın altı ücretlendirilmez.
/// Sanayici'nin çalışma stoğunu korur.
///
/// **Ölçeğe bağlı.** Depolama birim başına mutlak fiyatlanmış; üretim
/// ölçeği büyüyünce mal adedi artıyor ama para sabit kalıyor ve gider
/// dolaşımdaki parayı yiyor. Ölçüm (20 oyun × 350 tick):
///
/// ```text
///   ölçek  depolama gideri  para arzı
///   ×100         128.809₺      -6.1%
///   ×1000        480.904₺     -35.9%
/// ```
///
/// Eşik ölçekle büyüyünce "çalışma stoğu bedava" kuralı ölçekten bağımsız
/// hale geliyor; gider stok/ölçek oranına bakıyor, mutlak adede değil.
fn storage_free_units() -> u32 {
    moneywar_domain::balance::scaled_output(25)
}
/// Standart depolama maliyeti (cents/birim/periyot) — Tüccar, Spek, Çiftçi.
const STORAGE_COST_CENTS: i64 = 6;
/// Sanayici depolama maliyeti — fabrika deposu var, daha ucuz.
const STORAGE_COST_SANAYICI_CENTS: i64 = 1;
/// Çalışan başına ücret — her wage period'da Sanayici'den çıkar, Alıcı'lara
/// (hane halkı) eşit dağıtılır. Ekonominin kapalı döngü bacağı.
///
/// Eskiden ücret **fabrika başına** sabitti (300₺) ve emek diye bir kavram
/// yoktu; üretim saf sermaye fonksiyonuydu. Artık fabrikanın kadrosu var,
/// ücret kadroya bağlı: firma işçi tutar, üretimi artar, ama ücret yükü de
/// artar — satamazsa batar. Seviye-1 kadrosu 3 kişi olduğundan kişi başı
/// 100₺ eski toplam akışı yaklaşık korur.
const WAGE_PER_EMPLOYEE_LIRA: i64 = moneywar_domain::balance::wage_per_employee_lira();

/// Fab maintenance (işletme gideri) periyodu.
const MAINTENANCE_PERIOD: u32 = 10;
/// Fabrika başına maintenance ücreti — Sanayici'den çekilir, sistem dışına
/// atılır (amortisman). Aktif/atıl fark etmez → boş fab kuran cezalı.
/// Anno 1800 inspiration: building maintenance.
/// F4 tuning: 100 → 250 (Sanayici tekel kıs). Çoklu fab pasif gider artar
/// → 3 fab × 250 × 9 sefer = 6.75K maintenance/sezon. Sanayici `PnL` +46K
/// → ~+15K hedef (Tüccar/Esnaf'la dengeli).
const MAINTENANCE_PER_FACTORY_LIRA: i64 = 250;

/// Alıcı tüketim periyodu — her N tick'te Alıcı mamul stoğunun bir kısmını
/// tüketir (envanterden silinir). Vic3 pop needs inspiration.
/// Mevcut sorun: Alıcı mamul alır ama tüketmez → varlık birikir → `PnL` pozitif.
/// Tüketici negatif `PnL` için consume mekaniği şart.
// v8.24: 5 → 8. Alıcı %38 daha az tüketir → BUY pressure düşer →
// tâtonnement asimetrisi yumuşar (fiyatlar monoton artmaz). Bkz. fiyat
// trend analizi: 18/18 bucket sezonda +50-99% kayıyordu.
const CONSUME_PERIOD: u32 = 8;

/// Mahsul refill periyodu — her N tick'te Çiftçi'lere stok inject.
const HARVEST_PERIOD: u32 = 8;
/// Mahsul miktarı (birim) — her Çiftçi'ye specialty ürünü.
/// v8.19 (A): 200-400 → 120-240. Esnaf emekli olunca ham BUY tarafı çöktü
/// (Sanayici 5 + Spek 3 alıcı, Çiftçi 6 × 80 birim/tick = 480 ham/tick arz
/// emilemiyor). Match eff %12-39, `FactoryIdle` 1260. Mahsul %40 düşürülür
/// → Çiftçi sezon başına ~3500 birim üretir (eski 5800).
const HARVEST_QTY_MIN: u32 = 60;
const HARVEST_QTY_MAX: u32 = 120;

/// Vergi periyodu — şu an aktif değil (wages closed loop yeterli).
const TAX_PERIOD: u32 = 10;
/// Vergi yüzdesi — şu an aktif değil. Gelecekte gerekirse `tick_economy`'de açılır.
#[allow(dead_code)]
const TAX_PCT: i64 = 2;

/// `advance_tick` içinde çağrılır — periyodik ekonomi akışlarını uygular.
pub(crate) fn tick_economy(
    state: &mut GameState,
    rng: &mut ChaCha8Rng,
    report: &mut TickReport,
    tick: Tick,
) {
    let t = tick.value();

    // Wages — Sanayici fab'ları işçilere (Alıcı'lara) ücret ödesin (closed loop)
    if t > 0 && t.is_multiple_of(WAGE_PERIOD) {
        pay_factory_wages(state, report, tick);
    }

    // Depolama maliyeti — büyük stok tutmanın bedeli.
    // Sanayici'ye indirimli (fabrika deposu), diğerlerine standart fiyat.
    if t > 0 && t.is_multiple_of(STORAGE_PERIOD) {
        charge_storage_costs(state, report, tick);
    }

    // Basılan sabit maaş **kaldırıldı**. Hane halkı geliri artık iki gerçek
    // kanaldan geliyor: fabrika ücretleri (çalışan başına) ve sermaye
    // harcamasının inşaat/makine ödemesi. Ölçüm: basım sezonda 1.86M₺ idi,
    // capex kanalı 2.04M₺ taşıyor — yerini fazlasıyla dolduruyor ve bu para
    // yoktan yaratılmıyor, firmanın cebinden çıkıyor.

    // Maintenance — fab işletme gideri (Anno mekaniği): Sanayici'den çek, sistem dışı
    if t > 0 && t.is_multiple_of(MAINTENANCE_PERIOD) {
        charge_factory_maintenance(state, report, tick);
    }

    // v8.25: Consume + harvest her tick çağrılır, içinde her player kendi
    // offset'inde tetiklenir (player_id % PERIOD). Eski "her PERIOD tickte
    // tüm Alıcı/Çiftçi aynı anda" senkronize emir patlaması yaratıyordu →
    // tüm bucket aynı yönde kayıyordu (15 artan / 13 azalan blok'lar).
    // Offset ile ritim dağılır; bucket'lar birbirinden bağımsız hareket eder.
    if t > 0 {
        consume_alici_inventory(state, tick);
        harvest_ciftci_stock(state, rng, report, tick);
    }

    // World fab — engine-driven baseline mamul üretim her şehir × her mamul.
    // Sanayici NPC fab dağılımı 9 mamul bucket'ı kapsayamadığında (5 NPC ×
    // 1-2 fab = 6-7 fab) kalan bucket'lara baseline arz garanti. World
    // player yoksa (sim) no-op — TUI seed_world World'u oluşturur.
    if t > 0 && t.is_multiple_of(WORLD_FAB_PERIOD) {
        tick_world_factories(state, report, tick);
    }

    // Vergi KALDIRILDI — wages ile çakışıyordu (çift gelir transferi).
    // Closed loop artık sadece wages üzerinden: Sanayici → Alıcı.
    // Vergi gerekirse çağrılır (constants kalıyor, fonksiyon ölü kod değil).
    let _ = TAX_PERIOD;
}

/// Depolama maliyeti — eşik üzeri stok için nakit keser, sisteme sink eder.
/// Sanayici fabrika deposu avantajıyla daha düşük ücret öder.
fn charge_storage_costs(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let mut pool_cents: i64 = 0;
    let player_ids: Vec<moneywar_domain::PlayerId> =
        state.players.keys().copied().collect();

    for pid in player_ids {
        let is_sanayici = state
            .players
            .get(&pid)
            .is_some_and(|p| p.npc_kind == Some(NpcKind::Sanayici));
        // Birim maliyeti de ölçeğe bölünür. Eşiği büyütmek tek başına
        // yetmiyor: eşik üstü miktar da ölçekle büyüdüğü için toplam gider
        // yine ölçekle çarpılırdı.
        let scale = i64::from(moneywar_domain::balance::PRODUCTION_SCALE_PCT).max(1);
        let base = if is_sanayici {
            STORAGE_COST_SANAYICI_CENTS
        } else {
            STORAGE_COST_CENTS
        };
        let cost_per_unit = (base * 100 / scale).max(1);

        let mut total_cost_cents: i64 = 0;
        for city in moneywar_domain::CityId::ALL {
            for product in moneywar_domain::ProductKind::ALL {
                let qty = state
                    .players
                    .get(&pid)
                    .map_or(0, |p| p.inventory.get(city, product));
                let chargeable = i64::from(qty.saturating_sub(storage_free_units()));
                if chargeable > 0 {
                    total_cost_cents += chargeable * cost_per_unit;
                }
            }
        }

        if total_cost_cents <= 0 {
            continue;
        }
        let Some(player) = state.players.get_mut(&pid) else { continue };
        let cost = moneywar_domain::Money::from_cents(
            total_cost_cents.min(player.cash.as_cents()),
        );
        if cost.as_cents() > 0 {
            let _ = player.debit(cost);
            pool_cents = pool_cents.saturating_add(cost.as_cents());
            report.push(LogEntry {
                tick,
                actor: Some(pid),
                event: crate::LogEvent::StorageCost { player: pid, amount: cost },
            });
        }
    }

    // Depo gideri de bir ödemedir: ambar işçisine, hamala, kiraya gider.
    // Yok etmek ekonomiden **stokla orantılı** para sızdırıyordu ve stok
    // sezon boyunca biriktiği için sızıntı hızlanıyordu — sorun ancak uzun
    // sezonda görünüyordu. Ölçüm (20 oyun, sızıntı açıkken):
    //
    //   tick   makas   Sanayici    Tüccar    Alıcı  iflas   para
    //    350    2,4×    227.521   369.306   61.948    0,6   -6,0%
    //    420    2,9×    341.391   219.439   -5.309    1,1   -7,8%
    //    460    5,0×    397.409   -38.353  -27.017    3,0   -8,9%
    //    500     38×    465.530   -94.784  -37.948    3,8  -10,0%
    //
    // En çok stok tutan Tüccar en çok ödüyordu; tüketici parasız kalınca
    // talep düşüyor, Tüccar satamıyor ama depoyu ödemeye devam ediyordu.
    // Bakım gideri aynı sebeple daha önce haneye yönlendirilmişti.
    distribute_capex_to_households(state, report, tick, Money::from_cents(pool_cents));
}

/// Wages — Sanayici fabrikalarından ücret keser, Alıcı'lara eşit dağıtır.
/// Closed loop: Alıcı'nın geliri Sanayici cebinden gelir (eski sabit maaş yerine).
/// Vic3 inspiration: building wages → pop income.
/// Dünyadaki toplam istihdam — fabrika **ve** tarla.
///
/// Tarlalar uzun süre bedava işçiyle çalışıyordu ve havuz muhasebesinde
/// görünmüyorlardı. Aynı havuzdan çekmeleri, tarla sayısına sert kota
/// koymayı gereksizleştiren şey: yirmi tarla kuran Sanayici kendi
/// fabrikalarını kadrosuz bırakır.
#[must_use]
pub(crate) fn total_employed(state: &GameState) -> u32 {
    let factories: u32 = state.factories.values().map(|f| f.employees).sum();
    let farms: u32 = state.private_farms.values().map(|f| f.employees).sum();
    factories.saturating_add(farms)
}

fn pay_factory_wages(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    // Her Sanayici NPC'si kaç fab'a sahip → o kadar wage öder.
    let factories_by_owner: std::collections::BTreeMap<PlayerId, u32> = {
        let mut map = std::collections::BTreeMap::new();
        for f in state.factories.values() {
            *map.entry(f.owner).or_insert(0) += 1;
        }
        // Tarla ırgadı da maaş alır. Tarlayı bedava işgücüyle çalıştırmak,
        // ham maddeyi hem pazarı hem emek piyasasını atlayarak üretmek
        // demekti — iki kere haksız avantaj.
        for f in state.private_farms.values() {
            map.entry(f.owner).or_insert(0);
        }
        map
    };
    if factories_by_owner.is_empty() {
        return;
    }

    let alici_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Alici))
        .map(|(id, _)| *id)
        .collect();
    if alici_ids.is_empty() {
        return;
    }

    // Toplam wage havuzu — sadece AKTİF fabrikalardan kesilir (idle = ücret yok).
    let mut wage_pool_cents: i64 = 0;
    for owner in factories_by_owner.keys() {
        // Aktif fab sayısı: son WAGE_PERIOD tick'te üretim yapmış olanlar.
        // IDLE_THRESHOLD (10) çok gevşekti — fabrika 3-4 tick'te bir üretip
        // her zaman "aktif" sayılıyordu. WAGE_PERIOD (5) ile gerçekten
        // son dönemde üretim yaptıysa ücret öde, yapmadıysa ödeme.
        // Ücret **çalışan başına** ödenir; atıl fabrikanın da kadrosu varsa
        // ücreti çıkar (işçi çalışmasa da maaşını alır — firma ya işçiyi
        // çıkarır ya yükü taşır). Kadro = 0 olan fabrika ücret doğurmaz.
        let factory_heads: i64 = state
            .factories
            .values()
            .filter(|f| f.owner == *owner)
            .map(|f| i64::from(f.employees))
            .sum();
        let farm_heads: i64 = state
            .private_farms
            .values()
            .filter(|f| f.owner == *owner)
            .map(|f| i64::from(f.employees))
            .sum();
        let headcount = factory_heads.saturating_add(farm_heads);
        if headcount == 0 {
            continue;
        }
        let wage_per_owner_cents = WAGE_PER_EMPLOYEE_LIRA
            .saturating_mul(headcount)
            .saturating_mul(100);
        if let Some(p) = state.players.get_mut(owner) {
            let actual_cents = wage_per_owner_cents.min(p.cash.as_cents());
            if actual_cents <= 0 {
                continue;
            }
            let amount = Money::from_cents(actual_cents);
            if p.debit(amount).is_ok() {
                wage_pool_cents = wage_pool_cents.saturating_add(actual_cents);
            }
        }
    }
    // ── Tüccar ve Çiftçi de işçi çalıştırır ──────────────────────────────
    //
    // Ücreti uzun süre **yalnız Sanayici** ödüyordu; Tüccar'ın kervancısı,
    // Çiftçi'nin ırgatı bedava çalışıyordu. Bu, hane halkı gelirini
    // yükseltmeyi imkânsız kılıyordu: maaş artırılınca fatura tek başına
    // Sanayici'ye biniyor, ama tüketici o parayla ağırlıkla **Tüccar'dan**
    // alışveriş yaptığı için para aracıya gidiyordu.
    //
    // Ölçüm (maaş 60 → 240): Sanayici 90.668 → 18.246 çökerken Tüccar
    // 236.106 → 302.611'e, Spekülatör 70.232 → 129.136'ya çıkıyordu.
    // Yani ücret artışı emeğe değil aracıya yarıyordu.
    //
    // Artık her rol istihdam ettiği kadar öder: Tüccar kervan başına
    // mürettebat, Çiftçi tarla başına ırgat.
    for (id, player) in state
        .players
        .iter()
        .map(|(id, p)| (*id, p.npc_kind))
        .collect::<Vec<_>>()
    {
        let headcount: i64 = match player {
            Some(NpcKind::Tuccar) => {
                let caravans = state.caravans.values().filter(|c| c.owner == id).count();
                i64::try_from(caravans).unwrap_or(0) * CREW_PER_CARAVAN
            }
            Some(NpcKind::Ciftci) => CREW_PER_FARMER,
            _ => 0,
        };
        if headcount == 0 {
            continue;
        }
        let owed = WAGE_PER_EMPLOYEE_LIRA
            .saturating_mul(headcount)
            .saturating_mul(100);
        if let Some(p) = state.players.get_mut(&id) {
            let actual = owed.min(p.cash.as_cents());
            if actual > 0 && p.debit(Money::from_cents(actual)).is_ok() {
                wage_pool_cents = wage_pool_cents.saturating_add(actual);
            }
        }
    }

    if wage_pool_cents == 0 {
        return;
    }

    // Alıcı'lara eşit dağıt.
    let per_alici = wage_pool_cents / i64::from(u32::try_from(alici_ids.len()).unwrap_or(1));
    if per_alici <= 0 {
        return;
    }
    let amount = Money::from_cents(per_alici);
    for pid in alici_ids {
        if let Some(p) = state.players.get_mut(&pid) {
            let _ = p.credit(amount);
            report.push(LogEntry::economy_salary(tick, pid, amount));
        }
    }
}

/// Toplanan işlem vergisini hane halkına dağıt — kamu harcaması kanalı.
///
/// Vergi `market.rs`'te alıcıdan ek olarak kesiliyor ama satıcıya
/// verilmiyordu; aradaki fark yok oluyordu (ölçüm: sezonda ~145K₺ sızıntı).
/// `EconomyTaxRedistributed` olayı tanımlıydı ama hiç yayınlanmıyordu —
/// yeniden dağıtım tasarlanmış, bağlanmamıştı.
///
/// [`distribute_capex_to_households`] ile aynı bölüştürme kuralı: eşit pay,
/// artan kuruş ilk haneye. Tek fark bu fonksiyonun ayrı bir olay yayması —
/// vergi geliri ücretten ayrı izlenebilsin.
pub(crate) fn distribute_tax_to_households(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    amount: Money,
) {
    if !amount.is_positive() {
        return;
    }
    let households: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Alici))
        .map(|(id, _)| *id)
        .collect();
    if households.is_empty() {
        return;
    }
    let n = i64::from(u32::try_from(households.len()).unwrap_or(1));
    let share = amount.as_cents() / n;
    let mut remainder = amount.as_cents() - share * n;
    let mut paid: i64 = 0;

    for pid in &households {
        let mut cents = share;
        if remainder > 0 {
            cents += 1;
            remainder -= 1;
        }
        if cents <= 0 {
            continue;
        }
        if let Some(p) = state.players.get_mut(pid) {
            let _ = p.credit(Money::from_cents(cents));
            paid = paid.saturating_add(cents);
        }
    }
    if paid > 0 {
        report.push(LogEntry::economy_tax_redistributed(
            tick,
            Money::from_cents(paid),
            u32::try_from(households.len()).unwrap_or(0),
        ));
    }
}

/// Sermaye harcamasını hane halkına gelir olarak dağıt.
///
/// Fabrika kurmak, yükseltmek, kervan ya da tarla almak gerçek hayatta parayı
/// **yok etmez** — inşaat işçisine, makine üreticisine ödeme yapar ve o para
/// gelir olarak dolaşıma döner. Motor eskiden yatırımı sadece nakitten
/// düşüyordu; ölçüm sonucu para arzı sezon boyunca %28.5 kuruyordu ve
/// kaybın 2.04M'i bu kanaldandı.
///
/// Alıcı NPC'leri hane halkını temsil ettiği için pay onlara eşit bölünür.
/// Bölümden artan kuruş ilk alıcıya yazılır — para korunumu bozulmasın.
/// Hane halkı yoksa (test dünyaları) para yine yok olur, çağıran zaten
/// nakitten düşmüştür.
pub(crate) fn distribute_capex_to_households(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    amount: Money,
) {
    if !amount.is_positive() {
        return;
    }
    let households: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Alici))
        .map(|(id, _)| *id)
        .collect();
    if households.is_empty() {
        return;
    }
    let n = i64::from(u32::try_from(households.len()).unwrap_or(1));
    let share = amount.as_cents() / n;
    let mut remainder = amount.as_cents() - share * n;

    for pid in households {
        let mut cents = share;
        if remainder > 0 {
            cents += 1;
            remainder -= 1;
        }
        if cents <= 0 {
            continue;
        }
        let money = Money::from_cents(cents);
        if let Some(p) = state.players.get_mut(&pid) {
            let _ = p.credit(money);
            report.push(LogEntry::economy_salary(tick, pid, money));
        }
    }
}

/// Alıcı NPC'lerinin mamul stoğunu tüket (envanterden sil). v8.25: Her Alıcı
/// kendi `player_id` offset'inde tetiklenir → senkronize toplu BUY patlaması
/// kalkar, ritim dağılır.
fn consume_alici_inventory(state: &mut GameState, tick: Tick) {
    let t = tick.value();
    let alici_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Alici))
        .map(|(id, _)| *id)
        .collect();
    for pid in alici_ids {
        // Player offset: her Alıcı kendi tick'inde tüketir.
        let offset = (pid.value() % u64::from(CONSUME_PERIOD)) as u32;
        if !(t + offset).is_multiple_of(CONSUME_PERIOD) {
            continue;
        }
        let Some(player) = state.players.get_mut(&pid) else {
            continue;
        };
        let entries: Vec<(CityId, ProductKind, u32)> = player.inventory.entries().collect();
        for (city, product, qty) in entries {
            let pct = product.alici_consume_pct();
            if pct == 0 {
                continue;
            }
            let consumed = (qty.saturating_mul(pct)) / 100;
            if consumed > 0 {
                let _ = player.inventory.remove(city, product, consumed);
            }
        }
    }
}

/// Maintenance — her fab için Sanayici'den maintenance ücreti çekilir,
/// sistem dışı atılır (amortisman). Anno 1800 inspiration. Aktif/atıl fark
/// etmez → boş fab kuran cezalı, akıllı kurulum.
fn charge_factory_maintenance(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let factories_by_owner: std::collections::BTreeMap<PlayerId, u32> = {
        let mut map = std::collections::BTreeMap::new();
        for f in state.factories.values() {
            *map.entry(f.owner).or_insert(0) += 1;
        }
        map
    };
    let mut pool_cents: i64 = 0;
    for (owner, count) in factories_by_owner {
        let cost_cents = MAINTENANCE_PER_FACTORY_LIRA
            .saturating_mul(i64::from(count))
            .saturating_mul(100);
        if let Some(p) = state.players.get_mut(&owner) {
            let actual = cost_cents.min(p.cash.as_cents());
            if actual > 0 {
                let _ = p.debit(Money::from_cents(actual));
                pool_cents = pool_cents.saturating_add(actual);
            }
        }
    }

    // Bakım da bir ödemedir: gerçekte tamir ekibine ve yedek parçaya gider.
    // "Amortisman" diye yok etmek ekonomiden sezonda ~250K₺ sızdırıyordu.
    distribute_capex_to_households(state, report, tick, Money::from_cents(pool_cents));
}

/// Çiftçi NPC'lere periyodik mahsul üretir. **Şehir-tabanlı 2-katmanlı**
/// (v8): her Çiftçi bir şehre atanır (`PlayerId` mod 3), o şehrin
/// `city_specialty` (prime) hamından **full qty** ve `city_secondary` (az)
/// hamından **qty/4** hasat yapar. Eski v6.5 `PlayerId` mod product yaklaşımı:
/// 3 ham × 1 şehir = 3 bucket besliyordu; v8: 3 prime + 3 secondary = 6 bucket.
/// `city_demand` slotu kasıtlı boş — Tüccar arbitrage ithalat hedefi.
/// `city_specialty` populate edilmemişse `CityId::cheap_raw()` fallback'i.
fn harvest_ciftci_stock(
    state: &mut GameState,
    rng: &mut ChaCha8Rng,
    report: &mut TickReport,
    tick: Tick,
) {
    let t = tick.value();
    let mut seed_pool_cents: i64 = 0;
    let ciftci_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Ciftci))
        .map(|(id, _)| *id)
        .collect();

    for pid in ciftci_ids {
        // v8.25: Player offset — her Çiftçi kendi tick'inde mahsul alır.
        let offset = (pid.value() % u64::from(HARVEST_PERIOD)) as u32;
        if !(t + offset).is_multiple_of(HARVEST_PERIOD) {
            continue;
        }
        // Çiftçi şehir ataması — PlayerId mod 3 ile 3 şehir.
        let city_idx = (pid.value() as usize) % CityId::ALL.len();
        let city = CityId::ALL[city_idx];

        // 3-katmanlı hasat (v8.6 D adımı):
        //   prime    : full qty             — şehrin uzmanlık hamı
        //   secondary: qty/4 (~%25)          — az üretim
        //   demand   : qty/8 (~%12.5)        — minimum, "az ihraç" sinyali
        // Önceki sürümde demand katmanı YOKTU → 3 demand-bucket arz tarafı kuru,
        // Tüccar arbitrage AI'sı doyuramıyordu (4 ölü bucket/oyun). v8.6:
        // Çiftçi 9/9 ham bucket besler, profile sistemi tam çalışır.
        let prime = state
            .city_specialty
            .get(&city)
            .copied()
            .unwrap_or_else(|| city.cheap_raw());
        // Dinamik hasat: piyasa fiyatı baseline'ın 2×'inden yüksekse (kıtlık)
        // daha fazla üret, 0.6×'inden düşükse (bolluk) daha az üret.
        // Ekonomiyi kırmadan doğal denge sağlar — Çiftçi de fiyata bakar.
        let price_factor_pct = {
            let ref_price = state.reference_price(city, prime)
                .map_or(0, moneywar_domain::Money::as_cents);
            let baseline = state.price_baseline.get(&(city, prime))
                .map_or(1, |m| m.as_cents()).max(1);
            let ratio = if baseline > 0 { ref_price * 100 / baseline } else { 100 };
            if ref_price == 0 {
                100
            } else if ratio > 300 {
                // Kıtlık tavanı. Ölçümde bu dal zamanın **%60'ında**
                // seçiliyordu (bkz. `harvest_probe`): tepki doymuş, kıtlık
                // sinyali var ama arz artamıyor. Tavan gerçek kısıt.
                moneywar_domain::balance::HARVEST_SCARCITY_CAP_PCT
            } else if ratio > 150 {
                200 // 1.5× baseline → 2× üretim
            } else if ratio > 110 {
                130 // hafif yüksek → %30 fazla
            } else if ratio < 50 {
                50  // yarı fiyat → yarı üretim
            } else if ratio < 80 {
                75  // düşük → az üretim
            } else {
                100
            }
        };
        // Hasat da reel üretim ölçeğine tabi — fabrika batch'i büyürken
        // ham madde arzı sabit kalırsa zincir daha da sıkışır.
        let base_qty = moneywar_domain::balance::scaled_output(
            rng.random_range(HARVEST_QTY_MIN..=HARVEST_QTY_MAX),
        );
        let prime_qty = (base_qty * price_factor_pct / 100).max(1);

        let secondary = state.city_secondary.get(&city).copied();
        let secondary_qty = secondary.map(|_| {
            let base = moneywar_domain::balance::scaled_output(
                rng.random_range(HARVEST_QTY_MIN..=HARVEST_QTY_MAX),
            );
            base / 4
        });

        let demand = state.city_demand.get(&city).copied();
        let demand_qty = demand.map(|_| {
            let base = moneywar_domain::balance::scaled_output(
                rng.random_range(HARVEST_QTY_MIN..=HARVEST_QTY_MAX),
            );
            base / 4
        });

        // Tek pas — prime + sec + demand için ortak cash debit (tohum maliyeti
        // toplam birim üzerinden orantılı).
        let total_qty = prime_qty + secondary_qty.unwrap_or(0) + demand_qty.unwrap_or(0);
        if total_qty == 0 {
            continue;
        }

        if let Some(p) = state.players.get_mut(&pid) {
            // Vic3 ilhamı: tohum/işçilik maliyeti. Para yetmiyorsa mahsul
            // orantılı azalır (kısmi hasat). Sıfır cash ise mahsul yok →
            // Çiftçi satmadan geri dönemez (closed loop).
            // Tohum maliyeti ölçeğe bölünür. Hasat miktarı üretim ölçeğiyle
            // büyüyor; birim maliyet sabit kalırsa gider de ölçekle çarpılıp
            // dolaşımdaki parayı yiyor. Ölçümde ×1000'de para arzı -28.7%'ye
            // düşüyordu ve sızıntının bu ayağı depolamadan büyüktü.
            //
            // Maliyet Çiftçi'nin cebinden çıkıp **kimseye gitmiyor** (sink);
            // o yüzden mutlak değil, üretimin oranı olarak tanımlanmalı.
            let scale = i64::from(moneywar_domain::balance::PRODUCTION_SCALE_PCT).max(1);
            let want_cost_cents = i64::from(total_qty)
                .saturating_mul(SEED_COST_PER_RAW_LIRA)
                .saturating_mul(100)
                .saturating_mul(100)
                / scale;
            let have_cents = p.cash.as_cents();
            let actual_cost_cents = want_cost_cents.min(have_cents);
            let scale_num = actual_cost_cents.max(0);
            let actual_total = if want_cost_cents > 0 {
                u32::try_from(i64::from(total_qty).saturating_mul(scale_num) / want_cost_cents)
                    .unwrap_or(total_qty)
            } else {
                total_qty
            };
            if actual_cost_cents > 0 {
                let _ = p.debit(Money::from_cents(actual_cost_cents));
                seed_pool_cents = seed_pool_cents.saturating_add(actual_cost_cents);
            }
            if actual_total == 0 {
                continue;
            }
            // 3 katmana orantılı dağıt (tam sayı bölmesi). prime + sec
            // hesaplanır, demand kalan = total - prime - sec.
            let actual_prime = u32::try_from(
                u64::from(prime_qty).saturating_mul(u64::from(actual_total)) / u64::from(total_qty),
            )
            .unwrap_or(prime_qty);
            let actual_secondary = u32::try_from(
                u64::from(secondary_qty.unwrap_or(0)).saturating_mul(u64::from(actual_total))
                    / u64::from(total_qty),
            )
            .unwrap_or(0);
            let actual_demand = actual_total
                .saturating_sub(actual_prime)
                .saturating_sub(actual_secondary);

            if actual_prime > 0 {
                let _ = p.inventory.add(city, prime, actual_prime);
                report.push(LogEntry::economy_harvest(
                    tick,
                    pid,
                    city,
                    prime,
                    actual_prime,
                ));
            }
            if let Some(sec_raw) = secondary
                && actual_secondary > 0 {
                    let _ = p.inventory.add(city, sec_raw, actual_secondary);
                    report.push(LogEntry::economy_harvest(
                        tick,
                        pid,
                        city,
                        sec_raw,
                        actual_secondary,
                    ));
                }
            if let Some(dem_raw) = demand
                && actual_demand > 0 {
                    let _ = p.inventory.add(city, dem_raw, actual_demand);
                    report.push(LogEntry::economy_harvest(
                        tick,
                        pid,
                        city,
                        dem_raw,
                        actual_demand,
                    ));
                }
        }
    }
    // Tohum parası da bir ödemedir: tohumcuya, ırgada, sulamaya gider.
    // Yok etmek ekonomiden **hasatla orantılı** para sızdırıyordu; depo
    // gideriyle birlikte uzun sezonu bozan iki sızıntıdan biriydi.
    distribute_capex_to_households(state, report, tick, Money::from_cents(seed_pool_cents));
}

/// World Fabrikaları — her (şehir, mamul) için baseline mamul üretim ve
/// SELL emir injection (v8.11). Sanayici NPC fab dağılımı yetersiz kaldığında
/// (her oyunda 1-3 mamul bucket fab'sız) "1500+ BUY 0 SELL" ölü pazarı
/// önler. World player (`PlayerId(0)`) yoksa (sim) no-op.
///
/// **Mekanik:**
/// 1. Her periyotta her (city, `finished_product`) için World envanterine
///    `WORLD_FAB_QTY_PER_PERIOD` birim ekle
/// 2. Aynı miktarda SELL emrini direkt `order_book`'a inject et (`process_submit`
///    by-pass, World relist cooldown'a tâbi değil)
/// 3. Fiyat: `effective_baseline × 0.95` (markdown — hızlı eşleşme, baseline
///    fair-value referansı)
/// 4. TTL: kısa (3 tick), her periyotta yenilenir
fn tick_world_factories(state: &mut GameState, _report: &mut TickReport, tick: Tick) {
    let world_id = PlayerId::new(WORLD_PLAYER_ID_VALUE);
    if !state.players.contains_key(&world_id) {
        return; // sim'de World yok — engine fallback
    }

    for city in CityId::ALL {
        for product in ProductKind::FINISHED_GOODS {
            let qty = WORLD_FAB_QTY_PER_PERIOD;
            // Stok ekle (World envanter sınırsız büyür, settle düşürür)
            if let Some(p) = state.players.get_mut(&world_id) {
                let _ = p.inventory.add(city, product, qty);
            }

            // Fiyat: effective_baseline × 0.95 — baseline fair-value, %5
            // markdown ile fast match. Baseline yoksa hardcoded fallback.
            let baseline = state.effective_baseline(city, product).unwrap_or_else(|| {
                Money::from_lira(npc_base_price_finished_lira()).unwrap_or(Money::ZERO)
            });
            let unit_price =
                Money::from_cents(baseline.as_cents().saturating_mul(95).saturating_div(100));
            if !unit_price.is_positive() {
                continue;
            }

            let order_id = OrderId::new(state.counters.next_order_id);
            state.counters.next_order_id = state.counters.next_order_id.saturating_add(1);

            let Ok(order) = MarketOrder::new_with_ttl(
                order_id,
                world_id,
                city,
                product,
                OrderSide::Sell,
                qty,
                unit_price,
                tick,
                WORLD_FAB_SELL_TTL,
            ) else {
                continue;
            };

            state
                .order_book
                .entry((city, product))
                .or_default()
                .push(order);
        }
    }
}

/// Üretici NPC'lerin (Çiftçi, Toptancı, Sanayici) cash'inden `TAX_PCT` %
/// vergi al, Alıcı'lara eşit dağıt. Şu an aktif değil — wages closed loop
/// olduğu için. Gelecekte `tick_economy`'den çağrılabilir.
#[allow(dead_code)]
fn collect_and_redistribute_tax(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let producer_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| {
            matches!(
                p.npc_kind,
                Some(NpcKind::Ciftci | NpcKind::Esnaf | NpcKind::Sanayici)
            )
        })
        .map(|(id, _)| *id)
        .collect();

    let mut total_tax_cents: i64 = 0;
    for pid in &producer_ids {
        let cash_cents = state.players.get(pid).map_or(0, |p| p.cash.as_cents());
        let tax_cents = cash_cents * TAX_PCT / 100;
        if tax_cents <= 0 {
            continue;
        }
        let tax = Money::from_cents(tax_cents);
        if let Some(p) = state.players.get_mut(pid)
            && p.cash >= tax {
                let _ = p.debit(tax);
                total_tax_cents += tax_cents;
            }
    }

    if total_tax_cents == 0 {
        return;
    }

    let alici_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Alici))
        .map(|(id, _)| *id)
        .collect();

    if alici_ids.is_empty() {
        return;
    }

    let per_alici_cents = total_tax_cents / alici_ids.len() as i64;
    if per_alici_cents <= 0 {
        return;
    }
    let per_alici = Money::from_cents(per_alici_cents);

    for pid in &alici_ids {
        if let Some(p) = state.players.get_mut(pid) {
            let _ = p.credit(per_alici);
        }
    }

    report.push(LogEntry::economy_tax_redistributed(
        tick,
        Money::from_cents(total_tax_cents),
        alici_ids.len() as u32,
    ));
}
