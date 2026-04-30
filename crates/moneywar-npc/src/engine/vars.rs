//! Standart fuzzy variable library — tüm rol rule base'leri tarafından paylaşılan
//! `LinguisticVar` koleksiyonu.
//!
//! Tasarım:
//! - Her değişkenin 3 term'i var: `dusuk` / `orta` / `yuksek` — basit ve
//!   anlaşılır kural yazımı için.
//! - Trapezoidal Mf'lerle örtüşmeli plato — terim sınırlarında smooth geçiş.
//! - Tüm girdiler `[0.0, 1.0]` ölçeğinde — `engine::inputs::compute_inputs`
//!   ile uyumlu.
//!
//! Term sınırları (`a, b, c, d` for trapezoidal):
//! ```text
//! dusuk:   [0.0, 0.0, 0.2, 0.4]    full → düşüş bandı
//! orta:    [0.2, 0.4, 0.6, 0.8]    yükseliş → plato → düşüş
//! yuksek:  [0.6, 0.8, 1.0, 1.0]    yükseliş → full
//! ```
//!
//! Bu yapı klasik 3-term Mamdani fuzzy stili. Faz 4'te kurallar yazılırken
//! ihtiyaç olursa `cok_dusuk` / `cok_yuksek` ekstra terimleri eklenebilir.

use crate::fuzzy::{LinguisticVar, Mf};

/// 3-term standart membership shape — değişken adı verilince hazır var döner.
fn three_term_var(name: &'static str) -> LinguisticVar {
    LinguisticVar::new(name)
        .term(
            "dusuk",
            Mf::Trapezoidal {
                a: 0.0,
                b: 0.0,
                c: 0.2,
                d: 0.4,
            },
        )
        .term(
            "orta",
            Mf::Trapezoidal {
                a: 0.2,
                b: 0.4,
                c: 0.6,
                d: 0.8,
            },
        )
        .term(
            "yuksek",
            Mf::Trapezoidal {
                a: 0.6,
                b: 0.8,
                c: 1.0,
                d: 1.0,
            },
        )
}

/// Cash değişkeni — NPC nakdi / typical (20K).
#[must_use]
pub fn cash_var() -> LinguisticVar {
    three_term_var("cash")
}

/// Stock değişkeni — `(city, product)` için NPC stoğu / 100.
#[must_use]
pub fn stock_var() -> LinguisticVar {
    three_term_var("stock")
}

/// Price relative to fair value — 0=ucuz, 0.5=adil, 1=pahalı.
/// Term anlamları doğal olarak: dusuk=ucuz, orta=adil, yuksek=pahalı.
#[must_use]
pub fn price_rel_avg_var() -> LinguisticVar {
    three_term_var("price_rel_avg")
}

/// Momentum — 0=düşüyor, 0.5=sabit, 1=yükseliyor.
/// Term anlamları: dusuk=düşüyor, orta=sabit, yuksek=yükseliyor.
#[must_use]
pub fn momentum_var() -> LinguisticVar {
    three_term_var("momentum")
}

/// Urgency — sezon ilerlemesi.
#[must_use]
pub fn urgency_var() -> LinguisticVar {
    three_term_var("urgency")
}

/// Arbitrage — şehirler arası max fark normalize.
#[must_use]
pub fn arbitrage_var() -> LinguisticVar {
    three_term_var("arbitrage")
}

/// Event — aktif şok şiddeti.
#[must_use]
pub fn event_var() -> LinguisticVar {
    three_term_var("event")
}

/// Competition — bu pazarda rakip emir baskısı.
#[must_use]
pub fn competition_var() -> LinguisticVar {
    three_term_var("competition")
}

/// Bid/supply ratio — talep/arz dengesi. dusuk=talep yok, orta=denge, yuksek=alıcı çok.
#[must_use]
pub fn bid_supply_ratio_var() -> LinguisticVar {
    three_term_var("bid_supply_ratio")
}

/// İflas riski — dusuk=güvende, orta=dikkat, yuksek=iflas yakın.
#[must_use]
pub fn bankruptcy_risk_var() -> LinguisticVar {
    three_term_var("bankruptcy_risk")
}

/// Fabrika sayısı (Sanayici) — dusuk=0, orta=1-2, yuksek=3+.
#[must_use]
pub fn factory_count_var() -> LinguisticVar {
    three_term_var("factory_count")
}

/// Kervan sayısı (Tüccar) — dusuk=0, orta=1-2, yuksek=3+.
#[must_use]
pub fn caravan_count_var() -> LinguisticVar {
    three_term_var("caravan_count")
}

/// Sezon kalan — yuksek=başlangıç, dusuk=son.
#[must_use]
pub fn season_remaining_var() -> LinguisticVar {
    three_term_var("season_remaining")
}

/// Rakip aksiyon baskısı (Plan v5 reactive) — bu (city, product) için bu
/// NPC dışında **kaç farklı rakip** açık emir tutuyor. Yüksek = yoğun rekabet,
/// düşük = sakin pazar. NPC'lerin birbirine react etmesini fuzzy seviyede
/// modeller (cluster signal'in güçlü versiyonu).
#[must_use]
pub fn rival_action_pressure_var() -> LinguisticVar {
    three_term_var("rival_action_pressure")
}

/// Ask/supply ratio — `bid_supply_ratio`'nun karşıtı (arz baskısı).
/// dusuk=arz yok, yuksek=arz çok (fırsat fiyatı).
#[must_use]
pub fn ask_supply_ratio_var() -> LinguisticVar {
    three_term_var("ask_supply_ratio")
}

/// Local raw advantage — (city, product) yerel uzmanlığa uyuyor mu?
/// 0 = uymuyor, 1 = uyuyor. Sanayici fabrika kurma ve Esnaf ham alım
/// kararlarını şehir-spesifik yapan binary sinyal.
#[must_use]
pub fn local_raw_advantage_var() -> LinguisticVar {
    three_term_var("local_raw_advantage")
}

/// Tüm standart fuzzy değişkenleri tek listede döndür. Rule base'ler
/// `Engine::new().add_var(...)` ile ekler.
#[must_use]
pub fn build_standard_vars() -> Vec<LinguisticVar> {
    vec![
        cash_var(),
        stock_var(),
        price_rel_avg_var(),
        momentum_var(),
        urgency_var(),
        arbitrage_var(),
        event_var(),
        competition_var(),
        bid_supply_ratio_var(),
        bankruptcy_risk_var(),
        factory_count_var(),
        caravan_count_var(),
        season_remaining_var(),
        rival_action_pressure_var(),
        ask_supply_ratio_var(),
        local_raw_advantage_var(),
    ]
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn standard_vars_count_is_sixteen() {
        // 8 ana + 5 ileri + 2 v5 + 1 v6 (local_raw_advantage) = 16.
        assert_eq!(build_standard_vars().len(), 16);
    }

    #[test]
    fn dusuk_term_peaks_at_zero() {
        let v = cash_var();
        assert_eq!(v.degree("dusuk", 0.0), 1.0);
        assert_eq!(v.degree("dusuk", 0.1), 1.0);
    }

    #[test]
    fn yuksek_term_peaks_at_one() {
        let v = cash_var();
        assert_eq!(v.degree("yuksek", 1.0), 1.0);
        assert_eq!(v.degree("yuksek", 0.9), 1.0);
    }

    #[test]
    fn orta_term_peaks_in_middle() {
        let v = cash_var();
        assert_eq!(v.degree("orta", 0.5), 1.0);
        assert_eq!(v.degree("orta", 0.4), 1.0);
    }

    #[test]
    fn dusuk_zero_at_high_input() {
        let v = cash_var();
        assert_eq!(v.degree("dusuk", 0.5), 0.0);
        assert_eq!(v.degree("dusuk", 1.0), 0.0);
    }

    #[test]
    fn yuksek_zero_at_low_input() {
        let v = cash_var();
        assert_eq!(v.degree("yuksek", 0.0), 0.0);
        assert_eq!(v.degree("yuksek", 0.5), 0.0);
    }

    #[test]
    fn overlapping_dusuk_orta_at_boundary() {
        // 0.3 hem dusuk (slope down) hem orta (slope up) içinde
        let v = cash_var();
        let d = v.degree("dusuk", 0.3);
        let o = v.degree("orta", 0.3);
        assert!(d > 0.0 && d < 1.0);
        assert!(o > 0.0 && o < 1.0);
    }

    #[test]
    fn variable_names_match_inputs_module() {
        // engine::inputs::compute_inputs aynı string anahtarları kullanır.
        // Kurallar için anahtarların eşleşmesi kritik.
        let names: Vec<&str> = build_standard_vars().iter().map(|v| v.name).collect();
        for expected in [
            "cash",
            "stock",
            "price_rel_avg",
            "momentum",
            "urgency",
            "arbitrage",
            "event",
            "competition",
            "bid_supply_ratio",
            "bankruptcy_risk",
            "factory_count",
            "caravan_count",
            "season_remaining",
            "rival_action_pressure",
            "ask_supply_ratio",
            "local_raw_advantage",
        ] {
            assert!(
                names.contains(&expected),
                "expected variable {expected} missing from standard vars"
            );
        }
    }
}
