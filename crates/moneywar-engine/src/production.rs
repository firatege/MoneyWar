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
    CityId, DomainError, Factory, FactoryBatch, FactoryId, GameState, PlayerId, ProductKind, Role,
    Tick,
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

    // 2) Yeni batch başlatma — ham madde mevcut mu?
    let raw = product
        .raw_input()
        .expect("finished product always has raw_input");
    let Some(player) = state.players.get_mut(&owner) else {
        return;
    };
    let have_raw = player.inventory.get(city, raw);
    // Shortage soft penalty (Vic3 inspiration): tam batch yoksa, yarı batch
    // üret. Eski "100 yoksa idle" katı kuralı kâr akışını koparıyordu.
    // Min threshold = BATCH_SIZE/2. Altında idle.
    let partial_min = Factory::BATCH_SIZE / 2;
    let batch_size = if have_raw >= Factory::BATCH_SIZE {
        Factory::BATCH_SIZE
    } else if have_raw >= partial_min {
        have_raw
    } else {
        report.push(LogEntry::factory_idle(
            tick,
            owner,
            fid,
            city,
            format!(
                "raw {raw} shortage at {city}: have={have_raw}, need={partial_min}"
            ),
        ));
        return;
    };
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

    let completion = tick.checked_add(Factory::PRODUCTION_TICKS).unwrap_or(tick);
    let Some(factory) = state.factories.get_mut(&fid) else {
        return;
    };
    factory.batches.push(FactoryBatch {
        started_tick: tick,
        completion_tick: completion,
        units: batch_size,
    });
    report.push(LogEntry::production_started(
        tick,
        owner,
        fid,
        city,
        product,
        batch_size,
        completion,
    ));
}

#[cfg(test)]
mod tests {
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
    fn second_factory_costs_4k_and_debits_cash() {
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
        // İkinci 4k (FACTORY_BUILD_COSTS_LIRA[1]=4000).
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
            Money::from_lira(46_000).unwrap()
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
    fn production_starts_when_raw_available_and_completes_after_two_ticks() {
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

        // Tick 1: üretim başlar (batch=100, completion_tick=3).
        advance_production(&mut s, &mut r, Tick::new(1));
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            900
        );
        assert_eq!(s.factories.values().next().unwrap().batches.len(), 1);

        // Tick 2: hiçbir batch tamamlanmaz (completion=3 ve 4), yeni batch başlar.
        let mut r2 = TickReport::new(Tick::new(2));
        advance_production(&mut s, &mut r2, Tick::new(2));
        assert_eq!(s.factories.values().next().unwrap().batches.len(), 2);
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Kumas),
            0
        );

        // Tick 3: ilk batch tamamlanır (tick 1 + 2 = 3), yeni batch başlar.
        let mut r3 = TickReport::new(Tick::new(3));
        advance_production(&mut s, &mut r3, Tick::new(3));
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Kumas),
            100
        );
        // Pamuk 700 kaldı (3 batch × 100).
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            700
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
