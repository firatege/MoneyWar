//! Sugeno-singleton inference motoru.
//!
//! Algoritma:
//! 1. Her kural için firing strength = `min` over antecedent degrees.
//! 2. Her output için: `Σ(strength × singleton)` ve `Σ(strength)`.
//! 3. Defuzz: `output = weighted_sum / weight_total` (weighted average).
//!
//! Hiç kural ateşlenmezse output 0.0 (`weight_total == 0` durumu).

use std::collections::BTreeMap;

use crate::fuzzy::{LinguisticVar, Rule};

/// Inputs map: `variable_name → x` değeri (genellikle `[0.0, 1.0]` normalize).
pub type Inputs = BTreeMap<&'static str, f64>;

/// Outputs map: `output_name → defuzzified değer`.
pub type Outputs = BTreeMap<&'static str, f64>;

/// Bulanık inference motoru — değişken + kural koleksiyonu.
#[derive(Debug, Clone, Default)]
pub struct Engine {
    pub variables: Vec<LinguisticVar>,
    pub rules: Vec<Rule>,
}

impl Engine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn add_var(mut self, var: LinguisticVar) -> Self {
        self.variables.push(var);
        self
    }

    #[must_use]
    pub fn add_rule(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Kuralları girdilere uygula → output map.
    ///
    /// Hiç kural ateşlenmemiş output için map'te entry yok (caller'ın
    /// `.get()` ile default 0.0 alması beklenir).
    #[must_use]
    pub fn evaluate(&self, inputs: &Inputs) -> Outputs {
        let mut weighted_sums: BTreeMap<&'static str, f64> = BTreeMap::new();
        let mut weight_totals: BTreeMap<&'static str, f64> = BTreeMap::new();

        for rule in &self.rules {
            let strength = self.firing_strength(rule, inputs);
            if strength <= 0.0 {
                continue;
            }
            for (output_name, singleton) in &rule.consequents {
                *weighted_sums.entry(*output_name).or_insert(0.0) += strength * singleton;
                *weight_totals.entry(*output_name).or_insert(0.0) += strength;
            }
        }

        weighted_sums
            .iter()
            .map(|(name, sum)| {
                let weight = weight_totals.get(name).copied().unwrap_or(0.0);
                let value = if weight > 0.0 { sum / weight } else { 0.0 };
                (*name, value)
            })
            .collect()
    }

    fn firing_strength(&self, rule: &Rule, inputs: &Inputs) -> f64 {
        if rule.antecedents.is_empty() {
            return 0.0;
        }
        let mut min_degree = f64::INFINITY;
        for (var_name, term_name) in &rule.antecedents {
            let var = self.variables.iter().find(|v| v.name == *var_name);
            let x = inputs.get(*var_name).copied().unwrap_or(0.0);
            let d = var.map_or(0.0, |v| v.degree(term_name, x));
            if d < min_degree {
                min_degree = d;
            }
        }
        if min_degree.is_infinite() {
            0.0
        } else {
            min_degree
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzzy::Mf;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn cash_var() -> LinguisticVar {
        LinguisticVar::new("cash")
            .term(
                "dusuk",
                Mf::Triangular {
                    a: 0.0,
                    b: 0.0,
                    c: 0.5,
                },
            )
            .term(
                "yuksek",
                Mf::Triangular {
                    a: 0.5,
                    b: 1.0,
                    c: 1.0,
                },
            )
    }

    #[test]
    fn empty_engine_returns_empty_outputs() {
        let e = Engine::new();
        let mut inputs = Inputs::new();
        inputs.insert("cash", 0.5);
        let out = e.evaluate(&inputs);
        assert!(out.is_empty());
    }

    #[test]
    fn single_rule_high_cash_high_buy() {
        let e = Engine::new()
            .add_var(cash_var())
            .add_rule(Rule::new().when("cash", "yuksek").then("buy", 1.0));

        let mut inputs = Inputs::new();
        inputs.insert("cash", 1.0);
        let out = e.evaluate(&inputs);
        assert!(approx(*out.get("buy").unwrap(), 1.0));
    }

    #[test]
    fn opposing_rules_weighted_average() {
        // dusuk → buy=0, yuksek → buy=1. Tam orta (0.5) → her iki kural da
        // 0 ateşler (dusuk c=0.5'te 0, yuksek a=0.5'te 0). Hafif yukarı şift:
        let e = Engine::new()
            .add_var(cash_var())
            .add_rule(Rule::new().when("cash", "dusuk").then("buy", 0.0))
            .add_rule(Rule::new().when("cash", "yuksek").then("buy", 1.0));

        let mut inputs = Inputs::new();
        inputs.insert("cash", 0.75);
        let out = e.evaluate(&inputs);
        // 0.75 → dusuk degree=0, yuksek degree=0.5 → buy = (0×0 + 0.5×1)/0.5 = 1.0
        assert!(approx(*out.get("buy").unwrap(), 1.0));
    }

    #[test]
    fn min_and_combination_takes_minimum() {
        // İki antecedent. Sonuç min'leri olmalı.
        let cash = cash_var();
        let stock = LinguisticVar::new("stock").term(
            "bos",
            Mf::Triangular {
                a: 0.0,
                b: 0.0,
                c: 0.5,
            },
        );
        let e = Engine::new().add_var(cash).add_var(stock).add_rule(
            Rule::new()
                .when("cash", "yuksek")
                .when("stock", "bos")
                .then("buy", 1.0),
        );

        let mut inputs = Inputs::new();
        inputs.insert("cash", 0.8); // yuksek degree = (0.8-0.5)/0.5 = 0.6
        inputs.insert("stock", 0.1); // bos degree = (0.5-0.1)/0.5 = 0.8
        let out = e.evaluate(&inputs);
        // strength = min(0.6, 0.8) = 0.6 → buy = 0.6×1 / 0.6 = 1.0 (tek kural)
        assert!(approx(*out.get("buy").unwrap(), 1.0));
    }

    #[test]
    fn no_firing_rules_means_no_output_entry() {
        let e = Engine::new()
            .add_var(cash_var())
            .add_rule(Rule::new().when("cash", "yuksek").then("buy", 1.0));

        let mut inputs = Inputs::new();
        inputs.insert("cash", 0.0); // yuksek degree = 0
        let out = e.evaluate(&inputs);
        assert!(!out.contains_key("buy"));
    }

    #[test]
    fn missing_input_treats_as_zero() {
        let e = Engine::new()
            .add_var(cash_var())
            .add_rule(Rule::new().when("cash", "dusuk").then("buy", 0.5));

        let inputs = Inputs::new(); // cash yok
        let out = e.evaluate(&inputs);
        // x=0 → dusuk degree = 1 (a=0, b=0, c=0.5'te x=0 noktası tepe)
        assert!(approx(*out.get("buy").unwrap(), 0.5));
    }

    #[test]
    fn unknown_variable_treats_as_zero_degree() {
        let e = Engine::new()
            .add_var(cash_var())
            .add_rule(Rule::new().when("nonexistent", "term").then("buy", 1.0));

        let mut inputs = Inputs::new();
        inputs.insert("nonexistent", 0.5);
        let out = e.evaluate(&inputs);
        assert!(!out.contains_key("buy"));
    }

    #[test]
    fn evaluate_is_deterministic_for_same_inputs() {
        let e = Engine::new()
            .add_var(cash_var())
            .add_rule(Rule::new().when("cash", "yuksek").then("buy", 1.0))
            .add_rule(Rule::new().when("cash", "dusuk").then("buy", 0.0));

        let mut inputs = Inputs::new();
        inputs.insert("cash", 0.7);
        let out1 = e.evaluate(&inputs);
        let out2 = e.evaluate(&inputs);
        assert_eq!(out1, out2);
    }

    #[test]
    fn multi_output_rule_sets_all_outputs() {
        let e = Engine::new().add_var(cash_var()).add_rule(
            Rule::new()
                .when("cash", "yuksek")
                .then("buy", 1.0)
                .then("price_aggr", 0.8),
        );

        let mut inputs = Inputs::new();
        inputs.insert("cash", 1.0);
        let out = e.evaluate(&inputs);
        assert!(approx(*out.get("buy").unwrap(), 1.0));
        assert!(approx(*out.get("price_aggr").unwrap(), 0.8));
    }
}
