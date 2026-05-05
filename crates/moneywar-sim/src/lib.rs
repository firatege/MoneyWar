//! Headless simulation runner + observability for `MoneyWar`.
//!
//! Phase 0 of the fuzzy-AI test platform plan: kullanıcı şikayetlerini
//! ölçülebilir hale getirir, fix öncesi/sonrası karşılaştırma sağlar.
//!
//! # Hızlı kullanım
//!
//! ```no_run
//! use moneywar_sim::{SimRunner, Scenario, render_markdown};
//! use moneywar_npc::Difficulty;
//!
//! let result = SimRunner::new(42, &Scenario::ACTIVE_SANAYICI)
//!     .with_ticks(90)
//!     .with_difficulty(Difficulty::Hard)
//!     .run();
//! let report_md = render_markdown(&result);
//! println!("{report_md}");
//! ```

pub mod logbuilder;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod snapshot;
pub mod stats;
pub mod thresholds;
pub mod trace;

pub use report::render_markdown;
pub use runner::{NpcComposition, RoleActionMix, SimResult, SimRunner};
pub use scenario::Scenario;
pub use snapshot::{ClearingSnapshot, OrderBookSummary, PlayerSnapshot, TickSnapshot};
pub use stats::{PerRunMetrics, QualityScore, Stats, Summary};
pub use thresholds::{
    audit_game, audit_role, default_contracts, render_threshold_report, ActionField, CheckResult,
    GameThresholds, RequiredAction, RoleContract,
};
pub use trace::{NpcDecisionTrace, TickTrace};
