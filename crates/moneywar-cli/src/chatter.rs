//! NPC söylenti / chatter üretici.
//!
//! `LogEvent` akışını taranır, NPC kaynaklı dikkat çekici aksiyonlardan
//! kişiliğe-uygun Türkçe söylenti satırları üretilir. Sadece UI durumudur —
//! state'e veya determinizme dokunmaz.
//!
//! Şablon seçimi `(player_id, tick, kind)` üzerinden modüler bir karma ile
//! yapılır; aynı tick + aynı NPC + aynı aksiyon her zaman aynı satırı verir,
//! ama farklı NPC'ler ya da farklı tick'ler farklı satır seçer. Bu kullanıcı
//! için çeşitli ama yine de tekrar üretilebilir bir his bırakır.

use moneywar_domain::{GameState, Personality, PlayerId, Tick};
use moneywar_engine::LogEvent;

const MAX_LINE_LEN: usize = 96;

#[derive(Debug, Clone)]
pub struct ChatterEntry {
    pub tick: Tick,
    pub speaker: String,
    pub emoji: &'static str,
    pub text: String,
}

/// Bir tick raporundan üretilen chatter satırlarının listesi (sıralı).
#[must_use]
pub fn generate_chatter(
    state: &GameState,
    report: &moneywar_engine::TickReport,
) -> Vec<ChatterEntry> {
    let mut out: Vec<ChatterEntry> = Vec::new();
    for entry in &report.entries {
        if let Some(line) = chatter_for(state, entry.tick, &entry.event) {
            out.push(line);
        }
    }
    out
}

fn chatter_for(state: &GameState, tick: Tick, event: &LogEvent) -> Option<ChatterEntry> {
    let (pid, kind) = classify(event)?;
    // İnsan oyuncu için chatter üretme — sadece NPC'ler söylenir.
    let player = state.players.get(&pid)?;
    if !player.is_npc {
        return None;
    }
    let personality = player.personality.unwrap_or(Personality::TrendFollower);
    let templates = template_bank(personality, kind);
    if templates.is_empty() {
        return None;
    }
    let idx = pick_index(pid, tick, kind, templates.len());
    let raw = templates[idx];
    let text = render_template(raw, event);
    Some(ChatterEntry {
        tick,
        speaker: player.name.clone(),
        emoji: personality.emoji(),
        text: truncate(text, MAX_LINE_LEN),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ChatterKind {
    BoughtCaravan,
    BuiltFactory,
    ProposedContract,
    AcceptedContract,
    LoanTaken,
    LoanDefaulted,
    BigMatch,
}

fn classify(event: &LogEvent) -> Option<(PlayerId, ChatterKind)> {
    match event {
        LogEvent::CaravanBought { owner, .. } => Some((*owner, ChatterKind::BoughtCaravan)),
        LogEvent::FactoryBuilt { owner, .. } => Some((*owner, ChatterKind::BuiltFactory)),
        LogEvent::ContractProposed { seller, .. } => Some((*seller, ChatterKind::ProposedContract)),
        LogEvent::ContractAccepted { acceptor, .. } => {
            Some((*acceptor, ChatterKind::AcceptedContract))
        }
        LogEvent::LoanTaken { borrower, .. } => Some((*borrower, ChatterKind::LoanTaken)),
        LogEvent::LoanDefaulted { borrower, .. } => Some((*borrower, ChatterKind::LoanDefaulted)),
        LogEvent::OrderMatched {
            buyer, quantity, ..
        } if *quantity >= 30 => Some((*buyer, ChatterKind::BigMatch)),
        _ => None,
    }
}

fn render_template(raw: &str, event: &LogEvent) -> String {
    // Basit yer-tutucular: {city}, {product}, {qty}, {price}, {to}.
    let mut s = raw.to_string();
    match event {
        LogEvent::CaravanBought { starting_city, .. } => {
            s = s.replace("{city}", short_city(starting_city));
        }
        LogEvent::FactoryBuilt { city, product, .. } => {
            s = s.replace("{city}", short_city(city));
            s = s.replace("{product}", &format!("{product}"));
        }
        LogEvent::ContractProposed {
            product,
            quantity,
            unit_price,
            delivery_city,
            ..
        } => {
            s = s.replace("{city}", short_city(delivery_city));
            s = s.replace("{product}", &format!("{product}"));
            s = s.replace("{qty}", &quantity.to_string());
            s = s.replace("{price}", &format!("{unit_price}"));
        }
        LogEvent::ContractAccepted { .. } => {}
        LogEvent::LoanTaken { principal, .. } => {
            s = s.replace("{price}", &format!("{principal}"));
        }
        LogEvent::LoanDefaulted { unpaid_balance, .. } => {
            s = s.replace("{price}", &format!("{unpaid_balance}"));
        }
        LogEvent::OrderMatched {
            city,
            product,
            quantity,
            price,
            ..
        } => {
            s = s.replace("{city}", short_city(city));
            s = s.replace("{product}", &format!("{product}"));
            s = s.replace("{qty}", &quantity.to_string());
            s = s.replace("{price}", &format!("{price}"));
        }
        _ => {}
    }
    s
}

fn short_city(city: &moneywar_domain::CityId) -> &'static str {
    use moneywar_domain::CityId;
    match city {
        CityId::Istanbul => "İstanbul",
        CityId::Izmir => "İzmir",
        CityId::Ankara => "Ankara",
    }
}

/// Aynı (player, tick, kind) için aynı şablonu döndür → render deterministic.
/// Farklı player/tick için farklı şablon → çeşitlilik. RNG kullanmıyoruz çünkü
/// RNG state'i UI tarafından paylaşıldığında reproducibility kırılır.
fn pick_index(pid: PlayerId, tick: Tick, kind: ChatterKind, n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let kind_seed: u64 = match kind {
        ChatterKind::BoughtCaravan => 1,
        ChatterKind::BuiltFactory => 2,
        ChatterKind::ProposedContract => 4,
        ChatterKind::AcceptedContract => 5,
        ChatterKind::LoanTaken => 6,
        ChatterKind::LoanDefaulted => 7,
        ChatterKind::BigMatch => 8,
    };
    let mix = pid
        .value()
        .wrapping_mul(2_654_435_761)
        .wrapping_add(u64::from(tick.value()).wrapping_mul(11_400_714_785_074_694_791))
        .wrapping_add(kind_seed.wrapping_mul(1_597_334_677));
    (mix as usize) % n
}

fn truncate(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Personality × ChatterKind → Türkçe şablon havuzu. Boş slice → o NPC bu
/// olayı yorumlamaz (sessiz kalır).
fn template_bank(personality: Personality, kind: ChatterKind) -> &'static [&'static str] {
    use ChatterKind::*;
    use Personality::*;
    match (personality, kind) {
        // ⚡ Aggressive — hızlı, kestirip atan
        (Aggressive, BoughtCaravan) => &[
            "Yeni kervan {city}'da. Pazara dalmaya hazırım.",
            "Beklemek kaybetmek demek; kervan aldım.",
        ],
        (Aggressive, BuiltFactory) => &[
            "{city}'da fabrika kurdum, {product} hattı çalışacak.",
            "Yavaş işler kâr getirmiyor — fabrika kurdum, {product}.",
        ],
        (Aggressive, ProposedContract) => {
            &["{qty} {product} satışa çıktı, {price} fiyatla, kapan kapsın."]
        }
        (Aggressive, BigMatch) => &["{qty} {product} {city}'de aldım, fiyat takmıyorum."],
        (Aggressive, LoanTaken) => &["{price} kredi çektim, hızlı dönüş yapacağım."],

        // 📈 TrendFollower — momentum, akışa uyan
        (TrendFollower, BuiltFactory) => &["Trend {product} tarafında, {city}'de fabrika açtım."],
        (TrendFollower, BoughtCaravan) => &["Akış kuvvetli, kervan ekledim."],
        (TrendFollower, ProposedContract) => {
            &["{product} yükseliyor, {qty} birim {price}'a kontratladım."]
        }
        (TrendFollower, BigMatch) => &["{city}'de {product} dalgasına bindim — {qty} birim aldım."],

        // 🔄 MeanReverter — fiyat dönecek diye düşünen
        (MeanReverter, ProposedContract) => {
            &["{product} yüksek, {qty} birim {price}'tan ben satayım, döner."]
        }
        (MeanReverter, BigMatch) => {
            &["{product} {city}'de aşırı düştü, {qty} birim aldım — geri gelecek."]
        }
        (MeanReverter, BuiltFactory) => {
            &["Pazar normalleşince {product} kazandırır, fabrika hazır."]
        }

        // 🛣 Arbitrageur — şehirler arası fark avcısı
        (Arbitrageur, BoughtCaravan) => {
            &["İki şehir arası fark açıldı — kervan aldım, {city} merkezli."]
        }
        (Arbitrageur, ProposedContract) => &["{city}'ye {qty} {product} teslim edeceğim, {price}."],
        (Arbitrageur, AcceptedContract) => &["Şehir farkı bana yetiyor, kontratı aldım."],
        (Arbitrageur, BigMatch) => &["{qty} {product} {city}'de — diğer şehre çekeceğim."],

        // 🎲 EventTrader — olay önden pozisyon
        (EventTrader, BoughtCaravan) => &["Haber kanalı kıpırdıyor, kervan ekledim."],
        (EventTrader, BuiltFactory) => &["{city}'da {product} olayı yaklaşıyor, fabrika hazır."],
        (EventTrader, ProposedContract) => &["Olay öncesi {qty} {product} kontratladım, {price}."],
        (EventTrader, BigMatch) => &["Olay vurmadan {qty} {product} stokladım."],

        // 📦 Hoarder — stok biriktiren
        (Hoarder, BigMatch) => &[
            "{qty} {product} aldım, depoya gidiyor.",
            "Stok her zaman güzeldir — {qty} {product} eklendi.",
        ],
        (Hoarder, ProposedContract) => {
            &["Uzun vadeli sabit gelir: {qty} {product} kontratım çıktı."]
        }
        (Hoarder, AcceptedContract) => &["Sabit teslimat hoşuma gider — kontratı kaptım."],
        (Hoarder, BuiltFactory) => &["{city}'da {product} fabrikası — uzun vade için."],

        // 💀 Cartel — manipülasyon, büyük hareket
        (Cartel, ProposedContract) => {
            &["Pazara mesaj: {qty} {product} satışta, {price}. Anlayan anlar."]
        }
        (Cartel, BigMatch) => &["{city} {product} pazarı benimdir — {qty} birim aldım."],
        (Cartel, BuiltFactory) => {
            &["{city}'de {product} hattı kuruldu. Bundan sonra fiyat ben söylerim."]
        }
        (Cartel, LoanTaken) => &["{price} sermaye topladım — pazarı dizmek için."],

        // Tüm kişilikler için kredi default — komik anti-mesaj
        (_, LoanDefaulted) => &[
            "Borcum {price}, ödeyemedim. Kötü gidiyor.",
            "Banka her şeyi aldı — {price} kapanmadı.",
        ],

        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CityId, Money, NpcKind, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };
    use moneywar_engine::{LogEntry, TickReport};

    fn make_state_with_npc(personality: Personality) -> (GameState, PlayerId) {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let pid = PlayerId::new(100);
        let npc = Player::new(
            pid,
            "Hoarder Tahir",
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Tuccar)
        .with_personality(personality);
        s.players.insert(pid, npc);
        (s, pid)
    }

    fn empty_report() -> TickReport {
        TickReport {
            tick: Tick::new(1),
            entries: Vec::new(),
        }
    }

    #[test]
    fn empty_report_yields_no_chatter() {
        let (state, _) = make_state_with_npc(Personality::Hoarder);
        assert!(generate_chatter(&state, &empty_report()).is_empty());
    }

    #[test]
    fn caravan_buy_produces_personality_line() {
        let (state, pid) = make_state_with_npc(Personality::Aggressive);
        let mut report = empty_report();
        report.entries.push(LogEntry {
            tick: Tick::new(1),
            actor: Some(pid),
            event: LogEvent::CaravanBought {
                caravan_id: moneywar_domain::CaravanId::new(1),
                owner: pid,
                starting_city: CityId::Istanbul,
                capacity: 30,
                cost: Money::from_lira(2_000).unwrap(),
            },
        });
        let lines = generate_chatter(&state, &report);
        assert_eq!(lines.len(), 1);
        let entry = &lines[0];
        assert!(entry.text.contains("kervan") || entry.text.contains("Pazara"));
        assert_eq!(entry.speaker, "Hoarder Tahir");
    }

    #[test]
    fn human_actor_ignored() {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let pid = PlayerId::new(1);
        let human = Player::new(
            pid,
            "Sen",
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            false,
        )
        .unwrap();
        s.players.insert(pid, human);
        let mut report = empty_report();
        report.entries.push(LogEntry {
            tick: Tick::new(1),
            actor: Some(pid),
            event: LogEvent::CaravanBought {
                caravan_id: moneywar_domain::CaravanId::new(1),
                owner: pid,
                starting_city: CityId::Istanbul,
                capacity: 30,
                cost: Money::from_lira(2_000).unwrap(),
            },
        });
        assert!(generate_chatter(&s, &report).is_empty());
    }

    #[test]
    fn deterministic_for_same_input() {
        let (state, pid) = make_state_with_npc(Personality::Cartel);
        let mut report = empty_report();
        report.entries.push(LogEntry {
            tick: Tick::new(7),
            actor: Some(pid),
            event: LogEvent::FactoryBuilt {
                factory_id: moneywar_domain::FactoryId::new(1),
                owner: pid,
                city: CityId::Izmir,
                product: ProductKind::Kumas,
                cost: Money::from_lira(15_000).unwrap(),
            },
        });
        let a = generate_chatter(&state, &report);
        let b = generate_chatter(&state, &report);
        assert_eq!(a.len(), b.len());
        assert_eq!(a[0].text, b[0].text);
    }
}
