//! İnsan oyuncu davranışını **deterministik script** olarak modeller. NPC
//! tarafı `decide_all_npcs` üstünden gelir; insan tarafı için ad-hoc kurallar.
//!
//! Standart senaryolar: `PassivePlayer` (hiç aksiyon yok), `ActiveSanayici`
//! (fabrika kur + agresif al/sat), `ActiveTuccar` (arbitraj kervan).
//!
//! Yeni senaryo eklemek için `Scenario::script` field'ına closure yaz.

use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, OrderId, OrderSide, PlayerId, ProductKind, Tick,
};

/// İnsan oyuncu davranış scripti — her tick için komut listesi üretir.
pub type HumanScript =
    fn(state: &GameState, human_id: PlayerId, tick: Tick) -> Vec<Command>;

/// Önceden tanımlı senaryolar.
pub struct Scenario {
    pub name: &'static str,
    pub description: &'static str,
    pub script: HumanScript,
}

impl std::fmt::Debug for Scenario {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scenario")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish()
    }
}

impl Scenario {
    /// Hiçbir şey yapmayan oyuncu — pure NPC ekonomisi nasıl davranıyor görmek için.
    pub const PASSIVE: Self = Self {
        name: "passive",
        description: "İnsan oyuncu hiç aksiyon yapmaz — saf NPC ekonomisi gözlem.",
        script: passive_script,
    };

    /// Aktif Sanayici: t5'te starter fabrika otomatik kurulur (engine bedava
    /// veriyor), t10-30 arası agresif Pamuk al, t50+ Kumaş sat.
    pub const ACTIVE_SANAYICI: Self = Self {
        name: "active_sanayici",
        description: "Sanayici: t10-30 Pamuk al, t50+ Kumaş sat (piyasa altı).",
        script: active_sanayici_script,
    };

    /// Aktif Tüccar: t5-50 arası ucuz şehirden Pamuk al, sonra başka şehirde sat.
    pub const ACTIVE_TUCCAR: Self = Self {
        name: "active_tuccar",
        description: "Tüccar: ucuz şehirde al, başka şehirde piyasa altı sat.",
        script: active_tuccar_script,
    };
}

fn passive_script(_state: &GameState, _human_id: PlayerId, _tick: Tick) -> Vec<Command> {
    Vec::new()
}

fn active_sanayici_script(
    state: &GameState,
    human_id: PlayerId,
    tick: Tick,
) -> Vec<Command> {
    let t = tick.value();
    let mut cmds = Vec::new();
    let player = match state.players.get(&human_id) {
        Some(p) => p,
        None => return cmds,
    };

    // t10..=30 arası: her 5 tickte bir Pamuk al (her şehirde)
    if (10..=30).contains(&t) && t % 5 == 0 {
        for (idx, city) in CityId::ALL.iter().enumerate() {
            // Bid market×0.95 ile dene — piyasa altı; eşleşme zorlanmalı
            let avg = state
                .rolling_avg_price(*city, ProductKind::Pamuk, 5)
                .or_else(|| state.effective_baseline(*city, ProductKind::Pamuk))
                .unwrap_or_else(|| moneywar_domain::Money::from_lira(6).unwrap());
            let bid_cents = (avg.as_cents() * 95) / 100;
            let id = OrderId::new(1_000_000 + u64::from(t) * 10 + idx as u64);
            if let Ok(o) = MarketOrder::new_with_ttl(
                id,
                human_id,
                *city,
                ProductKind::Pamuk,
                OrderSide::Buy,
                30,
                moneywar_domain::Money::from_cents(bid_cents),
                tick,
                3,
            ) {
                cmds.push(Command::SubmitOrder(o));
            }
        }
    }

    // t50+ : elde Kumaş varsa piyasa altına sat — kullanıcı şikayetini
    // "piyasa altı kumaş alınmıyor" replikası.
    if t >= 50 && t % 5 == 0 {
        for (idx, city) in CityId::ALL.iter().enumerate() {
            let stock = player.inventory.get(*city, ProductKind::Kumas);
            if stock < 5 {
                continue;
            }
            let avg = state
                .rolling_avg_price(*city, ProductKind::Kumas, 5)
                .or_else(|| state.effective_baseline(*city, ProductKind::Kumas))
                .unwrap_or_else(|| moneywar_domain::Money::from_lira(20).unwrap());
            let ask_cents = (avg.as_cents() * 90) / 100;
            let id = OrderId::new(2_000_000 + u64::from(t) * 10 + idx as u64);
            let qty = stock.min(15);
            if let Ok(o) = MarketOrder::new_with_ttl(
                id,
                human_id,
                *city,
                ProductKind::Kumas,
                OrderSide::Sell,
                qty,
                moneywar_domain::Money::from_cents(ask_cents),
                tick,
                3,
            ) {
                cmds.push(Command::SubmitOrder(o));
            }
        }
    }

    cmds
}

fn active_tuccar_script(
    state: &GameState,
    human_id: PlayerId,
    tick: Tick,
) -> Vec<Command> {
    let t = tick.value();
    let mut cmds = Vec::new();

    // t5-40 arası: en ucuz şehirde Pamuk al
    if (5..=40).contains(&t) && t % 4 == 0 {
        let mut cheapest: Option<(CityId, i64)> = None;
        for city in CityId::ALL {
            if let Some(p) = state
                .rolling_avg_price(city, ProductKind::Pamuk, 5)
                .or_else(|| state.effective_baseline(city, ProductKind::Pamuk))
            {
                let cents = p.as_cents();
                if cheapest.is_none_or(|(_, c)| cents < c) {
                    cheapest = Some((city, cents));
                }
            }
        }
        if let Some((city, cents)) = cheapest {
            let id = OrderId::new(3_000_000 + u64::from(t));
            if let Ok(o) = MarketOrder::new_with_ttl(
                id,
                human_id,
                city,
                ProductKind::Pamuk,
                OrderSide::Buy,
                40,
                moneywar_domain::Money::from_cents((cents * 105) / 100),
                tick,
                3,
            ) {
                cmds.push(Command::SubmitOrder(o));
            }
        }
    }

    // t45+ : başka şehirde piyasa altı sat
    if t >= 45 && t % 4 == 0 {
        let player = match state.players.get(&human_id) {
            Some(p) => p,
            None => return cmds,
        };
        for (idx, city) in CityId::ALL.iter().enumerate() {
            let stock = player.inventory.get(*city, ProductKind::Pamuk);
            if stock < 10 {
                continue;
            }
            let avg = state
                .rolling_avg_price(*city, ProductKind::Pamuk, 5)
                .or_else(|| state.effective_baseline(*city, ProductKind::Pamuk))
                .unwrap_or_else(|| moneywar_domain::Money::from_lira(7).unwrap());
            let ask_cents = (avg.as_cents() * 92) / 100;
            let id = OrderId::new(4_000_000 + u64::from(t) * 10 + idx as u64);
            if let Ok(o) = MarketOrder::new_with_ttl(
                id,
                human_id,
                *city,
                ProductKind::Pamuk,
                OrderSide::Sell,
                stock.min(20),
                moneywar_domain::Money::from_cents(ask_cents),
                tick,
                3,
            ) {
                cmds.push(Command::SubmitOrder(o));
            }
        }
    }

    cmds
}
