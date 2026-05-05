//! Tüccar rol davranışı — şehirler arası arbitraj.
//!
//! Her ürün için ucuz ve pahalı şehri bul. Spread > %20 ise ucuz şehirde AL,
//! pahalı şehirde SAT (stok varsa). Synthetic ile aynı kural — şu an
//! `DispatchCaravan` çağrısı yok (basit pazar arbitrajı), ileride mal taşıma
//! eklenince kervan komutları da aday üreticisinden çıkacak.
//!
//! # `Weights` mantığı (`personality.rs`'te)
//!
//! - `arbitrage +0.6`: ana sürücü
//! - `cash +0.3`: cash varsa al
//! - `urgency +0.2`: sezon basıncı
//! - `competition -0.2`: rakip baskı
//! - `momentum +0.1`: trend yönü

use moneywar_domain::{
    Cargo, CaravanState, CityId, GameState, Money, OrderSide, Player, ProductKind,
    balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;

/// Arbitraj eşiği — bu yüzdeden az spread varsa arbitraj kârsız.
/// Faz F tuning: 20 → 15. Demand_for matrisi mamul baseline farkını
/// %25-28 yaratıyor; ham specialty farkı %14-75 (çoğunlukla yeterli).
const ARBITRAGE_SPREAD_PCT: i64 = 15;

#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    let bucket_cash = bucket_buy_budget(player);

    // 0. Multi-caravan filo: hedef 4 kervan (3 şehir + 1 fazlalık).
    //    F5 buff: Sanayici tekeli karşı Tüccar'ı güçlendir. 4 caravan/Tüccar
    //    × 4 NPC = 16 kervan filosu → daha çok şehirler arası dağıtım.
    const TARGET_CARAVANS: usize = 4;
    let owned_caravans = state
        .caravans
        .values()
        .filter(|c| c.owner == player.id)
        .count();
    if owned_caravans < TARGET_CARAVANS {
        let starting_city = CityId::ALL[owned_caravans % CityId::ALL.len()];
        out.push(ActionCandidate::BuyCaravan { starting_city });
    }

    // 0b. Idle caravan + bulunduğu şehirde stok varsa → pahalı şehre dispatch.
    //     Bu Tüccar'ın **şehirler arası mal taşıma** mekaniği. Cargo, kervan
    //     kapasitesi sınırlı, varış N tick sonra (mesafeye göre).
    for caravan in state.caravans.values() {
        if caravan.owner != player.id {
            continue;
        }
        let CaravanState::Idle { location: cur_city } = caravan.state else {
            continue;
        };
        // Tüm pozitif-profit (product, to_city) çiftlerini topla, kâr azalan
        // sırada sırala. Caravan_id ile rotation: 1. caravan en kârlıyı, 2.
        // caravan ikinciyi, vs. → farklı kervanlar farklı yönlere → off-
        // specialty bucket'ların hepsi (Ank-Pamuk dahil) Tüccar SAT'a kavuşur.
        let mut targets: Vec<(ProductKind, CityId, u32, i64)> = Vec::new();
        for product in ProductKind::ALL {
            let stock = player.inventory.get(cur_city, product);
            if stock == 0 {
                continue;
            }
            let from_price = state
                .effective_baseline(cur_city, product)
                .map(|m| m.as_cents())
                .unwrap_or(0);
            for to_city in CityId::ALL {
                if to_city == cur_city {
                    continue;
                }
                let to_price = state
                    .effective_baseline(to_city, product)
                    .map(|m| m.as_cents())
                    .unwrap_or(0);
                let profit_per_unit = to_price - from_price;
                if profit_per_unit <= 0 {
                    continue;
                }
                let qty = stock.min(caravan.capacity);
                targets.push((product, to_city, qty, profit_per_unit));
            }
        }
        if targets.is_empty() {
            continue;
        }
        // Kâr azalan + tie-break (city, product) ASC
        targets.sort_by(|a, b| {
            b.3.cmp(&a.3)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.0.cmp(&b.0))
        });
        // Caravan_id rotation: aynı NPC'nin farklı caravan'ları farklı target.
        let idx = (caravan.id.value() as usize) % targets.len();
        let (product, to_city, qty, _) = targets[idx];
        let mut cargo = Cargo::new();
        if cargo.add(product, qty).is_ok() {
            out.push(ActionCandidate::DispatchCaravan {
                caravan_id: caravan.id,
                from: cur_city,
                to: to_city,
                cargo,
            });
        }
    }

    for product in ProductKind::ALL {
        let Some((cheap_city, cheap_price)) = cheapest_city(state, product) else {
            continue;
        };
        let Some((rich_city, rich_price)) = richest_city(state, product) else {
            continue;
        };
        if cheap_city == rich_city || cheap_price.as_cents() <= 0 {
            continue;
        }
        let spread_pct =
            (rich_price.as_cents() - cheap_price.as_cents()) * 100 / cheap_price.as_cents();
        if spread_pct < ARBITRAGE_SPREAD_PCT {
            continue;
        }

        // AL ucuzda
        let buy_qty = affordable_qty(bucket_cash, cheap_price, 25);
        if buy_qty > 0 {
            out.push(ActionCandidate::SubmitOrder {
                side: OrderSide::Buy,
                city: cheap_city,
                product,
                quantity: buy_qty,
                unit_price: cheap_price,
            });
        }

        // SAT — stoğu olan **her** off-cheap şehirde SAT (sadece rich_city
        // değil). Caravan dispatch rotation Ist veya Izm'e mal götürdüğünde
        // o şehirde de SAT yaz → ölü ham bucket'lar canlanır.
        // Fiyat: o şehirin baseline × 95 (Esnaf 95% BUY'u yakalar).
        for to_city in CityId::ALL {
            if to_city == cheap_city {
                continue;
            }
            let stock = player.inventory.get(to_city, product);
            if stock == 0 {
                continue;
            }
            let to_baseline = state
                .effective_baseline(to_city, product)
                .map(|m| m.as_cents())
                .unwrap_or(0);
            if to_baseline <= cheap_price.as_cents() {
                continue;
            }
            let sell_price = Money::from_cents(to_baseline.saturating_mul(95) / 100);
            let sell_qty = stock.min(25);
            out.push(ActionCandidate::SubmitOrder {
                side: OrderSide::Sell,
                city: to_city,
                product,
                quantity: sell_qty,
                unit_price: sell_price,
            });
        }
        // rich_price kullanılmadı artık ama scope için referansta tut.
        let _ = rich_price;
    }
    out
}

fn cheapest_city(state: &GameState, product: ProductKind) -> Option<(CityId, Money)> {
    CityId::ALL
        .iter()
        .copied()
        .map(|city| (city, baseline_or_default(state, city, product)))
        .filter(|(_, p)| p.as_cents() > 0)
        .min_by_key(|(_, p)| p.as_cents())
}

fn richest_city(state: &GameState, product: ProductKind) -> Option<(CityId, Money)> {
    CityId::ALL
        .iter()
        .copied()
        .map(|city| (city, baseline_or_default(state, city, product)))
        .filter(|(_, p)| p.as_cents() > 0)
        .max_by_key(|(_, p)| p.as_cents())
}

fn baseline_or_default(state: &GameState, city: CityId, product: ProductKind) -> Money {
    state.effective_baseline(city, product).unwrap_or_else(|| {
        let lira = if product.is_finished() {
            moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA
        } else {
            moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA
        };
        Money::from_lira(lira).unwrap_or(Money::ZERO)
    })
}

fn bucket_buy_budget(player: &Player) -> Money {
    // 6 ürün × 1 al-sat çifti = 6 bucket.
    Money::from_cents((player.cash.as_cents() / 6).max(0))
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

    fn tuccar(cash: i64) -> Player {
        Player::new(
            PlayerId::new(100),
            "tuc",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Tuccar)
    }

    #[test]
    fn no_baseline_no_arbitrage_candidates() {
        // fresh_state'te baseline yok → spread = 0 → arbitraj boş.
        // (Tüccar caravan yoksa BuyCaravan adayı emit eder, o ayrı.)
        let s = fresh();
        let p = tuccar(15_000);
        let cands = enumerate(&s, &p);
        let arbitrage_cands = cands
            .iter()
            .filter(|c| matches!(c, ActionCandidate::SubmitOrder { .. } | ActionCandidate::DispatchCaravan { .. }))
            .count();
        assert_eq!(arbitrage_cands, 0, "spread sıfırsa arbitraj yok");
    }

    #[test]
    fn synthetic_spread_yields_arbitrage_candidates() {
        // Sun'i baseline'lar: İst Pamuk = 4, Ank = 6, Izm = 8 → spread 100%
        let mut s = fresh();
        s.price_baseline.insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline.insert((CityId::Ankara, ProductKind::Pamuk), Money::from_lira(6).unwrap());
        s.price_baseline.insert((CityId::Izmir, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        let p = tuccar(15_000);
        let cands = enumerate(&s, &p);
        let buy_in_istanbul = cands.iter().any(|c| matches!(c,
            ActionCandidate::SubmitOrder { side: OrderSide::Buy, city: CityId::Istanbul, product: ProductKind::Pamuk, .. }
        ));
        assert!(buy_in_istanbul, "ucuz şehirde AL emit etmeli");
    }

    #[test]
    fn stock_in_rich_city_yields_sell() {
        let mut s = fresh();
        s.price_baseline.insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline.insert((CityId::Izmir, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        let mut p = tuccar(15_000);
        p.inventory.add(CityId::Izmir, ProductKind::Pamuk, 30).unwrap();
        let cands = enumerate(&s, &p);
        let sell_in_izmir = cands.iter().any(|c| matches!(c,
            ActionCandidate::SubmitOrder { side: OrderSide::Sell, city: CityId::Izmir, product: ProductKind::Pamuk, .. }
        ));
        assert!(sell_in_izmir, "pahalı şehirde stok varsa SAT");
    }

    #[test]
    fn deterministic_no_rng() {
        let mut s = fresh();
        s.price_baseline.insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline.insert((CityId::Ankara, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        let p = tuccar(15_000);
        let a = enumerate(&s, &p);
        let b = enumerate(&s, &p);
        assert_eq!(a, b);
    }

    #[test]
    fn no_caravan_emits_buy_caravan_candidate() {
        let s = fresh();
        let p = tuccar(15_000);
        let cands = enumerate(&s, &p);
        let has_buy_caravan = cands.iter().any(|c| matches!(c, ActionCandidate::BuyCaravan { .. }));
        assert!(has_buy_caravan, "kervan yoksa BuyCaravan emit etmeli");
    }

    #[test]
    fn idle_caravan_with_stock_emits_dispatch() {
        use moneywar_domain::{Caravan, CaravanId};
        let mut s = fresh();
        // Şehirler arası fiyat farkı (Pamuk: Ist 4, Ank 8)
        s.price_baseline.insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline.insert((CityId::Ankara, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        let mut p = tuccar(15_000);
        // İstanbul'da Tüccar'ın 50 birim Pamuk stoğu
        p.inventory.add(CityId::Istanbul, ProductKind::Pamuk, 50).unwrap();
        // İdle kervan İstanbul'da
        let caravan = Caravan::new(CaravanId::new(1), p.id, 200, CityId::Istanbul);
        s.caravans.insert(caravan.id, caravan);
        let cands = enumerate(&s, &p);
        let has_dispatch = cands.iter().any(|c| matches!(c, ActionCandidate::DispatchCaravan { from: CityId::Istanbul, to: CityId::Ankara, .. }));
        assert!(has_dispatch, "idle kervan + ucuz şehir stoğu → pahalı şehre dispatch");
    }

    #[test]
    fn enroute_caravan_no_dispatch() {
        use moneywar_domain::{Caravan, CaravanId, Tick};
        let mut s = fresh();
        s.price_baseline.insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline.insert((CityId::Ankara, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        let mut p = tuccar(15_000);
        p.inventory.add(CityId::Istanbul, ProductKind::Pamuk, 50).unwrap();
        // EnRoute kervan — dispatch yapamaz
        let mut caravan = Caravan::new(CaravanId::new(1), p.id, 200, CityId::Istanbul);
        let mut cargo = moneywar_domain::Cargo::new();
        cargo.add(ProductKind::Pamuk, 10).unwrap();
        caravan.dispatch(CityId::Istanbul, CityId::Ankara, cargo, Tick::new(5)).unwrap();
        s.caravans.insert(caravan.id, caravan);
        let cands = enumerate(&s, &p);
        let has_dispatch = cands.iter().any(|c| matches!(c, ActionCandidate::DispatchCaravan { .. }));
        assert!(!has_dispatch, "enroute kervan dispatch emit etmemeli");
    }
}
