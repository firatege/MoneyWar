//! Utility scoring — skalar `score = Σ w_i × signal_i` formülü.
//!
//! Fuzzy `then("sell_score", 0.95)` sezgisinin yerini alır. Her ağırlık tek
//! sinyalin önemini söyler, tablo halinde tunable. Sayısal olarak izlenebilir
//! (`println!("{score} from {components:?}")` ile her kararın nereden geldiği
//! belli olur).

use crate::behavior::signals::Inputs;

/// Aday aksiyona puan veren ağırlık seti. Tüm sinyaller `[0,1]` aralığında →
/// skor da `[Σ |w|]` aralığında. Tipik ağırlık `[-1, +1]`.
///
/// Personality + Role kombinasyonu için `personality::for_role_personality`
/// bir `Weights` döner. Faz A'da hepsi `ZERO`; Faz B+'da rol-spesifik tablolar
/// dolar.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Weights {
    pub cash: f64,
    pub stock: f64,
    pub price_rel_avg: f64,
    pub momentum: f64,
    pub urgency: f64,
    pub arbitrage: f64,
    pub event: f64,
    pub competition: f64,
    pub local_raw_advantage: f64,
}

impl Weights {
    /// Her sinyal sıfır ağırlık — placeholder + nötr başlangıç.
    pub const ZERO: Self = Self {
        cash: 0.0,
        stock: 0.0,
        price_rel_avg: 0.0,
        momentum: 0.0,
        urgency: 0.0,
        arbitrage: 0.0,
        event: 0.0,
        competition: 0.0,
        local_raw_advantage: 0.0,
    };
}

/// Aday için skalar utility puanı. Eksik sinyal `0.0` olarak ele alınır.
#[must_use]
pub fn score_candidate(inputs: &Inputs, weights: &Weights) -> f64 {
    let g = |k: &str| inputs.get(k).copied().unwrap_or(0.0);
    g("cash") * weights.cash
        + g("stock") * weights.stock
        + g("price_rel_avg") * weights.price_rel_avg
        + g("momentum") * weights.momentum
        + g("urgency") * weights.urgency
        + g("arbitrage") * weights.arbitrage
        + g("event") * weights.event
        + g("competition") * weights.competition
        + g("local_raw_advantage") * weights.local_raw_advantage
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn inputs_with(values: &[(&'static str, f64)]) -> Inputs {
        let mut m: BTreeMap<&'static str, f64> = BTreeMap::new();
        for (k, v) in values {
            m.insert(*k, *v);
        }
        m
    }

    #[test]
    fn zero_weights_yields_zero_score() {
        let inputs = inputs_with(&[("cash", 1.0), ("stock", 0.5)]);
        let score = score_candidate(&inputs, &Weights::ZERO);
        assert!((score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn missing_signal_treated_as_zero() {
        let inputs = inputs_with(&[("cash", 0.8)]);
        let weights = Weights {
            cash: 1.0,
            stock: 5.0, // stock yok inputs'ta — 0 sayılır
            ..Weights::ZERO
        };
        let score = score_candidate(&inputs, &weights);
        // 0.8 * 1.0 + 0 * 5.0 = 0.8
        assert!((score - 0.8).abs() < 1e-9);
    }

    #[test]
    fn weighted_sum_combines_signals() {
        let inputs = inputs_with(&[("cash", 0.5), ("stock", 0.2), ("urgency", 0.9)]);
        let weights = Weights {
            cash: 1.0,
            stock: -2.0,
            urgency: 0.5,
            ..Weights::ZERO
        };
        let score = score_candidate(&inputs, &weights);
        // 0.5 - 0.4 + 0.45 = 0.55
        assert!((score - 0.55).abs() < 1e-9);
    }
}
