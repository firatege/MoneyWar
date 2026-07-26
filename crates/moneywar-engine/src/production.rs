//! Fabrika üretimi — `BuildFactory` komutu + tick başı üretim döngüsü.
//!
//! # Akış
//!
//! 1. **Komut:** `BuildFactory { owner, city, product }` → Sanayici tekeli,
//!    maliyet (§10 tablosu), debit cash, fabrika yarat.
//! 2. **Tick başı üretim pass'i:** `advance_production(state, report, tick)`:
//!    - Önce **tamamlanmış batch'ler** → bitmiş ürün sahip envanterine eklenir.
//!    - Sonra **yeni batch** → ham madde envanterde varsa tüketilip başlatılır.
//!    - Ham madde yetmezse fabrika atıl, `FactoryIdle` event.
//!
//! **Sıra önemli:** complete → start. Aynı tick'te tamamlanan bitmiş ürün,
//! aynı tick'te yeni üretim için ham madde olarak kullanılmaz (bitmiş farklı
//! üründür zaten, ama semantik olarak net).

use moneywar_domain::{
    CityId, DomainError, Factory, FactoryBatch, FactoryId, GameState, Money, PlayerId,
    PrivateFarm, PrivateFarmId, ProductKind, Role, Tick,
    balance::{FACTORY_MAX_LEVEL, PRIVATE_FARM_BUILD_COST_LIRA, PRIVATE_FARM_MAX_PER_OWNER},
};

use crate::{
    error::EngineError,
    report::{LogEntry, TickReport},
};

/// `BuildFactory` komutunu uygula. Sanayici tekeli, maliyet §10 tablosundan.
pub(crate) fn process_build_factory(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    owner: PlayerId,
    city: CityId,
    product: ProductKind,
) -> Result<(), EngineError> {
    // Owner var mı?
    let player = state.players.get(&owner).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!("player {owner} not found")))
    })?;

    // Sanayici tekeli.
    if !matches!(player.role, Role::Sanayici) {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "factory requires Sanayici role, {owner} is {}",
            player.role
        ))));
    }

    // Mevcut fabrika sayısı → maliyet.
    let existing_count = u32::try_from(
        state
            .factories
            .values()
            .filter(|f| f.owner == owner)
            .count(),
    )
    .unwrap_or(u32::MAX);
    let cost = Factory::build_cost(existing_count);

    // Cash kontrolü + debit.
    let player_mut = state.players.get_mut(&owner).expect("validated above");
    if player_mut.cash < cost {
        return Err(EngineError::Domain(DomainError::InsufficientFunds {
            have: player_mut.cash,
            want: cost,
        }));
    }
    player_mut.debit(cost)?;

    // Factory::new ürün bitmiş mi doğrular.
    let factory_id = FactoryId::new(state.counters.next_factory_id);
    state.counters.next_factory_id = state.counters.next_factory_id.saturating_add(1);
    let factory = Factory::new(factory_id, owner, city, product)?;
    state.factories.insert(factory_id, factory);

    report.push(LogEntry::factory_built(
        tick, owner, factory_id, city, product, cost,
    ));
    Ok(())
}

/// Geri ödeme yüzdesi: kaçıncı fabrika kapatılıyor.
/// İlk fabrikalar daha pahalı kuruldu ama daha az geri alınır (hızlı kapatma caydırıcı).
const DEMOLISH_REFUND_PCT: i64 = 50;

/// Özel çiftlikleri ilerlet — her tick sahibinin envanterine ham madde ekle.
/// Üretim miktarı seviyeye göre değişir: lv1=20, lv2=35, lv3=55
pub(crate) fn advance_private_farms(state: &mut GameState, _tick: Tick) {
    let farm_ids: Vec<PrivateFarmId> = state.private_farms.keys().copied().collect();
    for fid in farm_ids {
        let (owner, city, product, output) = {
            let f = &state.private_farms[&fid];
            (f.owner, f.city, f.product, f.output_per_tick())
        };
        if let Some(player) = state.players.get_mut(&owner) {
            let _ = player.inventory.add(city, product, output);
        }
    }
}

/// `UpgradeFarm` komutunu uygula — çiftlik seviye atlar, üretim artar.
pub(crate) fn process_upgrade_farm(
    state: &mut GameState,
    _report: &mut TickReport,
    _tick: Tick,
    owner: PlayerId,
    farm_id: PrivateFarmId,
) -> Result<(), EngineError> {
    let farm = state.private_farms.get(&farm_id).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!("farm {farm_id} not found")))
    })?;
    if farm.owner != owner {
        return Err(EngineError::Domain(DomainError::Validation("not owner".into())));
    }
    let current_level = farm.level;
    let cost = moneywar_domain::PrivateFarm::upgrade_cost(current_level).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation("farm at max level".into()))
    })?;
    let player = state.players.get_mut(&owner).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation("player not found".into()))
    })?;
    if player.cash < cost {
        return Err(EngineError::Domain(DomainError::InsufficientFunds { have: player.cash, want: cost }));
    }
    player.debit(cost)?;
    state.private_farms.get_mut(&farm_id).unwrap().level += 1;
    Ok(())
}

/// `BuildPrivateFarm` komutunu uygula.
pub(crate) fn process_build_private_farm(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    owner: PlayerId,
    city: CityId,
    product: ProductKind,
) -> Result<(), EngineError> {
    let player = state.players.get(&owner).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!("player {owner} not found")))
    })?;
    if !matches!(player.role, Role::Sanayici) {
        return Err(EngineError::Domain(DomainError::Validation(
            "BuildPrivateFarm requires Sanayici role".into(),
        )));
    }
    if !product.is_raw() {
        return Err(EngineError::Domain(DomainError::Validation(
            "PrivateFarm only produces raw materials".into(),
        )));
    }
    let owned = state.private_farms.values().filter(|f| f.owner == owner).count();
    if owned >= PRIVATE_FARM_MAX_PER_OWNER {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "max private farms ({PRIVATE_FARM_MAX_PER_OWNER}) reached"
        ))));
    }
    // Aynı (city, product) slot'unda başka tarla varsa 1.5× maliyet.
    // Her ek tarla için +%50 daha pahalı → tarla yeri kıymetli.
    let existing_in_slot = state.private_farms.values()
        .filter(|f| f.city == city && f.product == product)
        .count() as i64;
    let cost_lira = PRIVATE_FARM_BUILD_COST_LIRA + existing_in_slot * PRIVATE_FARM_BUILD_COST_LIRA / 2;
    let cost = Money::from_lira(cost_lira).map_err(|e| EngineError::Domain(e))?;
    let player_mut = state.players.get_mut(&owner).expect("validated");
    if player_mut.cash < cost {
        return Err(EngineError::Domain(DomainError::InsufficientFunds {
            have: player_mut.cash,
            want: cost,
        }));
    }
    player_mut.debit(cost)?;

    let fid = PrivateFarmId::new(state.counters.next_private_farm_id);
    state.counters.next_private_farm_id = state.counters.next_private_farm_id.saturating_add(1);
    state.private_farms.insert(fid, PrivateFarm::new(fid, owner, city, product));

    report.push(LogEntry::private_farm_built(tick, owner, fid, city, product, cost));
    Ok(())
}


/// `SetFactoryStaff` — fabrikanın kadrosunu hedef değere getirir.
///
/// Emek dünyada kıttır: toplam istihdam [`LABOR_POOL_SIZE`]'ı aşamaz. Artış
/// talebi havuzda kalan kadarıyla **kısmen** karşılanır (tamamen reddedilmez)
/// — gerçek işgücü piyasasında da "3 kişi arıyorum" ilanına 1 kişi başvurur.
/// Azaltma her zaman tam uygulanır ve işçileri havuza geri verir.
///
/// [`LABOR_POOL_SIZE`]: moneywar_domain::balance::LABOR_POOL_SIZE
pub(crate) fn process_set_factory_staff(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    owner: PlayerId,
    factory_id: FactoryId,
    target: u32,
) -> Result<(), EngineError> {
    use moneywar_domain::balance::LABOR_POOL_SIZE;

    let factory = state.factories.get(&factory_id).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!(
            "factory {factory_id} not found"
        )))
    })?;
    if factory.owner != owner {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "factory {factory_id} is not owned by {owner}"
        ))));
    }
    let current = factory.employees;
    if target == current {
        return Ok(());
    }

    let employed: u32 = state.factories.values().map(|f| f.employees).sum();
    let free = LABOR_POOL_SIZE.saturating_sub(employed);

    let applied = if target > current {
        current.saturating_add((target - current).min(free))
    } else {
        target
    };
    if applied == current {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "no free labor: pool {LABOR_POOL_SIZE} fully employed"
        ))));
    }

    let factory = state.factories.get_mut(&factory_id).expect("checked above");
    factory.employees = applied;

    let employed_after: u32 = state.factories.values().map(|f| f.employees).sum();
    report.push(LogEntry {
        tick,
        actor: Some(owner),
        event: crate::LogEvent::FactoryStaffChanged {
            factory_id,
            owner,
            employees: applied,
            hired: i64::from(applied) - i64::from(current),
            pool_left: LABOR_POOL_SIZE.saturating_sub(employed_after),
        },
    });
    Ok(())
}

/// `UpgradeFactory` komutunu uygula — level artar, batch büyür.
pub(crate) fn process_upgrade_factory(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    owner: PlayerId,
    factory_id: FactoryId,
) -> Result<(), EngineError> {
    let player = state.players.get(&owner).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!("player {owner} not found")))
    })?;
    if !matches!(player.role, Role::Sanayici) {
        return Err(EngineError::Domain(DomainError::Validation(
            "UpgradeFactory requires Sanayici role".into(),
        )));
    }
    let factory = state.factories.get(&factory_id).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!("factory {factory_id} not found")))
    })?;
    if factory.owner != owner {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "factory {factory_id} not owned by {owner}"
        ))));
    }
    let current_level = factory.level;
    if current_level >= FACTORY_MAX_LEVEL {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "factory {factory_id} already at max level {FACTORY_MAX_LEVEL}"
        ))));
    }
    let cost = Factory::upgrade_cost(current_level).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation("no upgrade cost available".into()))
    })?;

    let player_mut = state.players.get_mut(&owner).expect("validated above");
    if player_mut.cash < cost {
        return Err(EngineError::Domain(DomainError::InsufficientFunds {
            have: player_mut.cash,
            want: cost,
        }));
    }
    player_mut.debit(cost)?;

    let factory_mut = state.factories.get_mut(&factory_id).expect("validated above");
    factory_mut.level += 1;
    let new_level = factory_mut.level;
    let city = factory_mut.city;
    let product = factory_mut.product;

    report.push(LogEntry::factory_upgraded(tick, owner, factory_id, city, product, new_level, cost));
    Ok(())
}

/// `DemolishFactory` komutunu uygula. Sanayici kendi fabrikasını kapatır,
/// kuruş maliyetinin %50'sini nakit olarak geri alır.
pub(crate) fn process_demolish_factory(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    owner: PlayerId,
    factory_id: FactoryId,
) -> Result<(), EngineError> {
    // Owner var mı ve Sanayici mi?
    let player = state.players.get(&owner).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!("player {owner} not found")))
    })?;
    if !matches!(player.role, Role::Sanayici) {
        return Err(EngineError::Domain(DomainError::Validation(
            "DemolishFactory requires Sanayici role".into(),
        )));
    }

    // Fabrika bu kişiye ait mi?
    let factory = state
        .factories
        .get(&factory_id)
        .ok_or_else(|| {
            EngineError::Domain(DomainError::Validation(format!(
                "factory {factory_id} not found"
            )))
        })?;
    if factory.owner != owner {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "factory {factory_id} is not owned by {owner}"
        ))));
    }
    let city = factory.city;
    let product = factory.product;

    // Geri ödeme: owner'ın kaçıncı fabrikası olduğunu sayıp o sayıya göre
    // build_cost'un %50'sini iade et.
    let owned_before = u32::try_from(
        state.factories.values().filter(|f| f.owner == owner).count(),
    )
    .unwrap_or(1)
    .saturating_sub(1); // bu fabrika hariç diğerleri
    let build_cost = Factory::build_cost(owned_before);
    let refund = Money::from_cents(build_cost.as_cents() * DEMOLISH_REFUND_PCT / 100);

    // Fabrikayı sil.
    state.factories.remove(&factory_id);

    // Nakit iade.
    let player_mut = state.players.get_mut(&owner).expect("validated above");
    player_mut.credit(refund)?;

    report.push(LogEntry::factory_demolished(
        tick, owner, factory_id, city, product, refund,
    ));
    Ok(())
}

/// Tüm fabrikalar için tick başı üretim döngüsü.
///
/// Fabrikalar `BTreeMap` sırasında işlenir (deterministik). Her fabrika için:
/// 1. Biten batch'ler → owner inventory.
/// 2. Yeni batch başlatma denemesi.
pub(crate) fn advance_production(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let factory_ids: Vec<FactoryId> = state.factories.keys().copied().collect();
    for fid in factory_ids {
        step_factory(state, report, tick, fid);
    }
}

fn step_factory(state: &mut GameState, report: &mut TickReport, tick: Tick, fid: FactoryId) {
    // 1) Biten batch'leri bul + envantere yatır.
    let (owner, city, product, completed_units) = {
        let Some(factory) = state.factories.get_mut(&fid) else {
            return;
        };
        let completed: u32 = factory
            .batches
            .iter()
            .filter(|b| b.completion_tick <= tick)
            .map(|b| b.units)
            .sum();
        factory.batches.retain(|b| b.completion_tick > tick);
        if completed > 0 {
            factory.last_production_tick = Some(tick);
        }
        (factory.owner, factory.city, factory.product, completed)
    };

    if completed_units > 0 {
        if let Some(player) = state.players.get_mut(&owner) {
            // Inventory overflow teorik; invariant olarak başarmalı.
            if let Err(e) = player.inventory.add(city, product, completed_units) {
                report.push(LogEntry::factory_idle(
                    tick,
                    owner,
                    fid,
                    city,
                    format!("inventory add failed: {e}"),
                ));
                // Not: ürün kayboldu. Gelecek iyileştirme: taşma toleransı.
            } else {
                report.push(LogEntry::production_completed(
                    tick,
                    owner,
                    fid,
                    city,
                    product,
                    completed_units,
                ));
            }
        }
    }

    // 2) Yeni batch başlatma — önceki batch bitmeden yenisi başlamaz.
    // Pipeline üretim kaldırıldı: fabrika seri çalışır, paralel değil.
    {
        let Some(factory) = state.factories.get(&fid) else { return; };
        if !factory.batches.is_empty() {
            return; // aktif batch var, bekle
        }
    }

    let raw = product
        .raw_input()
        .expect("finished product always has raw_input");
    let extras = product.extra_inputs();
    let Some(player) = state.players.get_mut(&owner) else {
        return;
    };
    let have_raw = player.inventory.get(city, raw);
    // Shortage soft penalty: tam batch yoksa kısmi üret.
    // Batch boyutu fabrika seviyesine göre değişir (level 1=50, 2=75, 3=100).
    let level_batch = state.factories.get(&fid).map_or(Factory::BATCH_SIZE, |f| f.batch_size());
    let partial_min = (level_batch / 4).max(1);
    let mut batch_size = if have_raw >= level_batch {
        level_batch
    } else if have_raw >= partial_min {
        have_raw
    } else {
        report.push(LogEntry::factory_idle(
            tick,
            owner,
            fid,
            city,
            format!("raw {raw} shortage at {city}: have={have_raw}, need={partial_min}"),
        ));
        return;
    };

    // Faz 2: ek girdiler. Batch, en kısıtlı girdiye göre daralır — üç
    // parçalı üründe tek bir parçanın yokluğu bandı durdurur. Girdi
    // pazarını tutan rakip fabrikayı böyle boğar (SupplyChoke).
    for (input, pct) in extras {
        let need = (batch_size.saturating_mul(*pct) / 100).max(1);
        let have = player.inventory.get(city, *input);
        if have >= need {
            continue;
        }
        // Elimizdeki kadarına batch'i küçült: have = b×pct/100 → b = have×100/pct.
        let feasible = have.saturating_mul(100) / (*pct).max(1);
        if feasible < partial_min {
            report.push(LogEntry::factory_idle(
                tick,
                owner,
                fid,
                city,
                format!("input {input} shortage at {city}: have={have}, need={need}"),
            ));
            return;
        }
        batch_size = batch_size.min(feasible);
    }

    if let Err(e) = player.inventory.remove(city, raw, batch_size) {
        report.push(LogEntry::factory_idle(
            tick,
            owner,
            fid,
            city,
            format!("raw removal failed: {e}"),
        ));
        return;
    }
    for (input, pct) in extras {
        let need = (batch_size.saturating_mul(*pct) / 100).max(1);
        if let Err(e) = player.inventory.remove(city, *input, need) {
            // Ana girdi zaten düşüldü; tutarlılık için geri koy ve çık.
            let _ = player.inventory.add(city, raw, batch_size);
            report.push(LogEntry::factory_idle(
                tick,
                owner,
                fid,
                city,
                format!("input {input} removal failed: {e}"),
            ));
            return;
        }
    }

    // v0.4.1: Per-product üretim süresi + verim oranı.
    // - Süre: product.production_ticks() (Un=2, Zeytinyağı=3, Kumaş=4)
    // - Verim: batch_size × output_ratio_pct() / 100
    let prod_ticks = product.production_ticks().max(1);
    let completion = tick.checked_add(prod_ticks).unwrap_or(tick);
    let output_units = batch_size
        .saturating_mul(product.output_ratio_pct())
        .saturating_div(100)
        .max(1);
    let Some(factory) = state.factories.get_mut(&fid) else {
        return;
    };
    factory.batches.push(FactoryBatch {
        started_tick: tick,
        completion_tick: completion,
        units: output_units, // verim sonrası mamul miktarı
    });
    report.push(LogEntry::production_started(
        tick,
        owner,
        fid,
        city,
        product,
        output_units,
        completion,
    ));
}

#[cfg(test)]
mod tests {
    /// İki fabrikalı, tek sahipli sade dünya — emek havuzu testleri için.
    fn staff_state() -> GameState {
        let mut s = GameState::new(
            moneywar_domain::RoomId::new(1),
            moneywar_domain::RoomConfig::hizli(),
        );
        let p = moneywar_domain::Player::new(
            PlayerId::new(1),
            "S".to_string(),
            moneywar_domain::Role::Sanayici,
            moneywar_domain::Money::from_lira(100_000).unwrap(),
            false,
        )
        .unwrap();
        s.players.insert(p.id, p);
        for i in 1..=2u64 {
            let f = moneywar_domain::Factory::new(
                moneywar_domain::FactoryId::new(i),
                PlayerId::new(1),
                moneywar_domain::CityId::Istanbul,
                moneywar_domain::ProductKind::Kumas,
            )
            .unwrap();
            s.factories.insert(f.id, f);
        }
        s
    }

    fn pid(n: u64) -> PlayerId { PlayerId::new(n) }
    fn fid(n: u64) -> moneywar_domain::FactoryId { moneywar_domain::FactoryId::new(n) }

    // ── Emek havuzu ──────────────────────────────────────────────────────────

    #[test]
    fn hiring_is_capped_by_the_world_labor_pool() {
        use moneywar_domain::balance::{EMPLOYEES_PER_FACTORY_L1, LABOR_POOL_SIZE};
        let mut s = staff_state();
        let mut r = TickReport::new(Tick::new(1));

        // Havuzu doldur: tek fabrikayı havuz kadar büyütmeye çalış.
        process_set_factory_staff(&mut s, &mut r, Tick::new(1), pid(1), fid(1), LABOR_POOL_SIZE + 100)
            .unwrap();
        let employed: u32 = s.factories.values().map(|f| f.employees).sum();
        assert_eq!(employed, LABOR_POOL_SIZE, "istihdam havuzu aşamaz");

        // İkinci fabrika artık işçi bulamaz.
        let res = process_set_factory_staff(
            &mut s,
            &mut r,
            Tick::new(1),
            pid(1),
            fid(2),
            EMPLOYEES_PER_FACTORY_L1 + 5,
        );
        assert!(res.is_err(), "havuz boşken işe alım başarısız olmalı");
    }

    #[test]
    fn firing_returns_workers_to_the_pool() {
        use moneywar_domain::balance::LABOR_POOL_SIZE;
        let mut s = staff_state();
        let mut r = TickReport::new(Tick::new(1));

        process_set_factory_staff(&mut s, &mut r, Tick::new(1), pid(1), fid(1), LABOR_POOL_SIZE)
            .unwrap();
        // Hepsini çıkar → havuz boşalır, ikinci fabrika kadro bulabilir.
        process_set_factory_staff(&mut s, &mut r, Tick::new(1), pid(1), fid(1), 0).unwrap();
        process_set_factory_staff(&mut s, &mut r, Tick::new(1), pid(1), fid(2), 5).unwrap();

        assert_eq!(s.factories[&fid(2)].employees, 5);
    }

    #[test]
    fn partial_hire_when_pool_is_nearly_empty() {
        use moneywar_domain::balance::{EMPLOYEES_PER_FACTORY_L1, LABOR_POOL_SIZE};
        let mut s = staff_state();
        let mut r = TickReport::new(Tick::new(1));

        // Havuzda tam 2 kişi bırak. İki fabrika da tam kadro açıldığı için
        // 2. fabrikanın mevcut kadrosu da hesaba katılmalı.
        let target_f1 = LABOR_POOL_SIZE - 2 - EMPLOYEES_PER_FACTORY_L1;
        process_set_factory_staff(&mut s, &mut r, Tick::new(1), pid(1), fid(1), target_f1)
            .unwrap();
        let before = s.factories[&fid(2)].employees;
        // 10 iste, 2 bulabilmeli.
        process_set_factory_staff(&mut s, &mut r, Tick::new(1), pid(1), fid(2), before + 10)
            .unwrap();
        assert_eq!(
            s.factories[&fid(2)].employees,
            before + 2,
            "eksik işgücü kısmen karşılanmalı"
        );
    }

    #[test]
    fn cannot_staff_a_factory_you_do_not_own() {
        let mut s = staff_state();
        let mut r = TickReport::new(Tick::new(1));
        let res = process_set_factory_staff(&mut s, &mut r, Tick::new(1), pid(2), fid(1), 1);
        assert!(res.is_err(), "başkasının fabrikasına kadro atanamaz");
    }

    use super::*;
    use moneywar_domain::{CityId, Money, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId};

    fn state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn add_player(state: &mut GameState, id: u64, role: Role, cash_lira: i64) -> PlayerId {
        let p = Player::new(
            PlayerId::new(id),
            format!("P{id}"),
            role,
            Money::from_lira(cash_lira).unwrap(),
            false,
        )
        .unwrap();
        let pid = p.id;
        state.players.insert(pid, p);
        pid
    }

    #[test]
    fn build_factory_creates_entity_and_charges_starter_zero() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, Role::Sanayici, 100);

        process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap();

        assert_eq!(s.factories.len(), 1);
        // İlk fabrika bedava → 100₺ dokunulmamış.
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            Money::from_lira(100).unwrap()
        );
    }

    #[test]
    fn build_factory_rejects_non_sanayici() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, Role::Tuccar, 100_000);
        let err = process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Sanayici"));
        assert!(s.factories.is_empty());
    }

    #[test]
    fn build_factory_rejects_raw_product() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, Role::Sanayici, 100);
        let err = process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Pamuk,
        )
        .unwrap_err();
        assert!(err.to_string().contains("finished"));
    }

    #[test]
    fn second_factory_costs_8k_and_debits_cash() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, Role::Sanayici, 50_000);
        // İlk fabrika bedava (FACTORY_BUILD_COSTS_LIRA[0]=0).
        process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap();
        // İkinci 8k (FACTORY_BUILD_COSTS_LIRA[1]=8000).
        process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            PlayerId::new(1),
            CityId::Ankara,
            ProductKind::Un,
        )
        .unwrap();
        assert_eq!(s.factories.len(), 2);
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            Money::from_lira(42_000).unwrap() // 50K - 0 - 8K
        );
    }

    #[test]
    fn build_factory_insufficient_funds_is_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        add_player(&mut s, 1, Role::Sanayici, 1_000); // < 15k for 2nd
        process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            PlayerId::new(1),
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap(); // 1st free, ok
        let err = process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            PlayerId::new(1),
            CityId::Ankara,
            ProductKind::Un,
        )
        .unwrap_err();
        assert!(err.to_string().contains("insufficient"));
        // Sadece 1 fabrika, cash değişmedi.
        assert_eq!(s.factories.len(), 1);
        assert_eq!(
            s.players[&PlayerId::new(1)].cash,
            Money::from_lira(1_000).unwrap()
        );
    }

    #[test]
    fn production_starts_when_raw_available_and_completes_after_kumas_ticks() {
        // v0.4.1: Kumaş 4 tick + %80 verim. BATCH_SIZE Pamuk → 80% Kumaş.
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Sanayici, 0);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 1000)
            .unwrap();
        let mut r = TickReport::new(Tick::new(1));
        process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap();

        // Tick 1: batch başlar (BATCH_SIZE Pamuk → 80% Kumaş, completion=5).
        advance_production(&mut s, &mut r, Tick::new(1));
        let batch = moneywar_domain::balance::FACTORY_BATCH_SIZE;
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            1000 - batch
        );
        assert_eq!(s.factories.values().next().unwrap().batches.len(), 1);

        // Tick 2-4: aktif batch bitmeden yeni başlamaz (seri üretim).
        for t in 2u32..=4 {
            let mut rt = TickReport::new(Tick::new(t));
            advance_production(&mut s, &mut rt, Tick::new(t));
        }
        // Sadece 1 batch aktif (pipeline kaldırıldı)
        assert_eq!(s.factories.values().next().unwrap().batches.len(), 1);
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Kumas),
            0
        );

        // Tick 5: ilk batch tamamlanır (t1'de başladı, 4 tick = t5).
        // Seri üretim: t1'de 1 batch başladı, t5'te biter + yeni batch başlar.
        // Pamuk: 1000 - 2×BATCH_SIZE (t1 + t5 başlangıcı).
        let mut r5 = TickReport::new(Tick::new(5));
        advance_production(&mut s, &mut r5, Tick::new(5));
        let expected_kumas = batch * 80 / 100;
        let expected_pamuk = 1000 - 2 * batch; // seri: sadece 2 batch tüketildi
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Kumas),
            expected_kumas
        );
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            expected_pamuk
        );
    }

    #[test]
    fn factory_idle_when_no_raw_material() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Sanayici, 0);
        let mut r = TickReport::new(Tick::new(1));
        process_build_factory(
            &mut s,
            &mut r,
            Tick::new(1),
            pid,
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap();
        let mut r2 = TickReport::new(Tick::new(2));
        advance_production(&mut s, &mut r2, Tick::new(2));

        let idle = r2.entries.iter().any(|e| {
            matches!(
                e.event,
                crate::report::LogEvent::FactoryIdle { ref reason, .. } if reason.contains("shortage")
            )
        });
        assert!(idle, "expected FactoryIdle event");
    }

    #[test]
    fn production_is_deterministic_across_ticks() {
        let mut a = state();
        let mut b = state();
        let pid = add_player(&mut a, 1, Role::Sanayici, 0);
        add_player(&mut b, 1, Role::Sanayici, 0);
        a.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100)
            .unwrap();
        b.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100)
            .unwrap();
        let mut ra = TickReport::new(Tick::new(1));
        let mut rb = TickReport::new(Tick::new(1));
        process_build_factory(
            &mut a,
            &mut ra,
            Tick::new(1),
            pid,
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap();
        process_build_factory(
            &mut b,
            &mut rb,
            Tick::new(1),
            pid,
            CityId::Istanbul,
            ProductKind::Kumas,
        )
        .unwrap();
        for t in 1..=5 {
            let mut ar = TickReport::new(Tick::new(t));
            let mut br = TickReport::new(Tick::new(t));
            advance_production(&mut a, &mut ar, Tick::new(t));
            advance_production(&mut b, &mut br, Tick::new(t));
            assert_eq!(ar.entries, br.entries);
        }
        assert_eq!(a, b);
    }
}
