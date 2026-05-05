//! Sanayici rol davranışı — fabrika kuran üretici.
//!
//! Sanayici 3 tür aksiyon yapar:
//! 1. **Fabrika kur** (cash varsa, fab sayısı az ise) — şehir × mamul seçer
//! 2. **Ham madde AL** — production için raw input (her şehir × ham mal)
//! 3. **Mamul SAT** — fabrika çıktısı stoktan satar
//!
//! Production zinciri Pamuk→Kumas, Buğday→Un, Zeytin→Zeytinyağı (otomatik
//! engine `step_factory` ile). Sanayici sadece input/output pazarlamasını
//! yönetir.
//!
//! # `Weights` mantığı (`personality.rs`'te)
//!
//! - `cash +0.4` — cash varsa hareket (BUY raw / Build)
//! - `urgency +0.3` — sezon ilerledikçe agresifleş
//! - `price_rel_avg +0.2` — fiyat fırsatlarını yakala
//! - `arbitrage +0.3` — şehirler arası fark
//! - `competition -0.2` — rakip baskı varsa bekle

use moneywar_domain::{
    CityId, GameState, Money, OrderSide, Player, ProductKind,
    balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;

/// Yeni fabrika kurma eşiği — Sanayici en az bu kadar fab istemeli.
const TARGET_FACTORIES: usize = 3;

/// Sanayici'nin bu tick için aday listesi.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();

    // 1) Fabrika kurma: hedef sayıdan azsa + 1 fab kuruluş maliyeti
    //    karşılanabiliyorsa.
    let owned = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .count();
    if owned < TARGET_FACTORIES {
        let next_cost = moneywar_domain::Factory::build_cost(u32::try_from(owned).unwrap_or(0));
        if player.cash >= next_cost {
            // Önce mevcut fabrikaların kapsamadığı (city, mamul) seç.
            if let Some((city, product)) = pick_factory_target(state, player) {
                out.push(ActionCandidate::BuildFactory { city, product });
            }
        }
    }

    // 2) Ham madde AL — base × 0.95 markdown (rekabet alanı korunur, insan
    //    oyuncu %96 yazıp Sanayici'yi geçebilir). Sadece şehir specialty
    //    raw'ı (3 emir/tick) — önceki 9 bucket kitabı kaynatıyordu.
    let bucket_cash = Money::from_cents((player.cash.as_cents() / 6).max(0));
    for city in CityId::ALL {
        let product = city.cheap_raw();
        let baseline = state.effective_baseline(city, product).unwrap_or_else(|| {
            Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA)
                .unwrap_or(Money::ZERO)
        });
        let unit_price = scale_pct(baseline, 95);
        if unit_price.as_cents() <= 0 {
            continue;
        }
        let quantity = affordable_qty(bucket_cash, unit_price, 30);
        if quantity == 0 {
            continue;
        }
        out.push(ActionCandidate::SubmitOrder {
            side: OrderSide::Buy,
            city,
            product,
            quantity,
            unit_price,
        });
    }

    // 3) Mamul SAT — base × 1.05 markup, stoktaki mamul.
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_finished() || qty == 0 {
            continue;
        }
        let quantity = (qty / 2).max(1).min(50);
        let baseline = state
            .effective_baseline(city, product)
            .unwrap_or_else(|| {
                Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA)
                    .unwrap_or(Money::ZERO)
            });
        let unit_price = scale_pct(baseline, 105);
        if unit_price.as_cents() <= 0 {
            continue;
        }
        out.push(ActionCandidate::SubmitOrder {
            side: OrderSide::Sell,
            city,
            product,
            quantity,
            unit_price,
        });
    }

    out
}

fn scale_pct(price: Money, pct: i64) -> Money {
    Money::from_cents(price.as_cents().saturating_mul(pct) / 100)
}

/// Kuracak fab hedefini seç: Sanayici'nin henüz kuramadığı (city, mamul)
/// çiftlerinden birini deterministik döner. Yoksa `None`.
fn pick_factory_target(state: &GameState, player: &Player) -> Option<(CityId, ProductKind)> {
    let already: std::collections::BTreeSet<(CityId, ProductKind)> = state
        .factories
        .values()
        .filter(|f| f.owner == player.id)
        .map(|f| (f.city, f.product))
        .collect();
    for city in CityId::ALL {
        for product in ProductKind::FINISHED_GOODS {
            if !already.contains(&(city, product)) {
                return Some((city, product));
            }
        }
    }
    None
}


/// Tax-aware satın alma qty.
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

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        Factory, FactoryId, NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn fresh() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn sanayici(cash_lira: i64) -> Player {
        Player::new(
            PlayerId::new(104),
            "san",
            Role::Sanayici,
            Money::from_lira(cash_lira).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Sanayici)
    }

    #[test]
    fn no_factory_emits_build_candidate() {
        let s = fresh();
        let p = sanayici(50_000);
        let cands = enumerate(&s, &p);
        let has_build = cands.iter().any(|c| matches!(c, ActionCandidate::BuildFactory { .. }));
        assert!(has_build, "fab yoksa BuildFactory emit etmeli");
    }

    #[test]
    fn target_factories_reached_no_build() {
        let mut s = fresh();
        let p = sanayici(50_000);
        // 3 fab kurulu say
        for (i, city) in CityId::ALL.iter().enumerate() {
            let fid = FactoryId::new(i as u64 + 1);
            let f = Factory::new(fid, p.id, *city, ProductKind::Kumas).unwrap();
            s.factories.insert(fid, f);
        }
        s.players.insert(p.id, p.clone());
        let cands = enumerate(&s, &p);
        let has_build = cands.iter().any(|c| matches!(c, ActionCandidate::BuildFactory { .. }));
        assert!(!has_build, "hedef sayıda fab varsa Build durur");
    }

    #[test]
    fn rich_sanayici_emits_three_specialty_buy_candidates() {
        // Sanayici sadece şehrin specialty raw'ı için BUY emit eder (3 BUY).
        let s = fresh();
        let p = sanayici(50_000);
        let cands = enumerate(&s, &p);
        let buy_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Buy, product, .. } if product.is_raw()))
            .count();
        assert_eq!(buy_count, 3, "3 şehir × specialty = 3 BUY adayı");
    }

    #[test]
    fn sanayici_buys_only_city_specialty_raw() {
        let s = fresh();
        let p = sanayici(50_000);
        let cands = enumerate(&s, &p);
        for c in &cands {
            if let ActionCandidate::SubmitOrder { side: OrderSide::Buy, city, product, .. } = c {
                assert_eq!(*product, city.cheap_raw(),
                    "Sanayici BUY {city:?}'in specialty'si dışında ürün almamalı");
            }
        }
    }

    #[test]
    fn no_cash_no_buy_candidates() {
        let s = fresh();
        let p = sanayici(0);
        let cands = enumerate(&s, &p);
        let buy_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Buy, .. }))
            .count();
        assert_eq!(buy_count, 0);
    }

    #[test]
    fn finished_stock_yields_sell_candidates() {
        let s = fresh();
        let mut p = sanayici(50_000);
        p.inventory.add(CityId::Istanbul, ProductKind::Kumas, 100).unwrap();
        let cands = enumerate(&s, &p);
        let sell_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, product, .. } if product.is_finished()))
            .count();
        assert!(sell_count >= 1, "mamul stok varsa SELL emit");
    }

    #[test]
    fn raw_stock_does_not_yield_sell() {
        let s = fresh();
        let mut p = sanayici(50_000);
        // Sanayici raw'ı satmaz (sadece mamul SAT).
        p.inventory.add(CityId::Istanbul, ProductKind::Pamuk, 100).unwrap();
        let cands = enumerate(&s, &p);
        let sell_raw = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, product, .. } if product.is_raw()))
            .count();
        assert_eq!(sell_raw, 0);
    }

    #[test]
    fn deterministic_no_rng() {
        let s = fresh();
        let p = sanayici(50_000);
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }
}
