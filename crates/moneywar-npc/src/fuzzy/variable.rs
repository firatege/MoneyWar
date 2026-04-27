//! Linguistic variable — bir girdiye birden çok term (linguistic label) atar.
//!
//! Örnek: `cash_ratio` değişkeninin terimleri `dusuk`, `orta`, `yuksek`.
//! Her term'ün kendi membership function'ı var.

use crate::fuzzy::Mf;

/// Bir girdiye atanmış linguistic terimler kümesi.
#[derive(Debug, Clone)]
pub struct LinguisticVar {
    pub name: &'static str,
    pub terms: Vec<(&'static str, Mf)>,
}

impl LinguisticVar {
    /// Boş bir değişken oluşturur. Terimler `term()` chain'i ile eklenir.
    #[must_use]
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            terms: Vec::new(),
        }
    }

    /// Yeni bir term ekler. Builder-style — chain'lenebilir.
    #[must_use]
    pub fn term(mut self, name: &'static str, mf: Mf) -> Self {
        self.terms.push((name, mf));
        self
    }

    /// `x` girdisi için verilen `term_name`'in üyelik derecesi.
    /// Term bulunmazsa 0 döner (defansif — kural eşleşmez).
    #[must_use]
    pub fn degree(&self, term_name: &str, x: f64) -> f64 {
        self.terms
            .iter()
            .find(|(n, _)| *n == term_name)
            .map_or(0.0, |(_, mf)| mf.degree(x))
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

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
                "orta",
                Mf::Triangular {
                    a: 0.25,
                    b: 0.5,
                    c: 0.75,
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
    fn variable_name_preserved() {
        let v = cash_var();
        assert_eq!(v.name, "cash");
        assert_eq!(v.terms.len(), 3);
    }

    #[test]
    fn unknown_term_returns_zero() {
        let v = cash_var();
        assert_eq!(v.degree("nonexistent", 0.5), 0.0);
    }

    #[test]
    fn term_lookup_returns_correct_degree() {
        let v = cash_var();
        // 0.5 = orta'nın tepesi
        assert!((v.degree("orta", 0.5) - 1.0).abs() < 1e-9);
        // 0.5 = yuksek'in başlangıcı (a=0.5)
        assert_eq!(v.degree("yuksek", 0.5), 0.0);
    }

    #[test]
    fn overlapping_terms_partial_membership() {
        let v = cash_var();
        // 0.4 hem dusuk (slope down) hem orta (slope up) içinde
        let d_dusuk = v.degree("dusuk", 0.4);
        let d_orta = v.degree("orta", 0.4);
        assert!(d_dusuk > 0.0);
        assert!(d_orta > 0.0);
    }
}
