//! Üyelik fonksiyonları (membership functions) — `Triangular` ve `Trapezoidal`.
//!
//! Klasik `min(sol, sağ)` slope formülü; kenar dik durumları (`a == b`
//! ya da `b == c` üçgen için, `a == b` ya da `c == d` yamuk için) doğal
//! olarak destekler.
//!
//! Sonuç her zaman `[0.0, 1.0]` aralığında — `clamp` ile.

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mf {
    /// Üçgen: `a`'da 0, `b`'de tepe (1), `c`'de 0. `a ≤ b ≤ c` olmalı.
    /// Kenar dik tip: `a == b` (sınır-sol) veya `b == c` (sınır-sağ).
    Triangular { a: f64, b: f64, c: f64 },
    /// Yamuk: `a`'da 0, `[b, c]` plato (1), `d`'de 0. `a ≤ b ≤ c ≤ d`.
    Trapezoidal { a: f64, b: f64, c: f64, d: f64 },
}

impl Mf {
    #[must_use]
    pub fn degree(self, x: f64) -> f64 {
        match self {
            Self::Triangular { a, b, c } => {
                let left = slope_left(x, a, b);
                let right = slope_right(x, b, c);
                left.min(right).clamp(0.0, 1.0)
            }
            Self::Trapezoidal { a, b, c, d } => {
                let left = slope_left(x, a, b);
                let right = slope_right(x, c, d);
                left.min(right).clamp(0.0, 1.0)
            }
        }
    }
}

/// Sol kenarın eğimi: `x` `a`'dan `b`'ye yükselirken 0 → 1.
/// Eğer `a == b` (kenar dik), `x ≥ a` ise 1 yoksa 0.
fn slope_left(x: f64, a: f64, b: f64) -> f64 {
    if (b - a).abs() < f64::EPSILON {
        if x >= a {
            1.0
        } else {
            0.0
        }
    } else {
        (x - a) / (b - a)
    }
}

/// Sağ kenarın eğimi: `x` `b`'den (veya `c`'den) `c` (veya `d`)'ye düşerken 1 → 0.
/// Eğer kenar dik (`b == c` triangular ya da `c == d` trapezoidal), `x ≤ c`
/// (ya da `x ≤ d`) ise 1 yoksa 0.
fn slope_right(x: f64, b: f64, c: f64) -> f64 {
    if (c - b).abs() < f64::EPSILON {
        if x <= c {
            1.0
        } else {
            0.0
        }
    } else {
        (c - x) / (c - b)
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn triangular_peak_is_one() {
        let m = Mf::Triangular {
            a: 0.0,
            b: 0.5,
            c: 1.0,
        };
        assert!(approx(m.degree(0.5), 1.0));
    }

    #[test]
    fn triangular_edges_are_zero() {
        let m = Mf::Triangular {
            a: 0.0,
            b: 0.5,
            c: 1.0,
        };
        assert_eq!(m.degree(0.0), 0.0);
        assert_eq!(m.degree(1.0), 0.0);
        assert_eq!(m.degree(-0.1), 0.0);
        assert_eq!(m.degree(1.5), 0.0);
    }

    #[test]
    fn triangular_left_slope_linear() {
        let m = Mf::Triangular {
            a: 0.0,
            b: 1.0,
            c: 2.0,
        };
        assert!(approx(m.degree(0.5), 0.5));
        assert!(approx(m.degree(0.25), 0.25));
    }

    #[test]
    fn triangular_right_slope_linear() {
        let m = Mf::Triangular {
            a: 0.0,
            b: 1.0,
            c: 2.0,
        };
        assert!(approx(m.degree(1.5), 0.5));
        assert!(approx(m.degree(1.75), 0.25));
    }

    #[test]
    fn triangular_degenerate_left_edge() {
        // a == b → sol kenar dik (sınır term: 0'da hemen 1)
        let m = Mf::Triangular {
            a: 0.0,
            b: 0.0,
            c: 1.0,
        };
        assert!(approx(m.degree(0.0), 1.0));
        assert!(approx(m.degree(0.5), 0.5));
    }

    #[test]
    fn triangular_degenerate_right_edge() {
        // b == c → sağ kenar dik (sınır term: 1.0'da hâlâ tepe)
        let m = Mf::Triangular {
            a: 0.5,
            b: 1.0,
            c: 1.0,
        };
        assert!(approx(m.degree(1.0), 1.0));
        assert!(approx(m.degree(0.75), 0.5));
        assert_eq!(m.degree(0.5), 0.0);
    }

    #[test]
    fn trapezoidal_plateau_is_one() {
        let m = Mf::Trapezoidal {
            a: 0.0,
            b: 0.3,
            c: 0.7,
            d: 1.0,
        };
        assert!(approx(m.degree(0.4), 1.0));
        assert!(approx(m.degree(0.5), 1.0));
        assert!(approx(m.degree(0.6), 1.0));
    }

    #[test]
    fn trapezoidal_edges_are_zero() {
        let m = Mf::Trapezoidal {
            a: 0.0,
            b: 0.3,
            c: 0.7,
            d: 1.0,
        };
        assert_eq!(m.degree(0.0), 0.0);
        assert_eq!(m.degree(1.0), 0.0);
    }

    #[test]
    fn trapezoidal_slopes() {
        let m = Mf::Trapezoidal {
            a: 0.0,
            b: 0.4,
            c: 0.6,
            d: 1.0,
        };
        assert!(approx(m.degree(0.2), 0.5));
        assert!(approx(m.degree(0.8), 0.5));
    }

    #[test]
    fn degree_always_in_unit_interval() {
        let m = Mf::Triangular {
            a: 0.0,
            b: 0.5,
            c: 1.0,
        };
        for i in -10..=20 {
            let x = f64::from(i) * 0.1;
            let d = m.degree(x);
            assert!((0.0..=1.0).contains(&d), "x={x} d={d}");
        }
    }
}
