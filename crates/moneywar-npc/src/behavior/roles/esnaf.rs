//! Esnaf rol davranışı — perakendeci + ham toptancı.
//!
//! Yeni tasarım (gerçek tedarik zinciri):
//! - **Mamul BUY** (toptan): Sanayici'den baseline'da mamul alır → her şehirde
//! - **Mamul SAT** (perakende): stoktaki mamulu Alıcı'ya `base × 1.05` markup
//! - **Ham BUY**: Çiftçi'den specialty raw toptan (`base × 0.95`)
//! - **Ham SAT**: stoktaki raw'u Sanayici'ye markup (`base × 1.05`)
//!
//! Esnaf artık **gerçek perakendeci** rolü oynuyor — Sanayici → Esnaf → Alıcı
//! zincirinin orta halkası. Mamul üretmez (fab yok), sadece dağıtım.
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

    // 1) Ham AL — base × 0.95 markdown (rekabet alanı korunur, insan oyuncu
    //    %96 yazıp Esnaf'ı geçebilir). Sadece şehrin specialty raw'ı için
    //    emit (3 emir/tick). Önceki "9 bucket" hepsini kıyıyordu, artık 3.
    let bucket_cash = bucket_buy_budget(player);
    for city in CityId::ALL {
        let product = city.cheap_raw();
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

    // 2) Mamul AL — Sanayici'den toptan, baseline'da. 9 BUY (3 şehir × 3 mamul).
    //    Esnaf'ın yeni perakendeci rolü: Sanayici → Esnaf → Alıcı zinciri.
    let mamul_bucket_cash =
        Money::from_cents((player.cash.as_cents() / 9).max(0));
    for city in CityId::ALL {
        for product in ProductKind::FINISHED_GOODS {
            let baseline = state
                .effective_baseline(city, product)
                .unwrap_or_else(|| {
                    Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA)
                        .unwrap_or(Money::ZERO)
                });
            // Toptan fiyat = baseline (Sanayici'nin SAT fiyatı). Markdown yok
            // → Esnaf kâr marjını perakende SAT'tan kazanır.
            let unit_price = baseline;
            if unit_price.as_cents() <= 0 {
                continue;
            }
            let quantity = affordable_qty(mamul_bucket_cash, unit_price, 25);
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

    // 3) SAT — stoktaki her şey baseline × 1.05 markup
    //    (raw → Sanayici toptan, mamul → Alıcı perakende).
    for (city, product, qty) in player.inventory.entries() {
        if qty == 0 {
            continue;
        }
        let baseline = state
            .effective_baseline(city, product)
            .unwrap_or_else(|| {
                let lira = if product.is_finished() {
                    moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA
                } else {
                    moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA
                };
                Money::from_lira(lira).unwrap_or(Money::ZERO)
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
    // 3 BUY bucket (3 şehir × specialty raw). Cash bütçesi 3'e böl.
    Money::from_cents((player.cash.as_cents() / 3).max(0))
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
    fn rich_esnaf_emits_three_specialty_buy_candidates() {
        // Esnaf sadece şehrin specialty raw'ı için BUY emit eder (3 şehir
        // × 1 specialty = 3 BUY). Kitabı kaynatmama amaçlı.
        let s = fresh();
        let p = esnaf(50_000);
        let cands = enumerate(&s, &p);
        let buy_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Buy, product, .. } if product.is_raw()))
            .count();
        assert_eq!(buy_count, 3);
    }

    #[test]
    fn esnaf_buys_specialty_raw_per_city() {
        // Esnaf raw BUY sadece şehir specialty (3 emir). Mamul BUY ayrı (9 emir).
        let s = fresh();
        let p = esnaf(50_000);
        let cands = enumerate(&s, &p);
        for c in &cands {
            if let ActionCandidate::SubmitOrder { side: OrderSide::Buy, city, product, .. } = c {
                if product.is_raw() {
                    assert_eq!(*product, city.cheap_raw(),
                        "Esnaf raw BUY: {city:?}'in specialty'si dışında ham almamalı");
                }
            }
        }
    }

    #[test]
    fn finished_stock_yields_sell_candidate() {
        // Yeni tasarım: Esnaf perakendeci, mamul stoğunu Alıcı'ya markup'la satar.
        let s = fresh();
        let mut p = esnaf(0);
        p.inventory.add(CityId::Istanbul, ProductKind::Kumas, 100).unwrap();
        let cands = enumerate(&s, &p);
        let sell_finished_count = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { side: OrderSide::Sell, product, .. } if product.is_finished()))
            .count();
        assert_eq!(sell_finished_count, 1, "mamul stoğu varsa SELL emit");
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
    fn raw_buy_price_below_baseline() {
        // Ham BUY hâlâ %95 markdown (toptan kâr için).
        let s = fresh();
        let p = esnaf(50_000);
        let cands = enumerate(&s, &p);
        let raw_baseline = Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA).unwrap();
        for c in &cands {
            if let ActionCandidate::SubmitOrder { side: OrderSide::Buy, product, unit_price, .. } = c {
                if product.is_raw() {
                    assert!(unit_price.as_cents() < raw_baseline.as_cents(),
                        "Esnaf raw BUY < baseline (%95 markdown)");
                }
            }
        }
    }

    #[test]
    fn finished_buy_at_baseline() {
        // Mamul BUY toptan baseline'da (Sanayici'den) — markdown yok, kâr
        // perakende SAT'tan gelir.
        let s = fresh();
        let p = esnaf(50_000);
        let cands = enumerate(&s, &p);
        let finished_baseline = Money::from_lira(moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA).unwrap();
        let mut found = false;
        for c in &cands {
            if let ActionCandidate::SubmitOrder { side: OrderSide::Buy, product, unit_price, .. } = c {
                if product.is_finished() {
                    assert_eq!(*unit_price, finished_baseline,
                        "Esnaf mamul BUY = baseline (toptan)");
                    found = true;
                }
            }
        }
        assert!(found, "Esnaf mamul BUY adayları olmalı");
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
