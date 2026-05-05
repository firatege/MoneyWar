//! Headless deterministic simulation runner.
//!
//! `SimRunner` setup → run → `SimResult` (snapshots + traces). Senaryo
//! pluggable, seed deterministic. NPC tarafı `decide_all_npcs` ile gelir,
//! insan tarafı senaryo scripti ile.

use moneywar_domain::{
    CityId, Command, DemandLevel, GameState, Money, NewsTier, Personality, Player, PlayerId,
    ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::{TickReport, advance_tick, rng_for};
use moneywar_npc::{Difficulty, decide_all_npcs};

use crate::{
    scenario::{HumanScript, Scenario},
    snapshot::TickSnapshot,
    trace::{NpcDecisionTrace, TickTrace},
};

/// `NpcKind` başına aksiyon dağılımı + ihlal sayacı (audit için).
///
/// Her `NpcKind` kendi şeridinde kalmalı (Plan v4 Faz 2 gate). Bu metrikler
/// gate sonrası emit edilen komutları sayar — gate çalışmıyorsa
/// `forbidden_action_count` artar.
#[derive(Debug, Default, Clone)]
pub struct RoleActionMix {
    pub buy_raw: u32,
    pub buy_finished: u32,
    pub sell_raw: u32,
    pub sell_finished: u32,
    pub build_factory: u32,
    pub buy_caravan: u32,
    pub dispatch: u32,
    pub propose_contract: u32,
    pub accept_contract: u32,
    pub take_loan: u32,
    pub repay_loan: u32,
    /// Bu `NpcKind` için yasak aksiyon (Plan v4 gate ihlal sayacı).
    pub forbidden_action_count: u32,
    /// Toplam emit edilen komut.
    pub total_commands: u32,
}

/// Sim sonucu: tick başı snapshot ve karar trace'leri.
#[derive(Debug)]
pub struct SimResult {
    pub seed: u64,
    pub ticks: u32,
    pub scenario_name: String,
    pub difficulty: Difficulty,
    pub snapshots: Vec<TickSnapshot>,
    pub traces: Vec<TickTrace>,
    /// Plan v4 davranış audit — `NpcKind` başına aksiyon dağılımı.
    pub action_mix_by_kind: std::collections::BTreeMap<String, RoleActionMix>,
    /// Banka NPC'lerinin toplam kredi açma sayısı.
    pub bank_loans_issued: u32,
}

/// Sim koşturucu — config + scenario alır, `run()` ile sonuç verir.
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

/// NPC kompozisyonu — v4 tek-görev tasarımı: 24 NPC.
#[derive(Debug, Clone, Copy)]
pub struct NpcComposition {
    pub tuccar: u8,
    pub sanayici: u8,
    pub esnaf: u8, // = Toptancı (revize)
    pub spekulator: u8,
    pub alici: u8,
    pub ciftci: u8, // yeni v4 (3 ürün × 1 = 3 Çiftçi)
    pub banka: u8,  // yeni v4 (3 şehir × 1 = 3 Banka)
}

impl Default for NpcComposition {
    fn default() -> Self {
        // v6 ölçek + ham arz boost: 4T/5S/4Top/6Ç/8A/3Sp/3B = 33 NPC.
        // Çiftçi 4→6: ham arzı +%50 (15 fabrika talebini doyurmak için).
        // Tüccar mamul BUY (314 emit) Sanayici SELL (205 emit) arz/talep
        // dengesizliği match verimini %4.6'da tutuyordu — kök neden ham yetersizliği.
        Self {
            tuccar: 4,
            sanayici: 5,
            esnaf: 4,
            spekulator: 3,
            alici: 8,
            ciftci: 6,
            banka: 3,
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
        let mut action_mix_by_kind: std::collections::BTreeMap<String, RoleActionMix> =
            std::collections::BTreeMap::new();
        let mut bank_loans_issued: u32 = 0;

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
                // DispatchCaravan'ın requester() placeholder döndürür (PlayerId(0));
                // gerçek owner caravan'da. Trace ve audit için lookup et.
                let actor = match cmd {
                    Command::DispatchCaravan { caravan_id, .. } => state
                        .caravans
                        .get(caravan_id).map_or_else(|| cmd.requester(), |c| c.owner),
                    _ => cmd.requester(),
                };
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
                entry.kind = kind.clone();
                entry.personality = pers;
                entry.actions_emitted.push(action_summary);
                tick_trace.npc_decisions.push(entry);

                // Plan v4 audit — komut tipine göre NpcKind aksiyon mix'ine ekle.
                if let Some(kind_label) = kind {
                    let mix = action_mix_by_kind.entry(kind_label.clone()).or_default();
                    record_action(cmd, &kind_label, mix, &state);
                }
            }

            let (next_state, report) = match advance_tick(&state, &all_cmds) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("sim error tick={t}: {e}");
                    break;
                }
            };
            state = next_state;

            // Banka tarafından açılan kredileri say (LoanTaken event'leri).
            for entry in &report.entries {
                if let moneywar_engine::LogEvent::LoanTaken { .. } = &entry.event {
                    bank_loans_issued += 1;
                }
            }

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
            action_mix_by_kind,
            bank_loans_issued,
        }
    }
}

/// Plan v4 audit — komutu `NpcKind` şeritine göre kategorize et + ihlalleri say.
///
/// `kind_label` `Debug` çıktısı (`"Ciftci"`, `"Sanayici"` …). Yasak aksiyon
/// emit edildiyse `forbidden_action_count` artar — gate ihlali demek.
fn record_action(cmd: &Command, kind_label: &str, mix: &mut RoleActionMix, _state: &GameState) {
    mix.total_commands += 1;
    match cmd {
        Command::SubmitOrder(o) => {
            let is_raw = o.product.is_raw();
            let is_buy = matches!(o.side, moneywar_domain::OrderSide::Buy);
            match (is_buy, is_raw) {
                (true, true) => mix.buy_raw += 1,
                (true, false) => mix.buy_finished += 1,
                (false, true) => mix.sell_raw += 1,
                (false, false) => mix.sell_finished += 1,
            }
            // Yasaklı kombinasyonlar (gate çalışıyor olmalı, çift kontrol):
            let forbidden = match kind_label {
                "Ciftci" => is_buy || !is_raw, // Çiftçi sadece SELL raw
                "Banka" => true,                // Banka komut emit etmemeli
                "Alici" => is_buy && is_raw,    // Alıcı raw almasın
                "Esnaf" => is_buy && !is_raw,   // Toptancı mamul almasın
                _ => false,
            };
            if forbidden {
                mix.forbidden_action_count += 1;
            }
        }
        Command::BuildFactory { .. } => {
            mix.build_factory += 1;
            // Sadece Sanayici fabrika kursun.
            if !matches!(kind_label, "Sanayici") {
                mix.forbidden_action_count += 1;
            }
        }
        Command::BuyCaravan { .. } => {
            mix.buy_caravan += 1;
            if !matches!(kind_label, "Tuccar") {
                mix.forbidden_action_count += 1;
            }
        }
        Command::DispatchCaravan { .. } => {
            mix.dispatch += 1;
            if !matches!(kind_label, "Tuccar") {
                mix.forbidden_action_count += 1;
            }
        }
        Command::ProposeContract(_) => mix.propose_contract += 1,
        Command::AcceptContract { .. } => mix.accept_contract += 1,
        Command::TakeLoan { .. } => mix.take_loan += 1,
        Command::RepayLoan { .. } => mix.repay_loan += 1,
        _ => {}
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
        Command::TakeLoan { amount, .. } => format!("TakeLoan {amount}"),
        Command::RepayLoan { loan_id, .. } => format!("RepayLoan #{}", loan_id.value()),
        Command::CreditNpcCash { amount, .. } => format!("CreditNpcCash {amount}"),
    }
}

/// Test scenario için `GameState` kur — insan + NPC'ler eklenmiş.
/// CLI seed mantığını basitleştirir; deterministic seed.
fn build_state(runner: &SimRunner) -> GameState {
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    let mut s = GameState::new(RoomId::new(runner.seed), RoomConfig::hizli());
    let mut rng = ChaCha8Rng::seed_from_u64(runner.seed);

    // price_baseline'ı doldur — domain'de hiç insert yok, BTreeMap boş başlıyor.
    // Sonuç: effective_baseline() hep None → arbitrage_price_cents fallback'i
    // sezon başında çalışmıyor → dead bucket'lar self-reinforcing oluyor.
    // Şema:
    // - Ham: lokal specialty 4₺, off-specialty 7₺ (şehir-uzmanlık spread'i)
    // - Mamul: şehir talebine göre farklılaştırılmış (gerçek hayat tedarik
    //   zinciri — lüks/popüler şehirde mamul pahalı, taşrada ucuz):
    //     * High talep (örn. İst-Kumas, Ank-Un) → 36₺
    //     * Normal talep                       → 28₺ (default)
    // Bu farklılaştırma Tüccar mamul arbitrajı için %25-30 spread yaratır.
        for city in CityId::ALL {
        let cheap = city.cheap_raw();
        for product in ProductKind::ALL {
            let lira = if product.is_finished() {
                match city.demand_for(product) {
                    DemandLevel::High => 36,
                    DemandLevel::Normal => 28,
                    DemandLevel::Low => 22,
                }
            } else if product == cheap {
                4
            } else {
                7
            };
            s.price_baseline
                .insert((city, product), Money::from_lira(lira).unwrap());
        }
    }

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

    // NPC-Sanayici — 30K cash 3 fab kuruluşunda (0+8+15=23K) tükeniyordu,
    // kalan 7K bankruptcy_risk "yuksek" tetikleyip buy_score=0 yapıyordu.
    // Sonuç: 5 NPC'den 3'ü hiç ham almıyordu → fabrikalar boş → mamul üretimi
    // düşük → match verim düştü. 50K → 23K kuruluş + 27K ham alma bütçesi.
    for _ in 0..runner.include_npcs.sanayici {
        let pers = pick_personality(&mut rng);
        let mut npc = Player::new(
            PlayerId::new(next_id),
            format!("Sanayici-{next_id}"),
            Role::Sanayici,
            Money::from_lira(50_000).unwrap(),
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

    // NPC-Esnaf (Toptancı) — Tuning v6: boş raf, sezon başı Çiftçi'den ham
    // almaya başlasın. 50K/5K başlangıç stoğu fuzzy `stock` signal'i hep
    // "yuksek" yapıp BUY skorunu baskılıyordu.
    for _ in 0..runner.include_npcs.esnaf {
        let npc = Player::new(
            PlayerId::new(next_id),
            format!("Esnaf-{next_id}"),
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(moneywar_domain::NpcKind::Esnaf);
        s.news_subscriptions
            .insert(PlayerId::new(next_id), NewsTier::Free);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Spekulator — 8K başlangıç stoğu hep "stock yuksek" yapıp SELL spam'ı
    // tetikliyordu (BUY:SELL = 1:3 asimetrisi). 2K'ya indirildi: piyasaya
    // başlangıç likidite verir ama market maker rolü kâr ettikçe BUY-SELL
    // dengeli kalır. Match verim hedefi %5+.
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
        distribute_inv(&mut npc, &mut rng, 2_000);
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

    // NPC-Ciftci (yeni v4) — uzman, sezonluk mahsul akışı
    for _ in 0..runner.include_npcs.ciftci {
        let npc = Player::new(
            PlayerId::new(next_id),
            format!("Ciftci-{next_id}"),
            Role::Tuccar,
            Money::from_lira(8_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(moneywar_domain::NpcKind::Ciftci);
        s.news_subscriptions
            .insert(PlayerId::new(next_id), NewsTier::Free);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Banka (yeni v4) — likidite sağlayıcı
    for _ in 0..runner.include_npcs.banka {
        let npc = Player::new(
            PlayerId::new(next_id),
            format!("Banka-{next_id}"),
            Role::Tuccar,
            Money::from_lira(200_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(moneywar_domain::NpcKind::Banka);
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
