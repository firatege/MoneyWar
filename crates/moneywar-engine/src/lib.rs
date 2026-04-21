//! `MoneyWar` tick motoru.
//!
//! Saf fonksiyon: `advance_tick(state, commands, seed) → (new_state, report)`.
//! Determinism kritik: aynı input + aynı seed = aynı output. Bu crate
//! `tokio`, `std::time`, dosya I/O veya global state kullanmaz. Faz 2+'de
//! doldurulacak.

mod error;

pub use error::EngineError;
