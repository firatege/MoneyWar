//! Alıcı rol davranışı — tüketici, buy-only mamul.
//!
//! Alıcı her CONSUME_PERIOD (5) tick'te mamul stoğunun %50'sini tüketir
//! (Vic3 pop needs). Sürekli alım yapması doğal — yoksa açlık çeker.
//!
//! # Aday üretim kuralı
//!
//! Her `(şehir × mamul)` için bir Buy adayı (3 şehir × 3 mamul = 9 aday):
//! - quantity = `affordable_qty(cash_bucket, price, want=30)` — tax-aware
//! - unit_price = `effective_baseline(city, product)` (clamp etkisi dahil)
//! - skor → orchestrator hesaplar (Alıcı `Weights`'i ile)
//!
//! # Alıcı `Weights` mantığı (`personality.rs`'te)
//!
//! - `cash` +1.0 → cash varsa al (ana sürücü)
//! - `price_rel_avg` -0.5 → ucuzken al
//! - `stock` -0.3 → kendi mamul stoğu varsa azalt iştahı
//! - `momentum` +0.2 → yükseliyor → şimdi al
//! - `urgency` +0.2 → sezon sonu hafif basınç
//! - `competition` -0.2 → rakip baskı varsa bekle

use moneywar_domain::{
    GameState, Money, OrderSide, Player, ProductKind, balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;

/// Alıcı'nın bu tick için olası alım adayları (3 şehir × 3 mamul = 9 max).
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    let bucket_cash = bucket_budget(player);

    for city in moneywar_domain::CityId::ALL {
        for product in ProductKind::FINISHED_GOODS {
            let baseline = state.effective_baseline(city, product).unwrap_or_else(|| {
                Money::from_lira(default_finished_price()).unwrap_or(Money::ZERO)
            });
            if baseline.as_cents() <= 0 {
                continue;
            }
            // Dinamik rezerv fiyatı (Vic3 pop needs urgency).
            // Stoğu boşsa 110% verir, doluysa 100%. Sanayici 105% asking
            // → stok ortadayken (urgency 0.5) 105% Alıcı bid ile eşleşir.
            let unit_price = bid_with_urgency(baseline, player, city, product);
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
    out
}

/// Alıcı cash'inin 9 bucket'a bölünmüş payı (3 şehir × 3 mamul).
/// Her bucket için bağımsız satın alım — hepsi aynı anda taze.
fn bucket_budget(player: &Player) -> Money {
    let cents = player.cash.as_cents() / 9;
    Money::from_cents(cents.max(0))
}

/// Stoğa-bağımlı rezerv fiyat. Vic3 pop needs urgency mantığı:
/// - Stok dolu (≥30 birim): baseline × 1.00 — acelesi yok, tutumlu
/// - Stok orta (15 birim) : baseline × 1.05 — orta urgency, Sanayici karşılar
/// - Stok boş (0 birim)   : baseline × 1.10 — kıtlık, max prim öder
///
/// Bu **rezerv tavan**: Alıcı baseline'ın %110'undan fazlasını ödemez.
/// Sanayici 200₺ asking yazsa hiç eşleşmez, başka Sanayici 105'i kapar →
/// rekabet doğal şekilde fiyat dengeler.
const URGENCY_REFERENCE_STOCK: f64 = 30.0;
const MAX_URGENCY_PREMIUM_PCT: i64 = 10;

fn bid_with_urgency(
    baseline: Money,
    player: &Player,
    city: moneywar_domain::CityId,
    product: ProductKind,
) -> Money {
    let stock = f64::from(player.inventory.get(city, product));
    let urgency = (1.0 - (stock / URGENCY_REFERENCE_STOCK).min(1.0)).clamp(0.0, 1.0);
    let premium_pct = (urgency * MAX_URGENCY_PREMIUM_PCT as f64) as i64;
    let multiplier = 100 + premium_pct;
    Money::from_cents(baseline.as_cents().saturating_mul(multiplier) / 100)
}

/// Tax-aware satın alma miktarı: alıcı `qty × price × (100+TAX)/100 ≤ cash`
/// karşılamalı, yoksa settle reject olur.
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

const fn default_finished_price() -> i64 {
    moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CityId, NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn alici_with_cash(lira: i64) -> (GameState, Player) {
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let p = Player::new(
            PlayerId::new(116),
            "alici",
            Role::Tuccar,
            Money::from_lira(lira).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        (s, p)
    }

    #[test]
    fn rich_alici_emits_nine_buy_candidates() {
        let (s, p) = alici_with_cash(100_000);
        let cands = enumerate(&s, &p);
        // 3 şehir × 3 mamul = 9 aday (baseline > 0 olmalı; sim runner doluyor
        // ama bu test fresh_state kullanıyor → baseline None → fallback fiyat).
        assert_eq!(cands.len(), 9);
        for cand in &cands {
            let ActionCandidate::SubmitOrder {
                side, product, ..
            } = cand else {
                panic!("Alıcı sadece SubmitOrder emit etmeli");
            };
            assert_eq!(*side, OrderSide::Buy);
            assert!(product.is_finished(), "Alıcı sadece mamul AL");
        }
    }

    #[test]
    fn no_cash_yields_no_candidates() {
        let (s, p) = alici_with_cash(0);
        assert!(enumerate(&s, &p).is_empty());
    }

    #[test]
    fn raw_products_skipped_only_finished() {
        let (s, p) = alici_with_cash(100_000);
        let cands = enumerate(&s, &p);
        for cand in &cands {
            let ActionCandidate::SubmitOrder { product, .. } = cand else {
                panic!()
            };
            assert!(!product.is_raw(), "Alıcı ham almaz");
        }
    }

    #[test]
    fn affordable_qty_respects_tax() {
        // 100₺ cash, 10₺ unit price → tax dahil 10.20 → 9 birim alabilir (90.18 ≤ 100, 100.20 > 100).
        let cash = Money::from_lira(100).unwrap();
        let price = Money::from_lira(10).unwrap();
        let qty = affordable_qty(cash, price, 30);
        assert_eq!(qty, 9, "tax (%2) sebebiyle 10 yerine 9");
    }

    #[test]
    fn affordable_qty_capped_at_want() {
        // Bol cash → want sınırı.
        let cash = Money::from_lira(1_000_000).unwrap();
        let price = Money::from_lira(10).unwrap();
        let qty = affordable_qty(cash, price, 30);
        assert_eq!(qty, 30);
    }

    #[test]
    fn deterministic_no_rng_in_enumerate() {
        let (s, p) = alici_with_cash(50_000);
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }

    fn alici_with_stock(stock: u32) -> Player {
        let mut p = Player::new(
            PlayerId::new(116),
            "alici",
            Role::Tuccar,
            Money::from_lira(100_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        if stock > 0 {
            p.inventory
                .add(CityId::Istanbul, ProductKind::Kumas, stock)
                .unwrap();
        }
        p
    }

    #[test]
    fn empty_stock_yields_max_premium_bid() {
        let p = alici_with_stock(0);
        let bid = bid_with_urgency(
            Money::from_lira(36).unwrap(),
            &p,
            CityId::Istanbul,
            ProductKind::Kumas,
        );
        // Stok 0 → urgency 1.0 → 110% × 36 = 39.6 → 3960 cents
        assert_eq!(bid.as_cents(), 36 * 100 * 110 / 100);
    }

    #[test]
    fn full_stock_yields_baseline_bid() {
        let p = alici_with_stock(30);
        let bid = bid_with_urgency(
            Money::from_lira(36).unwrap(),
            &p,
            CityId::Istanbul,
            ProductKind::Kumas,
        );
        assert_eq!(bid, Money::from_lira(36).unwrap());
    }

    #[test]
    fn half_stock_yields_mid_premium() {
        let p = alici_with_stock(15);
        let bid = bid_with_urgency(
            Money::from_lira(36).unwrap(),
            &p,
            CityId::Istanbul,
            ProductKind::Kumas,
        );
        // 15/30 = 0.5 stock → urgency 0.5 → 105% × 36 = 3780 cents
        assert_eq!(bid.as_cents(), 36 * 100 * 105 / 100);
    }

    #[test]
    fn city_product_set_covers_all_finished() {
        use std::collections::BTreeSet;
        let (s, p) = alici_with_cash(100_000);
        let cands = enumerate(&s, &p);
        let pairs: BTreeSet<(CityId, ProductKind)> = cands
            .iter()
            .filter_map(|c| match c {
                ActionCandidate::SubmitOrder { city, product, .. } => Some((*city, *product)),
                _ => None,
            })
            .collect();
        assert_eq!(pairs.len(), 9);
        for city in CityId::ALL {
            for product in ProductKind::FINISHED_GOODS {
                assert!(pairs.contains(&(city, product)));
            }
        }
    }
}
