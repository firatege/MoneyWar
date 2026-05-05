//! NPC karar motoru (fuzzy/DSS hibrit) — Faz 2-7 inşa edilen ortak yapı.
//!
//! - `inputs`: `GameState`'ten normalize fuzzy girdileri hesaplar.
//! - (Faz 3) `vars`: paylaşılan `LinguisticVar` set.
//! - (Faz 4) `rules/{role}`: rol başına fuzzy rule base.
//! - (Faz 5) `decide`: tüm orchestrator (inputs → fuzzy → action emit).

pub mod decide;
pub mod inputs;
pub mod rules;
pub mod vars;

pub use decide::decide_npc_fuzzy;
pub use inputs::compute_inputs;
pub use rules::engine_for;
pub use vars::build_standard_vars;
