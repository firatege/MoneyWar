//! Denge denetimi — "oyun adil mi?" sorusunun ölçülebilir hali.
//!
//! [`metrics`](crate::metrics) yoğunlaşmayı (kim domine ediyor), [`drama`](crate::drama)
//! hikâye üretimini ölçer. Burası üçüncü ekseni ölçer: **rol adaleti ve
//! piyasa mekaniğinin sağlığı**.
//!
//! - Rol adaleti: kişi başı `PnL` — 4 Tüccar'ın toplamı 10 Sanayici'yi geçiyorsa
//!   toplam `PnL` yanıltır, kişi başı yanıltmaz.
//! - Emir akışı: gönderilen emrin kaçı doldu, kaçı TTL'den düştü, kaçı daha
//!   motora girmeden reddedildi. Reddedilenler sebep sınıfına göre toplanır.
//! - Fiyat kayması: sezon sonu baseline / sezon başı baseline. 1.0 = stabil.
//!
//! Biriktirici tick loop'unda beslenir, `finalize` sezon sonu state'ten
//! servet tarafını okur.

use std::collections::BTreeMap;

use moneywar_domain::{CityId, Command, GameState, NpcKind, PlayerId, ProductKind};
use moneywar_engine::{LogEvent, leaderboard};

/// Kâr amacı güden roller — adalet makası yalnız bunlar arasında anlamlıdır.
/// Alıcı (tüketici sink) ve Banka (likidite sağlayıcı) hariç.
pub const PROFIT_ROLES: [NpcKind; 4] = [
    NpcKind::Sanayici,
    NpcKind::Tuccar,
    NpcKind::Spekulator,
    NpcKind::Ciftci,
];

/// Raporda sabit sırada gösterilen roller — çıktı oyunlar arası karşılaştırılabilir olsun.
pub const ROLE_ORDER: [NpcKind; 6] = [
    NpcKind::Sanayici,
    NpcKind::Tuccar,
    NpcKind::Spekulator,
    NpcKind::Ciftci,
    NpcKind::Alici,
    NpcKind::Banka,
];

// =============================================================================
// Biriktirici
// =============================================================================

/// Bir rolün sezon boyunca biriken emir akışı sayaçları.
#[derive(Debug, Default, Clone, Copy)]
pub struct FlowCounters {
    /// Motorun kabul ettiği `SubmitOrder` sayısı.
    pub submitted: u64,
    /// Bu rolün alıcı ya da satıcı olduğu fill sayısı.
    pub fills: u64,
    /// Fill'lerde el değiştiren birim.
    pub filled_qty: u64,
    /// TTL dolup kitaptan düşen emir sayısı.
    pub expired: u64,
    /// TTL dolduğunda eşleşmemiş kalan birim.
    pub expired_qty: u64,
    /// Motora girmeden reddedilen komut sayısı.
    pub rejected: u64,
}

impl FlowCounters {
    /// Emir başına düşen fill sayısı — doluluk yoğunluğu.
    ///
    /// Bir emir kısmi eşleşmelerle birden çok fill üretebildiği için **1'i
    /// aşabilir**; yüzde değildir. Yüksek değer emirlerin karşılık bulduğunu,
    /// sıfıra yakın değer kitapta öylece beklediğini gösterir.
    #[must_use]
    pub fn fills_per_order(&self) -> f64 {
        if self.submitted == 0 {
            0.0
        } else {
            self.fills as f64 / self.submitted as f64
        }
    }

    /// Denenen komutun (kabul + red) kaçta kaçı boşa gitti.
    #[must_use]
    pub fn reject_ratio(&self) -> f64 {
        let attempts = self.submitted + self.rejected;
        if attempts == 0 {
            0.0
        } else {
            self.rejected as f64 / attempts as f64
        }
    }
}

/// Tick loop'unda beslenen denge biriktiricisi.
#[derive(Debug, Default)]
pub struct BalanceAccumulator {
    flow: BTreeMap<NpcKind, FlowCounters>,
    /// Sebep sınıfı → red sayısı. Sınıf için bkz. [`classify_reason`].
    reject_reasons: BTreeMap<String, u64>,
    /// Settlement aşamasında uygulanamayan fill sayısı (nakit/stok yetmedi).
    fill_rejected: u64,
    /// Fabrika atıl tick sayısı — üretim zinciri tıkanıklığının doğrudan ölçüsü.
    factory_idle_ticks: u64,
    /// Başlatılan / tamamlanan üretim batch sayısı.
    production_started: u64,
    production_completed: u64,
    /// Kredi akışı — bankanın gerçekten çalışıp çalışmadığı.
    loans_taken: u64,
    loans_defaulted: u64,
    /// Ürün başına piyasa akışı — arz/talep dengesizliğinin tek kaynağı.
    market: BTreeMap<ProductKind, MarketFlow>,
    /// Ham madde alımı: rol → alınan birim. "Fabrika neden aç" sorusunun
    /// ikinci yarısı — ham üretiliyor ama kim kapıyor?
    raw_buy_by_role: BTreeMap<NpcKind, u64>,
    /// Rol → (şehir, ürün) → (alınan, satılan). Aynı pazarda hem alıp hem
    /// satmak = çevirme (flip): mal ne dönüştürülüyor ne taşınıyor.
    churn: BTreeMap<NpcKind, ChurnBuckets>,
    /// Ürün → rol → (alınan, satılan) birim. "Bu malı kim üretip kim
    /// tüketiyor" defteri — zincirin nerede koptuğunu gösterir.
    product_flow: BTreeMap<ProductKind, BTreeMap<NpcKind, (u64, u64)>>,

    // ── Para akışı (emek piyasası / banka çalışması için zemin) ───────────────
    /// Maaş + ücret olarak dağıtılan toplam (cent). Şu an iki kanal karışık:
    /// Sanayici→Alıcı fabrika ücreti (transfer) ve Alıcı sabit maaşı (basım).
    salary_paid_cents: i64,
    /// Açılan kredilerin toplam anaparası (cent).
    loan_principal_cents: i64,
    /// Temerrütte silinen borç (cent) — sistemden kaybolan para.
    loan_written_off_cents: i64,
    /// Sermaye harcaması (cent): fabrika kurulum + yükseltme, kervan, tarla.
    /// Para kimseye ödenmez, varlığa dönüşür — dolaşımdan çıkar.
    capex_cents: i64,
    /// İşletme gideri (cent): depolama. Bakım motorda loglanmıyor.
    opex_cents: i64,
    /// Tick örnekleriyle toplam nakit arzı (cent). Deflasyon/enflasyon eğrisi.
    money_samples: Vec<i64>,
}

/// Bir ürünün alıcı/satıcı dökümü — "kim üretiyor, kim tüketiyor".
#[derive(Debug, Clone)]
pub struct ProductLedger {
    pub product: ProductKind,
    /// El değiştiren toplam birim.
    pub matched: u64,
    /// Rol → alınan birim, azalan.
    pub buyers: Vec<(NpcKind, u64)>,
    /// Rol → satılan birim, azalan.
    pub sellers: Vec<(NpcKind, u64)>,
}

/// Bir rolün pazar başına (alınan, satılan) miktarı.
type ChurnBuckets = BTreeMap<(CityId, ProductKind), (u64, u64)>;

/// Sezon boyunca para arzı ve onu değiştiren kalemler (hepsi cent).
///
/// Emek piyasası ve banka çalışmasının zemini: kapalı döngüye geçince para
/// arzının sabit kalıp kalmadığı, deflasyona girip girmediği buradan görülür.
#[derive(Debug, Default, Clone, Copy)]
pub struct MoneyFlow {
    pub supply_start: i64,
    pub supply_end: i64,
    pub supply_min: i64,
    pub supply_max: i64,
    /// Maaş + ücret olarak dağıtılan toplam.
    pub salary_paid: i64,
    /// Açılan kredilerin anaparası.
    pub loan_principal: i64,
    /// Temerrütte silinen borç.
    pub loan_written_off: i64,
    /// Sermaye harcaması: fabrika + yükseltme + kervan + tarla.
    pub capex: i64,
    /// İşletme gideri (depolama).
    pub opex: i64,
}

impl MoneyFlow {
    /// Sezon boyunca para arzının yüzde değişimi. Negatif = ekonomi kurudu.
    #[must_use]
    pub fn supply_change_pct(&self) -> f64 {
        if self.supply_start == 0 {
            return 0.0;
        }
        (self.supply_end - self.supply_start) as f64 / self.supply_start as f64 * 100.0
    }
}

/// Ekonomideki toplam nakit — tüm oyuncuların kasası (cent).
///
/// Stok, fabrika ve escrow hariç; yalnız likit para. Para arzının sezon
/// boyunca büyüyüp küçülmesini izlemek için.
#[must_use]
pub fn money_supply_cents(state: &GameState) -> i64 {
    state.players.values().map(|p| p.cash.as_cents()).sum()
}

/// Bir ürünün sezon boyunca kitapta görülen arz/talebi ve gerçekleşen hacmi.
///
/// **Dikkat — `demand`/`supply` mutlak değer olarak okunmaz.** Kaynak
/// `MarketCleared`, her clear pass'te kitaptaki *açık* emirleri raporlar;
/// TTL'i 3 tick olan bir emir 3 pass'te 3 kez sayılır. Dolayısıyla:
/// - [`demand_supply_ratio`](Self::demand_supply_ratio) güvenilir (şişme iki
///   tarafta da aynı yönde çalışır),
/// - [`supply_clear_rate`](Self::supply_clear_rate) **alt sınırdır** — gerçek
///   oran yaklaşık `ort. TTL` katı kadar yüksektir,
/// - [`priced_rate`](Self::priced_rate) tam doğrudur (pass sayımı).
#[derive(Debug, Default, Clone, Copy)]
pub struct MarketFlow {
    /// Kitapta görülen toplam alış miktarı (TTL boyunca tekrar sayılır).
    pub demand: u64,
    /// Kitapta görülen toplam satış miktarı (TTL boyunca tekrar sayılır).
    pub supply: u64,
    /// El değiştiren miktar.
    pub matched: u64,
    /// Kaç clear pass yapıldı (şehir × tick).
    pub passes: u64,
    /// Kaçında fiyat oluştu (eşleşme vardı).
    pub priced_passes: u64,
}

impl MarketFlow {
    /// Talep / arz. >1 arz açığı, <1 arz fazlası.
    #[must_use]
    pub fn demand_supply_ratio(&self) -> f64 {
        if self.supply == 0 {
            f64::INFINITY
        } else {
            self.demand as f64 / self.supply as f64
        }
    }

    /// Kitapta görülen arzın kaçta kaçı satıldı — eşleşme veriminin **alt sınırı**
    /// (paydadaki TTL tekrar sayımı yüzünden gerçek oran daha yüksektir).
    #[must_use]
    pub fn supply_clear_rate(&self) -> f64 {
        if self.supply == 0 {
            0.0
        } else {
            self.matched as f64 / self.supply as f64
        }
    }

    /// Clear pass'lerin kaçında fiyat oluştu (piyasanın canlılığı).
    #[must_use]
    pub fn priced_rate(&self) -> f64 {
        if self.passes == 0 {
            0.0
        } else {
            self.priced_passes as f64 / self.passes as f64
        }
    }
}

impl BalanceAccumulator {
    /// Bir tick raporundaki tüm olayları işle.
    ///
    /// `state` olayın **uygulandığı sonraki** durum; oyuncu → rol çözümü
    /// için kullanılır (iflas eden oyuncu da haritada kalır).
    pub fn record(&mut self, state: &GameState, event: &LogEvent) {
        match event {
            LogEvent::CommandAccepted {
                command: Command::SubmitOrder(order),
            } => {
                self.entry(state, order.player).submitted += 1;
            }
            LogEvent::CommandRejected { command, reason } => {
                if let Some(actor) = command_actor(command) {
                    self.entry(state, actor).rejected += 1;
                }
                *self
                    .reject_reasons
                    .entry(classify_reason(reason))
                    .or_default() += 1;
            }
            LogEvent::OrderMatched {
                city,
                product,
                buyer,
                seller,
                quantity,
                ..
            } => {
                let q = u64::from(*quantity);
                for side in [*buyer, *seller] {
                    let c = self.entry(state, side);
                    c.fills += 1;
                    c.filled_qty += q;
                }
                let kind_of = |pid: &PlayerId| state.players.get(pid).and_then(|p| p.npc_kind);
                if product.is_raw()
                    && let Some(kind) = kind_of(buyer)
                {
                    *self.raw_buy_by_role.entry(kind).or_default() += q;
                }
                if let Some(kind) = kind_of(buyer) {
                    self.churn
                        .entry(kind)
                        .or_default()
                        .entry((*city, *product))
                        .or_default()
                        .0 += q;
                }
                if let Some(kind) = kind_of(seller) {
                    self.churn
                        .entry(kind)
                        .or_default()
                        .entry((*city, *product))
                        .or_default()
                        .1 += q;
                }
                // Ürün defteri: bu malı kim alıp kim satıyor.
                let pf = self.product_flow.entry(*product).or_default();
                if let Some(kind) = kind_of(buyer) {
                    pf.entry(kind).or_default().0 += q;
                }
                if let Some(kind) = kind_of(seller) {
                    pf.entry(kind).or_default().1 += q;
                }
            }
            LogEvent::OrderExpired {
                player,
                leftover_qty,
                ..
            } => {
                let c = self.entry(state, *player);
                c.expired += 1;
                c.expired_qty += u64::from(*leftover_qty);
            }
            LogEvent::MarketCleared {
                product,
                clearing_price,
                matched_qty,
                submitted_buy_qty,
                submitted_sell_qty,
                ..
            } => {
                let f = self.market.entry(*product).or_default();
                f.demand += u64::from(*submitted_buy_qty);
                f.supply += u64::from(*submitted_sell_qty);
                f.matched += u64::from(*matched_qty);
                f.passes += 1;
                if clearing_price.is_some() {
                    f.priced_passes += 1;
                }
            }
            LogEvent::FillRejected { .. } => self.fill_rejected += 1,
            LogEvent::FactoryIdle { .. } => self.factory_idle_ticks += 1,
            LogEvent::ProductionStarted { .. } => self.production_started += 1,
            LogEvent::ProductionCompleted { .. } => self.production_completed += 1,
            LogEvent::EconomySalary { amount, .. } => {
                self.salary_paid_cents = self.salary_paid_cents.saturating_add(amount.as_cents());
            }
            LogEvent::FactoryBuilt { cost, .. }
            | LogEvent::FactoryUpgraded { cost, .. }
            | LogEvent::CaravanBought { cost, .. }
            | LogEvent::PrivateFarmBuilt { cost, .. } => {
                self.capex_cents = self.capex_cents.saturating_add(cost.as_cents());
            }
            LogEvent::StorageCost { amount, .. } => {
                self.opex_cents = self.opex_cents.saturating_add(amount.as_cents());
            }
            LogEvent::LoanTaken { principal, .. } => {
                self.loans_taken += 1;
                self.loan_principal_cents =
                    self.loan_principal_cents.saturating_add(principal.as_cents());
            }
            LogEvent::LoanDefaulted { unpaid_balance, .. } => {
                self.loans_defaulted += 1;
                self.loan_written_off_cents = self
                    .loan_written_off_cents
                    .saturating_add(unpaid_balance.as_cents());
            }
            _ => {}
        }
    }

    /// Bu tick'in toplam nakit arzını kaydet — sezon boyu eğri için.
    pub fn sample_money(&mut self, state: &GameState) {
        self.money_samples.push(money_supply_cents(state));
    }

    /// Oyuncunun rolüne ait sayaç girdisi; rolsüz oyuncu (insan) yok sayılır.
    fn entry(&mut self, state: &GameState, pid: PlayerId) -> &mut FlowCounters {
        let kind = state
            .players
            .get(&pid)
            .and_then(|p| p.npc_kind)
            .unwrap_or(NpcKind::Alici);
        self.flow.entry(kind).or_default()
    }

    /// Sezon sonu durumundan servet tarafını okuyup raporu üret.
    #[must_use]
    pub fn finalize(&self, state: &GameState) -> BalanceReport {
        let scores = leaderboard(state);

        // Rol → PnL listesi (lira).
        let mut pnl_by_role: BTreeMap<NpcKind, Vec<f64>> = BTreeMap::new();
        for s in &scores {
            if let Some(kind) = state.players.get(&s.player_id).and_then(|p| p.npc_kind) {
                pnl_by_role
                    .entry(kind)
                    .or_default()
                    .push(s.total.as_cents() as f64 / 100.0);
            }
        }

        let roles: Vec<RoleBalance> = ROLE_ORDER
            .iter()
            .filter_map(|&kind| {
                let pnls = pnl_by_role.get(&kind)?;
                if pnls.is_empty() {
                    return None;
                }
                let n = pnls.len();
                let total: f64 = pnls.iter().sum();
                let bankrupt = state
                    .intrigue
                    .bankrupt
                    .iter()
                    .filter(|pid| {
                        state.players.get(pid).and_then(|p| p.npc_kind) == Some(kind)
                    })
                    .count();
                Some(RoleBalance {
                    kind,
                    count: n,
                    pnl_total: total,
                    pnl_per_capita: total / n as f64,
                    pnl_min: pnls.iter().copied().fold(f64::INFINITY, f64::min),
                    pnl_max: pnls.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    losers: pnls.iter().filter(|v| **v < 0.0).count(),
                    bankrupt,
                    flow: self.flow.get(&kind).copied().unwrap_or_default(),
                })
            })
            .collect();

        // Fiyat kayması: ürün başına (sezon sonu / sezon başı) baseline oranı,
        // şehirler üzerinden ortalama.
        let mut price_drift: Vec<(ProductKind, f64)> = ProductKind::ALL
            .iter()
            .filter_map(|&product| {
                let ratios: Vec<f64> = state
                    .price_baseline
                    .iter()
                    .filter(|((_, p), _)| *p == product)
                    .filter_map(|((city, _), now)| {
                        let start = state.price_baseline_initial.get(&(*city, product))?;
                        if start.as_cents() == 0 {
                            return None;
                        }
                        Some(now.as_cents() as f64 / start.as_cents() as f64)
                    })
                    .collect();
                if ratios.is_empty() {
                    return None;
                }
                Some((
                    product,
                    ratios.iter().sum::<f64>() / ratios.len() as f64,
                ))
            })
            .collect();
        price_drift.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut top_rejects: Vec<(String, u64)> =
            self.reject_reasons.iter().map(|(k, v)| (k.clone(), *v)).collect();
        top_rejects.sort_by(|a, b| b.1.cmp(&a.1));
        top_rejects.truncate(4);

        // Piyasa akışı — arz açığı en büyük olan üstte.
        let mut market: Vec<(ProductKind, MarketFlow)> =
            self.market.iter().map(|(k, v)| (*k, *v)).collect();
        market.sort_by(|a, b| {
            b.1.demand_supply_ratio()
                .partial_cmp(&a.1.demand_supply_ratio())
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let raw_buy_by_role = ROLE_ORDER
            .iter()
            .filter_map(|&k| self.raw_buy_by_role.get(&k).map(|v| (k, *v)))
            .collect();

        // Çevirme oranı: aynı pazarda hem alınıp hem satılan miktar
        // (`min(alınan, satılan)`) / toplam alınan. Üretici için düşük olmalı
        // — aldığı hammaddeyi tüketir, sattığı mamulü üretir. Aracı için
        // yüksek: mal ne dönüşür ne taşınır, sadece el değiştirir.
        let churn_by_role = ROLE_ORDER
            .iter()
            .filter_map(|&kind| {
                let buckets = self.churn.get(&kind)?;
                let bought: u64 = buckets.values().map(|(b, _)| *b).sum();
                let flipped: u64 = buckets.values().map(|(b, s)| *b.min(s)).sum();
                if bought == 0 {
                    return None;
                }
                Some((kind, flipped as f64 / bought as f64))
            })
            .collect();

        let money = MoneyFlow {
            supply_start: self.money_samples.first().copied().unwrap_or(0),
            supply_end: self.money_samples.last().copied().unwrap_or(0),
            supply_min: self.money_samples.iter().copied().min().unwrap_or(0),
            supply_max: self.money_samples.iter().copied().max().unwrap_or(0),
            salary_paid: self.salary_paid_cents,
            loan_principal: self.loan_principal_cents,
            loan_written_off: self.loan_written_off_cents,
            capex: self.capex_cents,
            opex: self.opex_cents,
        };

        // Ürün defteri: rol payları, akış büyüklüğüne göre sıralı.
        let product_flow = ProductKind::ALL
            .iter()
            .filter_map(|p| {
                let per_role = self.product_flow.get(p)?;
                let total: u64 = per_role.values().map(|(b, _)| *b).sum();
                if total == 0 {
                    return None;
                }
                let mut buyers: Vec<(NpcKind, u64)> =
                    per_role.iter().map(|(k, (b, _))| (*k, *b)).filter(|(_, v)| *v > 0).collect();
                let mut sellers: Vec<(NpcKind, u64)> =
                    per_role.iter().map(|(k, (_, s))| (*k, *s)).filter(|(_, v)| *v > 0).collect();
                buyers.sort_by(|a, b| b.1.cmp(&a.1));
                sellers.sort_by(|a, b| b.1.cmp(&a.1));
                Some(ProductLedger { product: *p, matched: total, buyers, sellers })
            })
            .collect();

        BalanceReport {
            roles,
            product_flow,
            raw_buy_by_role,
            churn_by_role,
            money,
            market,
            price_drift,
            top_rejects,
            fill_rejected: self.fill_rejected,
            factory_idle_ticks: self.factory_idle_ticks,
            production_started: self.production_started,
            production_completed: self.production_completed,
            loans_taken: self.loans_taken,
            loans_defaulted: self.loans_defaulted,
        }
    }
}

/// Komutun sahibini çıkar — red sayacı doğru role yazılsın diye.
fn command_actor(command: &Command) -> Option<PlayerId> {
    match command {
        Command::SubmitOrder(order) => Some(order.player),
        Command::CancelOrder { requester, .. }
        | Command::CancelContractProposal { requester, .. } => Some(*requester),
        Command::AcceptContract { acceptor, .. } => Some(*acceptor),
        Command::BuildFactory { owner, .. } => Some(*owner),
        _ => None,
    }
}

/// Red sebebini sınıfa indirger — histogram oyuncu/ürün detayında patlamasın.
///
/// `"domain: validation failed: relist cooldown active for PLY-113 Un at Bursa: 2 tick kaldı"`
/// → `"relist cooldown active"`.
#[must_use]
pub fn classify_reason(reason: &str) -> String {
    let tail = reason
        .rsplit_once("validation failed: ")
        .map_or(reason, |(_, t)| t);
    let cut = tail
        .find(" for ")
        .or_else(|| tail.find(':'))
        .unwrap_or(tail.len());
    tail[..cut].trim().to_owned()
}

// =============================================================================
// Rapor
// =============================================================================

/// Tek rolün denge satırı.
#[derive(Debug, Clone)]
pub struct RoleBalance {
    pub kind: NpcKind,
    /// Roldeki oyuncu sayısı.
    pub count: usize,
    pub pnl_total: f64,
    /// Adalet ölçüsü — roller arası tek karşılaştırılabilir sayı.
    pub pnl_per_capita: f64,
    pub pnl_min: f64,
    pub pnl_max: f64,
    /// `PnL`'i negatif kapatan oyuncu sayısı.
    pub losers: usize,
    /// İflas damgası yemiş oyuncu sayısı.
    pub bankrupt: usize,
    pub flow: FlowCounters,
}

/// Bir oyunun denge denetimi.
#[derive(Debug, Clone)]
pub struct BalanceReport {
    pub roles: Vec<RoleBalance>,
    /// Ürün → piyasa akışı, arz açığı azalan sırada.
    pub market: Vec<(ProductKind, MarketFlow)>,
    /// Rol → satın alınan ham madde birimi ([`ROLE_ORDER`] sırasında).
    pub raw_buy_by_role: Vec<(NpcKind, u64)>,
    /// Ürün başına alıcı/satıcı rol dökümü.
    pub product_flow: Vec<ProductLedger>,
    /// Rol → çevirme oranı: aldığının ne kadarını **aynı pazarda** geri sattı.
    /// 0 = hep dönüştürüyor/taşıyor, 1 = saf aracılık.
    pub churn_by_role: Vec<(NpcKind, f64)>,
    /// Para arzı ve akış kalemleri.
    pub money: MoneyFlow,
    /// Ürün → sezon sonu/başı baseline oranı, azalan.
    pub price_drift: Vec<(ProductKind, f64)>,
    /// En sık 4 red sebebi sınıfı.
    pub top_rejects: Vec<(String, u64)>,
    pub fill_rejected: u64,
    pub factory_idle_ticks: u64,
    pub production_started: u64,
    pub production_completed: u64,
    pub loans_taken: u64,
    pub loans_defaulted: u64,
}

impl BalanceReport {
    /// Kâr amaçlı roller arasında en zengin ÷ en fakir kişi başı `PnL`.
    /// 1.0 = tam adalet.
    ///
    /// Yalnızca [`PROFIT_ROLES`] kıyaslanır: Alıcı tasarım gereği tüketici
    /// (mal alıp yok eder, `PnL`'i hep negatif), Banka likidite sağlayıcı.
    /// İkisi dahil edilirse oran negatif bölene düşer ve anlamsız büyür.
    ///
    /// En fakir rol de zarardaysa makas ölçülemez → `None`.
    #[must_use]
    pub fn fairness_spread(&self) -> Option<f64> {
        let vals: Vec<f64> = self
            .roles
            .iter()
            .filter(|r| PROFIT_ROLES.contains(&r.kind))
            .map(|r| r.pnl_per_capita)
            .collect();
        let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
        if !min.is_finite() || !max.is_finite() || min <= 0.0 {
            return None;
        }
        Some(max / min)
    }
}

// =============================================================================
// Testler
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_reason_strips_player_and_location_detail() {
        let raw = "domain: validation failed: relist cooldown active for PLY-113 Un at Bursa: 2 tick kaldı";
        assert_eq!(classify_reason(raw), "relist cooldown active");
    }

    #[test]
    fn classify_reason_cuts_at_colon_when_no_for_clause() {
        let raw = "domain: validation failed: insufficient funds: need 500";
        assert_eq!(classify_reason(raw), "insufficient funds");
    }

    #[test]
    fn classify_reason_passes_through_plain_text() {
        assert_eq!(classify_reason("capacity exceeded"), "capacity exceeded");
    }

    #[test]
    fn fills_per_order_is_zero_without_submissions() {
        assert_eq!(FlowCounters::default().fills_per_order(), 0.0);
    }

    #[test]
    fn fills_per_order_can_exceed_one_on_partial_fills() {
        let c = FlowCounters { submitted: 2, fills: 5, ..FlowCounters::default() };
        assert!((c.fills_per_order() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn reject_ratio_counts_rejects_against_all_attempts() {
        let c = FlowCounters {
            submitted: 3,
            rejected: 1,
            ..FlowCounters::default()
        };
        assert!((c.reject_ratio() - 0.25).abs() < 1e-9);
    }

    /// Yalnız `kind` ve kişi başı PnL'i anlamlı olan test satırı.
    fn role(kind: NpcKind, per_capita: f64) -> RoleBalance {
        RoleBalance {
            kind,
            count: 1,
            pnl_total: per_capita,
            pnl_per_capita: per_capita,
            pnl_min: per_capita,
            pnl_max: per_capita,
            losers: 0,
            bankrupt: 0,
            flow: FlowCounters::default(),
        }
    }

    fn report_of(roles: Vec<RoleBalance>) -> BalanceReport {
        BalanceReport {
            roles,
            product_flow: Vec::new(),
            raw_buy_by_role: Vec::new(),
            churn_by_role: Vec::new(),
            money: MoneyFlow::default(),
            market: Vec::new(),
            price_drift: Vec::new(),
            top_rejects: Vec::new(),
            fill_rejected: 0,
            factory_idle_ticks: 0,
            production_started: 0,
            production_completed: 0,
            loans_taken: 0,
            loans_defaulted: 0,
        }
    }

    #[test]
    fn fairness_spread_flags_perfect_parity_as_one() {
        let report = report_of(vec![
            role(NpcKind::Tuccar, 100.0),
            role(NpcKind::Sanayici, 100.0),
        ]);
        assert!((report.fairness_spread().unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn fairness_spread_reports_ratio_between_richest_and_poorest() {
        let report = report_of(vec![
            role(NpcKind::Tuccar, 500_000.0),
            role(NpcKind::Sanayici, 50_000.0),
            // Kâr rolü olmayanlar hariç — dahil olsalar makas 50× çıkardı.
            role(NpcKind::Banka, 10_000.0),
            role(NpcKind::Alici, -40_000.0),
        ]);
        assert!((report.fairness_spread().unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn fairness_spread_is_none_when_poorest_profit_role_loses_money() {
        let report = report_of(vec![
            role(NpcKind::Tuccar, 500_000.0),
            role(NpcKind::Sanayici, -1_000.0),
        ]);
        assert_eq!(report.fairness_spread(), None);
    }

    #[test]
    fn market_flow_ratio_flags_supply_shortage() {
        let f = MarketFlow {
            demand: 400,
            supply: 100,
            matched: 50,
            passes: 10,
            priced_passes: 4,
        };
        assert!((f.demand_supply_ratio() - 4.0).abs() < 1e-9);
        assert!((f.supply_clear_rate() - 0.5).abs() < 1e-9);
        assert!((f.priced_rate() - 0.4).abs() < 1e-9);
    }

    #[test]
    fn market_flow_without_supply_is_infinite_shortage() {
        let f = MarketFlow { demand: 10, ..MarketFlow::default() };
        assert!(f.demand_supply_ratio().is_infinite());
        assert_eq!(f.supply_clear_rate(), 0.0);
    }
}
