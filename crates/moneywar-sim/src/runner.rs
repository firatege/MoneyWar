//! Headless deterministic simulation runner.
//!
//! `SimRunner` setup → run → `SimResult` (snapshots + traces). Senaryo
//! pluggable, seed deterministic. NPC tarafı `decide_all_npcs` ile gelir,
//! insan tarafı senaryo scripti ile.

use moneywar_domain::{
    CityId, Command, GameState, Money, NewsTier, Personality, Player, PlayerId, ProductKind,
    Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::{TickReport, advance_tick, rng_for};
use moneywar_npc::{Difficulty, decide_all_npcs};

use crate::{
    scenario::{HumanScript, Scenario},
    snapshot::TickSnapshot,
    trace::{NpcDecisionTrace, TickTrace},
};

/// Sim sonucu: tick başı snapshot ve karar trace'leri.
#[derive(Debug)]
pub struct SimResult {
    pub seed: u64,
    pub ticks: u32,
    pub scenario_name: String,
    pub difficulty: Difficulty,
    pub snapshots: Vec<TickSnapshot>,
    pub traces: Vec<TickTrace>,
}

/// Sim koşturucu — config + scenario alır, run() ile sonuç verir.
#[derive(Debug)]
pub struct SimRunner {
    pub seed: u64,
    pub ticks: u32,
    pub scenario: &'static Scenario,
    pub difficulty: Difficulty,
    pub human_role: Role,
    pub human_starting_cash_lira: i64,
    pub include_npcs: NpcComposition,
}

/// NPC kompozisyonu — `RoomConfig::NpcsConfig`'in test-friendly versiyonu.
#[derive(Debug, Clone, Copy)]
pub struct NpcComposition {
    pub tuccar: u8,
    pub sanayici: u8,
    pub esnaf: u8,
    pub spekulator: u8,
    pub alici: u8,
}

impl Default for NpcComposition {
    fn default() -> Self {
        // v3 x2 canlı pazar: 4T/2S/4E/6A/4Sp = 20 NPC.
        // Pazar her tick aktif, "ölü emir" hissi yok.
        // Üretim: 2 Sanayici × ~30 = 60/tick mamul.
        // Talep: 6 Alıcı × ~7 + Esnaf rotation + Spek = ~50-70/tick.
        // Hafif denge, fuzzy match verimliliği artmalı.
        Self {
            tuccar: 4,
            sanayici: 2,
            esnaf: 4,
            spekulator: 4,
            alici: 6,
        }
    }
}

const HUMAN_ID: PlayerId = PlayerId::new(1);

impl SimRunner {
    #[must_use]
    pub fn new(seed: u64, scenario: &'static Scenario) -> Self {
        Self {
            seed,
            ticks: 90,
            scenario,
            difficulty: Difficulty::Hard,
            human_role: Role::Sanayici,
            human_starting_cash_lira: 25_000,
            include_npcs: NpcComposition::default(),
        }
    }

    #[must_use]
    pub fn with_ticks(mut self, ticks: u32) -> Self {
        self.ticks = ticks;
        self
    }

    #[must_use]
    pub fn with_difficulty(mut self, d: Difficulty) -> Self {
        self.difficulty = d;
        self
    }

    #[must_use]
    pub fn with_role(mut self, r: Role) -> Self {
        self.human_role = r;
        self
    }

    /// State'i hazırla + sezon koştur + sonuç döndür.
    pub fn run(self) -> SimResult {
        let mut state = build_state(&self);
        let script: HumanScript = self.scenario.script;

        let mut snapshots: Vec<TickSnapshot> = Vec::new();
        let mut traces: Vec<TickTrace> = Vec::new();

        // Tick 0 snapshot (başlangıç).
        let initial_report = TickReport::new(Tick::ZERO);
        snapshots.push(TickSnapshot::from_state(&state, &initial_report, Tick::ZERO));

        for t in 1..=self.ticks {
            let tick = Tick::new(t);
            let mut npc_rng = rng_for(state.room_id, tick);
            let npc_cmds = decide_all_npcs(&state, &mut npc_rng, tick, self.difficulty);

            // Human script tetikle.
            let human_cmds = script(&state, HUMAN_ID, tick);
            let mut all_cmds: Vec<Command> = Vec::with_capacity(npc_cmds.len() + human_cmds.len());
            all_cmds.extend(npc_cmds.iter().cloned());
            all_cmds.extend(human_cmds.iter().cloned());

            // Trace placeholder — Faz 4 sonrası fuzzy detay buraya gelir.
            let mut tick_trace = TickTrace {
                tick: t,
                npc_decisions: Vec::new(),
            };
            for cmd in &npc_cmds {
                let actor = cmd.requester();
                let player = state.players.get(&actor);
                let (name, kind, pers) = player
                    .map(|p| {
                        (
                            p.name.clone(),
                            p.npc_kind.map(|k| format!("{k:?}")),
                            p.personality.map(|p| format!("{p:?}")),
                        )
                    })
                    .unwrap_or_else(|| (format!("?{}", actor.value()), None, None));
                let action_summary = describe_command(cmd);
                let mut entry = NpcDecisionTrace::empty(t, actor.value(), name);
                entry.kind = kind;
                entry.personality = pers;
                entry.actions_emitted.push(action_summary);
                tick_trace.npc_decisions.push(entry);
            }

            let (next_state, report) = match advance_tick(&state, &all_cmds) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("sim error tick={t}: {e}");
                    break;
                }
            };
            state = next_state;

            snapshots.push(TickSnapshot::from_state(&state, &report, tick));
            traces.push(tick_trace);
        }

        SimResult {
            seed: self.seed,
            ticks: self.ticks,
            scenario_name: self.scenario.name.into(),
            difficulty: self.difficulty,
            snapshots,
            traces,
        }
    }
}

fn describe_command(cmd: &Command) -> String {
    match cmd {
        Command::SubmitOrder(o) => format!(
            "{:?} {} {:?}@{}₺ {:?} ttl={}",
            o.side, o.quantity, o.product, o.unit_price, o.city, o.ttl_ticks
        ),
        Command::CancelOrder { order_id, .. } => format!("CancelOrder #{}", order_id.value()),
        Command::ProposeContract(p) => format!(
            "ProposeContract {} {:?} qty={} @{}",
            if matches!(p.listing, moneywar_domain::ListingKind::Public) {
                "public"
            } else {
                "personal"
            },
            p.product,
            p.quantity,
            p.unit_price
        ),
        Command::AcceptContract { contract_id, .. } => {
            format!("AcceptContract #{}", contract_id.value())
        }
        Command::CancelContractProposal { contract_id, .. } => {
            format!("CancelContract #{}", contract_id.value())
        }
        Command::BuildFactory { city, product, .. } => {
            format!("BuildFactory {city:?}/{product:?}")
        }
        Command::BuyCaravan { starting_city, .. } => format!("BuyCaravan {starting_city:?}"),
        Command::DispatchCaravan { from, to, .. } => format!("Dispatch {from:?}→{to:?}"),
        Command::SubscribeNews { tier, .. } => format!("SubscribeNews {tier:?}"),
        Command::TakeLoan { amount, .. } => format!("TakeLoan {}", amount),
        Command::RepayLoan { loan_id, .. } => format!("RepayLoan #{}", loan_id.value()),
        Command::CreditNpcCash { amount, .. } => format!("CreditNpcCash {}", amount),
    }
}

/// Test scenario için `GameState` kur — insan + NPC'ler eklenmiş.
/// CLI seed mantığını basitleştirir; deterministic seed.
fn build_state(runner: &SimRunner) -> GameState {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let mut s = GameState::new(RoomId::new(runner.seed), RoomConfig::hizli());
    let mut rng = ChaCha8Rng::seed_from_u64(runner.seed);

    // İnsan oyuncu
    let mut human = Player::new(
        HUMAN_ID,
        "Insan",
        runner.human_role,
        Money::from_lira(runner.human_starting_cash_lira).unwrap(),
        false,
    )
    .unwrap();
    if matches!(runner.human_role, Role::Sanayici) {
        // Starter ham madde — Pamuk
        let _ = human
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100);
    }
    s.players.insert(human.id, human);
    s.news_subscriptions.insert(HUMAN_ID, NewsTier::Free);

    let mut next_id: u64 = 100;

    // NPC-Tüccar
    for _ in 0..runner.include_npcs.tuccar {
        let pers = pick_personality(&mut rng);
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("Tuccar-{next_id}"),
            Role::Tuccar,
            Money::from_lira(15_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(moneywar_domain::NpcKind::Tuccar)
        .with_personality(pers);
        distribute_inv(&mut npc, &mut rng, 8_000);
        s.news_subscriptions
            .insert(PlayerId::new(next_id), NewsTier::Free);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Sanayici
    for _ in 0..runner.include_npcs.sanayici {
        let pers = pick_personality(&mut rng);
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("Sanayici-{next_id}"),
            Role::Sanayici,
            Money::from_lira(30_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(moneywar_domain::NpcKind::Sanayici)
        .with_personality(pers);
        distribute_inv(&mut npc, &mut rng, 5_000);
        s.news_subscriptions
            .insert(PlayerId::new(next_id), NewsTier::Free);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Esnaf
    for _ in 0..runner.include_npcs.esnaf {
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("Esnaf-{next_id}"),
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(moneywar_domain::NpcKind::Esnaf);
        distribute_inv(&mut npc, &mut rng, 50_000);
        s.news_subscriptions
            .insert(PlayerId::new(next_id), NewsTier::Free);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Spekulator
    for _ in 0..runner.include_npcs.spekulator {
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("Spek-{next_id}"),
            Role::Tuccar,
            Money::from_lira(40_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(moneywar_domain::NpcKind::Spekulator);
        distribute_inv(&mut npc, &mut rng, 8_000);
        s.news_subscriptions
            .insert(PlayerId::new(next_id), NewsTier::Free);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Alici
    for _ in 0..runner.include_npcs.alici {
        let npc = Player::new(
            PlayerId::new(next_id),
            format!("Alici-{next_id}"),
            Role::Tuccar,
            Money::from_lira(100_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(moneywar_domain::NpcKind::Alici);
        s.news_subscriptions
            .insert(PlayerId::new(next_id), NewsTier::Free);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    s
}

fn pick_personality(rng: &mut rand_chacha::ChaCha8Rng) -> Personality {
    use rand::Rng;
    Personality::ALL[rng.random_range(0..Personality::ALL.len())]
}

fn distribute_inv(player: &mut Player, rng: &mut rand_chacha::ChaCha8Rng, total: u32) {
    use rand::Rng;
    let buckets: Vec<(CityId, ProductKind)> = CityId::ALL
        .iter()
        .flat_map(|c| ProductKind::ALL.iter().map(move |p| (*c, *p)))
        .collect();
    let weights: Vec<u32> = (0..buckets.len())
        .map(|_| rng.random_range(0u32..=10))
        .collect();
    let total_w: u32 = weights.iter().sum();
    if total_w == 0 {
        return;
    }
    for ((city, product), w) in buckets.iter().zip(weights.iter()) {
        let share =
            u32::try_from(u64::from(total) * u64::from(*w) / u64::from(total_w)).unwrap_or(0);
        if share > 0 {
            let _ = player.inventory.add(*city, *product, share);
        }
    }
}
