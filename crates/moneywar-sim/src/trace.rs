//! NPC karar trace — her tick × her NPC için neden o aksiyonu (veya yokluğu)
//! seçti, hangi sinyaller işliyor.
//!
//! Faz 0'da fuzzy entegre edilmediği için trace placeholder mantıkla çalışır:
//! NPC'nin emit ettiği komutları sayar + emir özeti tutar. Faz 4 sonrası fuzzy
//! inputs/outputs ve firing rules buraya yazılacak.
//!
//! `BTreeMap` deterministic ordering için.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Tek bir NPC'nin tek tick'teki karar trace'i.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpcDecisionTrace {
    pub tick: u32,
    pub npc_id: u64,
    pub npc_name: String,
    pub kind: Option<String>,
    pub personality: Option<String>,
    /// Fuzzy input degrees `(name → 0..1)`. Faz 0'da boş.
    pub inputs: BTreeMap<String, f64>,
    /// Fuzzy output utilities `(name → 0..1)`. Faz 0'da boş.
    pub outputs: BTreeMap<String, f64>,
    /// Tetiklenen kurallar (firing strength > 0). Faz 0'da boş.
    pub fired_rules: Vec<String>,
    /// Bu tick için emit edilen action özetleri (tip + miktar + fiyat).
    pub actions_emitted: Vec<String>,
    /// Aksiyon emit edilmediyse "neden" not. Boşsa aksiyon var demektir.
    pub no_action_reason: Option<String>,
}

impl NpcDecisionTrace {
    #[must_use]
    pub fn empty(tick: u32, npc_id: u64, name: String) -> Self {
        Self {
            tick,
            npc_id,
            npc_name: name,
            kind: None,
            personality: None,
            inputs: BTreeMap::new(),
            outputs: BTreeMap::new(),
            fired_rules: Vec::new(),
            actions_emitted: Vec::new(),
            no_action_reason: None,
        }
    }
}

/// Tick içindeki tüm NPC karar trace'i.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TickTrace {
    pub tick: u32,
    pub npc_decisions: Vec<NpcDecisionTrace>,
}
