//! Web DTO katmanı — `GameState` → JSON-serileştirilebilir düz struct'lar.
//!
//! Motorun iç temsili `BTreeMap<(CityId, ProductKind), V>` tuple key kullanır;
//! bu JSON object key'ine çevrilemez. Burada her tuple-keyed harita düz bir
//! `Vec<Cell>` dizisine açılır. Para birimi frontend kolaylığı için lira
//! (`f64`, cents/100) olarak gönderilir.

use moneywar_domain::{CityId, GameState, Money, NpcKind, OrderSide, PlayerId, ProductKind};
use moneywar_engine::{LogEntry, LogEvent, TickReport, leaderboard};
use moneywar_npc::BrainPool;
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
    pub private_farms: Vec<PrivateFarmDto>,
    pub relations: Vec<RelationDto>,
    pub recent_events: Vec<EventDto>,
    /// Haritanın entrika katmanı: aktif tekeller, savaşlar, tedarik boğmaları.
    pub intrigue: IntrigueDto,
    /// Ekonominin tek bakışta sağlığı — servet eşitsizliği, para arzı, istihdam.
    pub economy: EconomyDto,
    /// Rol bazlı toplam. Sıralama tablosu yalnız şirketleri (Sanayici)
    /// gösterdiği için izleyici ekonominin gerçek dağılımını göremiyordu:
    /// ölçümde en çok kazanan rol Çiftçi ve tabloda hiç görünmüyor.
    pub roles: Vec<RoleSummaryDto>,
}

/// Ekonominin özeti — hero sayılar ve sağlık göstergeleri.
#[derive(Debug, Clone, Serialize)]
pub struct EconomyDto {
    /// Servet Gini katsayısı (0 = tam eşit, 1 = tek elde).
    pub wealth_gini: f64,
    /// Dolaşımdaki toplam nakit.
    pub money_supply_lira: f64,
    /// Çalışan / dünya işgücü havuzu.
    pub employed: u32,
    pub labor_pool: u32,
    /// Aktif ve atıl fabrika sayısı.
    pub factories_active: u32,
    pub factories_idle: u32,
}

/// Bir rolün toplu durumu.
#[derive(Debug, Clone, Serialize)]
pub struct RoleSummaryDto {
    pub kind: String,
    pub label: String,
    pub count: u32,
    pub total_pnl_lira: f64,
    pub per_capita_pnl_lira: f64,
}

/// Harita ve panellerin çizdiği entrika durumu (docs/finish-plan.md Faz 3).
/// İzleyicinin "kim neyi tutuyor, kim kiminle savaşıyor" sorusunun cevabı.
#[derive(Debug, Clone, Serialize)]
pub struct IntrigueDto {
    pub monopolies: Vec<MonopolyDto>,
    pub price_wars: Vec<PriceWarDto>,
    pub supply_chokes: Vec<SupplyChokeDto>,
}

/// Bir pazarı elinde tutan firma — haritada taç.
#[derive(Debug, Clone, Serialize)]
pub struct MonopolyDto {
    pub city: String,
    pub product: String,
    pub firm_id: u64,
    pub firm_name: String,
    /// Manşete çıkmış saltanat mı (çekişmeli pazar), yoksa tek üretici mi?
    pub announced: bool,
}

/// Süregelen fiyat savaşı — haritada iki firma arasında çatışma.
#[derive(Debug, Clone, Serialize)]
pub struct PriceWarDto {
    pub city: String,
    pub product: String,
    pub attacker_id: u64,
    pub attacker_name: String,
    pub target_id: u64,
    pub target_name: String,
    pub since_tick: u32,
}

/// Süregelen tedarik boğma — kimin fabrikası kimin yüzünden aç.
#[derive(Debug, Clone, Serialize)]
pub struct SupplyChokeDto {
    pub city: String,
    pub product: String,
    pub choker_id: u64,
    pub choker_name: String,
    pub victim_id: u64,
    pub victim_name: String,
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
    /// Ajan hedef durumu: "Expand" | "Corner:istanbul:kumas" | "`PriceWar`:…" | "Consolidate" | "Retreat" | null
    pub goal: Option<String>,
    /// Kişilik trait vektörü [0,1] — sadece NPC'ler için.
    pub traits: Option<BrainTraitsDto>,
    /// Sahip olunan fabrika sayısı.
    pub factory_count: u32,
    /// Sahip olunan özel çiftlik sayısı. Tarla artık işçi çalıştırıyor ve
    /// sayısı sınırsız; kaç tane kurulduğu firmanın stratejisini anlatan
    /// bir bilgi — sıralamada görünmesi gerekiyordu.
    pub farm_count: u32,
    /// Fabrika + tarla toplam istihdamı.
    pub employees: u32,
}

/// NPC ajan kişilik trait özeti.
#[derive(Debug, Clone, Serialize)]
pub struct BrainTraitsDto {
    pub aggression: f64,
    pub patience: f64,
    pub risk: f64,
    pub greed: f64,
    /// Sezon boyunca `PnL` trendi: 0=kaybediyor, 0.5=sabit, 1=kazanıyor.
    pub pnl_trend: f64,
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
    /// Fabrika seviyesi (1-3) — batch boyutunu ve hızını belirler.
    pub level: u8,
    /// Çalışan sayısı ve tam kadro. Üretim `employees/required` oranıyla
    /// ölçeklenir; kadrosuz fabrika üretmez.
    pub employees: u32,
    pub required_employees: u32,
}

/// İki oyuncu arasındaki ilişki/güven özeti.
#[derive(Debug, Clone, Serialize)]
pub struct RelationDto {
    pub player_a: u64,
    pub player_b: u64,
    pub trade_count: u32,
    pub total_units: u64,
    /// [0,1] normalize güven skoru.
    pub trust_score: f64,
}

/// Özel çiftlik durum kartı.
#[derive(Debug, Clone, Serialize)]
pub struct PrivateFarmDto {
    pub id: u64,
    pub owner: u64,
    pub city: String,
    pub product: String,
    pub level: u8,
    pub output_per_tick: u32,
    /// Tarladaki ırgat sayısı ve seviyenin istediği kadro.
    pub employees: u32,
    pub required_employees: u32,
}

/// Kervan durum kartı.
#[derive(Debug, Clone, Serialize)]
pub struct CaravanDto {
    pub id: u64,
    pub owner: u64,
    pub idle: bool,
    pub current_city: Option<String>,
    pub cargo_units: u64,
    /// Yoldaysa rotanın uçları — harita kervanı bu iki şehir arasında çizer.
    pub from_city: Option<String>,
    pub to_city: Option<String>,
    /// Yolculuğun tamamlanma oranı [0,1]. Harita noktayı bu orana koyar.
    pub progress: Option<f64>,
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

pub(crate) fn lira(m: Money) -> f64 {
    m.as_cents() as f64 / 100.0
}

pub(crate) fn city_slug(c: CityId) -> &'static str {
    match c {
        CityId::Istanbul => "istanbul",
        CityId::Ankara => "ankara",
        CityId::Izmir => "izmir",
        CityId::Bursa => "bursa",
        CityId::Konya => "konya",
    }
}

pub(crate) fn product_slug(p: ProductKind) -> &'static str {
    match p {
        ProductKind::Pamuk => "pamuk",
        ProductKind::Bugday => "bugday",
        ProductKind::Zeytin => "zeytin",
        ProductKind::Boya => "boya",
        ProductKind::Uzum => "uzum",
        ProductKind::Kumas => "kumas",
        ProductKind::Un => "un",
        ProductKind::Zeytinyagi => "zeytinyagi",
        ProductKind::Sarap => "sarap",
        ProductKind::Elbise => "elbise",
        ProductKind::Ekmek => "ekmek",
        ProductKind::Ziyafet => "ziyafet",
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
    brains: &BrainPool,
) -> Snapshot {
    Snapshot {
        season,
        tick: state.current_tick.value(),
        season_ticks,
        seconds_per_tick,
        leaderboard: build_leaderboard(state, brains),
        prices: build_prices(state),
        factories: build_factories(state),
        caravans: build_caravans(state),
        private_farms: build_private_farms(state),
        relations: build_relations(state),
        recent_events: build_feed(state, report),
        intrigue: build_intrigue(state),
        economy: build_economy(state),
        roles: build_roles(state),
    }
}

/// `state.intrigue`'i frontend'in çizebileceği isimli listelere çevir.
fn build_intrigue(state: &GameState) -> IntrigueDto {
    let name = |id: PlayerId| -> String {
        state
            .players
            .get(&id)
            .map_or_else(|| format!("#{}", id.value()), |p| p.name.clone())
    };
    let monopolies = state
        .intrigue
        .monopolist
        .iter()
        .map(|((city, product), firm)| MonopolyDto {
            city: city_slug(*city).to_string(),
            product: product_slug(*product).to_string(),
            firm_id: firm.value(),
            firm_name: name(*firm),
            announced: state
                .intrigue
                .announced_monopolies
                .contains(&(*city, *product)),
        })
        .collect();
    let price_wars = state
        .intrigue
        .price_wars
        .iter()
        .map(|((attacker, target, city, product), track)| PriceWarDto {
            city: city_slug(*city).to_string(),
            product: product_slug(*product).to_string(),
            attacker_id: attacker.value(),
            attacker_name: name(*attacker),
            target_id: target.value(),
            target_name: name(*target),
            since_tick: track.declared_at.value(),
        })
        .collect();
    let supply_chokes = state
        .intrigue
        .active_chokes
        .iter()
        .map(|(choker, victim, city, product)| SupplyChokeDto {
            city: city_slug(*city).to_string(),
            product: product_slug(*product).to_string(),
            choker_id: choker.value(),
            choker_name: name(*choker),
            victim_id: victim.value(),
            victim_name: name(*victim),
        })
        .collect();
    IntrigueDto {
        monopolies,
        price_wars,
        supply_chokes,
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
    // Anlatı olayları her zaman feed'e girer — izleyicinin takip ettiği akış.
    if moneywar_engine::is_story_event(event) {
        return true;
    }
    matches!(
        event,
        LogEvent::OrderMatched { .. }
            | LogEvent::PrivateFarmBuilt { .. }
            | LogEvent::FactoryBuilt { .. }
            | LogEvent::ProductionCompleted { .. }
            | LogEvent::CaravanDispatched { .. }
            | LogEvent::CaravanArrived { .. }
            | LogEvent::EconomyHarvest { .. }
            | LogEvent::LoanTaken { .. }
            | LogEvent::EventScheduled { .. }
    )
}

fn build_leaderboard(state: &GameState, brains: &BrainPool) -> Vec<PlayerDto> {
    leaderboard(state)
        .into_iter()
        .filter_map(|score| {
            let player = state.players.get(&score.player_id)?;
            // Burası ham sıralamadır — **filtreleme arayüzün işi**.
            //
            // Eskiden Sanayici dışındaki herkes burada eleniyordu ("sıralama
            // şirketler tablosudur"). Sonuç: izleyici on satırlık, hepsi aynı
            // rozetli bir tablo görüyordu ve ekonominin gerçek dağılımını —
            // ölçümde en çok kazanan rolün Çiftçi, Sanayici'nin ekside
            // olduğunu — hiç göremiyordu. Şirket vurgusu duruyor, ama artık
            // sunum katmanında bir filtre olarak; veri tam geliyor.
            let brain = brains.get(score.player_id);
            let goal = brain.map(|b| b.goal_label().to_owned());
            let traits = brain.map(|b| BrainTraitsDto {
                aggression: b.traits.aggression,
                patience: b.traits.patience,
                risk: b.traits.risk,
                greed: b.traits.greed,
                pnl_trend: b.pnl_trend,
            });
            Some(PlayerDto {
                id: score.player_id.value(),
                name: player.name.clone(),
                role: player.role.display_name().to_string(),
                npc_kind: player.npc_kind.map(|k| k.label().to_string()),
                cash_lira: lira(player.cash),
                pnl_lira: lira(score.total),
                is_npc: player.is_npc,
                goal,
                traits,
                factory_count: u32::try_from(
                    state
                        .factories
                        .values()
                        .filter(|f| f.owner == score.player_id)
                        .count(),
                )
                .unwrap_or(0),
                farm_count: u32::try_from(
                    state
                        .private_farms
                        .values()
                        .filter(|f| f.owner == score.player_id)
                        .count(),
                )
                .unwrap_or(0),
                employees: state
                    .factories
                    .values()
                    .filter(|f| f.owner == score.player_id)
                    .map(|f| f.employees)
                    .sum::<u32>()
                    + state
                        .private_farms
                        .values()
                        .filter(|f| f.owner == score.player_id)
                        .map(|f| f.employees)
                        .sum::<u32>(),
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
            let (buy_qty, sell_qty) =
                state
                    .order_book
                    .get(&(city, product))
                    .map_or((0, 0), |book| {
                        let bq: u32 = book
                            .iter()
                            .filter(|o| o.side.is_buy())
                            .map(|o| o.quantity)
                            .sum();
                        let sq: u32 = book
                            .iter()
                            .filter(|o| o.side.is_sell())
                            .map(|o| o.quantity)
                            .sum();
                        (bq, sq)
                    });
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

/// Ekonomi özeti — snapshot'ın hero göstergeleri.
fn build_economy(state: &GameState) -> EconomyDto {
    let scores = leaderboard(state);
    let wealth: Vec<f64> = scores.iter().map(|s| s.total.as_cents() as f64).collect();
    let idle_threshold = moneywar_engine::IDLE_FACTORY_THRESHOLD;
    let (mut active, mut idle) = (0u32, 0u32);
    for f in state.factories.values() {
        if f.is_atil(state.current_tick, idle_threshold) {
            idle += 1;
        } else {
            active += 1;
        }
    }
    EconomyDto {
        wealth_gini: gini(&wealth),
        money_supply_lira: lira(moneywar_domain::Money::from_cents(
            state.players.values().map(|p| p.cash.as_cents()).sum(),
        )),
        // Tarla ırgadı da aynı havuzdan çıkıyor; saymazsak "havuz doluluğu"
        // yanlış görünür.
        employed: state.factories.values().map(|f| f.employees).sum::<u32>()
            + state.private_farms.values().map(|f| f.employees).sum::<u32>(),
        labor_pool: moneywar_domain::balance::labor_pool_at(state.current_tick.value()),
        factories_active: active,
        factories_idle: idle,
    }
}

/// Gini katsayısı (0 = tam eşit, 1 = tek elde). Negatif değerler kaydırılır.
fn gini(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let shift = if min < 0.0 { -min } else { 0.0 };
    let mut xs: Vec<f64> = values.iter().map(|v| v + shift).collect();
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

/// Rol bazlı PnL toplamı — ekonominin gerçek dağılımı.
fn build_roles(state: &GameState) -> Vec<RoleSummaryDto> {
    use std::collections::BTreeMap;
    let mut by: BTreeMap<moneywar_domain::NpcKind, (u32, f64)> = BTreeMap::new();
    for score in leaderboard(state) {
        if let Some(kind) = state.players.get(&score.player_id).and_then(|p| p.npc_kind) {
            let e = by.entry(kind).or_insert((0, 0.0));
            e.0 += 1;
            e.1 += lira(score.total);
        }
    }
    let mut out: Vec<RoleSummaryDto> = by
        .into_iter()
        .map(|(kind, (count, total))| RoleSummaryDto {
            kind: format!("{kind:?}").to_lowercase(),
            label: kind.label().to_string(),
            count,
            total_pnl_lira: total,
            per_capita_pnl_lira: if count == 0 {
                0.0
            } else {
                total / f64::from(count)
            },
        })
        .collect();
    out.sort_by(|a, b| {
        b.per_capita_pnl_lira
            .partial_cmp(&a.per_capita_pnl_lira)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
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
            level: f.level,
            employees: f.employees,
            required_employees: f.required_employees(),
        })
        .collect()
}

fn build_relations(state: &GameState) -> Vec<RelationDto> {
    state
        .relationships
        .iter()
        .filter(|(_, r)| r.trade_count > 0)
        .map(|((a, b), r)| RelationDto {
            player_a: a.value(),
            player_b: b.value(),
            trade_count: r.trade_count,
            total_units: r.total_units,
            trust_score: r.trust_score(),
        })
        .collect()
}

fn build_private_farms(state: &GameState) -> Vec<PrivateFarmDto> {
    state
        .private_farms
        .values()
        .map(|f| PrivateFarmDto {
            id: f.id.value(),
            owner: f.owner.value(),
            city: city_slug(f.city).to_string(),
            product: product_slug(f.product).to_string(),
            level: f.level,
            output_per_tick: f.output_per_tick(),
            employees: f.employees,
            required_employees: f.required_employees(),
        })
        .collect()
}

fn build_caravans(state: &GameState) -> Vec<CaravanDto> {
    state
        .caravans
        .values()
        .map(|c| {
            let (cargo_units, from_city, to_city, progress) = match &c.state {
                moneywar_domain::CaravanState::EnRoute {
                    cargo,
                    from,
                    to,
                    arrival_tick,
                } => {
                    // Yolculuk süresi rotanın sabit mesafesinden gelir; kalan
                    // tick'ten ilerleme oranını türetiyoruz.
                    let total = f64::from(from.distance_to(*to)).max(1.0);
                    let remaining = f64::from(
                        arrival_tick
                            .value()
                            .saturating_sub(state.current_tick.value()),
                    );
                    let progress = ((total - remaining) / total).clamp(0.0, 1.0);
                    (
                        cargo.total_units(),
                        Some(city_slug(*from).to_string()),
                        Some(city_slug(*to).to_string()),
                        Some(progress),
                    )
                }
                moneywar_domain::CaravanState::Idle { .. } => (0, None, None, None),
            };
            CaravanDto {
                id: c.id.value(),
                owner: c.owner.value(),
                idle: c.is_idle(),
                current_city: c
                    .state
                    .current_city()
                    .map(|city| city_slug(city).to_string()),
                cargo_units,
                from_city,
                to_city,
                progress,
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
                .map(|(t, p)| PricePoint {
                    tick: t.value(),
                    lira: lira(*p),
                })
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
    ProductKind::ALL
        .into_iter()
        .find(|p| product_slug(*p) == slug)
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
    state.players.get(&id).is_none_or(|p| {
        p.has_npc_kind(NpcKind::Alici)
            || p.has_npc_kind(NpcKind::Spekulator)
            || p.has_npc_kind(NpcKind::Banka)
    })
}

fn side_label(side: OrderSide) -> &'static str {
    if side.is_buy() { "AL" } else { "SAT" }
}

/// Anlatı olayının frontend etiketi — ikon ve renk seçimi buna bakar.
const fn story_kind(event: &LogEvent) -> &'static str {
    match event {
        LogEvent::MonopolyFormed { .. } => "monopoly_formed",
        LogEvent::MonopolyBroken { .. } => "monopoly_broken",
        LogEvent::UndercutCampaign { .. } => "undercut",
        LogEvent::PriceWarDeclared { .. } => "price_war",
        LogEvent::PriceWarWon { .. } => "price_war_won",
        LogEvent::FirmBankrupt { .. } => "bankrupt",
        LogEvent::GrudgeFormed { .. } => "grudge",
        LogEvent::SupplyChoke { .. } => "supply_choke",
        LogEvent::CartelFormed { .. } => "cartel",
        LogEvent::CartelBetrayed { .. } => "cartel_betrayed",
        _ => "other",
    }
}

/// `LogEntry` → okunabilir feed satırı. Yüksek-sinyalli olaylar özel
/// formatlanır, kalanlar `Debug` etiket + jenerik özet alır.
fn build_event(state: &GameState, entry: &LogEntry) -> EventDto {
    let tick = entry.tick.value();
    let (kind, summary) = match &entry.event {
        LogEvent::OrderMatched {
            city,
            product,
            buyer,
            seller,
            quantity,
            price,
            ..
        } => {
            let anon = is_anonymous_buyer(state, *buyer);
            let s = if anon {
                format!(
                    "{} · {}× {} @ {}₺ ({})",
                    name_of(state, *seller),
                    quantity,
                    product.display_name(),
                    lira(*price),
                    city.display_name(),
                )
            } else {
                format!(
                    "{} → {} · {}× {} @ {}₺ ({})",
                    name_of(state, *buyer),
                    name_of(state, *seller),
                    quantity,
                    product.display_name(),
                    lira(*price),
                    city.display_name(),
                )
            };
            ("match", s)
        }
        LogEvent::FactoryBuilt {
            owner,
            city,
            product,
            ..
        } => (
            "factory_built",
            format!(
                "{} fabrika kurdu · {} / {}",
                name_of(state, *owner),
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::FactoryIdle { .. } => ("factory_idle", "Fabrika atıl kaldı".to_string()),
        LogEvent::ProductionCompleted {
            city,
            product,
            units,
            ..
        } => (
            "production",
            format!(
                "{} üretildi · {}× {} ({})",
                actor_name(state, entry),
                units,
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::CaravanDispatched {
            from,
            to,
            cargo_total,
            ..
        } => (
            "caravan",
            format!(
                "{} kervan yolladı · {}× {}→{}",
                actor_name(state, entry),
                cargo_total,
                from.display_name(),
                to.display_name(),
            ),
        ),
        LogEvent::CaravanArrived {
            city, cargo_total, ..
        } => (
            "caravan",
            format!(
                "{} kervanı ulaştı · {}× ({})",
                actor_name(state, entry),
                cargo_total,
                city.display_name(),
            ),
        ),
        LogEvent::OrderExpired {
            player,
            side,
            product,
            leftover_qty,
            city,
            ..
        } => (
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
        LogEvent::PrivateFarmBuilt {
            owner,
            city,
            product,
            ..
        } => (
            "private_farm",
            format!(
                "{} özel çiftlik kurdu · {} ({})",
                name_of(state, *owner),
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::FactoryUpgraded {
            city,
            product,
            owner,
            new_level,
            ..
        } => (
            "factory_upgraded",
            format!(
                "{} fabrika yükseltti Lv{} · {} / {}",
                name_of(state, *owner),
                new_level,
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::FactoryDemolished {
            city,
            product,
            owner,
            ..
        } => (
            "factory_demolished",
            format!(
                "{} fabrika kapattı · {} / {}",
                name_of(state, *owner),
                product.display_name(),
                city.display_name(),
            ),
        ),
        LogEvent::EconomyHarvest { .. } => ("harvest", "Hasat — pazara ham arz".to_string()),
        LogEvent::LoanTaken {
            borrower,
            principal,
            ..
        } => (
            "loan",
            format!(
                "{} kredi aldı · {}₺",
                name_of(state, *borrower),
                lira(*principal)
            ),
        ),
        LogEvent::EventScheduled { .. } => ("news", "Piyasa olayı planlandı".to_string()),
        other => match moneywar_engine::story_headline(state, other) {
            // Anlatı olayı: metin motordaki tek kaynaktan, etiket türüne özel
            // (frontend bunlara göre ikon/renk seçer).
            Some(headline) => (story_kind(other), headline),
            None => ("other", format!("{other:?}")),
        },
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

/// Olayın bağlı olduğu pazar ve taraflar:
/// `(şehir, ürün, miktar, fiyat, alıcı, satıcı)`. Olay bu bilgileri
/// taşımıyorsa ilgili alan `None`.
type EventBucket = (
    Option<String>,
    Option<String>,
    Option<u32>,
    Option<f64>,
    Option<u64>,
    Option<u64>,
);

/// Olaya bağlı bucket + taraf bilgisi.
fn event_bucket(event: &LogEvent) -> EventBucket {
    match event {
        LogEvent::OrderMatched {
            city,
            product,
            quantity,
            price,
            buyer,
            seller,
            ..
        } => (
            Some(city_slug(*city).to_string()),
            Some(product_slug(*product).to_string()),
            Some(*quantity),
            Some(lira(*price)),
            Some(buyer.value()),
            Some(seller.value()),
        ),
        LogEvent::ProductionCompleted {
            city,
            product,
            units,
            ..
        } => (
            Some(city_slug(*city).to_string()),
            Some(product_slug(*product).to_string()),
            Some(*units),
            None,
            None,
            None,
        ),
        LogEvent::OrderExpired { city, product, .. } => (
            Some(city_slug(*city).to_string()),
            Some(product_slug(*product).to_string()),
            None,
            None,
            None,
            None,
        ),
        // Anlatı olayları: bucket'ı doldur ki harita olaya tıklayınca
        // doğru şehre odaklanabilsin.
        LogEvent::MonopolyFormed { city, product, .. }
        | LogEvent::MonopolyBroken { city, product, .. }
        | LogEvent::UndercutCampaign { city, product, .. }
        | LogEvent::PriceWarDeclared { city, product, .. }
        | LogEvent::PriceWarWon { city, product, .. }
        | LogEvent::SupplyChoke { city, product, .. }
        | LogEvent::CartelFormed { city, product, .. }
        | LogEvent::CartelBetrayed { city, product, .. } => (
            Some(city_slug(*city).to_string()),
            Some(product_slug(*product).to_string()),
            None,
            None,
            None,
            None,
        ),
        _ => (None, None, None, None, None, None),
    }
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
        assert_eq!(
            product_slugs.len(),
            p_deduped.len(),
            "çakışan ürün slug'ı var"
        );
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
