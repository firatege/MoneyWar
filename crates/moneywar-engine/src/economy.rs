//! Ekonomi motoru — periyodik para akışı (maaş, vergi, mahsul, faiz).
//!
//! v4 tek-görev tasarımı için pazarın **sürekli akışı**:
//! - **Maaş:** Alıcı NPC'lere her 10 tick cash injection (tüketici geliri)
//! - **Mahsul:** Çiftçi NPC'lere her 8 tick ham madde refill (tedarik)
//! - **Vergi:** Çiftçi/Toptancı/Sanayici cash'inden %5 her 10 tick → Alıcı'lara dağıtım
//! - **Faiz:** Banka NPC'lere kredi geliri (TODO: kredi sistemi entegre edilince)
//!
//! Bu mekanikler **closed loop** sağlar: para sezon boyu sistem içinde döner.

use moneywar_domain::{
    CityId, GameState, Money, NpcKind, PlayerId, ProductKind, Tick,
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::report::{LogEntry, TickReport};

/// Maaş periyodu — her N tick'te bir Alıcı'lara cash inject.
const SALARY_PERIOD: u32 = 10;
/// Maaş miktarı (lira) — Alıcı NPC başına.
/// 5000 → 3000 → 2000: Alıcı PnL +20K hâlâ büyük kazanç (hedef -5K to +5K).
/// İkinci tur azaltma: -%33. Alıcı tüketici rolüne uygun negatif PnL.
const SALARY_PER_ALICI_LIRA: i64 = 2_000;

/// Mahsul refill periyodu — her N tick'te Çiftçi'lere stok inject.
const HARVEST_PERIOD: u32 = 8;
/// Mahsul miktarı (birim) — her Çiftçi'ye specialty ürünü.
const HARVEST_QTY_MIN: u32 = 150;
const HARVEST_QTY_MAX: u32 = 300;

/// Vergi periyodu.
const TAX_PERIOD: u32 = 10;
/// Vergi yüzdesi (Çiftçi/Toptancı/Sanayici cash'inden).
/// 5 → 2: Sanayici cash'in %37'si vergi olarak Alıcı'ya akıyordu (9× %5
/// cumulative). %2 ile cumulative ~%17 — daha sürdürülebilir.
const TAX_PCT: i64 = 2;

/// `advance_tick` içinde çağrılır — periyodik ekonomi akışlarını uygular.
pub(crate) fn tick_economy(
    state: &mut GameState,
    rng: &mut ChaCha8Rng,
    report: &mut TickReport,
    tick: Tick,
) {
    let t = tick.value();

    // Maaş — Alıcı'lara cash inject
    if t > 0 && t % SALARY_PERIOD == 0 {
        pay_alici_salaries(state, report, tick);
    }

    // Mahsul — Çiftçi'lere ham madde inject
    if t > 0 && t % HARVEST_PERIOD == 0 {
        harvest_ciftci_stock(state, rng, report, tick);
    }

    // Vergi — Çiftçi/Toptancı/Sanayici'den çek, Alıcı'lara dağıt (closed loop)
    if t > 0 && t % TAX_PERIOD == 0 {
        collect_and_redistribute_tax(state, report, tick);
    }
}

/// Alıcı NPC'lerin her birine SALARY_PER_ALICI_LIRA cash inject.
fn pay_alici_salaries(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let alici_ids: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Alici))
        .map(|(id, _)| *id)
        .collect();

    let salary = match Money::from_lira(SALARY_PER_ALICI_LIRA) {
        Ok(m) => m,
        Err(_) => return,
    };

    for pid in alici_ids {
        if let Some(p) = state.players.get_mut(&pid) {
            let _ = p.credit(salary);
            report.push(LogEntry::economy_salary(tick, pid, salary));
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
            let _ = p.inventory.add(city, product, qty);
            report.push(LogEntry::economy_harvest(tick, pid, city, product, qty));
        }
    }
}

/// Üretici NPC'lerin (Çiftçi, Toptancı, Sanayici) cash'inden TAX_PCT %
/// vergi al, Alıcı'lara eşit dağıt. Closed loop garantisi.
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
                Some(NpcKind::Ciftci) | Some(NpcKind::Esnaf) | Some(NpcKind::Sanayici)
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
