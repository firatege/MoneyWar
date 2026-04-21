//! `MoneyWar` tick motoru.
//!
//! Saf fonksiyon: `advance_tick(state, commands) → (new_state, report)`.
//! Determinism kritik: aynı input → bit-perfect aynı output. Bu crate
//! `tokio`, `std::time`, dosya I/O veya global state kullanmaz. Rastgelelik
//! sadece `(room_id, tick)`'ten türetilen `ChaCha8Rng` üzerinden.
//!
//! # Genel Kullanım
//!
//! ```no_run
//! use moneywar_domain::{GameState, RoomConfig, RoomId};
//! use moneywar_engine::advance_tick;
//!
//! let state = GameState::new(RoomId::new(1), RoomConfig::hizli());
//! let commands = vec![]; // oyuncu komutları
//! let (new_state, report) = advance_tick(&state, &commands).unwrap();
//! assert_eq!(new_state.current_tick.value(), 1);
//! assert_eq!(report.tick, new_state.current_tick);
//! ```
//!
//! # Log / Analitik
//!
//! Her tick bir `TickReport` üretir: kim, ne yaptı, sonuç ne? Server
//! bu raporu DB'ye (Faz 10 `PostgreSQL` journal tablosu) yazar; analitik
//! sorgular (en çok reddedilen komut, rol bazlı `PnL`, balance tuning)
//! bu tablo üzerinden koşar.

mod contracts;
mod error;
mod market;
mod production;
mod report;
mod rng;
mod tick;
mod transport;

pub use error::EngineError;
pub use report::{LogEntry, LogEvent, TickReport};
pub use rng::{rng_for, seed_for};
pub use tick::advance_tick;
