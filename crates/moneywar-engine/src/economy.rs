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
    CityId, GameState, Money, NpcKind, PlayerId, ProductKind, Tick, balance::SEED_COST_PER_RAW_LIRA,
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::report::{LogEntry, TickReport};

/// Wages (ücret) periyodu — her N tick'te aktif fab başına ücret kesilir.
const WAGE_PERIOD: u32 = 10;
/// Sanayici fabrikası başına ücret (lira) — her wage period'da Sanayici'den
/// çıkar, Alıcı'lara eşit dağıtılır. Closed-loop ekonomi.
/// Faz E tuning: 100 → 250. Behavior motorda Sanayici +27K, Alıcı -60K idi.
/// 2.5× pompa: 250 × 9 sefer × ~3 fab = 6.75K transfer/Sanayici, 8 Alıcı'ya
/// ~22.5K toplam akış → Sanayici dengelenir, Alıcı kaybı azalır.
/// Faz F2 tuning: 250 → 400. Esnaf perakendeci (yeni tedarik zinciri)
/// Alıcı'dan daha fazla para çekiyor (+21K Esnaf kârı).
/// Faz F4: 400 → 500. Alıcı hâlâ -92K eşiğe yakın, wages 25% ek pompa
/// ile -75K civarına iner.
const WAGE_PER_FACTORY_LIRA: i64 = 500;

/// Fab maintenance (işletme gideri) periyodu.
const MAINTENANCE_PERIOD: u32 = 10;
/// Fabrika başına maintenance ücreti — Sanayici'den çekilir, sistem dışına
/// atılır (amortisman). Aktif/atıl fark etmez → boş fab kuran cezalı.
/// Anno 1800 inspiration: building maintenance.
/// 250 → 100: wages ile birleşip Sanayici'yi batırıyordu.
const MAINTENANCE_PER_FACTORY_LIRA: i64 = 100;

/// Alıcı tüketim periyodu — her N tick'te Alıcı mamul stoğunun bir kısmını
/// tüketir (envanterden silinir). Vic3 pop needs inspiration.
/// Mevcut sorun: Alıcı mamul alır ama tüketmez → varlık birikir → `PnL` pozitif.
/// Tüketici negatif `PnL` için consume mekaniği şart.
const CONSUME_PERIOD: u32 = 5;
/// Alıcı'nın stoğundan her cycle ne yüzdesi tüketilsin.
/// Faz E tuning: 50 → 25. Behavior'da Alıcı çok agresif alım, %50/5tick
/// tüketim cash'i hızla erittirdi. %25 ile mamul daha uzun kalır, alım
/// baskısı azalır → Alıcı PnL kaybı yumuşar.
const CONSUME_PCT: u32 = 25;

/// Mahsul refill periyodu — her N tick'te Çiftçi'lere stok inject.
const HARVEST_PERIOD: u32 = 8;
/// Mahsul miktarı (birim) — her Çiftçi'ye specialty ürünü.
/// Faz F3: 150-300 → 300-600 (2× artış). Sanayici fab-bazlı her şehirde
/// raw arıyor; Çiftçi arzı 9× az kalıyordu → off-specialty bucket'lar ölüydü.
/// Arz arttırılırsa Tüccar daha çok dağıtım yapar, ölü bucket'lar canlanır.
const HARVEST_QTY_MIN: u32 = 300;
const HARVEST_QTY_MAX: u32 = 600;

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
    if t > 0 && t % WAGE_PERIOD == 0 {
        pay_factory_wages(state, report, tick);
    }

    // Maintenance — fab işletme gideri (Anno mekaniği): Sanayici'den çek, sistem dışı
    if t > 0 && t % MAINTENANCE_PERIOD == 0 {
        charge_factory_maintenance(state, report, tick);
    }

    // Consume — Alıcı NPC'leri mamul stoğunun %50'sini tüketir (Vic3 pop needs)
    if t > 0 && t % CONSUME_PERIOD == 0 {
        consume_alici_inventory(state);
    }

    // Mahsul — Çiftçi'lere ham madde inject
    if t > 0 && t % HARVEST_PERIOD == 0 {
        harvest_ciftci_stock(state, rng, report, tick);
    }

    // Vergi KALDIRILDI — wages ile çakışıyordu (çift gelir transferi).
    // Closed loop artık sadece wages üzerinden: Sanayici → Alıcı.
    // Vergi gerekirse çağrılır (constants kalıyor, fonksiyon ölü kod değil).
    let _ = TAX_PERIOD;
}

/// Wages — Sanayici fabrikalarından ücret keser, Alıcı'lara eşit dağıtır.
/// Closed loop: Alıcı'nın geliri Sanayici cebinden gelir (eski sabit maaş yerine).
/// Vic3 inspiration: building wages → pop income.
fn pay_factory_wages(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    // Her Sanayici NPC'si kaç fab'a sahip → o kadar wage öder.
    let factories_by_owner: std::collections::BTreeMap<PlayerId, u32> = {
        let mut map = std::collections::BTreeMap::new();
        for f in state.factories.values() {
            *map.entry(f.owner).or_insert(0) += 1;
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

    // Toplam wage havuzu — her Sanayici cebinden kesilir.
    let mut wage_pool_cents: i64 = 0;
    for (owner, count) in &factories_by_owner {
        let wage_per_owner_cents =
            WAGE_PER_FACTORY_LIRA.saturating_mul(i64::from(*count)).saturating_mul(100);
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

/// Alıcı NPC'lerinin mamul stoğunu tüket (envanterden sil). Tüketici davranış
/// modeli: Alıcı aldığı mamulü "kullanır", varlık olarak biriktirmez.
/// Vic3 pop needs inspiration. Each cycle: stoğun `CONSUME_PCT`%'i silinir.
fn consume_alici_inventory(state: &mut GameState) {
    let alici_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Alici))
        .map(|(id, _)| *id)
        .collect();
    for pid in alici_ids {
        let Some(player) = state.players.get_mut(&pid) else {
            continue;
        };
        let entries: Vec<(CityId, ProductKind, u32)> = player.inventory.entries().collect();
        for (city, product, qty) in entries {
            if !product.is_finished() {
                continue;
            }
            let consumed = (qty.saturating_mul(CONSUME_PCT)) / 100;
            if consumed > 0 {
                let _ = player.inventory.remove(city, product, consumed);
            }
        }
    }
}

/// Maintenance — her fab için Sanayici'den maintenance ücreti çekilir,
/// sistem dışı atılır (amortisman). Anno 1800 inspiration. Aktif/atıl fark
/// etmez → boş fab kuran cezalı, akıllı kurulum.
fn charge_factory_maintenance(state: &mut GameState, _report: &mut TickReport, _tick: Tick) {
    let factories_by_owner: std::collections::BTreeMap<PlayerId, u32> = {
        let mut map = std::collections::BTreeMap::new();
        for f in state.factories.values() {
            *map.entry(f.owner).or_insert(0) += 1;
        }
        map
    };
    for (owner, count) in factories_by_owner {
        let cost_cents =
            MAINTENANCE_PER_FACTORY_LIRA.saturating_mul(i64::from(count)).saturating_mul(100);
        if let Some(p) = state.players.get_mut(&owner) {
            let actual = cost_cents.min(p.cash.as_cents());
            if actual > 0 {
                let _ = p.debit(Money::from_cents(actual));
            }
        }
    }
}

/// Çiftçi NPC'lere uzmanlık ürünlerini ekle. Her Çiftçi'nin "specialty"
/// ürünü `personality` veya başlangıç specialty'sinden alınır — basit
/// versiyon: Çiftçi'nin envanterinde en çok hangi raw varsa o.
fn harvest_ciftci_stock(
    state: &mut GameState,
    rng: &mut ChaCha8Rng,
    report: &mut TickReport,
    tick: Tick,
) {
    let ciftci_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Ciftci))
        .map(|(id, _)| *id)
        .collect();

    for pid in ciftci_ids {
        // Çiftçi için specialty ürün — PlayerId mod 3 ile dağılım.
        let products = [
            ProductKind::Pamuk,
            ProductKind::Bugday,
            ProductKind::Zeytin,
        ];
        let product_idx = (pid.value() as usize) % products.len();
        let product = products[product_idx];

        // BUG FIX (Tuning v6.5): Önceki kod state.city_specialty üzerinden
        // şehir buluyordu ama bu BTreeMap hiç populate edilmiyor (state.rs
        // boş init). Sonuç: tüm Çiftçi mahsulü Istanbul'a yığılıyor (fallback).
        // Çözüm: CityId.cheap_raw() API'sını kullan — Pamuk→Istanbul, Bugday→Ankara,
        // Zeytin→Izmir doğal eşleşmesi.
        let city = CityId::ALL
            .iter()
            .copied()
            .find(|c| c.cheap_raw() == product)
            .unwrap_or(CityId::Istanbul);

        let qty = rng.random_range(HARVEST_QTY_MIN..=HARVEST_QTY_MAX);
        if let Some(p) = state.players.get_mut(&pid) {
            // Vic3 ilhamı: tohum/işçilik maliyeti. Para yetmiyorsa mahsul
            // orantılı azalır (kısmi hasat). Sıfır cash ise mahsul yok →
            // Çiftçi geri dönmek için satması gerek (closed loop'a girer).
            let want_cost_cents = i64::from(qty)
                .saturating_mul(SEED_COST_PER_RAW_LIRA)
                .saturating_mul(100);
            let have_cents = p.cash.as_cents();
            let actual_cost_cents = want_cost_cents.min(have_cents);
            let actual_qty = if want_cost_cents > 0 {
                u32::try_from(
                    i64::from(qty).saturating_mul(actual_cost_cents) / want_cost_cents,
                )
                .unwrap_or(qty)
            } else {
                qty
            };
            if actual_cost_cents > 0 {
                let _ = p.debit(Money::from_cents(actual_cost_cents));
            }
            if actual_qty > 0 {
                let _ = p.inventory.add(city, product, actual_qty);
                report.push(LogEntry::economy_harvest(tick, pid, city, product, actual_qty));
            }
        }
    }
}

/// Üretici NPC'lerin (Çiftçi, Toptancı, Sanayici) cash'inden `TAX_PCT` %
/// vergi al, Alıcı'lara eşit dağıt. Şu an aktif değil — wages closed loop
/// olduğu için. Gelecekte `tick_economy`'den çağrılabilir.
#[allow(dead_code)]
fn collect_and_redistribute_tax(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
) {
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
        if let Some(p) = state.players.get_mut(pid) {
            if p.cash >= tax {
                let _ = p.debit(tax);
                total_tax_cents += tax_cents;
            }
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
