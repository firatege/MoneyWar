//! `MoneyWar` domain tipleri.
//!
//! Saf veri modeli — I/O yok, global state yok. Tüm motor ve server
//! katmanları bu crate üstüne oturur.

pub mod balance;

mod caravan;
mod city;
mod command;
mod config;
mod contract;
mod error;
mod event;
mod factory;
mod ids;
mod loan;
mod money;
mod news;
mod order;
mod player;
mod product;
mod state;
mod time;

pub use caravan::{Caravan, CaravanState, Cargo, CargoSpec};
pub use city::{CityId, DemandLevel};
pub use command::Command;
pub use config::{GameBalance, NpcComposition, Preset, RoomConfig};
pub use contract::{Contract, ContractProposal, ContractState, ListingKind};
pub use error::DomainError;
pub use event::{EventSeverity, GameEvent};
pub use factory::{Factory, FactoryBatch};
pub use ids::{
    CaravanId, ContractId, EventId, FactoryId, LoanId, NewsId, OrderId, PlayerId, RoomId,
};
pub use loan::Loan;
pub use money::Money;
pub use news::{NewsItem, NewsTier};
pub use order::{MarketOrder, OrderSide};
pub use player::{Inventory, NpcKind, Player, Role};
pub use product::{Perishability, ProductClass, ProductKind};
pub use state::{ActiveShock, GameState, IdCounters};
pub use time::{SeasonProgress, Tick};
