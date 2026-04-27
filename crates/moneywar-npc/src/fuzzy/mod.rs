//! Bulanık mantık motoru — Sugeno-singleton inference.
//!
//! Saf matematik, `GameState` bağımlılığı yok. NPC karar destek sistemi (DSS)
//! mikro parametrelerini (qty, fiyat agresifliği vb.) hesaplamak için
//! kullanılır. Üst katman (utility AI) hangi aksiyonu seçer, fuzzy alt
//! katman parametreleri ayarlar.
//!
//! # Determinism
//!
//! `f64` aritmetik IEEE 754 deterministik (aynı CPU + opt level). `BTreeMap`
//! sıralı iter. Aynı (rules, inputs) → aynı outputs.
//!
//! # Örnek
//!
//! ```
//! use moneywar_npc::fuzzy::{Engine, LinguisticVar, Mf, Rule, Inputs};
//!
//! let cash = LinguisticVar::new("cash")
//!     .term("dusuk", Mf::Triangular { a: 0.0, b: 0.0, c: 0.5 })
//!     .term("yuksek", Mf::Triangular { a: 0.5, b: 1.0, c: 1.0 });
//!
//! let engine = Engine::new()
//!     .add_var(cash)
//!     .add_rule(Rule::new().when("cash", "yuksek").then("invest", 1.0))
//!     .add_rule(Rule::new().when("cash", "dusuk").then("invest", 0.0));
//!
//! let mut inputs = Inputs::new();
//! inputs.insert("cash", 0.8);
//! let out = engine.evaluate(&inputs);
//! assert!(out.get("invest").copied().unwrap_or(0.0) > 0.5);
//! ```

mod engine;
mod membership;
mod rule;
mod variable;

pub use engine::{Engine, Inputs, Outputs};
pub use membership::Mf;
pub use rule::Rule;
pub use variable::LinguisticVar;
