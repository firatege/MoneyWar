//! Anlatı dedektörü (docs/finish-plan.md Faz 1) — tick kapanışında pazar
//! verisinden **gözlemlenebilir entrika gerçeklerini** çıkarır ve hikâye
//! event'i olarak damgalar.
//!
//! Tasarım ilkesi: motor niyet okumaz, davranış okur. Bir NPC'nin beyninde
//! "`PriceWar`" hedefi olması olay değildir; art arda 5 tick rakibinin
//! fiyatını gerçekten kırması olaydır. Böylece insan oyuncular da aynı
//! olayları tetikleyebilir ve dedektör determinizmi beyin implementasyonuna
//! bağlanmaz.
//!
//! Çıkarılan olaylar:
//! - `MonopolyFormed` / `MonopolyBroken` — kayan satış penceresinde pay
//!   eşiği (histerezisli: %60 kur, %50 kır).
//! - `UndercutCampaign` — 3 ardışık tick fiyat kırma; kin doğurur (`GrudgeFormed`).
//! - `PriceWarDeclared` — kampanya 5 tick'e ulaştı.
//! - `PriceWarWon` — mağdur 5 tick pazardan çekildi ya da iflas etti.
//! - `FirmBankrupt` — nakit + envanter + üretim varlığı tükendi.
//!
//! `clear_markets`'tan SONRA çağrılır: bu tick'in eşleşmeleri ve kabul
//! edilen emirleri raporda hazırdır.

use std::collections::{BTreeMap, BTreeSet};

use moneywar_domain::{
    CityId, Command, DEFAULTS_BEFORE_BANKRUPTCY, DOMINANCE_MIN_VOLUME, DOMINANCE_WINDOW_TICKS,
    GRUDGE_TICKS, GameState, INSOLVENCY_CONFIRM_TICKS, INSOLVENT_CASH_CENTS,
    MONOPOLY_BREAK_CONFIRM_TICKS, MONOPOLY_BREAK_PCT, MONOPOLY_CONFIRM_TICKS, MONOPOLY_FORM_PCT,
    CARTEL_CONFIRM_TICKS, CARTEL_JOINT_SHARE_PCT, CARTEL_MIN_ASK_PCT, NpcKind, OrderSide,
    PRICE_WAR_DECLARE_TICKS, PRICE_WAR_FIZZLE_TICKS, PRICE_WAR_RETREAT_TICKS,
    PlayerId, PriceWarTrack, ProductKind, Tick, TickSales, UNDERCUT_CAMPAIGN_TICKS,
};

use crate::report::{LogEntry, LogEvent, TickReport};

/// Bir olay anlatı (hikâye) olayı mı? Skorkart, feed ve harita bu listeyi
/// tek kaynak olarak kullanır — yeni anlatı olayı eklendiğinde burası da
/// güncellenmeli.
#[must_use]
pub const fn is_story_event(event: &LogEvent) -> bool {
    matches!(
        event,
        LogEvent::MonopolyFormed { .. }
            | LogEvent::MonopolyBroken { .. }
            | LogEvent::UndercutCampaign { .. }
            | LogEvent::PriceWarDeclared { .. }
            | LogEvent::PriceWarWon { .. }
            | LogEvent::FirmBankrupt { .. }
            | LogEvent::GrudgeFormed { .. }
            | LogEvent::SupplyChoke { .. }
            | LogEvent::CartelFormed { .. }
            | LogEvent::CartelBetrayed { .. }
    )
}

/// Anlatı olayını izleyicinin okuyacağı Türkçe manşete çevir.
/// Anlatı olayı değilse `None`. Sim özeti, web ticker'ı ve harita aynı
/// metni buradan alır — "kim ne yapıyor" tek yerde tanımlıdır.
#[must_use]
pub fn story_headline(state: &GameState, event: &LogEvent) -> Option<String> {
    let who = |id: PlayerId| -> String {
        state
            .players
            .get(&id)
            .map_or_else(|| format!("#{}", id.value()), |p| p.name.clone())
    };
    let market =
        |c: CityId, p: ProductKind| format!("{} {}", c.display_name(), p.display_name());
    /// Parayı manşette okunur biçime çevirir: 9412 cent → "94,12₺".
    fn lira(m: moneywar_domain::Money) -> String {
        format!("{:.2}₺", m.as_cents() as f64 / 100.0).replace('.', ",")
    }
    /// Kırma yüzdesi — mağdurun fiyatının kaç puan altına inildi.
    fn discount_pct(victim: moneywar_domain::Money, attacker: moneywar_domain::Money) -> i64 {
        let v = victim.as_cents();
        if v <= 0 {
            return 0;
        }
        ((v - attacker.as_cents()) * 100 / v).max(0)
    }

    let text = match event {
        LogEvent::MonopolyFormed { city, product, firm, share_percent } => format!(
            "{} {} pazarını ele geçirdi (%{share_percent})",
            who(*firm),
            market(*city, *product),
        ),
        LogEvent::MonopolyBroken { city, product, former, breaker } => match breaker {
            Some(b) => format!(
                "{}, {} tekelini kırdı — {} düştü",
                who(*b),
                market(*city, *product),
                who(*former),
            ),
            None => format!(
                "{} pazarındaki {} tekeli sona erdi",
                market(*city, *product),
                who(*former),
            ),
        },
        // Manşetlerde sayı olması bilinçli: "3 tick'tir kırıyor" izleyiciye
        // ne kadar sert kırdığını söylemiyordu. Fiyatlar tespit anında
        // zaten elde, olaya da kondu.
        LogEvent::UndercutCampaign {
            city,
            product,
            attacker,
            victim,
            ticks,
            attacker_ask,
            victim_ask,
        } => format!(
            "{}, {} pazarında {} fiyatını {ticks} tick'tir kırıyor — {} yerine {} (%{} altı)",
            who(*attacker),
            market(*city, *product),
            who(*victim),
            lira(*victim_ask),
            lira(*attacker_ask),
            discount_pct(*victim_ask, *attacker_ask),
        ),
        LogEvent::PriceWarDeclared {
            city,
            product,
            attacker,
            target,
            attacker_ask,
            target_ask,
        } => format!(
            "{}, {} pazarında {} firmasına fiyat savaşı açtı — {} → {} (%{} altı)",
            who(*attacker),
            market(*city, *product),
            who(*target),
            lira(*target_ask),
            lira(*attacker_ask),
            discount_pct(*target_ask, *attacker_ask),
        ),
        LogEvent::PriceWarWon { city, product, winner, loser } => format!(
            "{} savaşı kazandı — {} {} pazarından çekildi",
            who(*winner),
            who(*loser),
            market(*city, *product),
        ),
        LogEvent::FirmBankrupt { firm } => format!("{} iflas etti", who(*firm)),
        LogEvent::GrudgeFormed { holder, against } => {
            format!("{}, {} firmasına diş biledi", who(*holder), who(*against))
        }
        LogEvent::SupplyChoke { city, product, choker, victim } => format!(
            "{} tedariki kesti — {} fabrikası {} olmadan kaldı",
            who(*choker),
            who(*victim),
            market(*city, *product),
        ),
        LogEvent::CartelFormed { city, product, a, b } => format!(
            "{} ile {}, {} pazarında el sıkıştı",
            who(*a),
            who(*b),
            market(*city, *product),
        ),
        LogEvent::CartelBetrayed { city, product, betrayer, victim } => format!(
            "{} anlaşmayı bozdu — {} {} pazarında sırtından bıçaklandı",
            who(*betrayer),
            who(*victim),
            market(*city, *product),
        ),
        _ => return None,
    };
    Some(text)
}

/// Undercut sayılması için saldırgan fiyatı mağdurunkinin en az bu yüzdesi
/// kadar ALTINDA olmalı (98 → %2'den derin kırma). Yuvarlama gürültüsünü eler.
const UNDERCUT_MAX_PCT_OF_VICTIM: i64 = 98;
/// Pencerede "yerleşik" (undercut mağduru olabilir) sayılmak için gereken pay (%).
const INCUMBENT_MIN_SHARE_PCT: u64 = 30;

/// Tick kapanış dedektörü — `clear_markets` sonrası çağrılır.
/// `state.intrigue`'i günceller, anlatı event'lerini `report`'a ekler.
pub fn detect_intrigue(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let sales = collect_tick_sales(state, report);
    let asks = collect_tick_asks(report);

    update_sales_window(state, &sales, tick);
    detect_monopolies(state, report, tick);
    let undercuts = detect_undercuts(state, report, &asks, tick);
    advance_price_wars(state, report, &asks, &undercuts, tick);
    detect_cartels(state, report, &asks, &undercuts, tick);
    detect_supply_chokes(state, report, tick);
    decay_grudges(state);
    detect_bankruptcies(state, report, tick);
}

/// Tedarik boğma: bir fabrika girdi yokluğundan atıl kalıyor **ve** o girdinin
/// pazarını başka bir firma tekelinde tutuyor. Boğan taraf bunu bilinçli
/// yapmış olmak zorunda değil — sonuç aynı: rakibin bandı durdu.
/// Faz 2'nin çok girdili tarifleri bu olayı mümkün kılan şey.
/// Kartel: birbirini kırabilecekken **kırmayan** iki büyük satıcı.
///
/// # Neden bu ölçüt
///
/// Oyunda gizli anlaşma diye bir komut yok; firmalar konuşmuyor. Kartel
/// bu yüzden niyetten değil **davranıştan** okunur — tıpkı tekelin
/// paydan, fiyat savaşının ardışık kırmadan okunması gibi:
///
/// 1. İki firma pazarın büyük çoğunluğunu birlikte tutuyor
///    ([`CARTEL_JOINT_SHARE_PCT`]) — yani fiyatı ikisi belirleyebilir.
/// 2. İkisi de fiyatını baseline'ın üstünde tutuyor
///    ([`CARTEL_MIN_ASK_PCT`]) — rekabet etselerdi kırarlardı.
/// 3. Bu hâl [`CARTEL_CONFIRM_TICKS`] tick sürüyor — tesadüf değil.
///
/// Üçü birden sağlanınca "el sıkıştılar" demektir.
///
/// # İhanet
///
/// Kartel kurulduktan sonra üyelerden biri ötekinin fiyatını kırarsa
/// anlaşma bozulmuş sayılır: `CartelBetrayed` damgalanır, kurban kin
/// tutar, kartel dağılır. Kırma tespiti zaten `detect_undercuts`'ta
/// yapılıyor; burada yalnız üyelik kontrol edilir.
fn detect_cartels(
    state: &mut GameState,
    report: &mut TickReport,
    asks: &BTreeMap<(CityId, ProductKind), BTreeMap<PlayerId, i64>>,
    undercuts: &BTreeSet<(PlayerId, PlayerId, CityId, ProductKind)>,
    tick: Tick,
) {
    // ── İhanet: mevcut kartelin içinden biri ötekini kırdıysa ────────────
    let betrayals: Vec<((CityId, ProductKind), PlayerId, PlayerId)> = undercuts
        .iter()
        .filter_map(|(attacker, victim, city, product)| {
            let (a, b) = state.intrigue.cartels.get(&(*city, *product)).copied()?;
            let inside = (a == *attacker && b == *victim) || (b == *attacker && a == *victim);
            inside.then_some(((*city, *product), *attacker, *victim))
        })
        .collect();
    for (key, betrayer, victim) in betrayals {
        state.intrigue.cartels.remove(&key);
        state.intrigue.cartel_candidate.remove(&key);
        report.push(LogEntry {
            tick,
            actor: Some(betrayer),
            event: LogEvent::CartelBetrayed {
                city: key.0,
                product: key.1,
                betrayer,
                victim,
            },
        });
        // İhanet kin doğurur — sırtından bıçaklanan unutmaz.
        form_grudge(state, report, victim, betrayer, tick);
    }

    // ── Kuruluş ──────────────────────────────────────────────────────────
    let keys: Vec<(CityId, ProductKind)> = state.intrigue.sales_window.keys().copied().collect();
    for key in keys {
        if state.intrigue.cartels.contains_key(&key) {
            continue; // zaten kurulu
        }
        let (shares, total) = state.intrigue.window_shares(key.0, key.1);
        if total < DOMINANCE_MIN_VOLUME {
            state.intrigue.cartel_candidate.remove(&key);
            continue;
        }
        // En büyük iki satıcı.
        let mut ranked: Vec<(PlayerId, u64)> = shares.iter().map(|(p, q)| (*p, *q)).collect();
        ranked.sort_by_key(|(p, q)| (std::cmp::Reverse(*q), *p));
        if ranked.len() < 2 {
            state.intrigue.cartel_candidate.remove(&key);
            continue;
        }
        let (a, b) = (ranked[0].0.min(ranked[1].0), ranked[0].0.max(ranked[1].0));
        let joint_pct = (ranked[0].1 + ranked[1].1) * 100 / total;
        if joint_pct < CARTEL_JOINT_SHARE_PCT {
            state.intrigue.cartel_candidate.remove(&key);
            continue;
        }

        // İkisi de fiyatını yukarıda tutuyor mu? Fiyat vermeyen tick
        // sayılmaz — susmak anlaşma kanıtı değil.
        let Some(seller_asks) = asks.get(&key) else {
            state.intrigue.cartel_candidate.remove(&key);
            continue;
        };
        let Some(baseline) = state.effective_baseline(key.0, key.1) else {
            continue;
        };
        let floor = baseline.as_cents() * CARTEL_MIN_ASK_PCT / 100;
        let both_high = [a, b]
            .iter()
            .all(|p| seller_asks.get(p).is_some_and(|ask| *ask >= floor));
        // Aralarında kırma varsa anlaşma yok.
        let hostile = undercuts.iter().any(|(att, vic, c, pr)| {
            *c == key.0 && *pr == key.1 && ((*att == a && *vic == b) || (*att == b && *vic == a))
        });
        if !both_high || hostile {
            state.intrigue.cartel_candidate.remove(&key);
            continue;
        }

        let entry = state
            .intrigue
            .cartel_candidate
            .entry(key)
            .or_insert(((a, b), 0));
        if entry.0 == (a, b) {
            entry.1 += 1;
        } else {
            *entry = ((a, b), 1);
        }
        if entry.1 < CARTEL_CONFIRM_TICKS {
            continue;
        }
        state.intrigue.cartel_candidate.remove(&key);
        state.intrigue.cartels.insert(key, (a, b));
        report.push(LogEntry {
            tick,
            actor: Some(a),
            event: LogEvent::CartelFormed {
                city: key.0,
                product: key.1,
                a,
                b,
            },
        });
    }
}

/// Tedarik boğma yardımcısı.
fn detect_supply_chokes(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    // Bu tick girdi yokluğundan atıl kalan fabrikalar.
    let starved: Vec<(PlayerId, CityId, ProductKind)> = report
        .entries
        .iter()
        .filter_map(|entry| {
            let LogEvent::FactoryIdle { factory_id, city, reason } = &entry.event else {
                return None;
            };
            // `production.rs` açlık mesajları "raw X shortage" / "input X shortage".
            let missing = parse_shortage_input(reason)?;
            let owner = state.factories.get(factory_id)?.owner;
            Some((owner, *city, missing))
        })
        .collect();

    let mut new_chokes: Vec<(PlayerId, PlayerId, CityId, ProductKind)> = Vec::new();
    for (victim, city, missing) in starved {
        let Some(choker) = state.intrigue.rival_monopolist(victim, city, missing) else {
            continue;
        };
        let key = (choker, victim, city, missing);
        if state.intrigue.active_chokes.contains(&key) {
            continue; // süregelen boğma — her tick haber olmaz
        }
        new_chokes.push(key);
    }

    // Artık geçerli olmayan boğmaları unut ki tekrar haber olabilsinler.
    state.intrigue.active_chokes.retain(|(choker, _, city, product)| {
        state.intrigue.monopolist.get(&(*city, *product)) == Some(choker)
    });

    for key in new_chokes {
        state.intrigue.active_chokes.insert(key);
        report.push(LogEntry {
            tick,
            actor: Some(key.0),
            event: LogEvent::SupplyChoke {
                city: key.2,
                product: key.3,
                choker: key.0,
                victim: key.1,
            },
        });
        // Boğulan taraf bunu unutmaz.
        form_grudge(state, report, key.1, key.0, tick);
    }
}

/// `FactoryIdle` mesajından eksik girdiyi çıkar. Motor mesajı
/// `"raw <Ürün> shortage at ..."` ya da `"input <Ürün> shortage at ..."`.
fn parse_shortage_input(reason: &str) -> Option<ProductKind> {
    let rest = reason
        .strip_prefix("raw ")
        .or_else(|| reason.strip_prefix("input "))?;
    let name = rest.split(" shortage").next()?;
    ProductKind::ALL.into_iter().find(|p| p.display_name() == name)
}

/// Bu tick'in eşleşmelerinden pazar → (satıcı → birim) dökümü.
///
/// **Sadece üreticiler sayılır.** Tüccar/Spekülatör aldığı malı devrettiği
/// için satış hacmi yüksek görünür, ama arzı kontrol etmez — bir spekülatörün
/// üç tick mal çevirmesi tekel değildir. Pazar gücü üretimi elinde tutmaktır.
fn collect_tick_sales(
    state: &GameState,
    report: &TickReport,
) -> BTreeMap<(CityId, ProductKind), TickSales> {
    let mut out: BTreeMap<(CityId, ProductKind), TickSales> = BTreeMap::new();
    for entry in &report.entries {
        if let LogEvent::OrderMatched { city, product, seller, quantity, .. } = &entry.event {
            if !is_producer(state, *seller) {
                continue;
            }
            *out.entry((*city, *product))
                .or_default()
                .entry(*seller)
                .or_default() += u64::from(*quantity);
        }
    }
    out
}

/// Entrikanın öznesi olan **şirketler**: Sanayici ve insan oyuncular.
///
/// Çiftçi kasten dışarıda. Her Çiftçi kendi şehrinin tek hammadde üreticisi
/// olduğu için hammadde pazarını "ele geçirmesi" bir şirket hamlesi değil,
/// coğrafyanın sonucudur — manşet olarak anlamsız. Çiftçi ekonomiyi
/// besleyen arka plan tedarikçisidir. Aracılar (Tüccar, Spekülatör) arzı
/// üretmez, devreder; tüketiciler (Alıcı, Banka) zaten hariç.
fn is_producer(state: &GameState, player: PlayerId) -> bool {
    state
        .players
        .get(&player)
        .is_some_and(|p| p.npc_kind.is_none_or(|k| k == NpcKind::Sanayici))
}

/// Bu tick kabul edilen SELL emirlerinden pazar → (satıcı → en düşük fiyat cent).
fn collect_tick_asks(
    report: &TickReport,
) -> BTreeMap<(CityId, ProductKind), BTreeMap<PlayerId, i64>> {
    let mut out: BTreeMap<(CityId, ProductKind), BTreeMap<PlayerId, i64>> = BTreeMap::new();
    for entry in &report.entries {
        let LogEvent::CommandAccepted { command: Command::SubmitOrder(order) } = &entry.event
        else {
            continue;
        };
        if order.side != OrderSide::Sell {
            continue;
        }
        let cents = order.unit_price.as_cents();
        out.entry((order.city, order.product))
            .or_default()
            .entry(order.player)
            .and_modify(|best| *best = (*best).min(cents))
            .or_insert(cents);
    }
    out
}

/// Satış penceresine bu tick'i ekle, eskiyenleri buda.
fn update_sales_window(
    state: &mut GameState,
    sales: &BTreeMap<(CityId, ProductKind), TickSales>,
    tick: Tick,
) {
    for (key, tick_sales) in sales {
        state
            .intrigue
            .sales_window
            .entry(*key)
            .or_default()
            .push((tick, tick_sales.clone()));
    }
    let cutoff = tick.value().saturating_sub(DOMINANCE_WINDOW_TICKS);
    for entries in state.intrigue.sales_window.values_mut() {
        entries.retain(|(t, _)| t.value() > cutoff);
    }
    state.intrigue.sales_window.retain(|_, v| !v.is_empty());
}

/// Tekel tespiti — üç filtreli, gürültüye kapalı:
/// 1. **Hacim:** pencerede en az `DOMINANCE_MIN_VOLUME` birim satılmış olmalı.
/// 2. **Histerezis:** %60'ta kurulur, ancak %50'nin altında kırılır.
/// 3. **Süreklilik:** eşik `MONOPOLY_CONFIRM_TICKS` boyunca kesintisiz
///    korunmalı. Tek tick'lik pay sıçraması saltanat sayılmaz.
fn detect_monopolies(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let keys: Vec<(CityId, ProductKind)> = state.intrigue.sales_window.keys().copied().collect();
    for key in keys {
        let (shares, total) = state.intrigue.window_shares(key.0, key.1);
        if total < DOMINANCE_MIN_VOLUME {
            // Hacim gürültü seviyesinde — karar verme, sayaçlara da dokunma.
            continue;
        }
        let leader = shares.iter().max_by_key(|(pid, qty)| (**qty, *pid));
        let Some((&leader_id, &leader_qty)) = leader else { continue };
        let leader_pct = leader_qty * 100 / total;

        match state.intrigue.monopolist.get(&key).copied() {
            None => {
                if leader_pct < MONOPOLY_FORM_PCT {
                    state.intrigue.monopoly_candidate.remove(&key);
                    continue;
                }
                // Aday değiştiyse sayaç baştan başlar.
                let entry = state
                    .intrigue
                    .monopoly_candidate
                    .entry(key)
                    .or_insert((leader_id, 0));
                if entry.0 == leader_id {
                    entry.1 += 1;
                } else {
                    *entry = (leader_id, 1);
                }
                if entry.1 < MONOPOLY_CONFIRM_TICKS {
                    continue;
                }
                state.intrigue.monopoly_candidate.remove(&key);
                state.intrigue.monopoly_decay.remove(&key);
                state.intrigue.monopolist.insert(key, leader_id);
                // Haber değeri: pazarda başka üretici de sattıysa bu bir ele
                // geçirmedir. Tek üreticiyse sessiz statü (prim + taç) yeter.
                if shares.len() >= 2 {
                    state.intrigue.announced_monopolies.insert(key);
                    report.push(LogEntry {
                        tick,
                        actor: Some(leader_id),
                        event: LogEvent::MonopolyFormed {
                            city: key.0,
                            product: key.1,
                            firm: leader_id,
                            share_percent: u32::try_from(leader_pct).unwrap_or(100),
                        },
                    });
                }
            }
            Some(current) => {
                let current_qty = shares.get(&current).copied().unwrap_or(0);
                let current_pct = current_qty * 100 / total;
                if current_pct >= MONOPOLY_BREAK_PCT {
                    state.intrigue.monopoly_decay.remove(&key);
                    continue;
                }
                let decay = state.intrigue.monopoly_decay.entry(key).or_insert(0);
                *decay += 1;
                if *decay < MONOPOLY_BREAK_CONFIRM_TICKS {
                    continue;
                }
                state.intrigue.monopoly_decay.remove(&key);
                state.intrigue.monopolist.remove(&key);
                // Kırılma haberi yalnız ilan edilmiş saltanatlar için.
                if !state.intrigue.announced_monopolies.remove(&key) {
                    continue;
                }
                // Tekeli kıran: tekelci dışındaki en büyük satıcı.
                let breaker = shares
                    .iter()
                    .filter(|(pid, _)| **pid != current)
                    .max_by_key(|(pid, qty)| (**qty, *pid))
                    .map(|(pid, _)| *pid);
                report.push(LogEntry {
                    tick,
                    actor: breaker,
                    event: LogEvent::MonopolyBroken {
                        city: key.0,
                        product: key.1,
                        former: current,
                        breaker,
                    },
                });
            }
        }
    }
}

/// Undercut serileri: yerleşik satıcının fiyatını kıranları takip et.
/// Dönen küme: bu tick gerçekten kıran (saldırgan, mağdur, şehir, ürün) anahtarları.
fn detect_undercuts(
    state: &mut GameState,
    report: &mut TickReport,
    asks: &BTreeMap<(CityId, ProductKind), BTreeMap<PlayerId, i64>>,
    tick: Tick,
) -> BTreeSet<(PlayerId, PlayerId, CityId, ProductKind)> {
    let mut active: BTreeSet<(PlayerId, PlayerId, CityId, ProductKind)> = BTreeSet::new();

    for (key, seller_asks) in asks {
        let Some(victim) = incumbent_of(state, key.0, key.1) else { continue };
        let Some(&victim_ask) = seller_asks.get(&victim) else {
            continue; // mağdur bu tick fiyat vermedi — kırılacak fiyat yok
        };
        let cut_threshold = victim_ask * UNDERCUT_MAX_PCT_OF_VICTIM / 100;
        for (&attacker, &ask) in seller_asks {
            if attacker == victim || ask > cut_threshold {
                continue;
            }
            let streak_key = (attacker, victim, key.0, key.1);
            active.insert(streak_key);
            let streak = state.intrigue.undercut_streak.entry(streak_key).or_insert(0);
            *streak += 1;

            if *streak == UNDERCUT_CAMPAIGN_TICKS {
                report.push(LogEntry {
                    tick,
                    actor: Some(attacker),
                    event: LogEvent::UndercutCampaign {
                        city: key.0,
                        product: key.1,
                        attacker,
                        victim,
                        ticks: UNDERCUT_CAMPAIGN_TICKS,
                        attacker_ask: moneywar_domain::Money::from_cents(ask),
                        victim_ask: moneywar_domain::Money::from_cents(victim_ask),
                    },
                });
                form_grudge(state, report, victim, attacker, tick);
            } else if *streak == PRICE_WAR_DECLARE_TICKS
                && !state.intrigue.price_wars.contains_key(&streak_key)
            {
                state.intrigue.price_wars.insert(
                    streak_key,
                    PriceWarTrack {
                        declared_at: tick,
                        victim_absent_ticks: 0,
                        attacker_idle_ticks: 0,
                    },
                );
                report.push(LogEntry {
                    tick,
                    actor: Some(attacker),
                    event: LogEvent::PriceWarDeclared {
                        city: key.0,
                        product: key.1,
                        attacker,
                        target: victim,
                        attacker_ask: moneywar_domain::Money::from_cents(ask),
                        target_ask: moneywar_domain::Money::from_cents(victim_ask),
                    },
                });
            }
        }
    }

    // Bu tick kırmayan seriler sıfırlanır (savaş takibi ayrı yaşar).
    state.intrigue.undercut_streak.retain(|k, _| active.contains(k));
    active
}

/// Pazarın "yerleşiği": varsa tekelci, yoksa pencerede payı %30+ olan lider.
fn incumbent_of(state: &GameState, city: CityId, product: ProductKind) -> Option<PlayerId> {
    if let Some(&m) = state.intrigue.monopolist.get(&(city, product)) {
        return Some(m);
    }
    let (shares, total) = state.intrigue.window_shares(city, product);
    if total < DOMINANCE_MIN_VOLUME {
        return None;
    }
    shares
        .iter()
        .max_by_key(|(pid, qty)| (**qty, *pid))
        .filter(|&(_, &qty)| qty * 100 / total >= INCUMBENT_MIN_SHARE_PCT)
        .map(|(pid, _)| *pid)
}

/// Kin oluştur (yoksa) ve damgala. Var olan kin tazelenir, event tekrarlanmaz.
fn form_grudge(
    state: &mut GameState,
    report: &mut TickReport,
    holder: PlayerId,
    against: PlayerId,
    tick: Tick,
) {
    let is_new = !state.intrigue.grudges.contains_key(&(holder, against));
    state.intrigue.grudges.insert((holder, against), GRUDGE_TICKS);
    if is_new {
        report.push(LogEntry {
            tick,
            actor: Some(holder),
            event: LogEvent::GrudgeFormed { holder, against },
        });
    }
}

/// Aktif fiyat savaşlarını ilerlet: çekilme → zafer, ilgisizlik → sönme.
fn advance_price_wars(
    state: &mut GameState,
    report: &mut TickReport,
    asks: &BTreeMap<(CityId, ProductKind), BTreeMap<PlayerId, i64>>,
    undercuts: &BTreeSet<(PlayerId, PlayerId, CityId, ProductKind)>,
    tick: Tick,
) {
    let mut won: Vec<(PlayerId, PlayerId, CityId, ProductKind)> = Vec::new();
    let mut fizzled: Vec<(PlayerId, PlayerId, CityId, ProductKind)> = Vec::new();

    for (key, track) in &mut state.intrigue.price_wars {
        let (attacker, victim, city, product) = *key;
        let victim_present = asks
            .get(&(city, product))
            .is_some_and(|m| m.contains_key(&victim));
        if victim_present {
            track.victim_absent_ticks = 0;
        } else {
            track.victim_absent_ticks += 1;
        }
        // Saldırgan yalnız hedef sahadayken "atıl" sayılır. Hedef fiyat
        // vermiyorsa kırılacak fiyat da yoktur; bu saldırganın gevşemesi
        // değil mağdurun çekilmesidir ve zafer sayacına işler.
        if undercuts.contains(key) || !victim_present {
            track.attacker_idle_ticks = 0;
        } else {
            track.attacker_idle_ticks += 1;
        }

        let victim_bankrupt = state.intrigue.bankrupt.contains(&victim);
        if victim_bankrupt || track.victim_absent_ticks >= PRICE_WAR_RETREAT_TICKS {
            won.push(*key);
        } else if track.attacker_idle_ticks >= PRICE_WAR_FIZZLE_TICKS {
            fizzled.push(*key);
        }
        let _ = attacker;
    }

    for key in won {
        state.intrigue.price_wars.remove(&key);
        report.push(LogEntry {
            tick,
            actor: Some(key.0),
            event: LogEvent::PriceWarWon {
                city: key.2,
                product: key.3,
                winner: key.0,
                loser: key.1,
            },
        });
    }
    for key in fizzled {
        state.intrigue.price_wars.remove(&key);
    }
}

/// Kinler her tick 1 azalır; sıfırlananlar unutulur.
fn decay_grudges(state: &mut GameState) {
    for remaining in state.intrigue.grudges.values_mut() {
        *remaining = remaining.saturating_sub(1);
    }
    state.intrigue.grudges.retain(|_, r| *r > 0);
}

/// İflas — **acizlik** testi. İki yoldan biriyle olur:
///
/// 1. **Tekrarlı temerrüt:** firma borcunu `DEFAULTS_BEFORE_BANKRUPTCY` kez
///    ödeyemedi. Temerrüt artık bedelsiz değil.
/// 2. **Sermaye tükenmesi:** nakit aciz eşiğinin altında, satacak malı yok
///    ve üretim varlığı da yok — ayakta duracak hiçbir şeyi kalmamış.
///
/// Alıcı/Banka tüketici-altyapı rolleridir, iflas anlatısına girmez.
fn detect_bankruptcies(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let mut newly_bankrupt: Vec<PlayerId> = Vec::new();
    let mut streak_updates: Vec<(PlayerId, bool)> = Vec::new();
    for (pid, player) in &state.players {
        if state.intrigue.bankrupt.contains(pid) {
            continue;
        }
        // İnsan oyuncunun firması motor tarafından sessizce tasfiye edilmez —
        // oyuncu iflası interaktif katmanın açık bir "oyun bitti" akışıdır.
        if !player.is_npc {
            continue;
        }
        if matches!(player.npc_kind, Some(NpcKind::Alici | NpcKind::Banka)) {
            continue;
        }

        let defaults = state.intrigue.loan_defaults.get(pid).copied().unwrap_or(0);
        let repeat_defaulter = defaults >= DEFAULTS_BEFORE_BANKRUPTCY;

        let owns_production = state.factories.values().any(|f| f.owner == *pid)
            || state.private_farms.values().any(|f| f.owner == *pid)
            || state.caravans.values().any(|c| c.owner == *pid);
        let stripped = player.cash.as_cents() < INSOLVENT_CASH_CENTS
            && player.inventory.is_empty()
            && !owns_production;
        streak_updates.push((*pid, stripped));
        let sustained = stripped
            && state.intrigue.insolvency_streak.get(pid).copied().unwrap_or(0) + 1
                >= INSOLVENCY_CONFIRM_TICKS;

        if repeat_defaulter || sustained {
            newly_bankrupt.push(*pid);
        }
    }
    // Sayaçları yaz: acizlik sürüyorsa artır, toparladıysa sıfırla.
    for (pid, stripped) in streak_updates {
        if stripped {
            *state.intrigue.insolvency_streak.entry(pid).or_insert(0) += 1;
        } else {
            state.intrigue.insolvency_streak.remove(&pid);
        }
    }
    for pid in newly_bankrupt {
        state.intrigue.bankrupt.insert(pid);
        state.intrigue.insolvency_streak.remove(&pid);
        liquidate(state, pid);
        report.push(LogEntry {
            tick,
            actor: Some(pid),
            event: LogEvent::FirmBankrupt { firm: pid },
        });
    }
}

/// Batan firmanın tasfiyesi: üretim varlıkları ve stoğu piyasadan çekilir,
/// nakdi sıfırlanır. Arzın kaybolması rakipler için fırsat, tüketiciler için
/// fiyat baskısıdır — ölümün piyasada gerçek bir izi olur.
fn liquidate(state: &mut GameState, firm: PlayerId) {
    state.factories.retain(|_, f| f.owner != firm);
    state.private_farms.retain(|_, f| f.owner != firm);
    state.caravans.retain(|_, c| c.owner != firm);
    if let Some(player) = state.players.get_mut(&firm) {
        player.inventory = moneywar_domain::Inventory::default();
        player.cash = moneywar_domain::Money::ZERO;
    }
    // Batan firma artık kimsenin hedefi ya da ortağı değil.
    state.intrigue.price_wars.retain(|(a, v, _, _), _| *a != firm && *v != firm);
    state.intrigue.grudges.retain(|(h, g), _| *h != firm && *g != firm);
    state.intrigue.active_chokes.retain(|(c, v, _, _)| *c != firm && *v != firm);
    for bucket in state.intrigue.monopolist.clone().keys() {
        if state.intrigue.monopolist.get(bucket) == Some(&firm) {
            state.intrigue.monopolist.remove(bucket);
            state.intrigue.announced_monopolies.remove(bucket);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{Money, RoomConfig, RoomId};

    fn state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn pid(v: u64) -> PlayerId {
        PlayerId::new(v)
    }

    const BUCKET: (CityId, ProductKind) = (CityId::Istanbul, ProductKind::Un);

    /// Pencereye elle satış bas — dedektör birim testleri için kısayol.
    fn push_sales(s: &mut GameState, tick: u32, sales: &[(u64, u64)]) {
        let tick_sales: TickSales = sales.iter().map(|(p, q)| (pid(*p), *q)).collect();
        s.intrigue
            .sales_window
            .entry(BUCKET)
            .or_default()
            .push((Tick::new(tick), tick_sales));
    }

    fn story_events(report: &TickReport) -> Vec<&LogEvent> {
        report
            .entries
            .iter()
            .map(|e| &e.event)
            .filter(|ev| {
                matches!(
                    ev,
                    LogEvent::MonopolyFormed { .. }
                        | LogEvent::MonopolyBroken { .. }
                        | LogEvent::UndercutCampaign { .. }
                        | LogEvent::PriceWarDeclared { .. }
                        | LogEvent::PriceWarWon { .. }
                        | LogEvent::GrudgeFormed { .. }
                        | LogEvent::FirmBankrupt { .. }
                        | LogEvent::SupplyChoke { .. }
                        | LogEvent::CartelFormed { .. }
                        | LogEvent::CartelBetrayed { .. }
                )
            })
            .collect()
    }

    /// Dedektörü `n` tick koştur, üretilen anlatı olaylarının sayısını dön.
    fn run_monopoly_ticks(s: &mut GameState, from: u32, n: u32) -> usize {
        let mut count = 0;
        for t in from..from + n {
            let mut r = TickReport::new(Tick::new(t));
            detect_monopolies(s, &mut r, Tick::new(t));
            count += story_events(&r).len();
        }
        count
    }

    #[test]
    fn monopoly_needs_sustained_dominance() {
        let mut s = state();
        push_sales(&mut s, 1, &[(7, 70), (8, 30)]);

        // Onay süresi dolmadan ilan yok.
        let early = run_monopoly_ticks(&mut s, 2, MONOPOLY_CONFIRM_TICKS - 1);
        assert_eq!(early, 0, "süreklilik şartı dolmadan tekel ilan edilmemeli");
        assert!(s.intrigue.monopolist.is_empty());

        // Onay tick'inde tam bir kez ilan edilir.
        let declared = run_monopoly_ticks(&mut s, 100, 1);
        assert_eq!(declared, 1);
        assert_eq!(s.intrigue.monopolist.get(&BUCKET), Some(&pid(7)));

        // Sonraki tick'lerde tekrar edilmez.
        let repeated = run_monopoly_ticks(&mut s, 101, 3);
        assert_eq!(repeated, 0, "tekel her tick yeniden ilan edilmemeli");
    }

    #[test]
    fn sole_producer_gets_status_but_no_headline() {
        let mut s = state();
        // Tek üretici: pazarda başka satıcı yok → statü var, haber yok.
        push_sales(&mut s, 1, &[(7, 60)]);
        let events = run_monopoly_ticks(&mut s, 2, MONOPOLY_CONFIRM_TICKS);
        assert_eq!(s.intrigue.monopolist.get(&BUCKET), Some(&pid(7)));
        assert_eq!(events, 0, "tek üretici olmak haber değil");
        assert!(s.intrigue.announced_monopolies.is_empty());
    }

    #[test]
    fn brief_share_spike_does_not_form_monopoly() {
        let mut s = state();
        // Tek tick zirve, sonra pay düşüyor → sayaç sıfırlanır, ilan olmaz.
        push_sales(&mut s, 1, &[(7, 70), (8, 30)]);
        run_monopoly_ticks(&mut s, 2, 2);
        s.intrigue.sales_window.clear();
        push_sales(&mut s, 3, &[(7, 30), (8, 70)]);
        run_monopoly_ticks(&mut s, 4, 2);
        assert!(s.intrigue.monopolist.is_empty());
        assert_eq!(
            s.intrigue.monopoly_candidate.get(&BUCKET).map(|(p, _)| *p),
            Some(pid(8)),
            "aday liderle birlikte değişmeli"
        );
    }

    #[test]
    fn monopoly_needs_min_volume() {
        let mut s = state();
        push_sales(&mut s, 1, &[(7, 10)]); // %100 ama hacim < 30
        let mut report = TickReport::new(Tick::new(2));
        detect_monopolies(&mut s, &mut report, Tick::new(2));
        assert!(s.intrigue.monopolist.is_empty());
    }

    #[test]
    fn monopoly_survives_share_dip_above_break_threshold() {
        let mut s = state();
        s.intrigue.monopolist.insert(BUCKET, pid(7));
        // %55: kırılma eşiği %50'nin üstünde → histerezis tekeli korur.
        push_sales(&mut s, 1, &[(7, 55), (8, 45)]);
        assert_eq!(run_monopoly_ticks(&mut s, 2, MONOPOLY_BREAK_CONFIRM_TICKS + 2), 0);
        assert_eq!(s.intrigue.monopolist.get(&BUCKET), Some(&pid(7)));
    }

    #[test]
    fn monopoly_breaks_after_sustained_decline() {
        let mut s = state();
        s.intrigue.monopolist.insert(BUCKET, pid(7));
        s.intrigue.announced_monopolies.insert(BUCKET);
        push_sales(&mut s, 1, &[(7, 40), (8, 60)]);

        // Onay süresi dolmadan kırılma ilan edilmez.
        assert_eq!(run_monopoly_ticks(&mut s, 2, MONOPOLY_BREAK_CONFIRM_TICKS - 1), 0);
        assert_eq!(s.intrigue.monopolist.get(&BUCKET), Some(&pid(7)));

        // Onay tick'i: kırıldı, breaker = en büyük rakip.
        let mut report = TickReport::new(Tick::new(50));
        detect_monopolies(&mut s, &mut report, Tick::new(50));
        assert!(s.intrigue.monopolist.is_empty());
        match story_events(&report)[0] {
            LogEvent::MonopolyBroken { former, breaker, .. } => {
                assert_eq!(*former, pid(7));
                assert_eq!(*breaker, Some(pid(8)));
            }
            other => panic!("MonopolyBroken bekleniyordu, gelen: {other:?}"),
        }
    }

    #[test]
    fn recovering_share_cancels_pending_break() {
        let mut s = state();
        s.intrigue.monopolist.insert(BUCKET, pid(7));
        push_sales(&mut s, 1, &[(7, 40), (8, 60)]);
        run_monopoly_ticks(&mut s, 2, MONOPOLY_BREAK_CONFIRM_TICKS - 1);
        assert!(s.intrigue.monopoly_decay.contains_key(&BUCKET));

        // Pay toparlandı → kırılma sayacı sıfırlanır.
        s.intrigue.sales_window.clear();
        push_sales(&mut s, 20, &[(7, 80), (8, 20)]);
        run_monopoly_ticks(&mut s, 21, 1);
        assert!(!s.intrigue.monopoly_decay.contains_key(&BUCKET));
        assert_eq!(s.intrigue.monopolist.get(&BUCKET), Some(&pid(7)));
    }

    /// Ask haritası kur: (satıcı, cent) listesi.
    fn asks_of(entries: &[(u64, i64)]) -> BTreeMap<(CityId, ProductKind), BTreeMap<PlayerId, i64>> {
        let inner: BTreeMap<PlayerId, i64> =
            entries.iter().map(|(p, c)| (pid(*p), *c)).collect();
        BTreeMap::from([(BUCKET, inner)])
    }

    #[test]
    fn undercut_campaign_after_three_ticks_forms_grudge() {
        let mut s = state();
        s.intrigue.monopolist.insert(BUCKET, pid(7)); // yerleşik: 7
        let asks = asks_of(&[(7, 1000), (9, 900)]); // 9, %10 kırıyor

        for t in 1..=2 {
            let mut r = TickReport::new(Tick::new(t));
            detect_undercuts(&mut s, &mut r, &asks, Tick::new(t));
            assert!(story_events(&r).is_empty(), "tick {t}: erken damga");
        }
        let mut r3 = TickReport::new(Tick::new(3));
        detect_undercuts(&mut s, &mut r3, &asks, Tick::new(3));
        let evs = story_events(&r3);
        assert!(
            matches!(evs[0], LogEvent::UndercutCampaign { attacker, victim, .. }
                if *attacker == pid(9) && *victim == pid(7))
        );
        assert!(matches!(evs[1], LogEvent::GrudgeFormed { holder, against }
                if *holder == pid(7) && *against == pid(9)));
        assert_eq!(s.intrigue.grudges.get(&(pid(7), pid(9))), Some(&GRUDGE_TICKS));
    }

    #[test]
    fn shallow_cut_does_not_count() {
        let mut s = state();
        s.intrigue.monopolist.insert(BUCKET, pid(7));
        // 995 > 1000×98% = 980 → kırma sayılmaz.
        let asks = asks_of(&[(7, 1000), (9, 995)]);
        let mut r = TickReport::new(Tick::new(1));
        let active = detect_undercuts(&mut s, &mut r, &asks, Tick::new(1));
        assert!(active.is_empty());
        assert!(s.intrigue.undercut_streak.is_empty());
    }

    #[test]
    fn streak_resets_when_attacker_stops() {
        let mut s = state();
        s.intrigue.monopolist.insert(BUCKET, pid(7));
        let cutting = asks_of(&[(7, 1000), (9, 900)]);
        let peaceful = asks_of(&[(7, 1000), (9, 1000)]);

        let mut r = TickReport::new(Tick::new(1));
        detect_undercuts(&mut s, &mut r, &cutting, Tick::new(1));
        assert_eq!(s.intrigue.undercut_streak.len(), 1);

        let mut r2 = TickReport::new(Tick::new(2));
        detect_undercuts(&mut s, &mut r2, &peaceful, Tick::new(2));
        assert!(s.intrigue.undercut_streak.is_empty());
    }

    #[test]
    fn price_war_declared_then_won_on_retreat() {
        let mut s = state();
        s.intrigue.monopolist.insert(BUCKET, pid(7));
        let cutting = asks_of(&[(7, 1000), (9, 900)]);

        // 5 tick kesintisiz kırma → savaş ilanı.
        let mut declared = false;
        for t in 1..=5 {
            let mut r = TickReport::new(Tick::new(t));
            let uc = detect_undercuts(&mut s, &mut r, &cutting, Tick::new(t));
            advance_price_wars(&mut s, &mut r, &cutting, &uc, Tick::new(t));
            declared |= story_events(&r)
                .iter()
                .any(|e| matches!(e, LogEvent::PriceWarDeclared { .. }));
        }
        assert!(declared, "5 tick kırma savaş ilan etmeliydi");
        assert_eq!(s.intrigue.price_wars.len(), 1);

        // Mağdur pazardan çekiliyor (ask yok) ama saldırgan da artık kıramıyor
        // (kırılacak fiyat yok). RETREAT eşiği FIZZLE'dan küçük → zafer önce gelir.
        let attacker_only = asks_of(&[(9, 900)]);
        let mut won = false;
        for t in 6..=12 {
            let mut r = TickReport::new(Tick::new(t));
            let uc = detect_undercuts(&mut s, &mut r, &attacker_only, Tick::new(t));
            advance_price_wars(&mut s, &mut r, &attacker_only, &uc, Tick::new(t));
            won |= story_events(&r).iter().any(
                |e| matches!(e, LogEvent::PriceWarWon { winner, loser, .. }
                    if *winner == pid(9) && *loser == pid(7)),
            );
        }
        assert!(won, "mağdur çekilince savaş kazanılmalıydı");
        assert!(s.intrigue.price_wars.is_empty());
    }

    #[test]
    fn shortage_reason_parses_both_message_shapes() {
        assert_eq!(
            parse_shortage_input("raw Pamuk shortage at İstanbul: have=3, need=12"),
            Some(ProductKind::Pamuk)
        );
        assert_eq!(
            parse_shortage_input("input Boya shortage at Konya: have=0, need=20"),
            Some(ProductKind::Boya)
        );
        // Açlıkla ilgisiz atıl sebepleri boğma sayılmaz.
        assert_eq!(parse_shortage_input("inventory add failed: overflow"), None);
    }

    #[test]
    fn supply_choke_fires_once_while_monopoly_holds() {
        let mut s = state();
        // Boya tekeli 7'de; 5'in fabrikası Boya bulamıyor.
        s.intrigue.monopolist.insert((CityId::Konya, ProductKind::Boya), pid(7));
        let fid = moneywar_domain::FactoryId::new(1);
        s.factories.insert(
            fid,
            moneywar_domain::Factory::new(fid, pid(5), CityId::Konya, ProductKind::Elbise).unwrap(),
        );

        let idle = |t: u32| LogEntry {
            tick: Tick::new(t),
            actor: Some(pid(5)),
            event: LogEvent::FactoryIdle {
                factory_id: fid,
                city: CityId::Konya,
                reason: "input Boya shortage at Konya: have=0, need=20".into(),
            },
        };

        let mut r1 = TickReport::new(Tick::new(1));
        r1.push(idle(1));
        detect_supply_chokes(&mut s, &mut r1, Tick::new(1));
        assert_eq!(
            story_events(&r1)
                .iter()
                .filter(|e| matches!(e, LogEvent::SupplyChoke { .. }))
                .count(),
            1
        );

        // Aynı boğma sürerken tekrar haber olmaz.
        let mut r2 = TickReport::new(Tick::new(2));
        r2.push(idle(2));
        detect_supply_chokes(&mut s, &mut r2, Tick::new(2));
        assert!(
            !story_events(&r2)
                .iter()
                .any(|e| matches!(e, LogEvent::SupplyChoke { .. })),
            "süregelen boğma her tick manşet olmamalı"
        );

        // Tekel düşerse kayıt temizlenir → yeniden haber olabilir.
        s.intrigue.monopolist.remove(&(CityId::Konya, ProductKind::Boya));
        let mut r3 = TickReport::new(Tick::new(3));
        detect_supply_chokes(&mut s, &mut r3, Tick::new(3));
        assert!(s.intrigue.active_chokes.is_empty());
    }

    #[test]
    fn own_monopoly_does_not_choke_itself() {
        let mut s = state();
        s.intrigue.monopolist.insert((CityId::Konya, ProductKind::Boya), pid(5));
        let fid = moneywar_domain::FactoryId::new(1);
        s.factories.insert(
            fid,
            moneywar_domain::Factory::new(fid, pid(5), CityId::Konya, ProductKind::Elbise).unwrap(),
        );
        let mut r = TickReport::new(Tick::new(1));
        r.push(LogEntry {
            tick: Tick::new(1),
            actor: Some(pid(5)),
            event: LogEvent::FactoryIdle {
                factory_id: fid,
                city: CityId::Konya,
                reason: "input Boya shortage at Konya: have=0, need=20".into(),
            },
        });
        detect_supply_chokes(&mut s, &mut r, Tick::new(1));
        assert!(story_events(&r).is_empty(), "kendi tekelin seni boğamaz");
    }

    #[test]
    fn grudges_decay_to_zero() {
        let mut s = state();
        s.intrigue.grudges.insert((pid(1), pid(2)), 2);
        decay_grudges(&mut s);
        assert_eq!(s.intrigue.grudges.get(&(pid(1), pid(2))), Some(&1));
        decay_grudges(&mut s);
        assert!(s.intrigue.grudges.is_empty());
    }

    /// Varlıksız bir üretici ekle (aciz adayı).
    fn insert_broke_producer(s: &mut GameState, id: u64) {
        let p = moneywar_domain::Player::new(
            pid(id),
            "Batan",
            moneywar_domain::Role::Sanayici,
            Money::ZERO,
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Sanayici);
        s.players.insert(pid(id), p);
    }

    fn run_bankruptcy_ticks(s: &mut GameState, from: u32, n: u32) -> usize {
        let mut count = 0;
        for t in from..from + n {
            let mut r = TickReport::new(Tick::new(t));
            detect_bankruptcies(s, &mut r, Tick::new(t));
            count += story_events(&r).len();
        }
        count
    }

    #[test]
    fn bankruptcy_needs_sustained_insolvency() {
        let mut s = state();
        insert_broke_producer(&mut s, 5);

        // Onay süresi dolmadan iflas yok — mal yolda olabilir.
        let early = run_bankruptcy_ticks(&mut s, 1, INSOLVENCY_CONFIRM_TICKS - 1);
        assert_eq!(early, 0, "geçici nakit boşluğu iflas değil");
        assert!(s.intrigue.bankrupt.is_empty());

        // Onay tick'inde tam bir kez damgalanır.
        assert_eq!(run_bankruptcy_ticks(&mut s, 100, 1), 1);
        assert!(s.intrigue.bankrupt.contains(&pid(5)));
        assert_eq!(run_bankruptcy_ticks(&mut s, 101, 3), 0, "iflas bir kez damgalanır");
    }

    #[test]
    fn recovering_firm_escapes_bankruptcy() {
        let mut s = state();
        insert_broke_producer(&mut s, 5);
        run_bankruptcy_ticks(&mut s, 1, INSOLVENCY_CONFIRM_TICKS - 1);

        // Nakit geldi → sayaç sıfırlanır, firma kurtulur.
        s.players.get_mut(&pid(5)).unwrap().cash = Money::from_lira(5_000).unwrap();
        run_bankruptcy_ticks(&mut s, 50, 1);
        assert!(!s.intrigue.insolvency_streak.contains_key(&pid(5)));

        // Yeniden parasız kalsa bile sayaç baştan başlar.
        s.players.get_mut(&pid(5)).unwrap().cash = Money::ZERO;
        assert_eq!(run_bankruptcy_ticks(&mut s, 60, INSOLVENCY_CONFIRM_TICKS - 1), 0);
        assert!(s.intrigue.bankrupt.is_empty());
    }

    #[test]
    fn repeat_loan_default_bankrupts_immediately() {
        let mut s = state();
        insert_broke_producer(&mut s, 5);
        // Varlıklı ama borcunu iki kez ödeyememiş firma: aciz.
        s.players.get_mut(&pid(5)).unwrap().cash = Money::from_lira(20_000).unwrap();
        s.intrigue.loan_defaults.insert(pid(5), DEFAULTS_BEFORE_BANKRUPTCY);

        assert_eq!(run_bankruptcy_ticks(&mut s, 1, 1), 1, "tekrarlı temerrüt iflastır");
        assert!(s.intrigue.bankrupt.contains(&pid(5)));
    }

    #[test]
    fn liquidation_strips_assets_and_clears_intrigue() {
        let mut s = state();
        insert_broke_producer(&mut s, 5);
        s.players.get_mut(&pid(5)).unwrap().cash = Money::from_lira(9_000).unwrap();
        let fid = moneywar_domain::FactoryId::new(1);
        s.factories.insert(
            fid,
            moneywar_domain::Factory::new(fid, pid(5), CityId::Konya, ProductKind::Un).unwrap(),
        );
        s.intrigue.monopolist.insert(BUCKET, pid(5));
        s.intrigue.announced_monopolies.insert(BUCKET);
        s.intrigue.grudges.insert((pid(9), pid(5)), 20);

        liquidate(&mut s, pid(5));

        assert!(s.factories.is_empty(), "fabrikalar tasfiye edilmeli");
        assert_eq!(s.players[&pid(5)].cash, Money::ZERO);
        assert!(s.intrigue.monopolist.is_empty(), "batan firma tekelini bırakır");
        assert!(s.intrigue.announced_monopolies.is_empty());
        assert!(s.intrigue.grudges.is_empty(), "ölüye kin tutulmaz");
    }

    #[test]
    fn consumer_roles_never_go_bankrupt() {
        let mut s = state();
        let p = moneywar_domain::Player::new(
            pid(6),
            "Tüketici",
            moneywar_domain::Role::Tuccar,
            Money::ZERO,
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        s.players.insert(pid(6), p);

        let mut r = TickReport::new(Tick::new(1));
        detect_bankruptcies(&mut s, &mut r, Tick::new(1));
        assert!(story_events(&r).is_empty());
    }
}
