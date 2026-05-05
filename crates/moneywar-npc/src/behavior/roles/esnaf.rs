//! Esnaf rol davranışı — toptancı, ham mal aracısı.
//!
//! Esnaf'ın iki taraflı işi:
//! - **AL ham**: Çiftçi'den toptan ham mal alımı (`base × 0.95` markdown)
//! - **SAT ham**: Sanayici/Alıcı'ya markup'lı satış (`base × 1.05`)
//!
//! Mamul almaz (Sanayici'nin tekeli); mamul satmaz (üretim yok). Aday üretimi
//! tipini doğal olarak filtreliyor.
//!
//! # `Weights` mantığı (`personality.rs`'te)
//!
//! - `cash +0.5` — cash varsa al (BUY ana sürücü)
//! - `arbitrage +0.3` — şehirler arası fark fırsat
//! - `urgency +0.2` — sezon basıncı
//! - `competition -0.2` — rakip baskı varsa bekle

use moneywar_domain::{
    CityId, GameState, Money, OrderSide, Player, ProductKind,
    balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;

/// Esnaf'ın bu tick için aday listesi.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();

    // 1) Ham AL — base × 0.95 markdown (perakende kâr için).
    let bucket_cash = bucket_buy_budget(player);
    for city in CityId::ALL {
        for product in ProductKind::RAW_MATERIALS {
            let baseline = state
                .effective_baseline(city, product)
                .unwrap_or_else(|| {
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
    }

    // 2) Ham SAT — base × 1.05 markup, stoktaki ham mallar.
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_raw() || qty == 0 {
            continue;
        }
        let baseline = state
            .effective_baseline(city, product)
            .unwrap_or_else(|| {
                Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA)
                    .unwrap_or(Money::ZERO)
            });
        let unit_price = scale_pct(baseline, 105);
        if unit_price.as_cents() <= 0 {
            continue;
        }
        let quantity = (qty / 2).max(1).min(50);
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

fn bucket_buy_budget(player: &Player) -> Money {
    // Cash 9 BUY bucket'a böl (3 şehir × 3 ham).
    Money::from_cents((player.cash.as_cents() / 9).max(0))
}

fn scale_pct(price: Money, pct: i64) -> Money {
    Money::from_cents(price.as_cents().saturating_mul(pct) / 100)
}

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
    use moneywar_domain::{NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId};

    fn fresh() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn esnaf(cash: i64) -> Player {
        Player::new(
            PlayerId::new(109),
            "esnaf",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Esnaf)
    }

    #[test]
    fn rich_esnaf_emits_nine_buy_candidates() {
        let s = fresh();
        let p = esnaf(50_000);
        let cands = enumerate(&s, &p);
        let buy_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Buy, product, .. } if product.is_raw()))
            .count();
        assert_eq!(buy_count, 9);
    }

    #[test]
    fn finished_stock_does_not_yield_candidates() {
        // Esnaf elinde mamul olamaz normalde, ama gelirse SELL etmemeli.
        let s = fresh();
        let mut p = esnaf(0);
        p.inventory.add(CityId::Istanbul, ProductKind::Kumas, 100).unwrap();
        let cands = enumerate(&s, &p);
        assert!(cands.is_empty(), "Esnaf mamul satmaz, cash yok BUY da yok");
    }

    #[test]
    fn raw_stock_yields_sell_candidate() {
        let s = fresh();
        let mut p = esnaf(50_000);
        p.inventory.add(CityId::Ankara, ProductKind::Bugday, 100).unwrap();
        let cands = enumerate(&s, &p);
        let sell_raw_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, product, .. } if product.is_raw()))
            .count();
        assert_eq!(sell_raw_count, 1);
    }

    #[test]
    fn buy_price_below_baseline() {
        let s = fresh();
        let p = esnaf(50_000);
        let cands = enumerate(&s, &p);
        let baseline = Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA).unwrap();
        for c in &cands {
            if let ActionCandidate::SubmitOrder { side: OrderSide::Buy, unit_price, .. } = c {
                assert!(unit_price.as_cents() < baseline.as_cents(),
                    "Esnaf BUY < baseline (%95 markdown)");
            }
        }
    }

    #[test]
    fn deterministic_no_rng() {
        let s = fresh();
        let mut p = esnaf(50_000);
        p.inventory.add(CityId::Izmir, ProductKind::Zeytin, 50).unwrap();
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }
}
