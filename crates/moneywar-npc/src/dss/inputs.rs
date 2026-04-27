//! Aksiyon sinyalleri için ortak yardımcılar — `GameState`'ten utility AI
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_lossless,
    clippy::doc_markdown
)]
//!
//! girdilerini hesaplar.
//!
//! Her helper bir `[0.0, 1.0]` veya `[-1.0, 1.0]` aralığında normalize sinyal
//! döner. NPC'nin kişilik-bağımsız ham bilgisidir.

use moneywar_domain::{CityId, GameState, Money, ProductKind};

/// Bir (şehir, ürün) için fiyat momentum'u — son N tick fiyat trendi.
/// `[-1.0, 1.0]`: pozitif = yükseliyor, negatif = düşüyor.
///
/// Hesap: son 5 tick rolling avg'in **basit slope**'u (lin regr lite). Veri
/// 2'den az ise 0.0.
#[must_use]
pub fn price_momentum(state: &GameState, city: CityId, product: ProductKind) -> f64 {
    let Some(history) = state.price_history.get(&(city, product)) else {
        return 0.0;
    };
    if history.len() < 2 {
        return 0.0;
    }
    let last_n: Vec<f64> = history
        .iter()
        .rev()
        .take(5)
        .map(|(_, p)| p.as_cents() as f64)
        .collect();
    if last_n.len() < 2 {
        return 0.0;
    }
    let first = last_n.last().copied().unwrap_or(0.0);
    let last = last_n.first().copied().unwrap_or(0.0);
    if first.abs() < f64::EPSILON {
        return 0.0;
    }
    let raw = (last - first) / first;
    raw.clamp(-1.0, 1.0)
}

/// Bir ürün için max-min şehir fiyat farkı `[0.0, 1.0]` normalize.
/// 0 = tek şehir veya fark yok, 1 = max fark (~%100 spread).
#[must_use]
pub fn arbitrage_signal(state: &GameState, product: ProductKind) -> f64 {
    let mut prices: Vec<i64> = Vec::new();
    for city in CityId::ALL {
        if let Some(p) = state
            .rolling_avg_price(city, product, 5)
            .or_else(|| state.effective_baseline(city, product))
        {
            prices.push(p.as_cents());
        }
    }
    if prices.len() < 2 {
        return 0.0;
    }
    let min = *prices.iter().min().unwrap_or(&0);
    let max = *prices.iter().max().unwrap_or(&0);
    if min <= 0 {
        return 0.0;
    }
    let spread = ((max - min) as f64) / (min as f64);
    // %100 spread → 1.0; %50 → 0.5
    spread.clamp(0.0, 1.0)
}

/// `(city, product)` aktif şokunun mutlak yüzdesi `[0.0, 1.0]`. Şok yoksa 0.
/// Macro %35 → 0.35; Major %18 → 0.18.
#[must_use]
pub fn event_signal(state: &GameState, city: CityId, product: ProductKind) -> f64 {
    state
        .active_shocks
        .get(&(city, product))
        .map_or(0.0, |s| f64::from(s.multiplier_pct.unsigned_abs()) / 100.0)
        .clamp(0.0, 1.0)
}

/// **Pending event** sinyali — oyuncunun news_inbox'ında gelecek olay var mı?
/// Gold tier abone NPC'ler 2 tick önceden bilir. Bu sinyali pre-positioning
/// için kullanırlar (event_tick gelmeden ham/finished pozisyon al).
///
/// `[0.0, 1.0]` — event ne kadar yakın × ne kadar büyük şiddetli (severity).
/// Hiç pending yok → 0.0.
#[must_use]
pub fn pending_event_signal(
    state: &GameState,
    pid: moneywar_domain::PlayerId,
    city: CityId,
    product: ProductKind,
) -> f64 {
    let Some(inbox) = state.news_inbox.get(&pid) else {
        return 0.0;
    };
    let current = state.current_tick.value();
    let mut max_signal: f64 = 0.0;
    for item in inbox {
        // disclosed_tick geçmemişse henüz görünmüyor
        if current < item.disclosed_tick.value() {
            continue;
        }
        // event_tick gelmediyse "pending"
        if current >= item.event_tick.value() {
            continue;
        }
        // Bu (city, product) için mi?
        let cities = item.event.affected_cities();
        let event_product = item.event.affected_product();
        let matches_city = cities.contains(&city);
        let matches_product = event_product == Some(product) || event_product.is_none();
        if !matches_city || !matches_product {
            continue;
        }
        // Yakınlık: 1 tick kalmış → 1.0, 2 tick → 0.5
        let ticks_until = item.event_tick.value().saturating_sub(current);
        let proximity = 1.0 / f64::from(ticks_until.max(1));
        // Severity etkisi
        let severity = item
            .event
            .severity()
            .map_or(0.5, |s| f64::from(s.nominal_shock_percent()) / 100.0);
        let combined = (proximity * (1.0 + severity)).clamp(0.0, 1.0);
        if combined > max_signal {
            max_signal = combined;
        }
    }
    max_signal
}

/// `current_price / fair_value` oranı — `[0.0, 2.0+]` aralığında.
/// Centered around 1.0 (adil fiyat). `> 1` pahalı, `< 1` ucuz.
#[must_use]
pub fn price_ratio(state: &GameState, city: CityId, product: ProductKind) -> f64 {
    let current = state
        .price_history
        .get(&(city, product))
        .and_then(|v| v.last())
        .map(|(_, p)| *p);
    let baseline = state.effective_baseline(city, product);
    match (current, baseline) {
        (Some(cur), Some(base)) if base.as_cents() > 0 => {
            (cur.as_cents() as f64) / (base.as_cents() as f64)
        }
        _ => 1.0,
    }
}

/// Bir şehirde aynı (city, product) bucket'ında SELL ya da BUY emir baskısı.
/// Sonuç `[0.0, 1.0]` — 200 birim+ → 1.0, 0 → 0.0.
#[must_use]
pub fn competition_signal(state: &GameState, city: CityId, product: ProductKind) -> f64 {
    let total: u32 = state
        .order_book
        .get(&(city, product))
        .map_or(0, |orders| orders.iter().map(|o| o.quantity).sum());
    (f64::from(total) / 200.0).clamp(0.0, 1.0)
}

/// `Money` cents → `f64` lira (utility hesabında ölçek için).
#[must_use]
pub fn money_lira(m: Money) -> f64 {
    (m.as_cents() as f64) / 100.0
}

/// Adaptive difficulty / catch-up: insan oyuncunun NPC ortalamasına göre
/// ne kadar lider. > 1.0 = lider (NPC'ler agresifleşmeli), 0..1 = geride.
///
/// İnsan score / NPC ortalama. Insan yoksa 1.0 (nötr).
#[must_use]
pub fn human_lead_ratio(state: &GameState, human_id: moneywar_domain::PlayerId) -> f64 {
    use crate::dss::inputs::money_lira;
    let human_score = state.players.get(&human_id).map_or(0.0, |p| {
        money_lira(p.cash) + p.inventory.total_units() as f64 * 5.0
    });
    let mut npc_scores: Vec<f64> = state
        .players
        .iter()
        .filter_map(|(id, p)| {
            if *id == human_id || !p.is_npc {
                return None;
            }
            // Sadece rakip NPC'leri (Sanayici/Tüccar)
            match p.npc_kind {
                Some(moneywar_domain::NpcKind::Tuccar | moneywar_domain::NpcKind::Sanayici) => {
                    Some(money_lira(p.cash) + p.inventory.total_units() as f64 * 5.0)
                }
                _ => None,
            }
        })
        .collect();
    if npc_scores.is_empty() {
        return 1.0;
    }
    npc_scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let avg = npc_scores.iter().sum::<f64>() / npc_scores.len() as f64;
    if avg <= 0.0 {
        return 1.0;
    }
    (human_score / avg).clamp(0.0, 5.0)
}

/// Aynı arketipli diğer NPC'lerin son tick'te aktif emir sayısı (cluster
/// signal). Bandwagon davranışı için: aynı arketip aynı (city, product)'a
/// yığılırsa cluster büyür.
#[must_use]
pub fn cluster_signal(
    state: &GameState,
    self_id: moneywar_domain::PlayerId,
    personality: moneywar_domain::Personality,
    city: CityId,
    product: ProductKind,
) -> f64 {
    let same_archetype_ids: Vec<_> = state
        .players
        .iter()
        .filter_map(|(id, p)| {
            if *id == self_id {
                return None;
            }
            if p.personality == Some(personality) {
                Some(*id)
            } else {
                None
            }
        })
        .collect();
    if same_archetype_ids.is_empty() {
        return 0.0;
    }
    let total: u32 = state.order_book.get(&(city, product)).map_or(0, |orders| {
        orders
            .iter()
            .filter(|o| same_archetype_ids.contains(&o.player))
            .map(|o| o.quantity)
            .sum()
    });
    (f64::from(total) / 100.0).clamp(0.0, 1.0)
}
