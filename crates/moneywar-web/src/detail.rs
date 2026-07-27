//! Drill-down veri katmanı — şehir, firma, fabrika sayfaları.
//!
//! # Neden snapshot'ta değil
//!
//! Snapshot her tick WebSocket'ten yayınlanır ve şu an ~71 KB. Her oyuncunun
//! envanteri (~780 kayıt), her fabrikanın girdi durumu, her şehrin işlem
//! defteri eklenirse birkaç yüz KB'ye çıkar — saniyede bir, tüm izleyicilere.
//!
//! Bu sayfalar ise tıklanınca açılır. Talep üzerine hesaplamak hem yayını
//! ince tutar hem de sayfa başına çok daha derin veri vermeyi mümkün kılar.

use std::collections::BTreeMap;

use moneywar_domain::{CityId, FactoryId, GameState, Money, NpcKind, PlayerId, ProductKind};
use moneywar_engine::leaderboard;
use serde::Serialize;

use crate::dto::{city_slug, lira, product_slug};
use crate::ledger::Ledger;

/// Detay sayfalarında gösterilen işlem listesi uzunluğu.
const RECENT_LIMIT: usize = 25;

/// Sıralamalı listelerde gösterilen satır sayısı (top-N).
const TOP_N: usize = 8;

// ─────────────────────────────────────────────────────────────────────────────
// Ortak parçalar
// ─────────────────────────────────────────────────────────────────────────────

/// Bir oyuncunun kısa kimliği — listelerde isim göstermek için.
#[derive(Debug, Clone, Serialize)]
pub struct ActorRef {
    pub id: u64,
    pub name: String,
    pub role: Option<String>,
}

fn actor_ref(state: &GameState, id: PlayerId) -> ActorRef {
    let p = state.players.get(&id);
    ActorRef {
        id: id.value(),
        name: p.map_or_else(|| format!("#{}", id.value()), |p| p.name.clone()),
        role: p.and_then(|p| p.npc_kind).map(|k| k.label().to_string()),
    }
}

/// Tek bir gerçekleşmiş işlem satırı.
#[derive(Debug, Clone, Serialize)]
pub struct TradeRowDto {
    pub tick: u32,
    pub city: String,
    pub product: String,
    pub product_label: String,
    pub quantity: u32,
    pub price_lira: f64,
    pub value_lira: f64,
    pub buyer: ActorRef,
    pub seller: ActorRef,
}

/// İki taraf arasındaki iş hacmi — "kim kimle" grafiğinin kenarı.
#[derive(Debug, Clone, Serialize)]
pub struct PairFlowDto {
    pub buyer: ActorRef,
    pub seller: ActorRef,
    pub trades: u32,
    pub units: u64,
    pub value_lira: f64,
}

/// Ürün bazlı hacim — şehrin/firmanın neyle uğraştığı.
#[derive(Debug, Clone, Serialize)]
pub struct ProductVolumeDto {
    pub product: String,
    pub product_label: String,
    pub is_raw: bool,
    pub units: u64,
    pub value_lira: f64,
    pub avg_price_lira: f64,
}

/// Envanter satırı.
#[derive(Debug, Clone, Serialize)]
pub struct StockRowDto {
    pub city: String,
    pub product: String,
    pub product_label: String,
    pub units: u32,
    pub value_lira: f64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Şehir
// ─────────────────────────────────────────────────────────────────────────────

/// Şehir sayfası.
#[derive(Debug, Clone, Serialize)]
pub struct CityDetail {
    pub city: String,
    pub label: String,
    pub tick: u32,
    /// Defterin gördüğü en eski tick — istatistiklerin penceresi.
    pub window_from_tick: Option<u32>,

    pub factory_count: u32,
    pub idle_factory_count: u32,
    pub farm_count: u32,
    pub employees: u32,
    pub required_employees: u32,

    /// Bu şehirdeki fabrika sahipliği ne kadar yoğunlaşmış (0 = dağınık).
    pub factory_gini: f64,
    /// Bu şehirde tutulan stoğun sahipler arası dağılımı.
    pub stock_gini: f64,

    /// Şehirdeki oyuncular — fabrika, tarla, stok, işlem.
    pub actors: Vec<CityActorDto>,
    /// Şehirde üretilen ürünler ve fabrika sayıları (toplu).
    pub production: Vec<CityProductDto>,
    /// Şehirdeki fabrikalar tek tek — sahibi, kadrosu, durumu.
    /// Toplu görünüm "ne üretiliyor"u söyler; bu liste "kim üretiyor"u.
    pub factories: Vec<CityFactoryDto>,
    /// Şehirde tutulan toplam stok.
    pub stock: Vec<ProductStockDto>,
    /// Hacimce en büyük ürünler.
    pub volume: Vec<ProductVolumeDto>,
    /// En çok iş yapan çiftler.
    pub top_pairs: Vec<PairFlowDto>,
    /// Son işlemler.
    pub recent_trades: Vec<TradeRowDto>,
}

/// Şehirde varlığı olan bir oyuncu.
#[derive(Debug, Clone, Serialize)]
pub struct CityActorDto {
    pub actor: ActorRef,
    pub factories: u32,
    pub farms: u32,
    pub stock_units: u32,
    pub stock_value_lira: f64,
    pub bought_units: u64,
    pub sold_units: u64,
}

/// Şehirde üretilen bir ürün.
#[derive(Debug, Clone, Serialize)]
pub struct CityProductDto {
    pub product: String,
    pub product_label: String,
    pub factories: u32,
    pub idle: u32,
    pub produced_units: u64,
}

/// Şehirdeki tek bir fabrika — fabrika sayfasına geçiş noktası.
#[derive(Debug, Clone, Serialize)]
pub struct CityFactoryDto {
    pub id: u64,
    pub owner: ActorRef,
    pub product: String,
    pub product_label: String,
    pub level: u8,
    pub employees: u32,
    pub required_employees: u32,
    pub idle: bool,
    pub pending_units: u64,
    /// Defter penceresinde ürettiği toplam birim.
    pub produced_units: u64,
}

/// Şehirde tutulan bir ürünün toplamı.
#[derive(Debug, Clone, Serialize)]
pub struct ProductStockDto {
    pub product: String,
    pub product_label: String,
    pub is_raw: bool,
    pub units: u32,
    pub value_lira: f64,
    pub holders: u32,
}

/// Şehir sayfasının verisini kurar.
#[must_use]
pub fn city_detail(state: &GameState, ledger: &Ledger, city: CityId) -> CityDetail {
    let idle_threshold = moneywar_engine::IDLE_FACTORY_THRESHOLD;

    // ── Fabrikalar ve tarlalar ───────────────────────────────────────────────
    let factories: Vec<_> = state
        .factories
        .values()
        .filter(|f| f.city == city)
        .collect();
    let farms: Vec<_> = state
        .private_farms
        .values()
        .filter(|f| f.city == city)
        .collect();

    let mut by_product: BTreeMap<ProductKind, (u32, u32)> = BTreeMap::new();
    let mut factories_by_owner: BTreeMap<PlayerId, u32> = BTreeMap::new();
    let (mut employees, mut required) = (0u32, 0u32);
    let mut idle_count = 0u32;
    for f in &factories {
        let e = by_product.entry(f.product).or_default();
        e.0 += 1;
        if f.is_atil(state.current_tick, idle_threshold) {
            e.1 += 1;
            idle_count += 1;
        }
        *factories_by_owner.entry(f.owner).or_default() += 1;
        employees += f.employees;
        required += f.required_employees();
    }
    let mut farms_by_owner: BTreeMap<PlayerId, u32> = BTreeMap::new();
    for f in &farms {
        *farms_by_owner.entry(f.owner).or_default() += 1;
    }

    // ── Stok ─────────────────────────────────────────────────────────────────
    let mut stock_by_product: BTreeMap<ProductKind, (u32, u32)> = BTreeMap::new();
    let mut stock_by_owner: BTreeMap<PlayerId, (u32, i64)> = BTreeMap::new();
    for (id, player) in &state.players {
        for product in ProductKind::ALL {
            let units = player.inventory.get(city, product);
            if units == 0 {
                continue;
            }
            let e = stock_by_product.entry(product).or_default();
            e.0 += units;
            e.1 += 1;
            let value = unit_price(state, city, product).as_cents() * i64::from(units);
            let o = stock_by_owner.entry(*id).or_default();
            o.0 += units;
            o.1 += value;
        }
    }

    // ── Defterden akış ───────────────────────────────────────────────────────
    let trades: Vec<_> = ledger.trades().filter(|t| t.city == city).collect();
    let mut bought: BTreeMap<PlayerId, u64> = BTreeMap::new();
    let mut sold: BTreeMap<PlayerId, u64> = BTreeMap::new();
    for t in &trades {
        *bought.entry(t.buyer).or_default() += u64::from(t.quantity);
        *sold.entry(t.seller).or_default() += u64::from(t.quantity);
    }
    let mut produced: BTreeMap<ProductKind, u64> = BTreeMap::new();
    let mut produced_by_factory: BTreeMap<FactoryId, u64> = BTreeMap::new();
    for p in ledger.productions().filter(|p| p.city == city) {
        *produced.entry(p.product).or_default() += u64::from(p.units);
        *produced_by_factory.entry(p.factory).or_default() += u64::from(p.units);
    }

    // ── Aktörler ─────────────────────────────────────────────────────────────
    let mut actor_ids: Vec<PlayerId> = factories_by_owner
        .keys()
        .chain(farms_by_owner.keys())
        .chain(stock_by_owner.keys())
        .chain(bought.keys())
        .chain(sold.keys())
        .copied()
        .collect();
    actor_ids.sort_unstable();
    actor_ids.dedup();

    let mut actors: Vec<CityActorDto> = actor_ids
        .into_iter()
        .map(|id| {
            let (units, value) = stock_by_owner.get(&id).copied().unwrap_or((0, 0));
            CityActorDto {
                actor: actor_ref(state, id),
                factories: factories_by_owner.get(&id).copied().unwrap_or(0),
                farms: farms_by_owner.get(&id).copied().unwrap_or(0),
                stock_units: units,
                stock_value_lira: lira(Money::from_cents(value)),
                bought_units: bought.get(&id).copied().unwrap_or(0),
                sold_units: sold.get(&id).copied().unwrap_or(0),
            }
        })
        .collect();
    actors.sort_by(|a, b| {
        (b.factories, b.stock_units)
            .cmp(&(a.factories, a.stock_units))
            .then_with(|| a.actor.id.cmp(&b.actor.id))
    });

    let mut production: Vec<CityProductDto> = by_product
        .into_iter()
        .map(|(product, (count, idle))| CityProductDto {
            product: product_slug(product).to_string(),
            product_label: product.display_name().to_string(),
            factories: count,
            idle,
            produced_units: produced.get(&product).copied().unwrap_or(0),
        })
        .collect();
    production.sort_by(|a, b| b.factories.cmp(&a.factories));

    let mut stock: Vec<ProductStockDto> = stock_by_product
        .into_iter()
        .map(|(product, (units, holders))| ProductStockDto {
            product: product_slug(product).to_string(),
            product_label: product.display_name().to_string(),
            is_raw: product.is_raw(),
            units,
            value_lira: lira(Money::from_cents(
                unit_price(state, city, product).as_cents() * i64::from(units),
            )),
            holders,
        })
        .collect();
    stock.sort_by(|a, b| {
        b.value_lira
            .partial_cmp(&a.value_lira)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Çalışanlar önce, atıllar sonra; her grup üretime göre azalan.
    let mut factory_rows: Vec<CityFactoryDto> = factories
        .iter()
        .map(|f| CityFactoryDto {
            id: f.id.value(),
            owner: actor_ref(state, f.owner),
            product: product_slug(f.product).to_string(),
            product_label: f.product.display_name().to_string(),
            level: f.level,
            employees: f.employees,
            required_employees: f.required_employees(),
            idle: f.is_atil(state.current_tick, idle_threshold),
            pending_units: f.pending_units(),
            produced_units: produced_by_factory.get(&f.id).copied().unwrap_or(0),
        })
        .collect();
    factory_rows.sort_by(|a, b| {
        a.idle
            .cmp(&b.idle)
            .then_with(|| b.produced_units.cmp(&a.produced_units))
            .then_with(|| a.id.cmp(&b.id))
    });

    CityDetail {
        city: city_slug(city).to_string(),
        label: city.display_name().to_string(),
        tick: state.current_tick.value(),
        window_from_tick: ledger.earliest_tick().map(|t| t.value()),
        factory_count: u32::try_from(factories.len()).unwrap_or(u32::MAX),
        idle_factory_count: idle_count,
        farm_count: u32::try_from(farms.len()).unwrap_or(u32::MAX),
        employees,
        required_employees: required,
        factory_gini: gini_u32(factories_by_owner.values().copied()),
        stock_gini: gini_u32(stock_by_owner.values().map(|(u, _)| *u)),
        actors,
        production,
        factories: factory_rows,
        stock,
        volume: product_volumes(trades.iter().copied()),
        top_pairs: pair_flows(state, trades.iter().copied()),
        recent_trades: recent_rows(state, trades.iter().copied()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Firma
// ─────────────────────────────────────────────────────────────────────────────

/// Firma sayfası.
#[derive(Debug, Clone, Serialize)]
pub struct FirmDetail {
    pub actor: ActorRef,
    pub tick: u32,
    pub window_from_tick: Option<u32>,

    pub cash_lira: f64,
    pub stock_value_lira: f64,
    pub pnl_lira: f64,
    pub rank: Option<u32>,

    /// Fabrikaları — hangi şehirde, ne üretiyor, çalışıyor mu.
    pub factories: Vec<FirmFactoryDto>,
    pub farms: Vec<FirmFarmDto>,
    /// Envanteri (şehir × ürün).
    pub stock: Vec<StockRowDto>,
    /// En son işlemleri.
    pub recent_trades: Vec<TradeRowDto>,
    /// En çok iş yaptığı taraflar.
    pub partners: Vec<PartnerDto>,
    /// Ürün bazlı alım/satım dengesi.
    pub flow: Vec<FirmProductFlowDto>,
}

/// Firmanın bir fabrikası — sayfadan fabrika sayfasına geçiş noktası.
#[derive(Debug, Clone, Serialize)]
pub struct FirmFactoryDto {
    pub id: u64,
    pub city: String,
    pub city_label: String,
    pub product: String,
    pub product_label: String,
    pub level: u8,
    pub employees: u32,
    pub required_employees: u32,
    pub idle: bool,
    pub pending_units: u64,
    /// Defter penceresinde ürettiği toplam birim.
    pub produced_units: u64,
}

/// Firmanın bir özel çiftliği.
#[derive(Debug, Clone, Serialize)]
pub struct FirmFarmDto {
    pub id: u64,
    pub city: String,
    pub product: String,
    pub product_label: String,
    pub level: u8,
    pub output_per_tick: u32,
}

/// Bir ticaret ortağı — güven ilişkisi + gerçekleşen hacim.
#[derive(Debug, Clone, Serialize)]
pub struct PartnerDto {
    pub actor: ActorRef,
    /// Toplam güven ilişkisi (sezon boyu, `relationships`'ten).
    pub trade_count: u32,
    pub trust_score: f64,
    /// Defter penceresindeki hacim.
    pub bought_units: u64,
    pub sold_units: u64,
    pub value_lira: f64,
}

/// Bir üründe firmanın alım/satım dengesi.
#[derive(Debug, Clone, Serialize)]
pub struct FirmProductFlowDto {
    pub product: String,
    pub product_label: String,
    pub bought_units: u64,
    pub sold_units: u64,
    pub buy_value_lira: f64,
    pub sell_value_lira: f64,
    /// Ortalama alış/satış fiyatı — makas buradan okunur.
    pub avg_buy_lira: Option<f64>,
    pub avg_sell_lira: Option<f64>,
}

/// Firma sayfasının verisini kurar. Oyuncu yoksa `None`.
#[must_use]
pub fn firm_detail(state: &GameState, ledger: &Ledger, id: PlayerId) -> Option<FirmDetail> {
    let player = state.players.get(&id)?;
    let idle_threshold = moneywar_engine::IDLE_FACTORY_THRESHOLD;

    let scores = leaderboard(state);
    let rank = scores
        .iter()
        .position(|s| s.player_id == id)
        .and_then(|i| u32::try_from(i + 1).ok());
    let score = scores.iter().find(|s| s.player_id == id);

    // ── Üretim penceresi ─────────────────────────────────────────────────────
    let mut produced_by_factory: BTreeMap<FactoryId, u64> = BTreeMap::new();
    for p in ledger.productions().filter(|p| p.owner == id) {
        *produced_by_factory.entry(p.factory).or_default() += u64::from(p.units);
    }

    let mut factories: Vec<FirmFactoryDto> = state
        .factories
        .values()
        .filter(|f| f.owner == id)
        .map(|f| FirmFactoryDto {
            id: f.id.value(),
            city: city_slug(f.city).to_string(),
            city_label: f.city.display_name().to_string(),
            product: product_slug(f.product).to_string(),
            product_label: f.product.display_name().to_string(),
            level: f.level,
            employees: f.employees,
            required_employees: f.required_employees(),
            idle: f.is_atil(state.current_tick, idle_threshold),
            pending_units: f.pending_units(),
            produced_units: produced_by_factory.get(&f.id).copied().unwrap_or(0),
        })
        .collect();
    factories.sort_by(|a, b| b.produced_units.cmp(&a.produced_units));

    let farms: Vec<FirmFarmDto> = state
        .private_farms
        .values()
        .filter(|f| f.owner == id)
        .map(|f| FirmFarmDto {
            id: f.id.value(),
            city: city_slug(f.city).to_string(),
            product: product_slug(f.product).to_string(),
            product_label: f.product.display_name().to_string(),
            level: f.level,
            output_per_tick: f.output_per_tick(),
        })
        .collect();

    // ── Envanter ─────────────────────────────────────────────────────────────
    let mut stock: Vec<StockRowDto> = Vec::new();
    let mut stock_value = 0i64;
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let units = player.inventory.get(city, product);
            if units == 0 {
                continue;
            }
            let value = unit_price(state, city, product).as_cents() * i64::from(units);
            stock_value += value;
            stock.push(StockRowDto {
                city: city_slug(city).to_string(),
                product: product_slug(product).to_string(),
                product_label: product.display_name().to_string(),
                units,
                value_lira: lira(Money::from_cents(value)),
            });
        }
    }
    stock.sort_by(|a, b| {
        b.value_lira
            .partial_cmp(&a.value_lira)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ── Defterden akış ───────────────────────────────────────────────────────
    let trades: Vec<_> = ledger.trades().filter(|t| t.involves(id)).collect();

    let mut partner_acc: BTreeMap<PlayerId, (u64, u64, i64)> = BTreeMap::new();
    let mut flow_acc: BTreeMap<ProductKind, (u64, u64, i64, i64)> = BTreeMap::new();
    for t in &trades {
        let value = t.value().as_cents();
        let f = flow_acc.entry(t.product).or_default();
        if t.buyer == id {
            let e = partner_acc.entry(t.seller).or_default();
            e.0 += u64::from(t.quantity);
            e.2 += value;
            f.0 += u64::from(t.quantity);
            f.2 += value;
        } else {
            let e = partner_acc.entry(t.buyer).or_default();
            e.1 += u64::from(t.quantity);
            e.2 += value;
            f.1 += u64::from(t.quantity);
            f.3 += value;
        }
    }

    let mut partners: Vec<PartnerDto> = partner_acc
        .into_iter()
        .map(|(other, (bought, sold, value))| {
            let rel = state
                .relationships
                .get(&relation_key(id, other))
                .map(|r| (r.trade_count, r.trust_score()))
                .unwrap_or((0, 0.5));
            PartnerDto {
                actor: actor_ref(state, other),
                trade_count: rel.0,
                trust_score: rel.1,
                bought_units: bought,
                sold_units: sold,
                value_lira: lira(Money::from_cents(value)),
            }
        })
        .collect();
    partners.sort_by(|a, b| {
        b.value_lira
            .partial_cmp(&a.value_lira)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    partners.truncate(TOP_N);

    let mut flow: Vec<FirmProductFlowDto> = flow_acc
        .into_iter()
        .map(
            |(product, (bought, sold, buy_v, sell_v))| FirmProductFlowDto {
                product: product_slug(product).to_string(),
                product_label: product.display_name().to_string(),
                bought_units: bought,
                sold_units: sold,
                buy_value_lira: lira(Money::from_cents(buy_v)),
                sell_value_lira: lira(Money::from_cents(sell_v)),
                avg_buy_lira: avg_price(buy_v, bought),
                avg_sell_lira: avg_price(sell_v, sold),
            },
        )
        .collect();
    flow.sort_by(|a, b| (b.bought_units + b.sold_units).cmp(&(a.bought_units + a.sold_units)));

    Some(FirmDetail {
        actor: actor_ref(state, id),
        tick: state.current_tick.value(),
        window_from_tick: ledger.earliest_tick().map(|t| t.value()),
        cash_lira: lira(player.cash),
        stock_value_lira: lira(Money::from_cents(stock_value)),
        pnl_lira: score.map_or(0.0, |s| lira(s.total)),
        rank,
        factories,
        farms,
        stock,
        recent_trades: recent_rows(state, trades.iter().copied()),
        partners,
        flow,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Fabrika
// ─────────────────────────────────────────────────────────────────────────────

/// Fabrika sayfası.
#[derive(Debug, Clone, Serialize)]
pub struct FactoryDetail {
    pub id: u64,
    pub owner: ActorRef,
    pub city: String,
    pub city_label: String,
    pub product: String,
    pub product_label: String,
    pub tick: u32,
    pub window_from_tick: Option<u32>,

    pub level: u8,
    pub employees: u32,
    pub required_employees: u32,
    /// Kadro doluluğu [0,1] — üretim bu oranla ölçeklenir.
    pub staffing: f64,
    pub idle: bool,
    pub ticks_since_production: Option<u32>,
    pub pending_units: u64,
    /// İşlenmekte olan batch'ler.
    pub batches: Vec<BatchDto>,

    /// Tarifin girdileri ve sahibin bu şehirdeki stoğu — "neden duruyor"un
    /// cevabı. Eksik girdi burada görünür.
    pub inputs: Vec<InputStatusDto>,
    /// Sahibin bu şehirdeki bitmiş ürün stoğu.
    pub output_stock: u32,

    /// Birim maliyet (tarif, baseline'dan) ve piyasa fiyatı → marj.
    pub unit_cost_lira: Option<f64>,
    pub market_price_lira: f64,
    pub margin_pct: Option<f64>,

    /// Defter penceresinde üretim geçmişi (tick → birim).
    pub production_history: Vec<ProductionPointDto>,
    pub produced_units: u64,
    /// Sahibin bu şehirde bu ürünü sattığı işlemler.
    pub recent_sales: Vec<TradeRowDto>,
}

/// İşlenmekte olan batch.
#[derive(Debug, Clone, Serialize)]
pub struct BatchDto {
    pub started_tick: u32,
    pub completion_tick: u32,
    pub units: u32,
    /// Tamamlanmasına kalan tick.
    pub ticks_remaining: u32,
}

/// Bir girdinin durumu — tarif ne istiyor, stokta ne var.
///
/// Tarifteki sayı **birim başı değil, batch'in yüzdesidir**: ana girdi
/// batch'in %100'ü, ek girdiler tabloda yazan oranda. Motor tam batch
/// yoksa bandı durdurmaz, küçültür — bu yüzden iki eşik var: tam batch
/// (`required`) ve bandın hiç dönemeyeceği alt sınır (`min_required`).
#[derive(Debug, Clone, Serialize)]
pub struct InputStatusDto {
    pub product: String,
    pub product_label: String,
    /// Ana girdi mi (batch boyutunu bu belirler).
    pub is_primary: bool,
    /// Tam batch için gereken miktar.
    pub required: u32,
    /// Bunun altında fabrika hiç üretemez (motorun kısmi üretim tabanı).
    pub min_required: u32,
    /// Sahibin bu şehirdeki stoğu.
    pub available: u32,
    /// Stok kaç tam batch'e yeter.
    pub batches_covered: u32,
    /// Stok alt sınırın altında — **şu an** bu girdiyle batch başlatılamaz.
    ///
    /// Tek başına arıza demek değil: bandı yeni boşaltmış bir fabrika da
    /// bir tick boyunca böyle görünür. Gerçek tıkanıklık `idle` (ya da
    /// büyümüş `ticks_since_production`) ile birlikte okunur.
    pub blocking: bool,
    /// Bant dönüyor ama tam kapasitede değil.
    pub partial: bool,
}

/// Üretim geçmişinin bir noktası.
#[derive(Debug, Clone, Serialize)]
pub struct ProductionPointDto {
    pub tick: u32,
    pub units: u32,
}

/// Fabrika sayfasının verisini kurar. Fabrika yoksa `None`.
#[must_use]
pub fn factory_detail(state: &GameState, ledger: &Ledger, id: FactoryId) -> Option<FactoryDetail> {
    let f = state.factories.get(&id)?;
    let owner = state.players.get(&f.owner);
    let idle_threshold = moneywar_engine::IDLE_FACTORY_THRESHOLD;
    let now = state.current_tick.value();

    // ── Girdiler ─────────────────────────────────────────────────────────────
    //
    // Motorun kuralı: ana girdi batch'in tamamı, ek girdiler `pct`'si kadar.
    // Tam batch yoksa bant durmaz, `batch/4`'e kadar küçülür. Panelin işe
    // yaraması için ikisini de göstermek gerekiyor — "eksik" ile "durdu"
    // aynı şey değil.
    let batch = f.batch_size();
    let partial_min = (batch / 4).max(1);
    let primary = f.product.raw_input();
    let inputs: Vec<InputStatusDto> = f
        .product
        .recipe()
        .into_iter()
        .map(|(input, pct)| {
            let is_primary = primary == Some(input);
            let need_at = |b: u32| {
                if is_primary {
                    b
                } else {
                    (b.saturating_mul(pct) / 100).max(1)
                }
            };
            let required = need_at(batch);
            let min_required = need_at(partial_min);
            let available = owner.map_or(0, |p| p.inventory.get(f.city, input));
            InputStatusDto {
                product: product_slug(input).to_string(),
                product_label: input.display_name().to_string(),
                is_primary,
                required,
                min_required,
                available,
                batches_covered: if required == 0 {
                    0
                } else {
                    available / required
                },
                blocking: available < min_required,
                partial: available >= min_required && available < required,
            }
        })
        .collect();

    // ── Maliyet / marj ───────────────────────────────────────────────────────
    let market = unit_price(state, f.city, f.product);
    let unit_cost = state.recipe_unit_cost(f.city, f.product);
    let margin_pct = unit_cost.and_then(|c| {
        let cost = c.as_cents();
        (cost > 0).then(|| (market.as_cents() - cost) as f64 / cost as f64 * 100.0)
    });

    // ── Üretim geçmişi ───────────────────────────────────────────────────────
    let history: Vec<ProductionPointDto> = ledger
        .productions()
        .filter(|p| p.factory == id)
        .map(|p| ProductionPointDto {
            tick: p.tick.value(),
            units: p.units,
        })
        .collect();
    let produced_units: u64 = history.iter().map(|p| u64::from(p.units)).sum();

    let sales: Vec<_> = ledger
        .trades()
        .filter(|t| t.seller == f.owner && t.city == f.city && t.product == f.product)
        .collect();

    Some(FactoryDetail {
        id: id.value(),
        owner: actor_ref(state, f.owner),
        city: city_slug(f.city).to_string(),
        city_label: f.city.display_name().to_string(),
        product: product_slug(f.product).to_string(),
        product_label: f.product.display_name().to_string(),
        tick: now,
        window_from_tick: ledger.earliest_tick().map(|t| t.value()),
        level: f.level,
        employees: f.employees,
        required_employees: f.required_employees(),
        staffing: f64::from(f.staffing_pct()) / 100.0,
        idle: f.is_atil(state.current_tick, idle_threshold),
        ticks_since_production: f.ticks_since_last_production(state.current_tick),
        pending_units: f.pending_units(),
        batches: f
            .batches
            .iter()
            .map(|b| BatchDto {
                started_tick: b.started_tick.value(),
                completion_tick: b.completion_tick.value(),
                units: b.units,
                ticks_remaining: b.completion_tick.value().saturating_sub(now),
            })
            .collect(),
        inputs,
        output_stock: owner.map_or(0, |p| p.inventory.get(f.city, f.product)),
        unit_cost_lira: unit_cost.map(lira),
        market_price_lira: lira(market),
        margin_pct,
        production_history: history,
        produced_units,
        recent_sales: recent_rows(state, sales.into_iter()),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Ortak hesaplar
// ─────────────────────────────────────────────────────────────────────────────

/// Bir (şehir, ürün) için değerleme fiyatı — son takas, yoksa baseline.
fn unit_price(state: &GameState, city: CityId, product: ProductKind) -> Money {
    state
        .price_history
        .get(&(city, product))
        .and_then(|h| h.last())
        .map(|(_, p)| *p)
        .or_else(|| state.price_baseline.get(&(city, product)).copied())
        .unwrap_or_else(|| Money::from_cents(product.base_price_lira() * 100))
}

/// `relationships` anahtarı sıralı çift tutar.
fn relation_key(a: PlayerId, b: PlayerId) -> (PlayerId, PlayerId) {
    if a <= b { (a, b) } else { (b, a) }
}

fn avg_price(value_cents: i64, units: u64) -> Option<f64> {
    (units > 0).then(|| value_cents as f64 / units as f64 / 100.0)
}

/// En yeniden en eskiye, `RECENT_LIMIT` işlem.
fn recent_rows<'a>(
    state: &GameState,
    trades: impl DoubleEndedIterator<Item = &'a crate::ledger::Trade>,
) -> Vec<TradeRowDto> {
    trades
        .rev()
        .take(RECENT_LIMIT)
        .map(|t| TradeRowDto {
            tick: t.tick.value(),
            city: city_slug(t.city).to_string(),
            product: product_slug(t.product).to_string(),
            product_label: t.product.display_name().to_string(),
            quantity: t.quantity,
            price_lira: lira(t.price),
            value_lira: lira(t.value()),
            buyer: actor_ref(state, t.buyer),
            seller: actor_ref(state, t.seller),
        })
        .collect()
}

/// Ürün bazlı hacim, değere göre azalan.
fn product_volumes<'a>(
    trades: impl Iterator<Item = &'a crate::ledger::Trade>,
) -> Vec<ProductVolumeDto> {
    let mut acc: BTreeMap<ProductKind, (u64, i64)> = BTreeMap::new();
    for t in trades {
        let e = acc.entry(t.product).or_default();
        e.0 += u64::from(t.quantity);
        e.1 += t.value().as_cents();
    }
    let mut out: Vec<ProductVolumeDto> = acc
        .into_iter()
        .map(|(product, (units, value))| ProductVolumeDto {
            product: product_slug(product).to_string(),
            product_label: product.display_name().to_string(),
            is_raw: product.is_raw(),
            units,
            value_lira: lira(Money::from_cents(value)),
            avg_price_lira: avg_price(value, units).unwrap_or(0.0),
        })
        .collect();
    out.sort_by(|a, b| {
        b.value_lira
            .partial_cmp(&a.value_lira)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// "Kim kimle" — en çok iş yapan alıcı/satıcı çiftleri.
fn pair_flows<'a>(
    state: &GameState,
    trades: impl Iterator<Item = &'a crate::ledger::Trade>,
) -> Vec<PairFlowDto> {
    let mut acc: BTreeMap<(PlayerId, PlayerId), (u32, u64, i64)> = BTreeMap::new();
    for t in trades {
        let e = acc.entry((t.buyer, t.seller)).or_default();
        e.0 += 1;
        e.1 += u64::from(t.quantity);
        e.2 += t.value().as_cents();
    }
    let mut out: Vec<PairFlowDto> = acc
        .into_iter()
        .map(|((buyer, seller), (trades, units, value))| PairFlowDto {
            buyer: actor_ref(state, buyer),
            seller: actor_ref(state, seller),
            trades,
            units,
            value_lira: lira(Money::from_cents(value)),
        })
        .collect();
    out.sort_by(|a, b| {
        b.value_lira
            .partial_cmp(&a.value_lira)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(TOP_N);
    out
}

/// Gini katsayısı — negatif olmayan tam sayı dağılımı için.
fn gini_u32(values: impl Iterator<Item = u32>) -> f64 {
    let mut xs: Vec<f64> = values.map(f64::from).collect();
    if xs.len() < 2 {
        return 0.0;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = xs.len() as f64;
    let sum: f64 = xs.iter().sum();
    if sum <= 0.0 {
        return 0.0;
    }
    let weighted: f64 = xs
        .iter()
        .enumerate()
        .map(|(i, x)| (i as f64 + 1.0) * x)
        .sum();
    ((2.0 * weighted) / (n * sum) - (n + 1.0) / n).clamp(0.0, 1.0)
}

/// `NpcKind` etiketi — rol filtresi için dışa açık yardımcı.
#[must_use]
pub fn role_label(kind: NpcKind) -> &'static str {
    kind.label()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::SimDriver;

    /// Birkaç tick koşmuş gerçek bir dünya — pazar hareketi ve üretim olsun.
    fn warm_driver() -> SimDriver {
        let mut d = SimDriver::new(crate::DEFAULT_SEED, 400, 3, crate::DIFFICULTY);
        for _ in 0..40 {
            d.step();
        }
        d
    }

    #[test]
    fn city_detail_counts_match_the_state() {
        let d = warm_driver();
        let city = CityId::ALL[0];
        let detail = city_detail(&d.state, &d.ledger, city);

        let expected = d
            .state
            .factories
            .values()
            .filter(|f| f.city == city)
            .count();
        assert_eq!(detail.factory_count as usize, expected);
        assert!(detail.idle_factory_count <= detail.factory_count);
        assert_eq!(detail.city, city_slug(city));
    }

    #[test]
    fn city_actor_stock_matches_the_inventory() {
        let d = warm_driver();
        let city = CityId::ALL[0];
        let detail = city_detail(&d.state, &d.ledger, city);

        for a in &detail.actors {
            let player = d
                .state
                .players
                .get(&PlayerId::new(a.actor.id))
                .expect("aktör oyuncu olmalı");
            let units: u32 = ProductKind::ALL
                .into_iter()
                .map(|p| player.inventory.get(city, p))
                .sum();
            assert_eq!(a.stock_units, units, "{} stoğu tutmuyor", a.actor.name);
        }
    }

    #[test]
    fn city_pairs_and_volume_stay_within_the_ledger() {
        let d = warm_driver();
        let city = CityId::ALL[0];
        let detail = city_detail(&d.state, &d.ledger, city);

        let ledger_units: u64 = d
            .ledger
            .trades()
            .filter(|t| t.city == city)
            .map(|t| u64::from(t.quantity))
            .sum();
        let volume_units: u64 = detail.volume.iter().map(|v| v.units).sum();
        assert_eq!(volume_units, ledger_units, "hacim defterle örtüşmeli");

        assert!(detail.top_pairs.len() <= TOP_N);
        assert!(detail.recent_trades.len() <= RECENT_LIMIT);
        // Sıralama azalan olmalı — ilk satır en büyük iş.
        for w in detail.top_pairs.windows(2) {
            assert!(w[0].value_lira >= w[1].value_lira, "çiftler sıralı değil");
        }
    }

    #[test]
    fn firm_detail_totals_agree_with_the_player() {
        let d = warm_driver();
        let id = *d.state.players.keys().next().expect("oyuncu olmalı");
        let detail = firm_detail(&d.state, &d.ledger, id).expect("firma bulunmalı");

        let player = &d.state.players[&id];
        assert!((detail.cash_lira - lira(player.cash)).abs() < 0.01);

        let listed: u32 = detail.stock.iter().map(|s| s.units).sum();
        let actual: u32 = CityId::ALL
            .into_iter()
            .flat_map(|c| ProductKind::ALL.into_iter().map(move |p| (c, p)))
            .map(|(c, p)| player.inventory.get(c, p))
            .sum();
        assert_eq!(listed, actual, "envanter toplamı tutmalı");

        let owned = d.state.factories.values().filter(|f| f.owner == id).count();
        assert_eq!(detail.factories.len(), owned);
    }

    #[test]
    fn firm_recent_trades_only_involve_that_firm() {
        let d = warm_driver();
        // İşlem yapmış bir firma seç.
        let id = d
            .ledger
            .trades()
            .next_back()
            .map(|t| t.seller)
            .expect("defterde işlem olmalı");
        let detail = firm_detail(&d.state, &d.ledger, id).expect("firma bulunmalı");

        assert!(
            !detail.recent_trades.is_empty(),
            "satıcının işlemi görünmeli"
        );
        for t in &detail.recent_trades {
            assert!(
                t.buyer.id == id.value() || t.seller.id == id.value(),
                "yabancı işlem sızmış"
            );
        }
        // En yeni önce.
        for w in detail.recent_trades.windows(2) {
            assert!(
                w[0].tick >= w[1].tick,
                "işlemler yeniden eskiye sıralı değil"
            );
        }
    }

    #[test]
    fn unknown_firm_and_factory_return_none() {
        let d = warm_driver();
        assert!(firm_detail(&d.state, &d.ledger, PlayerId::new(999_999)).is_none());
        assert!(factory_detail(&d.state, &d.ledger, FactoryId::new(999_999)).is_none());
    }

    #[test]
    fn factory_inputs_expose_the_shortage() {
        let d = warm_driver();
        let f = d.state.factories.values().next().expect("fabrika olmalı");
        let detail = factory_detail(&d.state, &d.ledger, f.id).expect("fabrika bulunmalı");

        assert_eq!(detail.inputs.len(), f.product.recipe().len());
        let owner = &d.state.players[&f.owner];
        assert!(
            detail.inputs.iter().filter(|i| i.is_primary).count() <= 1,
            "ana girdi en fazla bir tane"
        );
        for i in &detail.inputs {
            assert!(i.min_required <= i.required, "alt sınır tam batch'i aşamaz");
            assert!(
                !(i.blocking && i.partial),
                "bir girdi hem durdurup hem kısmi çalıştıramaz"
            );
            assert_eq!(i.blocking, i.available < i.min_required);
            assert_eq!(
                i.batches_covered,
                if i.required == 0 {
                    0
                } else {
                    i.available / i.required
                }
            );
            // Panelin sayısı motorun tükettiğiyle aynı olmalı: ana girdide
            // batch'in kendisi, ek girdide batch'in yüzdesi. Bu doğrulama
            // olmadan tarifteki sayıyı "birim başı" sanmak sessizce 100×
            // şişmiş bir ihtiyaç gösteriyordu.
            let pct = f
                .product
                .recipe()
                .into_iter()
                .find(|(p, _)| product_slug(*p) == i.product)
                .map(|(_, pct)| pct)
                .expect("girdi tarifte olmalı");
            let expected = if i.is_primary {
                f.batch_size()
            } else {
                (f.batch_size().saturating_mul(pct) / 100).max(1)
            };
            assert_eq!(
                i.required, expected,
                "{} gereği motorla uyuşmuyor",
                i.product
            );
        }
        assert_eq!(detail.output_stock, owner.inventory.get(f.city, f.product));
        assert!((0.0..=1.0).contains(&detail.staffing));
    }

    #[test]
    fn factory_production_history_belongs_to_that_factory() {
        let d = warm_driver();
        // Üretmiş bir fabrika bul.
        let Some(fid) = d.ledger.productions().next_back().map(|p| p.factory) else {
            return; // Bu seed'de henüz üretim yoksa test anlamsız.
        };
        let detail = factory_detail(&d.state, &d.ledger, fid).expect("fabrika bulunmalı");

        let expected: u64 = d
            .ledger
            .productions()
            .filter(|p| p.factory == fid)
            .map(|p| u64::from(p.units))
            .sum();
        assert_eq!(detail.produced_units, expected);
        for w in detail.production_history.windows(2) {
            assert!(w[0].tick <= w[1].tick, "geçmiş eskiden yeniye olmalı");
        }
    }

    #[test]
    fn gini_is_zero_when_equal_and_high_when_concentrated() {
        assert!((gini_u32([5u32, 5, 5, 5].into_iter()) - 0.0).abs() < 1e-9);
        assert!(gini_u32([0u32, 0, 0, 100].into_iter()) > 0.7);
        assert_eq!(gini_u32(std::iter::once(7)), 0.0, "tek eleman ölçülemez");
        assert_eq!(gini_u32([0u32, 0].into_iter()), 0.0, "boş dağılım 0");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// İlişki ağı
// ─────────────────────────────────────────────────────────────────────────────

/// Firmalar arası ilişki ağı — kim kiminle çalışıyor, kim kime düşman.
///
/// # Neden ayrı bir sayfa
///
/// İlişki verisi motorda zaten var ve iki yönlü işliyor: olay ilişkiyi
/// doğuruyor (fiyat kırma → kin), ilişki de olayı etkiliyor (güven → daha
/// yüksek teklif, kin → sert davranış). Ama arayüzde hiçbiri görünmüyordu;
/// akışta "KİN" yazan bir satır geçiyor, kimin kime ne yaptığı kayboluyordu.
///
/// Bu yapı ağı tek resimde toplar: düğüm firma, kenar ilişki. Ticaret
/// kenarları bağı, çatışma kenarları husumeti gösterir.
#[derive(Debug, Clone, Serialize)]
pub struct RelationsGraph {
    pub tick: u32,
    pub window_from_tick: Option<u32>,
    pub nodes: Vec<GraphNodeDto>,
    pub edges: Vec<GraphEdgeDto>,
    pub summary: RelationsSummary,
}

/// Ağdaki bir firma.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNodeDto {
    pub id: u64,
    pub name: String,
    pub role: Option<String>,
    pub pnl_lira: f64,
    pub factories: u32,
    /// Kaç firmayla ticaret bağı, kaç firmayla husumeti var.
    pub partners: u32,
    pub rivals: u32,
    /// Elinde tuttuğu tekel sayısı — ağdaki güç göstergesi.
    pub monopolies: u32,
}

/// İki firma arasındaki tek bir ilişki.
#[derive(Debug, Clone, Serialize)]
pub struct GraphEdgeDto {
    pub from: u64,
    pub to: u64,
    /// "ticaret" | "kin" | "savas" | "bogma"
    pub kind: String,
    /// Çizgi kalınlığı için [0,1] — türü içinde normalize.
    pub strength: f64,
    /// Tek cümlelik açıklama; izleyici grafiğe bakıp anlamlandırabilsin.
    pub label: String,
    pub city: Option<String>,
    pub product: Option<String>,
    /// Ticaret kenarı için hacim ve güven.
    pub trade_count: Option<u32>,
    pub units: Option<u64>,
    pub value_lira: Option<f64>,
    pub trust: Option<f64>,
    /// Çatışma kenarı için kalan/geçen tick.
    pub ticks: Option<u32>,
}

/// Ağın tek bakışta özeti.
#[derive(Debug, Clone, Serialize)]
pub struct RelationsSummary {
    pub trade_edges: u32,
    pub conflict_edges: u32,
    pub grudges: u32,
    pub price_wars: u32,
    pub supply_chokes: u32,
    pub monopolies: u32,
    /// En çok ortağı olan firma ve en sert husumet.
    pub most_connected: Option<ActorRef>,
    pub fiercest_rivalry: Option<(ActorRef, ActorRef)>,
}

/// İlişki ağını kurar.
#[must_use]
pub fn relations_graph(state: &GameState, ledger: &Ledger) -> RelationsGraph {
    let intrigue = &state.intrigue;

    // ── Ticaret kenarları ────────────────────────────────────────────────────
    // Güven `relationships`'ten (sezon boyu), hacim defterden (son pencere).
    let mut flow: BTreeMap<(PlayerId, PlayerId), (u64, i64)> = BTreeMap::new();
    for t in ledger.trades() {
        let e = flow.entry(relation_key(t.seller, t.buyer)).or_default();
        e.0 += u64::from(t.quantity);
        e.1 += t.value().as_cents();
    }

    let mut edges: Vec<GraphEdgeDto> = Vec::new();
    let max_value = flow.values().map(|(_, v)| *v).max().unwrap_or(1).max(1);

    for ((a, b), rel) in &state.relationships {
        if rel.trade_count == 0 {
            continue;
        }
        let (units, value) = flow.get(&(*a, *b)).copied().unwrap_or((0, 0));
        // Defterde izi olmayan eski ilişkiyi çizmeye değmez — grafiği boğar.
        if units == 0 {
            continue;
        }
        let trust = rel.trust_score();
        edges.push(GraphEdgeDto {
            from: a.value(),
            to: b.value(),
            kind: "ticaret".into(),
            strength: (value as f64 / max_value as f64).clamp(0.0, 1.0),
            label: format!(
                "{} işlem · {} birim · güven {:.2}",
                rel.trade_count, units, trust
            ),
            city: None,
            product: None,
            trade_count: Some(rel.trade_count),
            units: Some(units),
            value_lira: Some(lira(Money::from_cents(value))),
            trust: Some(trust),
            ticks: None,
        });
    }
    let trade_edges = u32::try_from(edges.len()).unwrap_or(u32::MAX);

    // ── Çatışma kenarları ────────────────────────────────────────────────────
    for ((holder, target), ticks) in &intrigue.grudges {
        edges.push(GraphEdgeDto {
            from: holder.value(),
            to: target.value(),
            kind: "kin".into(),
            strength: (f64::from(*ticks) / 30.0).clamp(0.15, 1.0),
            label: format!("{ticks} tick daha kin tutuyor"),
            city: None,
            product: None,
            trade_count: None,
            units: None,
            value_lira: None,
            trust: None,
            ticks: Some(*ticks),
        });
    }

    for ((attacker, victim, city, product), track) in &intrigue.price_wars {
        let since = state
            .current_tick
            .value()
            .saturating_sub(track.declared_at.value());
        edges.push(GraphEdgeDto {
            from: attacker.value(),
            to: victim.value(),
            kind: "savas".into(),
            strength: 1.0,
            label: format!(
                "{} {} pazarında {} tick'tir fiyat savaşı",
                city.display_name(),
                product.display_name(),
                since
            ),
            city: Some(city_slug(*city).to_string()),
            product: Some(product_slug(*product).to_string()),
            trade_count: None,
            units: None,
            value_lira: None,
            trust: None,
            ticks: Some(since),
        });
    }

    for (choker, victim, city, product) in &intrigue.active_chokes {
        edges.push(GraphEdgeDto {
            from: choker.value(),
            to: victim.value(),
            kind: "bogma".into(),
            strength: 0.8,
            label: format!(
                "{} {} tedarikini kesiyor",
                city.display_name(),
                product.display_name()
            ),
            city: Some(city_slug(*city).to_string()),
            product: Some(product_slug(*product).to_string()),
            trade_count: None,
            units: None,
            value_lira: None,
            trust: None,
            ticks: None,
        });
    }

    // ── Düğümler ─────────────────────────────────────────────────────────────
    let scores = leaderboard(state);
    let pnl: BTreeMap<PlayerId, f64> = scores
        .iter()
        .map(|s| (s.player_id, lira(s.total)))
        .collect();

    let mut partners: BTreeMap<u64, u32> = BTreeMap::new();
    let mut rivals: BTreeMap<u64, u32> = BTreeMap::new();
    for e in &edges {
        let bucket = if e.kind == "ticaret" { &mut partners } else { &mut rivals };
        *bucket.entry(e.from).or_default() += 1;
        *bucket.entry(e.to).or_default() += 1;
    }

    let mut monopolies: BTreeMap<PlayerId, u32> = BTreeMap::new();
    for firm in intrigue.monopolist.values() {
        *monopolies.entry(*firm).or_default() += 1;
    }

    // Ağda yeri olan firmalar — bağı da husumeti de olmayanı çizmeye gerek yok.
    let mut ids: Vec<u64> = partners.keys().chain(rivals.keys()).copied().collect();
    ids.sort_unstable();
    ids.dedup();

    let nodes: Vec<GraphNodeDto> = ids
        .iter()
        .map(|id| {
            let pid = PlayerId::new(*id);
            let p = state.players.get(&pid);
            GraphNodeDto {
                id: *id,
                name: p.map_or_else(|| format!("#{id}"), |p| p.name.clone()),
                role: p.and_then(|p| p.npc_kind).map(|k| k.label().to_string()),
                pnl_lira: pnl.get(&pid).copied().unwrap_or(0.0),
                factories: u32::try_from(
                    state.factories.values().filter(|f| f.owner == pid).count(),
                )
                .unwrap_or(u32::MAX),
                partners: partners.get(id).copied().unwrap_or(0),
                rivals: rivals.get(id).copied().unwrap_or(0),
                monopolies: monopolies.get(&pid).copied().unwrap_or(0),
            }
        })
        .collect();

    let most_connected = nodes
        .iter()
        .max_by_key(|n| n.partners)
        .filter(|n| n.partners > 0)
        .map(|n| actor_ref(state, PlayerId::new(n.id)));

    // En sert husumet: savaş varsa o, yoksa en uzun kin.
    let fiercest = edges
        .iter()
        .filter(|e| e.kind == "savas")
        .max_by_key(|e| e.ticks.unwrap_or(0))
        .or_else(|| {
            edges
                .iter()
                .filter(|e| e.kind == "kin")
                .max_by_key(|e| e.ticks.unwrap_or(0))
        })
        .map(|e| {
            (
                actor_ref(state, PlayerId::new(e.from)),
                actor_ref(state, PlayerId::new(e.to)),
            )
        });

    RelationsGraph {
        tick: state.current_tick.value(),
        window_from_tick: ledger.earliest_tick().map(|t| t.value()),
        summary: RelationsSummary {
            trade_edges,
            conflict_edges: u32::try_from(edges.len()).unwrap_or(u32::MAX) - trade_edges,
            grudges: u32::try_from(intrigue.grudges.len()).unwrap_or(u32::MAX),
            price_wars: u32::try_from(intrigue.price_wars.len()).unwrap_or(u32::MAX),
            supply_chokes: u32::try_from(intrigue.active_chokes.len()).unwrap_or(u32::MAX),
            monopolies: u32::try_from(intrigue.monopolist.len()).unwrap_or(u32::MAX),
            most_connected,
            fiercest_rivalry: fiercest,
        },
        nodes,
        edges,
    }
}
