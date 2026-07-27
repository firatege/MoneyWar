//! Alıcı rol davranışı — hane halkı, buy-only mamul.
//!
//! Alıcı `CONSUME_PERIOD` tick'te bir mamul stoğunun bir kısmını tüketir
//! (bkz. `ProductKind::alici_consume_pct`). Sürekli alım yapması doğal —
//! yoksa açlık çeker.
//!
//! # Gereklilik sırası
//!
//! Hane bütçesini yukarı doğru harcar ([`NeedTier`]): önce karnını doyurur
//! (Ekmek/Un/Zeytinyağı), sonra giyinir (Elbise/Kumaş), en son keyfine bakar
//! (Şarap/Ziyafet). Sıra iki yerde iş görür — **ne kadar ister** (kiler
//! hedefi) ve **ne kadar öder** (basamak primi).
//!
//! Temel ve konfor kiler-sınırlı, lüks **cep**-sınırlı. Bu ayrım modelin bel
//! kemiği: her basamağı kilerle sınırlamak talebi yenileme hızına indirip
//! ekonomiyi küçültüyor (ölçüldü: üretim −%15, rol makası 3,0×→4,7×).
//! Basamak modeli talebi kısmaz, temel doyunca artan bütçeyi **lükse taşır**.
//! Yan etkisi bilinçli: hane un/kumaş yığmayı bırakınca fabrikaya girdi kalır.
//!
//! # Aday üretim kuralı
//!
//! Her `(şehir × mamul)` için bir Buy adayı:
//! - quantity = `affordable_qty(cash_bucket, price, want)` — tax-aware,
//!   `want` gereklilikten türer ve alt basamak açsa kısılır ([`MIN_GATE`])
//! - `unit_price` = `effective_baseline(city, product)` (clamp etkisi dahil)
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
    GameState, Money, NeedTier, OrderSide, Player, ProductKind, balance::TRANSACTION_TAX_PCT,
};

use crate::behavior::candidates::ActionCandidate;
use crate::behavior::pricing::{CrossPolicy, marketable_bid};

/// Alıcı'nın bu tick için olası alım adayları (3 şehir × 3 mamul = 9 max).
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    let bucket_cash = bucket_budget(player);
    let tick = state.current_tick.value();
    let satisfaction = tier_satisfaction(player);

    for (ci, city) in moneywar_domain::CityId::ALL.into_iter().enumerate() {
        for (pi, product) in ProductKind::FINISHED_GOODS.into_iter().enumerate() {
            // Kova fazı — bu kova her tick değil, `BUY_PERIOD` tick'te bir
            // alır; karşılığında o seferde `BUY_PERIOD` katı ister.
            //
            // Talep hızı aynı, emir sayısı 1/4. Faz kovaya *ve* oyuncuya
            // bağlı olduğu için hepsi aynı tick'te patlamaz: her tick
            // 15/4 ≈ 4 kova aktif olur, harcama düzgün yayılır.
            let bucket = ci * ProductKind::FINISHED_GOODS.len() + pi;
            let phase = (tick as usize + player.id.value() as usize + bucket)
                % BUY_PERIOD as usize;
            if phase != 0 {
                continue;
            }
            // Hane malı olmayan bir mamul varsa (katalog büyürse) atla.
            let Some(tier) = product.need_tier() else {
                continue;
            };
            // Bu basamağın kapısı: altındaki basamakların doyumu.
            let gate = lower_tier_gate(tier, &satisfaction);
            // effective_baseline: Walras clamp'lı referans (initial × [%60,%160]).
            // reference_price (rolling avg) kullanmak Sanayici'de olduğu gibi
            // fiyat spirali yaratıyordu — yüksek fill → avg artar → daha yüksek
            // bid → daha yüksek fill. Clamp'lı baseline spirali keser.
            let reference = state.effective_baseline(city, product).unwrap_or_else(|| {
                Money::from_lira(default_finished_price()).unwrap_or(Money::ZERO)
            });
            if reference.as_cents() <= 0 {
                continue;
            }
            // Alıcı CROSS policy — tüketici talep esnek değil, best_ask
            // üzerine atlar. Cash_ceiling = stok-based urgency (100-110%).
            let mut cash_ceiling = bid_with_urgency(reference, player, city, product);

            // Güven bonusu: tanıdık satıcı varsa daha fazla öde.
            let trust = state.max_trust_in_bucket(player.id, city, product);
            if trust > 0.3 {
                let bonus_pct = ((trust - 0.3) / 0.7 * 10.0) as i64;
                cash_ceiling = Money::from_cents(
                    cash_ceiling.as_cents().saturating_mul(100 + bonus_pct) / 100
                );
            }

            // Monopol kabulü: bucket'ta tek satıcı varsa daha yüksek fiyata razı ol.
            // Alıcının başka seçeneği yoksa premium ödemeye mecbur.
            let seller_count = state.order_book.get(&(city, product))
                .map_or(0, |orders| {
                    let sellers: std::collections::BTreeSet<_> = orders.iter()
                        .filter(|o| o.side.is_sell())
                        .map(|o| o.player)
                        .collect();
                    sellers.len()
                });
            if seller_count <= 1 {
                // Tek satıcı → %20 daha fazla öde (monopoly tax)
                cash_ceiling = Money::from_cents(
                    cash_ceiling.as_cents().saturating_mul(120) / 100
                );
            } else if seller_count == 2 {
                // Az satıcı → %10 daha fazla
                cash_ceiling = Money::from_cents(
                    cash_ceiling.as_cents().saturating_mul(110) / 100
                );
            }
            let Some(unit_price) = marketable_bid(
                state,
                player.id,
                city,
                product,
                cash_ceiling,
                CrossPolicy::Cross,
                state.current_tick,
            ) else {
                continue;
            };
            // Uzun süre buradaki istek düz bir sayıydı: Alıcı her ürüne, her
            // şehirde aynı miktarda talip oluyordu — yiyeceği ekmekle bir
            // fırının ihtiyacı olan unu ayırt etmeden. Ölçüm (bkz.
            // `moneywar-web/tests/order_size_probe.rs`):
            //
            //   Ekmek · Sanayici    825 emir × 40.8 birim = 33.660
            //   Ekmek · Alıcı     7.085 emir ×  5.9 birim = 41.800
            //
            // Emir *başına* oran doğruydu (tüketici 6, fabrika 41); tüketici
            // **adetçe** eziyor ve fabrika girdisini bulamıyordu.
            //
            // Miktarı kısarak çözmek dört kez denendi, dördü de ekonomiyi
            // küçülttü (makas 3,4×→4,3× · 5,0× · 4,7×; biri Ziyafet üretimini
            // 708'den 235'e indirdi). Kısmak yanlış lever'dı: talebin
            // **yerini** değil **seviyesini** düşürüyordu.
            //
            // İstek artık **gereklilik** üstünden. İki farklı mantık var ve
            // ikisinin ayrılması modelin bel kemiği:
            //
            // - Temel/Konfor: kiler hedefine kadar olan *eksik*. Karnı tok
            //   hane un çuvalı yığmaz; artan mal fabrikaya kalır.
            // - Lüks: kiler değil **cep** sınırlar. Temel ihtiyaç doyunca
            //   artan bütçe buraya akar.
            //
            // Lüksün cep-sınırlı kalması şart. Yalnız kiler eksiğine bakan
            // bir model talebi *yenileme hızına* indiriyor: tüketim 8 tick'te
            // %10-40, eski iştah 4 tick'te 40 birim — arada ~7× fark var ve
            // ekonomi o farkın üstünde duruyor. Ölçüldü: her kovayı kiler
            // eksiğiyle sınırlamak üretimi %15, rol makasını 3,0×→4,7×
            // götürdü. Basamak modeli talebi kısmıyor, **yukarı taşıyor**.
            let want = match tier {
                // Cep-sınırlı: tavanı `affordable_qty` + `bucket_budget`
                // koyar, kiler değil. Buraya bir miktar tavanı koymak
                // bütçe akışını kesiyor — ölçüldü: 40 birimlik tavanla
                // rol makası 3,8×'te takılı kaldı, herkes fakirleşti.
                NeedTier::Luks => u32::MAX,
                NeedTier::Temel | NeedTier::Konfor => {
                    larder_target(product).saturating_sub(player.inventory.get(city, product))
                }
            };
            // Alt basamak açken üst basamağın iştahı kısılır — ama
            // sıfırlanmaz. Tamamen kapatmak talebi uçurumdan atıyor;
            // amaç sırayı kurmak, alışverişi durdurmak değil.
            let want = scale_by_gate(want, gate);
            if want == 0 {
                continue;
            }
            let quantity = affordable_qty(bucket_cash, unit_price, want);
            if quantity == 0 {
                continue;
            }
            out.push(ActionCandidate::SubmitOrder {
                side: OrderSide::Buy,
                city,
                product,
                quantity,
                unit_price,
                ttl_override: None,
            });
        }
    }
    out
}

/// Kiler hedefi ince ayar düğmesi (%). Taban değerler
/// [`ProductKind::household_larder`]'da zaten süpürülmüş noktada; bu çarpan
/// dünyayı topluca gevşetip sıkmak için. Çok dar tutmak talebi yenileme
/// hızına indirip ekonomiyi küçültüyor — ölçüm defteri modül belgesinde.
const LARDER_SCALE_PCT: u32 = 100;

/// Bir malın bu dünyadaki kiler hedefi — ölçek ve çarpan uygulanmış.
fn larder_target(product: ProductKind) -> u32 {
    let base = product.household_larder() * LARDER_SCALE_PCT / 100;
    moneywar_domain::balance::scaled_output(base)
}

/// Alt basamak açken üst basamağa bırakılan asgari iştah payı.
///
/// Sıfır olamaz: kapıyı tamamen kapatmak talebi uçurumdan atıyor ve
/// ölçümde üretim %15 düşüyordu. Hane aç kalsa bile alışverişi büsbütün
/// kesmez — sadece önceliğini değiştirir.
const MIN_GATE: f64 = 0.25;

/// Bir basamağın doyumu: elde tutulan stok ÷ o basamağın kiler hedefi.
///
/// Kova kova değil **basamak toplamı** üzerinden bakılır; tek bir boş
/// şehir-mal kovası bütün basamağı "aç" göstermesin diye. 0.0 = hiç yok,
/// 1.0 = hedef dolu.
fn tier_satisfaction(player: &Player) -> [f64; NeedTier::ORDER.len()] {
    let mut have = [0u64; NeedTier::ORDER.len()];
    let mut target = [0u64; NeedTier::ORDER.len()];
    for (ti, tier) in NeedTier::ORDER.into_iter().enumerate() {
        for city in moneywar_domain::CityId::ALL {
            for product in ProductKind::FINISHED_GOODS {
                if product.need_tier() != Some(tier) {
                    continue;
                }
                have[ti] += u64::from(player.inventory.get(city, product));
                target[ti] += u64::from(larder_target(product));
            }
        }
    }
    let mut out = [1.0; NeedTier::ORDER.len()];
    for i in 0..NeedTier::ORDER.len() {
        if target[i] > 0 {
            #[allow(clippy::cast_precision_loss)]
            let ratio = have[i] as f64 / target[i] as f64;
            out[i] = ratio.clamp(0.0, 1.0);
        }
    }
    out
}

/// Bu basamağın kapısı — **altındaki** basamakların en açı belirler.
///
/// Temel için kapı hep açık (1.0): karnını doyurmanın önkoşulu yok.
fn lower_tier_gate(tier: NeedTier, satisfaction: &[f64; NeedTier::ORDER.len()]) -> f64 {
    let idx = NeedTier::ORDER
        .iter()
        .position(|t| *t == tier)
        .unwrap_or(0);
    let mut gate = 1.0_f64;
    for s in satisfaction.iter().take(idx) {
        gate = gate.min(*s);
    }
    gate.clamp(0.0, 1.0)
}

/// İştahı kapıya göre ölçekle — `MIN_GATE` tabanıyla.
fn scale_by_gate(want: u32, gate: f64) -> u32 {
    if want == 0 {
        return 0;
    }
    let factor = MIN_GATE + (1.0 - MIN_GATE) * gate.clamp(0.0, 1.0);
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let scaled = (f64::from(want) * factor).round() as u32;
    scaled.max(1)
}

/// Bir kovanın alışveriş dönemi (tick).
///
/// Alıcı emirlerin %64'ünü tek başına veriyordu: 42.000 emir × 5,4 birim.
/// Diğer roller 30-35 birimlik emir verirken akış tüketicinin serpintisiyle
/// doluyor, izleyici "herkes 1'er 1'er alıyor" görüyordu (ölçüm:
/// `moneywar-web/tests/scale_probe.rs`). Ekonomiyi ×10 ölçeklemek bunu
/// çözmezdi — aynı seli 10× büyütürdü.
///
/// Talep miktarı ve harcama hızı korunuyor; değişen yalnız emir ritmi.
const BUY_PERIOD: u32 = 4;

/// Alıcı cash'inin (şehir × mamul) bucket'a bölünmüş payı.
/// v0.6.0 Bursa+Konya: hardcoded 9 → dinamik (5 şehir × 7 mamul = 35).
/// Eski `/9` 3-şehir tasarımındaydı; 5 şehirde Alıcı her turn 1.67× cash
/// harcamak istiyordu → cash 40 tick'te bitiyordu.
fn bucket_budget(player: &Player) -> Money {
    let buckets = i64::try_from(moneywar_domain::CityId::ALL.len())
        .ok()
        .and_then(|n| n.checked_mul(i64::try_from(ProductKind::FINISHED_GOODS.len()).ok()?))
        .filter(|n| *n > 0)
        .unwrap_or(1);
    // Kova `BUY_PERIOD` tick'te bir alışveriş yapıyor; o sefer için biriken
    // bütçe de o kadar. Çarpmazsak seyrek alım = az harcama olur ve talep
    // gerçekten düşer — amaç talebi kısmak değil, aynı talebi daha az ve
    // daha büyük emirle vermek.
    let cents = player.cash.as_cents() * i64::from(BUY_PERIOD) / buckets;
    Money::from_cents(cents.max(0))
}

/// Stoğa **ve gerekliliğe** bağlı rezerv fiyat. Vic3 pop needs urgency:
/// kiler boşaldıkça prim artar, ama tavanı malın basamağı belirler.
///
/// Referans stok artık malın kendi kiler hedefi ([`ProductKind::household_larder`]),
/// düz bir 30 değil: hane 40 ekmekle doymuşken 12 unla da doymuş sayılır.
/// Tavan da basamaktan gelir ([`NeedTier::max_premium_pct`]) — ekmeğe %20,
/// şaraba %0. Pahalı lüks malın tüketici primiyle yukarı çekilememesi aynı
/// zamanda fiyat sarmalına karşı fren.
///
/// Bu **rezerv tavan**: Sanayici 200₺ asking yazsa hiç eşleşmez, başka
/// Sanayici 105'i kapar → rekabet doğal şekilde fiyat dengeler.
fn bid_with_urgency(
    baseline: Money,
    player: &Player,
    city: moneywar_domain::CityId,
    product: ProductKind,
) -> Money {
    let reference = f64::from(larder_target(product));
    if reference <= 0.0 {
        return baseline;
    }
    let stock = f64::from(player.inventory.get(city, product));
    let urgency = (1.0 - (stock / reference).min(1.0)).clamp(0.0, 1.0);
    let max_premium = product
        .need_tier()
        .map_or(0, NeedTier::max_premium_pct);
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    let premium_pct = (urgency * max_premium as f64) as i64;
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
    moneywar_domain::balance::npc_base_price_finished_lira()
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{CityId, NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId};

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

    /// `BUY_PERIOD` tick boyunca toplanan adaylar. Alıcı artık her kovaya
    /// her tick emir vermiyor; sözleşme "bir dönemde hepsi bir kez".
    fn candidates_over_one_period(mut s: GameState, p: &Player) -> Vec<ActionCandidate> {
        let mut all = Vec::new();
        for t in 0..BUY_PERIOD {
            s.current_tick = moneywar_domain::Tick::new(t);
            all.extend(enumerate(&s, p));
        }
        all
    }

    #[test]
    fn rich_alici_emits_buy_candidates_per_city_product() {
        let (s, p) = alici_with_cash(100_000);
        let cands = candidates_over_one_period(s, &p);
        // Bir dönemde her şehir × her mamul tam bir kez sıraya gelir.
        assert_eq!(
            cands.len(),
            CityId::ALL.len() * ProductKind::FINISHED_GOODS.len()
        );
        for cand in &cands {
            let ActionCandidate::SubmitOrder { side, product, .. } = cand else {
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

    /// Referans artık malın kendi kiler hedefi, düz 30 değil.
    fn larder(product: ProductKind) -> u32 {
        larder_target(product)
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
        // Kiler boş → urgency 1.0 → basamağın tam primi.
        let pct = 100 + NeedTier::Konfor.max_premium_pct();
        assert_eq!(bid.as_cents(), 36 * 100 * pct / 100);
    }

    #[test]
    fn full_stock_yields_baseline_bid() {
        let p = alici_with_stock(larder(ProductKind::Kumas));
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
        let p = alici_with_stock(larder(ProductKind::Kumas) / 2);
        let bid = bid_with_urgency(
            Money::from_lira(36).unwrap(),
            &p,
            CityId::Istanbul,
            ProductKind::Kumas,
        );
        // Kiler yarı dolu → urgency 0.5 → basamak priminin yarısı.
        let pct = 100 + NeedTier::Konfor.max_premium_pct() / 2;
        assert_eq!(bid.as_cents(), 36 * 100 * pct / 100);
    }

    /// Ekmek ile şarap aynı primi ödemez — gereklilik farkı fiyata yansır.
    #[test]
    fn premium_follows_need_tier_not_a_flat_rate() {
        let base = Money::from_lira(100).unwrap();
        let empty = alici_with_stock(0);
        let bread = bid_with_urgency(base, &empty, CityId::Istanbul, ProductKind::Ekmek);
        let wine = bid_with_urgency(base, &empty, CityId::Istanbul, ProductKind::Sarap);
        assert!(
            bread > base,
            "kiler boşken ekmeğe prim ödenmeli, {bread:?}"
        );
        assert_eq!(wine, base, "lüks mal prim ödememeli — sarmal freni");
        assert!(bread > wine);
    }

    /// Temel açken lüks iştahı kısılır ama sıfırlanmaz.
    #[test]
    fn hungry_basics_damp_luxury_appetite_without_killing_it() {
        let starving = [0.0, 0.0, 0.0];
        let fed = [1.0, 1.0, 1.0];
        let want = 40;
        let luks_hungry = scale_by_gate(want, lower_tier_gate(NeedTier::Luks, &starving));
        let luks_fed = scale_by_gate(want, lower_tier_gate(NeedTier::Luks, &fed));
        assert!(luks_hungry > 0, "kapı tamamen kapanmamalı — talep uçurumu");
        assert!(
            luks_hungry < luks_fed,
            "temel açken lüks iştahı kısılmalı: {luks_hungry} vs {luks_fed}"
        );
        // Temel her koşulda tam iştahla gider — doymanın önkoşulu yok.
        assert_eq!(scale_by_gate(want, lower_tier_gate(NeedTier::Temel, &starving)), want);
    }

    /// Kiler doyunca temel mal siparişi durur; lüks devam eder (bütçe akışı).
    #[test]
    fn full_larder_stops_basics_but_luxury_keeps_flowing() {
        let (s, p) = alici_with_cash(500_000);
        let mut p = p;
        for city in CityId::ALL {
            for product in ProductKind::FINISHED_GOODS {
                p.inventory.add(city, product, larder(product)).unwrap();
            }
        }
        let cands = candidates_over_one_period(s, &p);
        for cand in &cands {
            let ActionCandidate::SubmitOrder { product, .. } = cand else {
                panic!("Alıcı sadece SubmitOrder emit etmeli");
            };
            assert_eq!(
                product.need_tier(),
                Some(NeedTier::Luks),
                "{product:?} kileri doluyken hâlâ sipariş ediliyor"
            );
        }
        assert!(
            !cands.is_empty(),
            "her kiler doluyken bile lüks akmalı — bütçe kaybolmamalı"
        );
    }

    /// Temel mal kiler hedefini aşacak kadar istenmez.
    #[test]
    fn basics_never_ordered_beyond_the_larder() {
        let (s, p) = alici_with_cash(500_000);
        let mut p = p;
        let have = 3;
        for city in CityId::ALL {
            for product in ProductKind::FINISHED_GOODS {
                p.inventory.add(city, product, have).unwrap();
            }
        }
        let cands = candidates_over_one_period(s, &p);
        for cand in &cands {
            let ActionCandidate::SubmitOrder {
                product, quantity, ..
            } = cand
            else {
                panic!()
            };
            if product.need_tier() == Some(NeedTier::Luks) {
                continue;
            }
            assert!(
                *quantity <= larder(*product).saturating_sub(have),
                "{product:?}: {quantity} birim istendi, eksik yalnızca {}",
                larder(*product).saturating_sub(have)
            );
        }
    }

    #[test]
    fn city_product_set_covers_all_finished() {
        use std::collections::BTreeSet;
        let (s, p) = alici_with_cash(100_000);
        let cands = candidates_over_one_period(s, &p);
        let pairs: BTreeSet<(CityId, ProductKind)> = cands
            .iter()
            .filter_map(|c| match c {
                ActionCandidate::SubmitOrder { city, product, .. } => Some((*city, *product)),
                _ => None,
            })
            .collect();
        // Her şehir × her mamul — katalog büyüdükçe beklenti de büyür.
        assert_eq!(
            pairs.len(),
            CityId::ALL.len() * ProductKind::FINISHED_GOODS.len()
        );
        for city in CityId::ALL {
            for product in ProductKind::FINISHED_GOODS {
                assert!(pairs.contains(&(city, product)));
            }
        }
    }

    #[test]
    fn buying_load_is_spread_across_ticks() {
        // Fazın amacı: tek tick'te 15 emir yerine her tick birkaç emir.
        // Hepsi aynı tick'e düşerse emir seli geri gelir ve Alıcı yine
        // kendi kendine karşı teklif verir.
        let (mut s, p) = alici_with_cash(100_000);
        let buckets = CityId::ALL.len() * ProductKind::FINISHED_GOODS.len();
        for t in 0..BUY_PERIOD {
            s.current_tick = moneywar_domain::Tick::new(t);
            let n = enumerate(&s, &p).len();
            assert!(
                n < buckets,
                "tick {t}: {n} emir — yük dağılmamış, tüm kovalar aynı anda"
            );
        }
    }
}
