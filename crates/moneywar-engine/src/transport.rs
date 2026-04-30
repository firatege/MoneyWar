//! Kervan sistemi — `BuyCaravan`, `DispatchCaravan` komutları + tick başı
//! varış döngüsü.
//!
//! # Prensipler (§4)
//!
//! - **Süre riski var, kayıp riski yok.** Kervan `distance_to` tick sonra
//!   kesinlikle hedefe ulaşır; mal kaybı, haydut, kaza yok.
//! - **Kapasite + maliyet role'e göre** (§10):
//!   - Sanayici kervanı: 20 kapasite, ilk bedava sonra 5k/10k.
//!   - Tüccar kervanı: 50 kapasite, ilk bedava sonra 6k/10k/15k.
//! - Dispatch anında cargo sahip envanter'den çıkar, varışta hedef
//!   envanter'e yatırılır. Arada "transit"te sayılır (domain `Cargo` içinde).

use moneywar_domain::{
    Caravan, CaravanId, CargoSpec, CityId, DomainError, GameState, PlayerId, Tick,
};

use crate::{
    error::EngineError,
    report::{LogEntry, TickReport},
};

/// `BuyCaravan` komutu. Role'e göre fiyat + kapasite tablosu.
pub(crate) fn process_buy_caravan(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    owner: PlayerId,
    starting_city: CityId,
) -> Result<(), EngineError> {
    let player = state.players.get(&owner).ok_or_else(|| {
        EngineError::Domain(DomainError::Validation(format!("player {owner} not found")))
    })?;
    let role = player.role;

    let existing_count =
        u32::try_from(state.caravans.values().filter(|c| c.owner == owner).count())
            .unwrap_or(u32::MAX);
    let cost = Caravan::buy_cost(role, existing_count);
    let capacity = Caravan::capacity_for(role);

    let player_mut = state.players.get_mut(&owner).expect("validated above");
    if player_mut.cash < cost {
        return Err(EngineError::Domain(DomainError::InsufficientFunds {
            have: player_mut.cash,
            want: cost,
        }));
    }
    player_mut.debit(cost)?;

    let caravan_id = CaravanId::new(state.counters.next_caravan_id);
    state.counters.next_caravan_id = state.counters.next_caravan_id.saturating_add(1);
    let caravan = Caravan::new(caravan_id, owner, capacity, starting_city);
    state.caravans.insert(caravan_id, caravan);

    report.push(LogEntry::caravan_bought(
        tick,
        owner,
        caravan_id,
        starting_city,
        capacity,
        cost,
    ));
    Ok(())
}

/// `DispatchCaravan` komutu.
///
/// Validation:
/// - Kervan var mı, `Idle` at `from`?
/// - Sahibin envanter'inde cargo miktarları yeterli mi?
/// - Cargo kapasite aşmıyor mu?
/// - `from != to`?
///
/// Başarılı: cargo owner.inventory'den çıkar, kervan `EnRoute`'a geçer,
/// `arrival_tick = tick + distance_to`.
pub(crate) fn process_dispatch_caravan(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    caravan_id: CaravanId,
    from: CityId,
    to: CityId,
    cargo: &CargoSpec,
) -> Result<(), EngineError> {
    // Kervan var mı?
    let owner = state
        .caravans
        .get(&caravan_id)
        .map(|c| c.owner)
        .ok_or_else(|| {
            EngineError::Domain(DomainError::Validation(format!(
                "caravan {caravan_id} not found"
            )))
        })?;

    // Aynı şehir rota yok.
    if from == to {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "caravan {caravan_id} cannot dispatch {from} → {from}"
        ))));
    }

    // Envanter yeterliliği — her (product, qty) için kontrol.
    {
        let Some(player) = state.players.get(&owner) else {
            return Err(EngineError::Domain(DomainError::Validation(format!(
                "owner {owner} of caravan {caravan_id} not found"
            ))));
        };
        for (product, qty) in cargo.entries() {
            let have = player.inventory.get(from, product);
            if have < qty {
                return Err(EngineError::Domain(DomainError::InsufficientStock {
                    city: from,
                    product,
                    have,
                    want: qty,
                }));
            }
        }
    }

    // Varış zamanı: mesafe `distance_to` (aynı şehir zaten elendi).
    let distance = from.distance_to(to);
    let arrival_tick = tick.checked_add(distance)?;

    // Kervan dispatch — kapasite + state geçiş domain tarafında validate edilir.
    let caravan = state.caravans.get_mut(&caravan_id).expect("checked above");
    caravan.dispatch(from, to, cargo.clone(), arrival_tick)?;

    // Envanter'den cargo'yu düş (dispatch başarılı).
    let cargo_total = cargo.total_units();
    let player = state
        .players
        .get_mut(&owner)
        .expect("owner validated above");
    for (product, qty) in cargo.entries() {
        player
            .inventory
            .remove(from, product, qty)
            .expect("pre-flight validated");
    }

    report.push(LogEntry::caravan_dispatched(
        tick,
        owner,
        caravan_id,
        from,
        to,
        arrival_tick,
        cargo_total,
    ));
    Ok(())
}

/// Varış zamanı gelmiş kervanları boşaltır. `BTreeMap` sırasında işlenir.
pub(crate) fn advance_caravans(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let caravan_ids: Vec<CaravanId> = state.caravans.keys().copied().collect();
    for cid in caravan_ids {
        step_caravan(state, report, tick, cid);
    }
}

fn step_caravan(state: &mut GameState, report: &mut TickReport, tick: Tick, cid: CaravanId) {
    let Some(caravan) = state.caravans.get_mut(&cid) else {
        return;
    };
    // Sadece EnRoute + arrival_tick geçmişse varır.
    let should_arrive = match &caravan.state {
        moneywar_domain::CaravanState::EnRoute { arrival_tick, .. } => *arrival_tick <= tick,
        moneywar_domain::CaravanState::Idle { .. } => false,
    };
    if !should_arrive {
        return;
    }

    let owner = caravan.owner;
    let Ok((dest, cargo)) = caravan.arrive() else {
        return;
    };
    let cargo_total = cargo.total_units();

    if let Some(player) = state.players.get_mut(&owner) {
        for (product, qty) in cargo.entries() {
            let _ = player.inventory.add(dest, product, qty);
        }
    }

    report.push(LogEntry::caravan_arrived(
        tick,
        owner,
        cid,
        dest,
        cargo_total,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CargoSpec, CityId, Money, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

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

    fn cargo(product: ProductKind, qty: u32) -> CargoSpec {
        let mut c = CargoSpec::new();
        c.add(product, qty).unwrap();
        c
    }

    #[test]
    fn buy_caravan_tuccar_starter_zero_cost_capacity_500() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 0);
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap();
        assert_eq!(s.caravans.len(), 1);
        let c = s.caravans.values().next().unwrap();
        assert_eq!(c.capacity, 500);
        assert_eq!(c.state.current_city(), Some(CityId::Istanbul));
    }

    #[test]
    fn buy_caravan_sanayici_starter_zero_cost_capacity_200() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Sanayici, 0);
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap();
        let c = s.caravans.values().next().unwrap();
        assert_eq!(c.capacity, 200);
    }

    #[test]
    fn buy_second_caravan_tuccar_costs_6000() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 50_000);
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap();
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Ankara).unwrap();
        assert_eq!(s.caravans.len(), 2);
        assert_eq!(
            s.players[&pid].cash,
            Money::from_lira(50_000 - 6_000).unwrap()
        );
    }

    #[test]
    fn buy_caravan_insufficient_funds_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 1_000);
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap(); // free
        let err =
            process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Ankara).unwrap_err();
        assert!(err.to_string().contains("insufficient"));
        assert_eq!(s.caravans.len(), 1);
    }

    #[test]
    fn dispatch_caravan_drains_inventory_and_schedules_arrival() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 0);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 30)
            .unwrap();
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap();
        let cid = *s.caravans.keys().next().unwrap();

        process_dispatch_caravan(
            &mut s,
            &mut r,
            Tick::new(1),
            cid,
            CityId::Istanbul,
            CityId::Ankara,
            &cargo(ProductKind::Pamuk, 25),
        )
        .unwrap();

        // Envanter 30 - 25 = 5.
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            5
        );
        // Kervan EnRoute.
        let c = s.caravans.get(&cid).unwrap();
        match &c.state {
            moneywar_domain::CaravanState::EnRoute { arrival_tick, .. } => {
                // v3: Istanbul → Ankara = 2 tick. Tick 1'de dispatch → arrival 3.
                assert_eq!(*arrival_tick, Tick::new(3));
            }
            moneywar_domain::CaravanState::Idle { .. } => panic!("expected EnRoute"),
        }
    }

    #[test]
    fn dispatch_over_capacity_is_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 0);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 1000)
            .unwrap();
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap();
        let cid = *s.caravans.keys().next().unwrap();

        let err = process_dispatch_caravan(
            &mut s,
            &mut r,
            Tick::new(1),
            cid,
            CityId::Istanbul,
            CityId::Ankara,
            &cargo(ProductKind::Pamuk, 501), // capacity 500
        )
        .unwrap_err();
        assert!(err.to_string().contains("capacity"));
        // Envanter dokunulmadı.
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            1000
        );
    }

    #[test]
    fn dispatch_insufficient_stock_is_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 0);
        // Envanter yok.
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap();
        let cid = *s.caravans.keys().next().unwrap();

        let err = process_dispatch_caravan(
            &mut s,
            &mut r,
            Tick::new(1),
            cid,
            CityId::Istanbul,
            CityId::Ankara,
            &cargo(ProductKind::Pamuk, 10),
        )
        .unwrap_err();
        assert!(err.to_string().contains("insufficient stock"));
    }

    #[test]
    fn dispatch_same_city_is_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 0);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 30)
            .unwrap();
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap();
        let cid = *s.caravans.keys().next().unwrap();

        let err = process_dispatch_caravan(
            &mut s,
            &mut r,
            Tick::new(1),
            cid,
            CityId::Istanbul,
            CityId::Istanbul,
            &cargo(ProductKind::Pamuk, 10),
        )
        .unwrap_err();
        assert!(err.to_string().contains("İstanbul"));
    }

    #[test]
    fn caravan_arrives_and_deposits_cargo_to_destination() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 0);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 30)
            .unwrap();
        process_buy_caravan(&mut s, &mut r, Tick::new(1), pid, CityId::Istanbul).unwrap();
        let cid = *s.caravans.keys().next().unwrap();

        // v3: Dispatch tick 1, Istanbul→Ankara 2 tick, arrival_tick = 1 + 2 = 3.
        process_dispatch_caravan(
            &mut s,
            &mut r,
            Tick::new(1),
            cid,
            CityId::Istanbul,
            CityId::Ankara,
            &cargo(ProductKind::Pamuk, 25),
        )
        .unwrap();

        // Tick 2: hâlâ EnRoute.
        advance_caravans(&mut s, &mut r, Tick::new(2));
        assert!(!s.caravans[&cid].is_idle());
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Ankara, ProductKind::Pamuk),
            0
        );

        // Tick 3: varış.
        let mut r4 = TickReport::new(Tick::new(3));
        advance_caravans(&mut s, &mut r4, Tick::new(3));
        assert!(s.caravans[&cid].is_idle());
        assert_eq!(s.caravans[&cid].state.current_city(), Some(CityId::Ankara));
        assert_eq!(
            s.players[&pid]
                .inventory
                .get(CityId::Ankara, ProductKind::Pamuk),
            25
        );

        let arrived_event = r4.entries.iter().any(|e| {
            matches!(
                e.event,
                crate::report::LogEvent::CaravanArrived {
                    city: CityId::Ankara,
                    cargo_total: 25,
                    ..
                }
            )
        });
        assert!(arrived_event);
    }

    #[test]
    fn dispatch_ownership_inventory_check_by_caravan_owner() {
        // Farklı oyuncunun caravan'ını dispatch etmek → envanter başkasının
        // değil, caravan'ın sahibininkinden bakılır.
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let p1 = add_player(&mut s, 1, Role::Tuccar, 0);
        let _p2 = add_player(&mut s, 2, Role::Tuccar, 0);
        s.players
            .get_mut(&p1)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 30)
            .unwrap();
        // P1 kervan alır.
        process_buy_caravan(&mut s, &mut r, Tick::new(1), p1, CityId::Istanbul).unwrap();
        let cid = *s.caravans.keys().next().unwrap();

        // Dispatch komutu (zaten kervanın sahibi = p1; envanter kontrolü p1'de).
        process_dispatch_caravan(
            &mut s,
            &mut r,
            Tick::new(1),
            cid,
            CityId::Istanbul,
            CityId::Ankara,
            &cargo(ProductKind::Pamuk, 10),
        )
        .unwrap();
        assert_eq!(
            s.players[&p1]
                .inventory
                .get(CityId::Istanbul, ProductKind::Pamuk),
            20
        );
    }
}
