//! Sugeno-singleton kural: AND-bağlı antecedent'ler + sabit konsekvans.
//!
//! Format:
//!
//! ```text
//! IF cash IS yuksek AND stock IS bos THEN buy_intensity = 0.9, price_aggr = 0.7
//! ```
//!
//! Antecedent: `(var_name, term_name)` çifti. Tüm antecedent'ler min-AND ile
//! birleştirilir → firing strength.
//!
//! Consequent: `(output_name, singleton_value)`. Birden çok output destekli.
//! Sugeno-singleton aggregation: output = `Σ(strength × singleton) / Σ(strength)`.

/// Tek bir bulanık kural.
#[derive(Debug, Clone)]
pub struct Rule {
    pub antecedents: Vec<(&'static str, &'static str)>,
    pub consequents: Vec<(&'static str, f64)>,
}

impl Rule {
    #[must_use]
    pub fn new() -> Self {
        Self {
            antecedents: Vec::new(),
            consequents: Vec::new(),
        }
    }

    /// Yeni bir antecedent ekler — `var IS term`. Birden çok kez çağrılabilir
    /// (her ek antecedent AND ile birleşir).
    #[must_use]
    pub fn when(mut self, var: &'static str, term: &'static str) -> Self {
        self.antecedents.push((var, term));
        self
    }

    /// Yeni bir consequent ekler — `output = singleton`. Birden çok output
    /// olabilir (örn. aynı kural hem `buy_intensity` hem `price_aggr` set eder).
    #[must_use]
    pub fn then(mut self, output: &'static str, value: f64) -> Self {
        self.consequents.push((output, value));
        self
    }
}

impl Default for Rule {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_chain_works() {
        let r = Rule::new()
            .when("cash", "yuksek")
            .when("stock", "bos")
            .then("buy", 0.9)
            .then("price_aggr", 0.7);
        assert_eq!(r.antecedents.len(), 2);
        assert_eq!(r.consequents.len(), 2);
        assert_eq!(r.antecedents[0], ("cash", "yuksek"));
        assert_eq!(r.consequents[1], ("price_aggr", 0.7));
    }

    #[test]
    fn empty_rule_default() {
        let r = Rule::default();
        assert!(r.antecedents.is_empty());
        assert!(r.consequents.is_empty());
    }
}
