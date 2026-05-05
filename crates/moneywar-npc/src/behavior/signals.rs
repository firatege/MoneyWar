//! Sinyal hesaplama — `[0,1]` normalize girdiler.
//!
//! Faz A'da mevcut `crate::engine::inputs::compute_inputs` re-export edilir.
//! Sinyaller iyi tasarlanmış (8+ sinyal: cash, stock, price_rel_avg, momentum,
//! urgency, arbitrage, event, competition, local_raw_advantage), tekrar
//! yazmıyoruz.
//!
//! Faz D'de fuzzy silinince burası bağımsız modüle taşınacak — şu an sadece
//! re-export köprüsü.

pub use crate::engine::inputs::compute_inputs;
pub use crate::fuzzy::Inputs;
