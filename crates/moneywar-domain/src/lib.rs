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
mod intrigue;
mod loan;
mod money;
mod news;
mod order;
mod personality;
mod player;
mod product;
mod private_farm;
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
pub use intrigue::{
    DOMINANCE_MIN_VOLUME, DOMINANCE_WINDOW_TICKS, GRUDGE_TICKS, IntrigueState,
    MONOPOLY_BREAK_CONFIRM_TICKS, MONOPOLY_BREAK_PCT, MONOPOLY_CONFIRM_TICKS, MONOPOLY_FORM_PCT,
    PRICE_WAR_DECLARE_TICKS, PRICE_WAR_FIZZLE_TICKS, PRICE_WAR_RETREAT_TICKS, PriceWarTrack,
    TickSales, UNDERCUT_CAMPAIGN_TICKS,
};
pub use private_farm::{PrivateFarm, PrivateFarmId};
pub use loan::Loan;
pub use money::Money;
pub use news::{NewsItem, NewsTier};
pub use order::{MarketOrder, OrderSide};
pub use personality::Personality;
pub use player::{Inventory, NpcKind, Player, Role};
pub use product::{Perishability, ProductClass, ProductKind};
pub use state::{ActiveShock, GameState, IdCounters, MAX_NO_MATCH_STREAK, RelationScore};
pub use time::{SeasonProgress, Tick};
