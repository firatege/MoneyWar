//! Web DTO katmanı — `GameState` → JSON-serileştirilebilir düz struct'lar.
//!
//! Motorun iç temsili `BTreeMap<(CityId, ProductKind), V>` tuple key kullanır;
//! bu JSON object key'ine çevrilemez. Burada her tuple-keyed harita düz bir
//! `Vec<Cell>` dizisine açılır. Para birimi frontend kolaylığı için lira
//! (`f64`, cents/100) olarak gönderilir.

use moneywar_domain::{
    CityId, GameState, Money, NpcKind, OrderSide, PlayerId, ProductKind,
};
use moneywar_engine::{LogEntry, LogEvent, TickReport, leaderboard};
use serde::Serialize;

/// Bir tick anının tam görüntüsü — ilk yükleme + her tick WS push.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub season: u64,
    pub tick: u32,
    pub season_ticks: u32,
    pub seconds_per_tick: u32,
    pub leaderboard: Vec<PlayerDto>,
    pub prices: Vec<PriceCell>,
    pub factories: Vec<FactoryDto>,
    pub caravans: Vec<CaravanDto>,
    pub recent_events: Vec<EventDto>,
}

/// Leaderboard satırı — skor + oyuncu kimliği birleşmiş.
#[derive(Debug, Clone, Serialize)]
pub struct PlayerDto {
    pub id: u64,
    pub name: String,
    pub role: String,
    pub npc_kind: Option<String>,
    pub cash_lira: f64,
    pub pnl_lira: f64,
    pub is_npc: bool,
}

/// Tek bir (şehir, ürün) pazar hücresi — fiyat ızgarası kaynağı.
#[derive(Debug, Clone, Serialize)]
pub struct PriceCell {
    pub city: String,
    pub city_label: String,
    pub product: String,
    pub product_label: String,
    pub is_raw: bool,
    pub baseline_lira: f64,
    pub last_lira: Option<f64>,
    pub avg5_lira: Option<f64>,
    pub bid_lira: Option<f64>,
    pub ask_lira: Option<f64>,
    pub buy_qty: u32,
    pub sell_qty: u32,
}

/// Fabrika durum kartı.
#[derive(Debug, Clone, Serialize)]
pub struct FactoryDto {
    pub id: u64,
    pub owner: u64,
    pub city: String,
    pub product: String,
    pub pending_units: u64,
    pub idle: bool,
}

/// Kervan durum kartı.
#[derive(Debug, Clone, Serialize)]
pub struct CaravanDto {
    pub id: u64,
    pub owner: u64,
    pub idle: bool,
    pub current_city: Option<String>,
    pub cargo_units: u64,
}

/// Tek bir okunabilir olay (event feed satırı).
#[derive(Debug, Clone, Serialize)]
pub struct EventDto {
    pub tick: u32,
    pub kind: String,
    pub summary: String,
    /// Olayın bucket'ı (varsa) — frontend bucket-özel filtreleme için.
    pub city: Option<String>,
    pub product: Option<String>,
    /// Eşleşme miktarı + fiyatı (match olayları için) — bucket işlem listesi.
    pub qty: Option<u32>,
    pub price_lira: Option<f64>,
    /// Alıcı/satıcı kimliği (match olayları için) — kişi bazlı istatistik.
    pub buyer_id: Option<u64>,
    pub seller_id: Option<u64>,
}

/// Grafik için (şehir, ürün) fiyat zaman serisi.
#[derive(Debug, Clone, Serialize)]
pub struct PriceSeries {
    pub city: String,
    pub product: String,
    pub points: Vec<PricePoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PricePoint {
    pub tick: u32,
    pub lira: f64,
}

fn lira(m: Money) -> f64 {
    m.as_cents() as f64 / 100.0
}

fn city_slug(c: CityId) -> &'static str {
    match c {
        CityId::Istanbul => "istanbul",
        CityId::Ankara => "ankara",
        CityId::Izmir => "izmir",
        CityId::Bursa => "bursa",
        CityId::Konya => "konya",
    }
}

fn product_slug(p: ProductKind) -> &'static str {
    match p {
        ProductKind::Pamuk => "pamuk",
        ProductKind::Bugday => "bugday",
        ProductKind::Zeytin => "zeytin",
        ProductKind::Kumas => "kumas",
        ProductKind::Un => "un",
        ProductKind::Zeytinyagi => "zeytinyagi",
    }
}

/// `GameState` + son tick raporundan tam snapshot üret.
#[must_use]
pub fn build_snapshot(
    state: &GameState,
    report: &TickReport,
    season: u64,
    season_ticks: u32,
    seconds_per_tick: u32,
) -> Snapshot {
    Snapshot {
        season,
        tick: state.current_tick.value(),
        season_ticks,
        seconds_per_tick,
        leaderboard: build_leaderboard(state),
        prices: build_prices(state),
        factories: build_factories(state),
        caravans: build_caravans(state),
        recent_events: build_feed(state, report),
    }
}

/// Bu tick'in olaylarından yüksek-sinyalli feed satırlarını süz.
/// `CommandAccepted/Rejected`, `MarketCleared`, haber/maaş gürültüsü atılır;
/// match/üretim/fabrika/kervan/hasat/kredi gibi anlamlı olaylar kalır.
/// En fazla [`FEED_LIMIT`] satır (en yeniler).
fn build_feed(state: &GameState, report: &TickReport) -> Vec<EventDto> {
    let mut feed: Vec<EventDto> = report
        .entries
        .iter()
        .filter(|e| is_feed_worthy(&e.event))
        .map(|e| build_event(state, e))
        .collect();
    if feed.len() > FEED_LIMIT {
        feed.drain(0..feed.len() - FEED_LIMIT);
    }
    feed
}

/// Feed'de gösterilecek max olay sayısı (tick başına).
const FEED_LIMIT: usize = 40;

fn is_feed_worthy(event: &LogEvent) -> bool {
    // Sadece anlamlı, takip edilebilir olaylar. OrderExpired/FactoryIdle
    // gürültüsü feed'i boğuyordu — çıkarıldı.
    matches!(
        event,
        LogEvent::OrderMatched { .. }
            | LogEvent::FactoryBuilt { .. }
            | LogEvent::ProductionCompleted { .. }
            | LogEvent::CaravanDispatched { .. }
            | LogEvent::CaravanArrived { .. }
            | LogEvent::EconomyHarvest { .. }
            | LogEvent::LoanTaken { .. }
            | LogEvent::EventScheduled { .. }
    )
}

fn build_leaderboard(state: &GameState) -> Vec<PlayerDto> {
    leaderboard(state)
        .into_iter()
        .filter_map(|score| {
            let player = state.players.get(&score.player_id)?;
            Some(PlayerDto {
                id: score.player_id.value(),
                name: player.name.clone(),
                role: player.role.display_name().to_string(),
                npc_kind: player.npc_kind.map(|k| k.label().to_string()),
                cash_lira: lira(player.cash),
                pnl_lira: lira(score.total),
                is_npc: player.is_npc,
            })
        })
        .collect()
}

fn build_prices(state: &GameState) -> Vec<PriceCell> {
    let mut cells = Vec::with_capacity(CityId::ALL.len() * ProductKind::ALL.len());
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let baseline = state
                .effective_baseline(city, product)
                .unwrap_or(Money::ZERO);
            let last = state
                .price_history
                .get(&(city, product))
                .and_then(|h| h.last().map(|(_, p)| lira(*p)));
            let avg5 = state.rolling_avg_price(city, product, 5).map(lira);
            let bid = state.best_bid(city, product).map(|(p, _)| lira(p));
            let ask = state.best_ask(city, product).map(|(p, _)| lira(p));
            let (buy_qty, sell_qty) = state
                .order_book
                .get(&(city, product))
                .map(|book| {
                    let bq: u32 = book.iter().filter(|o| o.side.is_buy()).map(|o| o.quantity).sum();
                    let sq: u32 =
                        book.iter().filter(|o| o.side.is_sell()).map(|o| o.quantity).sum();
                    (bq, sq)
                })
                .unwrap_or((0, 0));
            cells.push(PriceCell {
                city: city_slug(city).to_string(),
                city_label: city.display_name().to_string(),
                product: product_slug(product).to_string(),
                product_label: product.display_name().to_string(),
                is_raw: product.is_raw(),
                baseline_lira: lira(baseline),
                last_lira: last,
                avg5_lira: avg5,
                bid_lira: bid,
                ask_lira: ask,
                buy_qty,
                sell_qty,
            });
        }
    }
    cells
}

fn build_factories(state: &GameState) -> Vec<FactoryDto> {
    state
        .factories
        .values()
        .map(|f| FactoryDto {
            id: f.id.value(),
            owner: f.owner.value(),
            city: city_slug(f.city).to_string(),
            product: product_slug(f.product).to_string(),
            pending_units: f.pending_units(),
            idle: f.is_atil(state.current_tick, moneywar_engine::IDLE_FACTORY_THRESHOLD),
        })
        .collect()
}

fn build_caravans(state: &GameState) -> Vec<CaravanDto> {
    state
        .caravans
        .values()
        .map(|c| {
            let cargo_units = match &c.state {
                moneywar_domain::CaravanState::EnRoute { cargo, .. } => cargo.total_units(),
                moneywar_domain::CaravanState::Idle { .. } => 0,
            };
            CaravanDto {
                id: c.id.value(),
                owner: c.owner.value(),
                idle: c.is_idle(),
                current_city: c.state.current_city().map(|city| city_slug(city).to_string()),
                cargo_units,
            }
        })
        .collect()
}

/// `(şehir, ürün)` için fiyat zaman serisini çıkar (grafik endpoint'i).
#[must_use]
pub fn build_series(state: &GameState, city: CityId, product: ProductKind) -> PriceSeries {
    let points = state
        .price_history
        .get(&(city, product))
        .map(|h| {
            h.iter()
                .map(|(t, p)| PricePoint { tick: t.value(), lira: lira(*p) })
                .collect()
        })
        .unwrap_or_default();
    PriceSeries {
        city: city_slug(city).to_string(),
        product: product_slug(product).to_string(),
        points,
    }
}

/// `slug` → `CityId` (series endpoint query parse).
#[must_use]
pub fn parse_city(slug: &str) -> Option<CityId> {
    CityId::ALL.into_iter().find(|c| city_slug(*c) == slug)
}

/// `slug` → `ProductKind` (series endpoint query parse).
#[must_use]
pub fn parse_product(slug: &str) -> Option<ProductKind> {
    ProductKind::ALL.into_iter().find(|p| product_slug(*p) == slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lira_converts_cents_to_lira() {
        assert_eq!(lira(Money::from_cents(12_345)), 123.45);
        assert_eq!(lira(Money::ZERO), 0.0);
        assert_eq!(lira(Money::from_cents(-500)), -5.0);
    }

    #[test]
    fn every_city_slug_round_trips_through_parse() {
        for city in CityId::ALL {
            let slug = city_slug(city);
            assert_eq!(
                parse_city(slug),
                Some(city),
                "city slug '{slug}' parse'a geri dönmedi",
            );
        }
    }

    #[test]
    fn every_product_slug_round_trips_through_parse() {
        for product in ProductKind::ALL {
            let slug = product_slug(product);
            assert_eq!(
                parse_product(slug),
                Some(product),
                "product slug '{slug}' parse'a geri dönmedi",
            );
        }
    }

    #[test]
    fn all_slugs_are_unique() {
        let city_slugs: Vec<&str> = CityId::ALL.into_iter().map(city_slug).collect();
        let mut deduped = city_slugs.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(city_slugs.len(), deduped.len(), "çakışan şehir slug'ı var");

        let product_slugs: Vec<&str> = ProductKind::ALL.into_iter().map(product_slug).collect();
        let mut p_deduped = product_slugs.clone();
        p_deduped.sort_unstable();
        p_deduped.dedup();
        assert_eq!(product_slugs.len(), p_deduped.len(), "çakışan ürün slug'ı var");
    }

    #[test]
    fn parse_rejects_unknown_slugs() {
        assert_eq!(parse_city("atlantis"), None);
        assert_eq!(parse_city(""), None);
        assert_eq!(parse_product("altin"), None);
        assert_eq!(parse_product("ISTANBUL"), None); // büyük/küçük harfe duyarlı
    }

    #[test]
    fn feed_worthy_filters_noise_events() {
        // Gürültü olayı (FactoryIdle) feed'e girmemeli.
        assert!(!is_feed_worthy(&LogEvent::FactoryIdle {
            factory_id: moneywar_domain::FactoryId::new(1),
            city: CityId::Istanbul,
            reason: "shortage".to_string(),
        }));
    }
}

fn name_of(state: &GameState, id: PlayerId) -> String {
    state
        .players
        .get(&id)
        .map_or_else(|| format!("#{}", id.value()), |p| p.name.clone())
}

/// Olayın aktörünü (varsa) isimle döndürür, yoksa "sistem".
fn actor_name(state: &GameState, entry: &LogEntry) -> String {
    entry
        .actor
        .map_or_else(|| "sistem".to_string(), |id| name_of(state, id))
}

fn is_anonymous_buyer(state: &GameState, id: PlayerId) -> bool {
    // Alıcı/Spekülatör/Banka → anonim (sadece satıcı gösterilir).
    // p.has_npc_kind metodu PartialEq ile karşılaştırır.
    state.players.get(&id).map_or(true, |p| {
        p.has_npc_kind(NpcKind::Alici)
            || p.has_npc_kind(NpcKind::Spekulator)
            || p.has_npc_kind(NpcKind::Banka)
    })
}

fn side_label(side: OrderSide) -> &'static str {
    if side.is_buy() { "AL" } else { "SAT" }
}


/// `LogEntry` → okunabilir feed satırı. Yüksek-sinyalli olaylar özel
/// formatlanır, kalanlar `Debug` etiket + jenerik özet alır.
fn build_event(state: &GameState, entry: &LogEntry) -> EventDto {
    let tick = entry.tick.value();
    let (kind, summary) = match &entry.event {
        LogEvent::OrderMatched { city, product, buyer, seller, quantity, price, .. } => {
            let anon = is_anonymous_buyer(state, *buyer);
            let s = if anon {
                format!(
                    "{} · {}× {} @ {}₺ ({})",
                    name_of(state, *seller), quantity,
                    product.display_name(), lira(*price), city.display_name(),
                )
            } else {
                format!(
                    "{} → {} · {}× {} @ {}₺ ({})",
                    name_of(state, *buyer), name_of(state, *seller), quantity,
                    product.display_name(), lira(*price), city.display_name(),
                )
            };
            ("match", s)
        }
        LogEvent::FactoryBuilt { owner, city, product, .. } => (
            "factory_built",
            format!(
                "{} fabrika kurdu · {} / {}",
                name_of(state, *owner),
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::FactoryIdle { .. } => ("factory_idle", "Fabrika atıl kaldı".to_string()),
        LogEvent::ProductionCompleted { city, product, units, .. } => (
            "production",
            format!(
                "{} üretildi · {}× {} ({})",
                actor_name(state, entry),
                units,
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::CaravanDispatched { from, to, cargo_total, .. } => (
            "caravan",
            format!(
                "{} kervan yolladı · {}× {}→{}",
                actor_name(state, entry),
                cargo_total,
                from.display_name(),
                to.display_name(),
            ),
        ),
        LogEvent::CaravanArrived { city, cargo_total, .. } => (
            "caravan",
            format!(
                "{} kervanı ulaştı · {}× ({})",
                actor_name(state, entry),
                cargo_total,
                city.display_name(),
            ),
        ),
        LogEvent::OrderExpired { player, side, product, leftover_qty, city, .. } => (
            "expired",
            format!(
                "{} emri doldu · {} {}× {} ({})",
                name_of(state, *player),
                side_label(*side),
                leftover_qty,
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::FactoryUpgraded { city, product, owner, new_level, .. } => (
            "factory_upgraded",
            format!(
                "{} fabrika yükseltti Lv{} · {} / {}",
                name_of(state, *owner),
                new_level,
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::FactoryDemolished { city, product, owner, .. } => (
            "factory_demolished",
            format!(
                "{} fabrika kapattı · {} / {}",
                name_of(state, *owner),
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::EconomyHarvest { .. } => ("harvest", "Hasat — pazara ham arz".to_string()),
        LogEvent::LoanTaken { borrower, principal, .. } => (
            "loan",
            format!("{} kredi aldı · {}₺", name_of(state, *borrower), lira(*principal)),
        ),
        LogEvent::EventScheduled { .. } => ("news", "Piyasa olayı planlandı".to_string()),
        other => ("other", format!("{other:?}")),
    };
    let (city, product, qty, price_lira, buyer_id, seller_id) = event_bucket(&entry.event);
    EventDto {
        tick,
        kind: kind.to_string(),
        summary,
        city,
        product,
        qty,
        price_lira,
        buyer_id,
        seller_id,
    }
}

/// Olaya bağlı bucket + taraf bilgisi.
fn event_bucket(
    event: &LogEvent,
) -> (Option<String>, Option<String>, Option<u32>, Option<f64>, Option<u64>, Option<u64>) {
    match event {
        LogEvent::OrderMatched { city, product, quantity, price, buyer, seller, .. } => (
            Some(city_slug(*city).to_string()),
            Some(product_slug(*product).to_string()),
            Some(*quantity),
            Some(lira(*price)),
            Some(buyer.value()),
            Some(seller.value()),
        ),
        LogEvent::ProductionCompleted { city, product, units, .. } => (
            Some(city_slug(*city).to_string()),
            Some(product_slug(*product).to_string()),
            Some(*units),
            None, None, None,
        ),
        LogEvent::OrderExpired { city, product, .. } => (
            Some(city_slug(*city).to_string()),
            Some(product_slug(*product).to_string()),
            None, None, None, None,
        ),
        _ => (None, None, None, None, None, None),
    }
}
