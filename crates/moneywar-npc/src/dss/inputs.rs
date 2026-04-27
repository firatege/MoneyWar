//! Aksiyon sinyalleri için ortak yardımcılar — `GameState`'ten utility AI
#![allow(clippy::cast_precision_loss, clippy::cast_lossless)]
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
