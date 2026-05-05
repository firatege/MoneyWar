//! Tick başına state snapshot. Sezon boyunca toplanır, rapor üretiminde
//! delta/aggregation hesabı için kullanılır.
//!
//! Tasarım: değer tipleri (Copy/Clone), `serde`-friendly. Heavy state'i
//! kopyalamayız — sadece NPC/oyuncu davranışını görmek için yeterli kompakt
//! özet. Order book toplam Σ + best ask/bid, envanter map, cash, role.

use moneywar_domain::{
    CityId, GameState, OrderSide, PlayerId, ProductKind, Tick,
};
use serde::{Deserialize, Serialize};

/// Tek bir oyuncunun bu tick'teki kompakt durumu.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub id: u64,
    pub name: String,
    pub is_npc: bool,
    pub role: String,
    pub npc_kind: Option<String>,
    pub personality: Option<String>,
    pub cash_cents: i64,
    /// `(city, product, qty)` non-zero entries.
    pub inventory: Vec<((u8, u8), u32)>,
    /// Toplam birim stok (tüm `(city, product)` üzerinden).
    pub inventory_total: u64,
    /// Inventory'nin baseline fiyat × qty toplamı (cents). `PnL` hesabında stok
    /// varlığını hesaba katmak için.
    #[serde(default)]
    pub inventory_value_cents: i64,
    /// NPC'ye ait fabrika sayısı. `PnL`'de fab sermaye yatırımını hesaba katmak için.
    #[serde(default)]
    pub factory_count: u32,
}

/// `(city, product)` bucket için aktif emir özeti.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookSummary {
    pub city: u8,
    pub product: u8,
    pub bid_count: u32,
    pub bid_total_qty: u32,
    pub best_bid_cents: Option<i64>,
    pub ask_count: u32,
    pub ask_total_qty: u32,
    pub best_ask_cents: Option<i64>,
    /// Bu bucket'taki en eski emrin yaşı (tick).
    pub oldest_order_age: u32,
}

/// Bu tick'te clearing olan `(city, product)` özeti.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClearingSnapshot {
    pub city: u8,
    pub product: u8,
    pub clearing_price_cents: Option<i64>,
    pub matched_qty: u32,
    pub submitted_buy_qty: u32,
    pub submitted_sell_qty: u32,
}

/// Tek bir tick'in tam snapshot'ı.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickSnapshot {
    pub tick: u32,
    pub players: Vec<PlayerSnapshot>,
    pub order_book: Vec<OrderBookSummary>,
    pub clearings: Vec<ClearingSnapshot>,
    /// Kabul edilen + reddedilen toplam komut sayısı.
    pub commands_accepted: u32,
    pub commands_rejected: u32,
}

impl TickSnapshot {
    /// `GameState`'ten kompakt snapshot çıkar. `last_report` aynı tick'in
    /// engine raporu — clearing ve match metriklerini buradan alır.
    pub fn from_state(
        state: &GameState,
        last_report: &moneywar_engine::TickReport,
        current_tick: Tick,
    ) -> Self {
        let players = state
            .players
            .iter()
            .map(|(id, p)| {
                // Inventory varlık değeri: qty × effective_baseline (cents).
                // PnL hesabında stok değeri sayılsın diye.
                let inventory_value_cents: i64 = p
                    .inventory
                    .entries()
                    .map(|(c, prod, qty)| {
                        let price = state
                            .effective_baseline(c, prod)
                            .map_or(0, moneywar_domain::Money::as_cents);
                        price.saturating_mul(i64::from(qty))
                    })
                    .sum();
                let factory_count = u32::try_from(
                    state.factories.values().filter(|f| f.owner == *id).count(),
                )
                .unwrap_or(0);
                PlayerSnapshot {
                    id: id.value(),
                    name: p.name.clone(),
                    is_npc: p.is_npc,
                    role: format!("{}", p.role),
                    npc_kind: p.npc_kind.map(|k| format!("{k:?}")),
                    personality: p.personality.map(|p| format!("{p:?}")),
                    cash_cents: p.cash.as_cents(),
                    inventory: p
                        .inventory
                        .entries()
                        .map(|(c, prod, qty)| ((c as u8, prod as u8), qty))
                        .collect(),
                    inventory_total: p.inventory.total_units(),
                    inventory_value_cents,
                    factory_count,
                }
            })
            .collect();

        let order_book = state
            .order_book
            .iter()
            .map(|((city, product), orders)| {
                let mut bid_count = 0u32;
                let mut bid_total_qty = 0u32;
                let mut ask_count = 0u32;
                let mut ask_total_qty = 0u32;
                let mut best_bid_cents: Option<i64> = None;
                let mut best_ask_cents: Option<i64> = None;
                let mut oldest_age: u32 = 0;
                for o in orders {
                    let age = current_tick
                        .value()
                        .saturating_sub(o.submitted_tick.value());
                    if age > oldest_age {
                        oldest_age = age;
                    }
                    match o.side {
                        OrderSide::Buy => {
                            bid_count += 1;
                            bid_total_qty += o.quantity;
                            let p = o.unit_price.as_cents();
                            best_bid_cents = Some(best_bid_cents.map_or(p, |b| b.max(p)));
                        }
                        OrderSide::Sell => {
                            ask_count += 1;
                            ask_total_qty += o.quantity;
                            let p = o.unit_price.as_cents();
                            best_ask_cents = Some(best_ask_cents.map_or(p, |b| b.min(p)));
                        }
                    }
                }
                OrderBookSummary {
                    city: *city as u8,
                    product: *product as u8,
                    bid_count,
                    bid_total_qty,
                    best_bid_cents,
                    ask_count,
                    ask_total_qty,
                    best_ask_cents,
                    oldest_order_age: oldest_age,
                }
            })
            .collect();

        // Clearing: bu tick'teki MarketCleared event'lerini topla.
        let mut clearings: Vec<ClearingSnapshot> = Vec::new();
        for entry in &last_report.entries {
            if let moneywar_engine::LogEvent::MarketCleared {
                city,
                product,
                clearing_price,
                matched_qty,
                submitted_buy_qty,
                submitted_sell_qty,
                ..
            } = &entry.event
            {
                clearings.push(ClearingSnapshot {
                    city: *city as u8,
                    product: *product as u8,
                    clearing_price_cents: clearing_price.map(moneywar_domain::Money::as_cents),
                    matched_qty: *matched_qty,
                    submitted_buy_qty: *submitted_buy_qty,
                    submitted_sell_qty: *submitted_sell_qty,
                });
            }
        }

        let mut accepted = 0u32;
        let mut rejected = 0u32;
        for entry in &last_report.entries {
            match &entry.event {
                moneywar_engine::LogEvent::CommandAccepted { .. } => accepted += 1,
                moneywar_engine::LogEvent::CommandRejected { .. } => rejected += 1,
                _ => {}
            }
        }

        // Unused: PlayerId for compile.
        let _ = PlayerId::new(0);
        let _ = (CityId::Istanbul, ProductKind::Pamuk);

        Self {
            tick: current_tick.value(),
            players,
            order_book,
            clearings,
            commands_accepted: accepted,
            commands_rejected: rejected,
        }
    }
}
