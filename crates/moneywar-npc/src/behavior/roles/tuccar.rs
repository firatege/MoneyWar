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
use crate::behavior::pricing::apply_jitter;

/// Arbitraj eşiği — bu yüzdeden az spread varsa arbitraj kârsız.
/// Faz F tuning: 20 → 15. Demand_for matrisi mamul baseline farkını
/// %25-28 yaratıyor; ham specialty farkı %14-75 (çoğunlukla yeterli).
const ARBITRAGE_SPREAD_PCT: i64 = 15;

/// Bir (şehir, ürün) için açık order book'taki en yüksek BUY emir fiyatı.
/// Tüccar'ın "buraya mal getirirsem hangi fiyata satabilirim" sorusunun
/// cevabı — pay-as-bid clearing'de Tüccar'ın SELL'i bu bid ile eşleşir.
/// v8.20: state'in `best_bid` helper'ını çağırır, fiyat-only döner.
fn best_bid_in_city(state: &GameState, city: CityId, product: ProductKind) -> Option<Money> {
    state.best_bid(city, product).map(|(p, _)| p)
}

#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    let bucket_cash = bucket_buy_budget(player);

    // 0. Dinamik kervan filosu (v8.12 C): sabit limit yok. Tüccar nakit +
    //    arbitraj sinyali olduğu sürece kervan satın alır.
    //    Formül: arbitraj_fırsatı_var (en az 1 ürün > %15 spread) AND
    //            sonraki kervan maliyeti ≤ cash / 3 (rezerv: 1/3 ham BUY için)
    //    Her tick max 1 kervan satın alma — patlama önler.
    let owned_caravans = state
        .caravans
        .values()
        .filter(|c| c.owner == player.id)
        .count();
    let any_arbitrage = ProductKind::ALL.iter().any(|prod| {
        let Some((_, cheap_p)) = cheapest_city(state, *prod) else { return false; };
        let Some((_, rich_p)) = richest_city(state, *prod) else { return false; };
        let cheap_c = cheap_p.as_cents();
        if cheap_c <= 0 { return false; }
        (rich_p.as_cents() - cheap_c) * 100 / cheap_c >= ARBITRAGE_SPREAD_PCT
    });
    let next_cost = moneywar_domain::Caravan::buy_cost(
        player.role,
        u32::try_from(owned_caravans).unwrap_or(0),
    );
    let cash_reserve_threshold = Money::from_cents(player.cash.as_cents() / 3);
    if any_arbitrage && next_cost <= cash_reserve_threshold {
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
                .reference_price(cur_city, product)
                .map(|m| m.as_cents())
                .unwrap_or(0);
            for to_city in CityId::ALL {
                if to_city == cur_city {
                    continue;
                }
                // to_price: önce hedef şehirdeki açık BUY emirlerinin en yüksek
                // bid'i (Tüccar bu fiyata satabileceğini bilir), o yoksa
                // reference_price (rolling avg / baseline). Bu, "1703 BUY 0
                // SELL" gibi devasa talep sinyallerini yakalar — eskiden
                // reference_price baseline'a düşüp arbitraj görünmez oluyordu.
                let to_price = best_bid_in_city(state, to_city, product)
                    .map(|m| m.as_cents())
                    .or_else(|| {
                        state
                            .reference_price(to_city, product)
                            .map(|m| m.as_cents())
                    })
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

        // AL ucuzda + jitter
        let buy_price = apply_jitter(
            cheap_price,
            state.current_tick,
            cheap_city,
            product,
            OrderSide::Buy,
        );
        let buy_qty = affordable_qty(bucket_cash, buy_price, 25);
        if buy_qty > 0 {
            out.push(ActionCandidate::SubmitOrder {
                side: OrderSide::Buy,
                city: cheap_city,
                product,
                quantity: buy_qty,
                unit_price: buy_price,
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
            // SELL hedef fiyat: önce o şehirdeki en yüksek bid (Tüccar
            // doğrudan o bid ile eşleşir), o yoksa reference_price.
            // %95 markdown → bid'in altına gel, garanti eşleşme. Talep
            // baskılı bucket'larda (1703 BUY 0 SELL) bu fiyat doğal yüksek.
            let to_target = best_bid_in_city(state, to_city, product)
                .map(|m| m.as_cents())
                .or_else(|| state.reference_price(to_city, product).map(|m| m.as_cents()))
                .unwrap_or(0);
            if to_target <= cheap_price.as_cents() {
                continue;
            }
            let sell_base = Money::from_cents(to_target.saturating_mul(95) / 100);
            let sell_price = apply_jitter(
                sell_base,
                state.current_tick,
                to_city,
                product,
                OrderSide::Sell,
            );
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

    // v8.23: Forward kontrat propose — Tüccar arbitraj fırsatını taahhüt
    // formuna çevirir. "5 tick sonra X şehrine 30 birim getireceğim" deyip
    // Public listing açar. Sanayici kapıyorsa kâr garantili (escrow + delivery).
    out.extend(enumerate_contract_proposals(state, player));
    out
}

/// Tüccar'ın forward delivery kontratı önerileri.
/// - En az 1 aktif open kontrat varsa pas (cap 1)
/// - En geniş arbitraj fırsatını seç (richest - cheapest spread > %15)
/// - Public listing, delivery_tick=current+5
/// - unit_price = best_bid_in_dest × 0.95 (Tüccar margin)
/// - seller_deposit = unit_price × qty × 0.05
fn enumerate_contract_proposals(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    use moneywar_domain::{ContractProposal, ContractState, ListingKind};

    // Cap: aynı anda max 1 aktif (Proposed/Active) kontrat
    let active_count = state
        .contracts
        .values()
        .filter(|c| c.seller == player.id)
        .filter(|c| matches!(c.state, ContractState::Proposed | ContractState::Active))
        .count();
    if active_count >= 1 {
        return Vec::new();
    }

    // En iyi arbitraj fırsatı: spread % maksimum
    let mut best_opportunity: Option<(ProductKind, CityId, CityId, Money, Money, i64)> = None;
    for product in ProductKind::ALL {
        let Some((from_city, from_price)) = cheapest_city(state, product) else {
            continue;
        };
        let Some((to_city, to_price)) = richest_city(state, product) else {
            continue;
        };
        if from_city == to_city || from_price.as_cents() <= 0 {
            continue;
        }
        let spread_pct = ((to_price.as_cents() - from_price.as_cents()) * 100)
            / from_price.as_cents().max(1);
        if spread_pct < ARBITRAGE_SPREAD_PCT {
            continue;
        }
        if best_opportunity
            .as_ref()
            .is_none_or(|(_, _, _, _, _, p)| spread_pct > *p)
        {
            best_opportunity = Some((product, from_city, to_city, from_price, to_price, spread_pct));
        }
    }
    let Some((product, _from_city, to_city, _, to_price, _)) = best_opportunity else {
        return Vec::new();
    };

    // unit_price = to_city BID × 0.95 (Tüccar margin), qty=30
    let quantity = 30u32;
    let unit_price_cents = to_price.as_cents().saturating_mul(95) / 100;
    if unit_price_cents <= 0 {
        return Vec::new();
    }
    let unit_price = Money::from_cents(unit_price_cents);
    let total = unit_price_cents.saturating_mul(i64::from(quantity));
    let seller_deposit = Money::from_cents(total / 20); // %5
    let buyer_deposit = Money::from_cents(total / 20);
    if player.cash.as_cents() < seller_deposit.as_cents() {
        return Vec::new(); // Tüccar deposit ödeyemiyor
    }

    let delivery_tick = match state.current_tick.checked_add(5) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let proposal = ContractProposal {
        seller: player.id,
        listing: ListingKind::Public,
        product,
        quantity,
        unit_price,
        delivery_city: to_city,
        delivery_tick,
        seller_deposit,
        buyer_deposit,
    };
    vec![ActionCandidate::ProposeContract(proposal)]
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
    // "Richest" = Tüccar oraya mal götürürse hangi fiyatı alabileceğinin ölçütü.
    // Önce best_bid (açık BUY emri varsa Tüccar'ın gerçek satabileceği fiyat),
    // o yoksa reference_price'a düşer. Bu sayede "1809 BUY 0 SELL" gibi yoğun
    // talep buradan görünür ve arbitraj sinyali tetiklenir — eski reference
    // tabanlı tespit clearing yokken baseline'a düştüğü için spread sıfır
    // görünüyor, Tüccar fırsat algılamıyordu.
    CityId::ALL
        .iter()
        .copied()
        .map(|city| {
            let signal = best_bid_in_city(state, city, product)
                .unwrap_or_else(|| baseline_or_default(state, city, product));
            (city, signal)
        })
        .filter(|(_, p)| p.as_cents() > 0)
        .max_by_key(|(_, p)| p.as_cents())
}

fn baseline_or_default(state: &GameState, city: CityId, product: ProductKind) -> Money {
    state.reference_price(city, product).unwrap_or_else(|| {
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
        // v8.12 (C): Tüccar arbitraj sinyali + cash rezerv yeterli olunca
        // BuyCaravan emit eder. fresh state'te baseline boş → arbitraj yok.
        // Test için baseline doldur (fiyat farkı %15+ → arbitraj sinyali var).
        let mut s = fresh();
        s.price_baseline
            .insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(4).unwrap());
        s.price_baseline
            .insert((CityId::Ankara, ProductKind::Pamuk), Money::from_lira(8).unwrap());
        s.price_baseline
            .insert((CityId::Izmir, ProductKind::Pamuk), Money::from_lira(6).unwrap());
        let p = tuccar(15_000);
        let cands = enumerate(&s, &p);
        let has_buy_caravan = cands.iter().any(|c| matches!(c, ActionCandidate::BuyCaravan { .. }));
        assert!(has_buy_caravan, "arbitraj sinyali + cash varsa BuyCaravan emit etmeli");
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
