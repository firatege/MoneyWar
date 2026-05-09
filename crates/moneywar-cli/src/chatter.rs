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
///
/// v0.5: Üç ek katman:
/// - **ShockEvent**: `EventScheduled` rastgele NPC tarafından yorumlanır
/// - **Reactive**: BigMatch sonrası ikinci NPC `@isim` ile reaksiyon
/// - **IndexShift**: caller `app` tarafında hesaplanır (`generate_index_chatter`)
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
        // v0.5: BigMatch'e reaktif `@isim` mention.
        if let Some(reactive) = reactive_for(state, entry.tick, &entry.event) {
            out.push(reactive);
        }
        // v0.5: Şok event yorumu — rastgele bir NPC.
        if let Some(shock) = shock_for(state, entry.tick, &entry.event) {
            out.push(shock);
        }
    }
    out
}

/// v0.5: Endeks değişimine göre 0-1 chatter satırı. App caller'ı tarafından
/// `cached_indices_prev` ile karşılaştırılarak çağrılır.
#[must_use]
pub fn generate_index_chatter(
    state: &GameState,
    tick: Tick,
    index_label: &str,
    pct_change: i64,
) -> Option<ChatterEntry> {
    if pct_change.abs() < 5 {
        return None;
    }
    let speaker = pick_npc_speaker(state, tick, 31)?;
    let player = state.players.get(&speaker)?;
    let personality = player.personality.unwrap_or(Personality::TrendFollower);
    let kind = ChatterKind::IndexShift;
    let templates = template_bank(personality, kind);
    if templates.is_empty() {
        return None;
    }
    let idx = pick_index(speaker, tick, kind, templates.len());
    let raw = templates[idx];
    let arrow = if pct_change > 0 {
        "yükseldi"
    } else {
        "düştü"
    };
    let text = raw
        .replace("{index}", index_label)
        .replace("{arrow}", arrow)
        .replace("{pct}", &pct_change.abs().to_string());
    Some(ChatterEntry {
        tick,
        speaker: player.name.clone(),
        emoji: personality.emoji(),
        text: truncate(text, MAX_LINE_LEN),
    })
}

/// v0.5: BigMatch'e ikinci NPC reaktif yorumu (mention `@buyer_name`).
fn reactive_for(state: &GameState, tick: Tick, event: &LogEvent) -> Option<ChatterEntry> {
    let LogEvent::OrderMatched {
        buyer,
        city,
        product,
        quantity,
        ..
    } = event
    else {
        return None;
    };
    if *quantity < 30 {
        return None;
    }
    let original = state.players.get(buyer)?;
    if !original.is_npc {
        return None;
    }
    // İkinci konuşmacı: orijinal hariç bir NPC. Tick + event seed'inden seç.
    let speaker_id = pick_npc_speaker_excluding(state, tick, 13, *buyer)?;
    let speaker = state.players.get(&speaker_id)?;
    let personality = speaker.personality.unwrap_or(Personality::TrendFollower);
    let kind = ChatterKind::Reactive;
    let templates = template_bank(personality, kind);
    if templates.is_empty() {
        return None;
    }
    let idx = pick_index(speaker_id, tick, kind, templates.len());
    let raw = templates[idx];
    let text = raw
        .replace("{name}", &original.name)
        .replace("{city}", short_city(city))
        .replace("{product}", &format!("{product}"))
        .replace("{qty}", &quantity.to_string());
    Some(ChatterEntry {
        tick,
        speaker: speaker.name.clone(),
        emoji: personality.emoji(),
        text: truncate(text, MAX_LINE_LEN),
    })
}

/// v0.5: Şok event başladı — rastgele bir NPC yorumlar.
fn shock_for(state: &GameState, tick: Tick, event: &LogEvent) -> Option<ChatterEntry> {
    let LogEvent::EventScheduled {
        event_id,
        game_event,
        ..
    } = event
    else {
        return None;
    };
    let speaker_id = pick_npc_speaker(state, tick, u64::from(event_id.value()))?;
    let speaker = state.players.get(&speaker_id)?;
    let personality = speaker.personality.unwrap_or(Personality::TrendFollower);
    let kind = ChatterKind::ShockEvent;
    let templates = template_bank(personality, kind);
    if templates.is_empty() {
        return None;
    }
    let idx = pick_index(speaker_id, tick, kind, templates.len());
    let raw = templates[idx];
    let label = describe_event(game_event);
    let text = raw.replace("{event}", &label);
    Some(ChatterEntry {
        tick,
        speaker: speaker.name.clone(),
        emoji: personality.emoji(),
        text: truncate(text, MAX_LINE_LEN),
    })
}

fn describe_event(ev: &moneywar_domain::GameEvent) -> String {
    use moneywar_domain::GameEvent;
    match ev {
        GameEvent::Drought { city, product, .. } => {
            format!("{}/{} kuraklık", short_city(city), product)
        }
        GameEvent::Strike { city, .. } => format!("{} grevi", short_city(city)),
        GameEvent::BumperHarvest { city, product, .. } => {
            format!("{}/{} bol hasat", short_city(city), product)
        }
        GameEvent::NewMarket { city, product, .. } => {
            format!("{}/{} yeni pazar", short_city(city), product)
        }
        GameEvent::RoadClosure { from, to, .. } => {
            format!("{}↔{} yol kapandı", short_city(from), short_city(to))
        }
    }
}

/// Tick + nonce'a göre deterministik NPC seç. None → hiç NPC yoksa.
fn pick_npc_speaker(state: &GameState, tick: Tick, nonce: u64) -> Option<PlayerId> {
    let npcs: Vec<PlayerId> = state
        .players
        .values()
        .filter(|p| p.is_npc)
        .map(|p| p.id)
        .collect();
    if npcs.is_empty() {
        return None;
    }
    let mix = u64::from(tick.value())
        .wrapping_mul(2_654_435_761)
        .wrapping_add(nonce.wrapping_mul(11_400_714_785_074_694_791));
    Some(npcs[(mix as usize) % npcs.len()])
}

fn pick_npc_speaker_excluding(
    state: &GameState,
    tick: Tick,
    nonce: u64,
    exclude: PlayerId,
) -> Option<PlayerId> {
    let npcs: Vec<PlayerId> = state
        .players
        .values()
        .filter(|p| p.is_npc && p.id != exclude)
        .map(|p| p.id)
        .collect();
    if npcs.is_empty() {
        return None;
    }
    let mix = u64::from(tick.value())
        .wrapping_mul(2_654_435_761)
        .wrapping_add(nonce.wrapping_mul(11_400_714_785_074_694_791));
    Some(npcs[(mix as usize) % npcs.len()])
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
    /// v0.5: Şok event başladı — rastgele bir NPC yorumlar.
    ShockEvent,
    /// v0.5: BigMatch'e ikinci NPC reaktif yorum (`@isim` mention).
    Reactive,
    /// v0.5: Endeks ±%5 üstü hareket etti — rastgele bir NPC yorumlar.
    IndexShift,
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
        CityId::Bursa => "Bursa",
        CityId::Konya => "Konya",
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
        ChatterKind::ShockEvent => 9,
        ChatterKind::Reactive => 10,
        ChatterKind::IndexShift => 11,
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

        // ⚡ ShockEvent — şok başlama yorumları, kişiliğe göre ton
        (Aggressive, ShockEvent) => &[
            "{event} — herkes panikleyene kadar pozisyon alacağım.",
            "Hareket var: {event}. Hızlı olan kazanır.",
        ],
        (Hoarder, ShockEvent) => &[
            "{event}. Stoğa basacak vakit yine geldi.",
            "{event} — mal toplama zamanı, fiyatlar oturmadan.",
        ],
        (Arbitrageur, ShockEvent) => &[
            "{event} — şehir farkı açılacak, hazırlanıyorum.",
            "Pazar kıpırdadı: {event}. İki şehir arası bana yetiyor.",
        ],
        (EventTrader, ShockEvent) => &[
            "Beklediğim haber: {event}. Pozisyon önceden kuruldu.",
            "{event} — bu olayı önceden okudum, sıra hasat.",
        ],
        (TrendFollower, ShockEvent) => &[
            "{event} duyumu var, akış buradan kuruluyor.",
            "Trend dönüyor: {event}. Yön belli olunca girerim.",
        ],
        (MeanReverter, ShockEvent) => &[
            "{event} aşırılık üretir, geri dönüş için bekleyeceğim.",
            "Şu {event} fırtınası geçer; sonrası benim.",
        ],
        (Cartel, ShockEvent) => &[
            "{event} — pazarı dizmenin tam zamanı.",
            "Bu olay {event}, masa benimdir.",
        ],

        // 💬 Reactive — başka NPC'nin BigMatch'ine cevap (`@isim` mention)
        (TrendFollower, Reactive) => &[
            "@{name} hamlesi gözümden kaçmadı, {city}/{product}'i ben de izliyorum.",
            "@{name} {qty} {product} aldı — ben de pozisyon büyüteceğim.",
        ],
        (MeanReverter, Reactive) => &[
            "@{name} {product}'a girdi ama bence dönecek, ben karşı tarafım.",
            "@{name} hızlı davranıyor; ben tersine pozisyon kuruyorum.",
        ],
        (Cartel, Reactive) => &[
            "@{name} {city}/{product}'a girmiş — pazarın benim olduğunu unutmasın.",
            "@{name} hamlesi yetersiz; {qty} adet bana göre değil.",
        ],
        (Arbitrageur, Reactive) => &[
            "@{name} {city}'de aldı, ben diğer şehre satarım.",
            "@{name} {product} için fiyatı yukarı çekti — şehir farkı bana avantaj.",
        ],
        (Aggressive, Reactive) => &[
            "@{name} hızlandı, ben daha hızlıyım.",
            "@{name} {qty} aldı, ben üzerine basıyorum.",
        ],
        (EventTrader, Reactive) => &[
            "@{name} olayı kaçırdı; ben önceden hazırdım.",
            "@{name} {product}'a girdi, asıl haberi ben biliyorum.",
        ],
        (Hoarder, Reactive) => &[
            "@{name} aldı, satarım demiyor; ben de stoklamaya devam.",
            "@{name} hareketi gördüm, depo dolu kalsın.",
        ],

        // 📊 IndexShift — endeks ±%5 üstü hareket
        (Aggressive, IndexShift) => &[
            "{index} {arrow} %{pct} — bu hızda durmak yok.",
            "{index} %{pct} {arrow}, ben de pozisyonu büyüttüm.",
        ],
        (TrendFollower, IndexShift) => &[
            "{index} {arrow} %{pct}, akış buradan.",
            "{index} ivme kazandı (%{pct} {arrow}), trende biniyorum.",
        ],
        (MeanReverter, IndexShift) => &[
            "{index} %{pct} {arrow}, aşırılık var; ters pozisyon kuruyorum.",
            "{index} {arrow}; geri dönecek, sabırlıyım.",
        ],
        (Hoarder, IndexShift) => &["{index} %{pct} {arrow} — depoda yer var, panik yok."],
        (Cartel, IndexShift) => {
            &["{index} {arrow} %{pct}; bu hareketin arkasında ben varım, daha bitmedi."]
        }
        (Arbitrageur, IndexShift) => {
            &["{index} {arrow} %{pct} — şehirler arası fark çoktan açıldı."]
        }
        (EventTrader, IndexShift) => &["{index} %{pct} {arrow}, haberim vardı."],

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
