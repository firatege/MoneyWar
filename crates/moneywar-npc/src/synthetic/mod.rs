//! Synthetic NPC dispatch — sade, deterministic, AI'sız.
//!
//! Amaç: ekonomi mekaniğini AI varyansından bağımsız test etmek. Her NPC rolü
//! sabit kuralla çalışır, RNG yok, kişilik yok, strateji yok. Davranış değişmez
//! → ekonomi parametre etkisi izole edilebilir.
//!
//! # Davranış tablosu
//!
//! - **Çiftçi**: stoğun yarısını SAT @ baseline (sell-only)
//! - **Esnaf**: ham AL @ base × 0.95, stok SAT @ base × 1.05
//! - **Sanayici**: ilk tick fab kur, ham AL × 0.95, mamul SAT × 1.05
//! - **Tüccar**: spread > %20 ise arbitraj (ucuz şehir AL + pahalı şehir SAT)
//! - **Alıcı**: her şehir mamul AL @ baseline (cash/9 ile sınırlı)
//! - **Spekülatör**: her bucket BID × 0.95 + ASK × 1.05 (sabit %5 spread)
//! - **Banka**: özel akış (engine `tick_banks` tarafından), synthetic dispatch yok
//!
//! # Determinism
//!
//! Hiçbir RNG çağrısı yok. Aynı (state) → aynı komut seti garantili.
//! `BTreeMap` iterasyon sırası `players`/`factories`/`inventory` üzerinden.

use moneywar_domain::{
    CityId, Command, MarketOrder, Money, NpcKind, OrderId, OrderSide, Player, PlayerId,
    ProductKind, Tick,
    balance::{NPC_DEFAULT_ORDER_TTL, TRANSACTION_TAX_PCT},
};

use crate::npc_order_id;

/// Tax-aware cash budget — alıcı emri verirken `qty × price × (100+tax)/100`
/// cash karşılamalı, yoksa settle reject olur. Bu helper güvenli budget döner.
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

/// Synthetic motorun girişi — `decide_all_npcs(Difficulty::Synthetic)` buradan
/// her NPC için dispatch eder. Ham komutu döner; engine tarafı ekstra kontrol
/// uygular (cooldown, validation).
#[must_use]
pub fn decide_synthetic(
    state: &moneywar_domain::GameState,
    pid: PlayerId,
    tick: Tick,
) -> Vec<Command> {
    let Some(player) = state.players.get(&pid) else {
        return Vec::new();
    };
    if !player.is_npc {
        return Vec::new();
    }
    match player.npc_kind {
        Some(NpcKind::Ciftci) => ciftci(player, state, tick),
        Some(NpcKind::Esnaf) => esnaf(player, state, tick),
        Some(NpcKind::Sanayici) => sanayici(player, state, tick),
        Some(NpcKind::Tuccar) => tuccar(player, state, tick),
        Some(NpcKind::Alici) => alici(player, state, tick),
        Some(NpcKind::Spekulator) => spekulator(player, state, tick),
        Some(NpcKind::Banka) | None => Vec::new(),
    }
}

fn baseline(state: &moneywar_domain::GameState, city: CityId, product: ProductKind) -> Money {
    state.effective_baseline(city, product).unwrap_or_else(|| {
        let lira = if product.is_finished() { 18 } else { 6 };
        Money::from_lira(lira).expect("base price literal valid")
    })
}

/// Fiyatı `pct%`'sine ölçekle (95 → %95, 105 → %105).
fn scale_pct(price: Money, pct: i64) -> Money {
    Money::from_cents(price.as_cents().saturating_mul(pct) / 100)
}

fn submit(
    pid: PlayerId,
    tick: Tick,
    seq: u32,
    side: OrderSide,
    city: CityId,
    product: ProductKind,
    qty: u32,
    unit_price: Money,
) -> Option<Command> {
    if qty == 0 || unit_price.as_cents() <= 0 {
        return None;
    }
    let order = MarketOrder::new_with_ttl(
        OrderId::new(npc_order_id(pid, tick, seq)),
        pid,
        city,
        product,
        side,
        qty,
        unit_price,
        tick,
        NPC_DEFAULT_ORDER_TTL,
    )
    .ok()?;
    Some(Command::SubmitOrder(order))
}

// ============================================================================
// 7 rol davranışı
// ============================================================================

fn ciftci(player: &Player, state: &moneywar_domain::GameState, tick: Tick) -> Vec<Command> {
    // Stoğun yarısını SAT @ baseline.
    let mut cmds = Vec::new();
    let mut seq: u32 = 0;
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_raw() || qty == 0 {
            continue;
        }
        let half = (qty / 2).max(1).min(100);
        let price = baseline(state, city, product);
        if let Some(c) = submit(player.id, tick, seq, OrderSide::Sell, city, product, half, price)
        {
            cmds.push(c);
            seq = seq.saturating_add(1);
        }
    }
    cmds
}

fn alici(player: &Player, state: &moneywar_domain::GameState, tick: Tick) -> Vec<Command> {
    // Her şehir × her mamul için baseline'da AL. Cash 9 bucket'a böl.
    let mut cmds = Vec::new();
    let mut seq: u32 = 0;
    let bucket_cash_cents = player.cash.as_cents() / 9;
    let bucket_cash = Money::from_cents(bucket_cash_cents.max(0));
    for city in CityId::ALL {
        for product in ProductKind::FINISHED_GOODS {
            let price = baseline(state, city, product);
            let qty = affordable_qty(bucket_cash, price, 30);
            if let Some(c) = submit(player.id, tick, seq, OrderSide::Buy, city, product, qty, price)
            {
                cmds.push(c);
                seq = seq.saturating_add(1);
            }
        }
    }
    cmds
}

fn esnaf(player: &Player, state: &moneywar_domain::GameState, tick: Tick) -> Vec<Command> {
    // Ham AL @ base × 0.95, stok SAT @ base × 1.05.
    let mut cmds = Vec::new();
    let mut seq: u32 = 0;
    let bucket_cash_cents = player.cash.as_cents() / 9;
    let bucket_cash = Money::from_cents(bucket_cash_cents.max(0));
    for city in CityId::ALL {
        for product in ProductKind::RAW_MATERIALS {
            let price = scale_pct(baseline(state, city, product), 95);
            let qty = affordable_qty(bucket_cash, price, 30);
            if let Some(c) = submit(player.id, tick, seq, OrderSide::Buy, city, product, qty, price)
            {
                cmds.push(c);
                seq = seq.saturating_add(1);
            }
        }
    }
    for (city, product, qty) in player.inventory.entries() {
        if qty == 0 {
            continue;
        }
        let half = (qty / 2).max(1).min(50);
        let price = scale_pct(baseline(state, city, product), 105);
        if let Some(c) = submit(player.id, tick, seq, OrderSide::Sell, city, product, half, price)
        {
            cmds.push(c);
            seq = seq.saturating_add(1);
        }
    }
    cmds
}

fn sanayici(
    player: &Player,
    state: &moneywar_domain::GameState,
    tick: Tick,
) -> Vec<Command> {
    let mut cmds = Vec::new();
    let mut seq: u32 = 0;

    // İlk tick fab yoksa İstanbul-Kumaş kur.
    let owned = state.factories.values().filter(|f| f.owner == player.id).count();
    if owned == 0 && tick.value() <= 2 {
        cmds.push(Command::BuildFactory {
            owner: player.id,
            city: CityId::Istanbul,
            product: ProductKind::Kumas,
        });
    }

    // Ham AL @ base × 0.95.
    let bucket_cash_cents = player.cash.as_cents() / 9;
    let bucket_cash = Money::from_cents(bucket_cash_cents.max(0));
    for city in CityId::ALL {
        for product in ProductKind::RAW_MATERIALS {
            let price = scale_pct(baseline(state, city, product), 95);
            let qty = affordable_qty(bucket_cash, price, 30);
            if let Some(c) = submit(player.id, tick, seq, OrderSide::Buy, city, product, qty, price)
            {
                cmds.push(c);
                seq = seq.saturating_add(1);
            }
        }
    }

    // Mamul SAT @ base × 1.05.
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_finished() || qty == 0 {
            continue;
        }
        let half = (qty / 2).max(1).min(50);
        let price = scale_pct(baseline(state, city, product), 105);
        if let Some(c) = submit(player.id, tick, seq, OrderSide::Sell, city, product, half, price)
        {
            cmds.push(c);
            seq = seq.saturating_add(1);
        }
    }
    cmds
}

fn tuccar(
    player: &Player,
    state: &moneywar_domain::GameState,
    tick: Tick,
) -> Vec<Command> {
    // Arbitraj: her ürün için ucuz vs pahalı şehir karşılaştır. Spread > %20
    // ise ucuzda AL + pahalıda SAT (stoğu varsa).
    let mut cmds = Vec::new();
    let mut seq: u32 = 0;
    let bucket_cash_cents = player.cash.as_cents() / 6;
    let bucket_cash = Money::from_cents(bucket_cash_cents.max(0));

    for product in ProductKind::ALL {
        let mut min_pair: Option<(CityId, Money)> = None;
        let mut max_pair: Option<(CityId, Money)> = None;
        for city in CityId::ALL {
            let price = baseline(state, city, product);
            if min_pair.is_none_or(|(_, p)| price < p) {
                min_pair = Some((city, price));
            }
            if max_pair.is_none_or(|(_, p)| price > p) {
                max_pair = Some((city, price));
            }
        }
        let (Some((cheap_city, cheap_price)), Some((rich_city, rich_price))) =
            (min_pair, max_pair)
        else {
            continue;
        };
        if cheap_city == rich_city || cheap_price.as_cents() <= 0 {
            continue;
        }
        let spread_pct =
            (rich_price.as_cents() - cheap_price.as_cents()) * 100 / cheap_price.as_cents();
        if spread_pct < 20 {
            continue;
        }
        // AL ucuzda
        let buy_qty = affordable_qty(bucket_cash, cheap_price, 25);
        if let Some(c) = submit(
            player.id,
            tick,
            seq,
            OrderSide::Buy,
            cheap_city,
            product,
            buy_qty,
            cheap_price,
        ) {
            cmds.push(c);
            seq = seq.saturating_add(1);
        }
        // SAT pahalıda (stoğu varsa)
        let stock = player.inventory.get(rich_city, product);
        if stock > 0 {
            let sell_qty = stock.min(25);
            if let Some(c) = submit(
                player.id,
                tick,
                seq,
                OrderSide::Sell,
                rich_city,
                product,
                sell_qty,
                rich_price,
            ) {
                cmds.push(c);
                seq = seq.saturating_add(1);
            }
        }
    }
    cmds
}

fn spekulator(
    player: &Player,
    state: &moneywar_domain::GameState,
    tick: Tick,
) -> Vec<Command> {
    // Her bucket için BID × 0.95 ve (stokta varsa) ASK × 1.05.
    let mut cmds = Vec::new();
    let mut seq: u32 = 0;
    let bucket_cash_cents = player.cash.as_cents() / 18;
    let bucket_cash = Money::from_cents(bucket_cash_cents.max(0));
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let bp = baseline(state, city, product);
            let bid = scale_pct(bp, 95);
            let ask = scale_pct(bp, 105);
            // BID
            let qty = affordable_qty(bucket_cash, bid, 15);
            if let Some(c) = submit(player.id, tick, seq, OrderSide::Buy, city, product, qty, bid)
            {
                cmds.push(c);
                seq = seq.saturating_add(1);
            }
            // ASK (stokta varsa)
            let stock = player.inventory.get(city, product);
            if stock > 0 {
                let sell_qty = stock.min(15);
                if let Some(c) = submit(
                    player.id,
                    tick,
                    seq,
                    OrderSide::Sell,
                    city,
                    product,
                    sell_qty,
                    ask,
                ) {
                    cmds.push(c);
                    seq = seq.saturating_add(1);
                }
            }
        }
    }
    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{GameState, RoomConfig, RoomId};

    fn fresh_state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn add(state: &mut GameState, id: u64, kind: NpcKind, cash: i64) -> PlayerId {
        let pid = PlayerId::new(id);
        let p = Player::new(
            pid,
            format!("synth-{id}"),
            moneywar_domain::Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(kind);
        state.players.insert(pid, p);
        pid
    }

    #[test]
    fn human_player_returns_no_commands() {
        let mut s = fresh_state();
        let pid = PlayerId::new(1);
        let p = Player::new(
            pid,
            "human",
            moneywar_domain::Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            false,
        )
        .unwrap();
        s.players.insert(pid, p);
        let cmds = decide_synthetic(&s, pid, Tick::new(1));
        assert!(cmds.is_empty());
    }

    #[test]
    fn alici_emits_buy_orders_for_finished_goods() {
        let mut s = fresh_state();
        let pid = add(&mut s, 100, NpcKind::Alici, 100_000);
        let cmds = decide_synthetic(&s, pid, Tick::new(1));
        // 3 şehir × 3 mamul = 9 AL emri beklenir.
        assert!(!cmds.is_empty());
        for cmd in &cmds {
            let Command::SubmitOrder(o) = cmd else {
                panic!("Alıcı sadece SubmitOrder emit etmeli");
            };
            assert_eq!(o.side, OrderSide::Buy);
            assert!(o.product.is_finished(), "Alıcı sadece mamul AL");
        }
    }

    #[test]
    fn synthetic_order_uses_npc_default_ttl() {
        let mut s = fresh_state();
        let pid = add(&mut s, 100, NpcKind::Ciftci, 5_000);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 200)
            .unwrap();
        let cmds = decide_synthetic(&s, pid, Tick::new(1));
        let Command::SubmitOrder(o) = &cmds[0] else {
            panic!()
        };
        assert_eq!(
            o.ttl_ticks,
            moneywar_domain::balance::NPC_DEFAULT_ORDER_TTL,
            "synthetic emirleri NPC_DEFAULT_ORDER_TTL ile yazılmalı"
        );
    }

    #[test]
    fn ciftci_with_stock_sells_half() {
        let mut s = fresh_state();
        let pid = add(&mut s, 100, NpcKind::Ciftci, 5_000);
        s.players.get_mut(&pid).unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 200)
            .unwrap();
        let cmds = decide_synthetic(&s, pid, Tick::new(1));
        assert_eq!(cmds.len(), 1);
        let Command::SubmitOrder(o) = &cmds[0] else {
            panic!()
        };
        assert_eq!(o.side, OrderSide::Sell);
        assert_eq!(o.quantity, 100);
    }

    #[test]
    fn deterministic_same_state_same_commands() {
        let mut s = fresh_state();
        let pid = add(&mut s, 100, NpcKind::Spekulator, 50_000);
        s.players.get_mut(&pid).unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100)
            .unwrap();
        let a = decide_synthetic(&s, pid, Tick::new(1));
        let b = decide_synthetic(&s, pid, Tick::new(1));
        assert_eq!(a, b);
    }

    #[test]
    fn sanayici_first_tick_builds_factory() {
        let mut s = fresh_state();
        let pid = add(&mut s, 100, NpcKind::Sanayici, 50_000);
        let cmds = decide_synthetic(&s, pid, Tick::new(1));
        let has_build = cmds.iter().any(|c| matches!(c, Command::BuildFactory { .. }));
        assert!(has_build, "ilk tick'te fab kurmalı");
    }

    #[test]
    fn banka_emits_no_commands() {
        let mut s = fresh_state();
        let pid = add(&mut s, 100, NpcKind::Banka, 200_000);
        let cmds = decide_synthetic(&s, pid, Tick::new(1));
        assert!(cmds.is_empty(), "Banka synthetic'te dispatch'sız");
    }
}
