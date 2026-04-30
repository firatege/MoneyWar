//! 🎛️ **Oyun dengesi** — tek yerden ayarla.
//!
//! Bu dosya, motoru etkileyen tüm **sayısal** parametreleri `pub const`
//! olarak merkezileştirir. Bir değeri değiştirip `cargo build` ile yeniden
//! derlediğinizde tüm motor o değere göre çalışır. Deterministik kalır —
//! `GameState` içinde saklanmaz, derleme zamanı sabitidir.
//!
//! # Bölümler
//!
//! | Bölüm | İçerik |
//! |---|---|
//! | [Zaman](#zaman) | Fabrika üretim süresi, batch boyutu |
//! | [Fabrika](#fabrika) | Kurulum maliyet tablosu (§10) |
//! | [Kervan](#kervan) | Rol bazlı kapasite + maliyet |
//! | [Piyasa](#piyasa) | Doygunluk eşiği formülü (§10) |
//! | [Haber](#haber) | Tier ücretleri + lead-time (§6) |
//! | [Olay](#olay-motoru) | Olasılık, severity (§6) |
//! | [Kredi](#kredi) | NPC faizi (§7) |
//! | [Skor](#skor) | Atıl eşiği, rolling avg penceresi (§9) |
//! | [Mesafe](#ehir-mesafeleri) | Şehirler arası tick (§3) |
//! | [Bozulma](#bozulma) | Perishability kuralları (§4) |
//! | [NPC](#npc-likidite) | `MarketMaker` baz fiyatları |
//!
//! # Balance workflow
//!
//! 1. Parametreyi bu dosyada değiştir.
//! 2. `cargo test --workspace` — invariantlar ve integration testler
//!    yeni denge ile hala geçiyor mu?
//! 3. `cargo run -p moneywar-cli` — tam sezon simüle et, leaderboard'a bak.
//! 4. Anlamlı değişim = commit.
//!
//! # Dinamik config (v2 opsiyonu)
//!
//! v2'de oda-başı farklı denge isteniyorsa bu sabitler bir `GameBalance`
//! struct'ına taşınıp `GameState`'e eklenebilir. Şimdilik derleme zamanı
//! sabit — basit ve determinism için güvenli.

// =============================================================================
// Zaman
// =============================================================================

/// Her fabrika tick başına bu kadar ham madde → bitmiş ürün (§10).
/// 10 → 100: tüccar oyunu hissi için tüm hacim 10× ölçeklendi.
/// Üretim batch boyutu. 100 sezon başına ~30 batch potansiyel sağlar.
/// 50'ye düşürmek Çiftçi'yi kazandırdı (+6.7K) ama Sanayici'yi -32K'ya
/// sürükledi (kâr marjı erimişti). 100 koru.
pub const FACTORY_BATCH_SIZE: u32 = 100;

/// Batch başlatıldıktan kaç tick sonra biter (§4).
/// Eski yolculuk: 2 → 3 (Sanayici aşırı kârlı diye yavaşlatıldı), şimdi
/// 3 → 2 (NPC Sanayici sezon boyu hammadde bulamayıp 321/sezon `FactoryIdle`
/// veriyordu, %50 batch artışı dengeye getirir; Tüccar arbitrajı zaten
/// hacim 10× ölçekten kazandığı için fark Tüccar lehine değil).
pub const FACTORY_PRODUCTION_TICKS: u32 = 2;

// =============================================================================
// Fabrika
// =============================================================================

/// Kurulum maliyet tablosu — `existing_count` index'i ile oku.
/// İlk fabrika bedava, sonra artan maliyet.
/// Sanayici PnL -28K — fab kuruluş 23K cash sink + ham maliyet > kâr.
/// Tablo [8K,15K,25K,40K] → [4K,10K,18K,30K]: 3 fab kuruluş 14K (eski 23K),
/// kalan 9K cash ham bütçesinde değerlendirilir, Sanayici kârlı işletme yapar.
pub const FACTORY_BUILD_COSTS_LIRA: [i64; 5] = [0, 4_000, 10_000, 18_000, 30_000];

// =============================================================================
// Kervan
// =============================================================================

/// Sanayici kervanı kapasitesi (§10).
/// 20 → 200: hacim 10× ölçek revizyonu.
pub const CARAVAN_CAPACITY_SANAYICI: u32 = 200;

/// Tüccar kervanı kapasitesi — daha büyük (§10).
/// 50 → 500: hacim 10× ölçek revizyonu.
pub const CARAVAN_CAPACITY_TUCCAR: u32 = 500;

/// Sanayici kervan maliyet tablosu (§10).
pub const CARAVAN_COSTS_SANAYICI_LIRA: [i64; 3] = [0, 5_000, 10_000];

/// Tüccar kervan maliyet tablosu (§10). Tüccar 4'e kadar alır, ucuz.
pub const CARAVAN_COSTS_TUCCAR_LIRA: [i64; 4] = [0, 6_000, 10_000, 15_000];

// =============================================================================
// Piyasa
// =============================================================================

/// Doygunluk eşiği formülü (§10):
/// `threshold = SATURATION_BASE + (player_count - SATURATION_MIN_PLAYERS) × SATURATION_PER_PLAYER`.
///
/// Eski değerler 40+10/oyuncu → 12 oyuncu için 140 birim. Pratikte tek bucket'ta
/// 140 birim eşleşme nadir → eşik tetiklenmiyordu (ölü kod). Şu an 25+5/oyuncu →
/// 12 oyuncu için 75 — tetiklenebilir, anti-snowball mekanizması canlı.
/// 25 → 250: hacim 10× ölçek revizyonu — büyük emirler ceza yememeli.
pub const SATURATION_BASE: u32 = 250;
/// Her ek oyuncu başına doygunluk eşiği artışı.
/// 5 → 50: hacim 10× ölçek revizyonu.
pub const SATURATION_PER_PLAYER: u32 = 50;
/// Doygunluk formülünün alt sınırı (bu sayının altında formül devreye girmez).
pub const SATURATION_MIN_PLAYERS: u8 = 2;

// =============================================================================
// Haber (4-tier abonelik, recurring tick fee)
// =============================================================================
//
// 4 tier: Free < Bronze < Silver < Gold. Free herkese bedava (varsayılan).
// Tüccar her tier'da indirimli — bilgi onun mesleği. Bronze tüm Tüccar'lara
// bedava; daha üstü ucuz. Tüm ücretler **tick başına** kesilir; oyuncu cash'i
// yetmezse 1 tick uyarı, sonraki tick yine yetmezse Free'ye düşer.

/// Free — bedava, sadece "var/yok" + rolling avg.
pub const NEWS_TICK_COST_FREE_LIRA: i64 = 0;
/// Bronz — kategorik (yok/az/orta/bol) + ask/bid bandı.
pub const NEWS_TICK_COST_BRONZE_LIRA: i64 = 5;
/// Gümüş — 5'e yuvarlı miktar + ask/bid 5 kuruşa yuvarlı.
pub const NEWS_TICK_COST_SILVER_LIRA: i64 = 15;
/// Altın — tam veri + tüm olay haberleri.
pub const NEWS_TICK_COST_GOLD_LIRA: i64 = 40;

/// Tüccar — Bronze hafif indirimli (rol avantajı korunur ama bedava değil).
pub const NEWS_TICK_COST_BRONZE_TUCCAR_LIRA: i64 = 2;
/// Tüccar — Gümüş indirimli.
pub const NEWS_TICK_COST_SILVER_TUCCAR_LIRA: i64 = 5;
/// Tüccar — Altın indirimli.
pub const NEWS_TICK_COST_GOLD_TUCCAR_LIRA: i64 = 15;

/// Free: olay haberi yok (sürpriz şoklar).
pub const NEWS_LEAD_FREE: u32 = 0;
/// Bronz: olay tick'inde duyurulur.
pub const NEWS_LEAD_BRONZE: u32 = 0;
/// Gümüş: 1 tick önceden.
pub const NEWS_LEAD_SILVER: u32 = 1;
/// Altın: 2 tick önceden.
pub const NEWS_LEAD_GOLD: u32 = 2;

// =============================================================================
// Olay motoru
// =============================================================================

/// Motor olayı `current_tick + EVENT_LEAD_TICKS`'e zamanlar.
/// Minimum `NEWS_LEAD_GOLD` olmalı — Altın aboneler lead-time görebilsin.
pub const EVENT_LEAD_TICKS: u32 = 2;

/// Erken sezon (< %50 progress) olay olasılığı (yüzde).
pub const EVENT_PROB_EARLY_PCT: u32 = 5;
/// Mid sezon (%50–80) olasılığı.
pub const EVENT_PROB_MID_PCT: u32 = 10;
/// Geç sezon (%80+) olasılığı — makro şok penceresi.
pub const EVENT_PROB_LATE_PCT: u32 = 20;

/// Severity → fiyat şok yüzdeleri (§6, motor Faz 12'de kullanır).
pub const SHOCK_MINOR_PCT: u32 = 8;
pub const SHOCK_MAJOR_PCT: u32 = 18;
pub const SHOCK_MACRO_PCT: u32 = 35;

// =============================================================================
// Kredi
// =============================================================================

/// NPC bankasının sabit faiz oranı (§7, basit yüzde).
pub const LOAN_INTEREST_RATE_PERCENT: u32 = 15;

// =============================================================================
// Skor
// =============================================================================

/// Atıl fabrika eşiği (§9): son bu kadar tick'te üretim yoksa skora 0.
pub const IDLE_FACTORY_THRESHOLD: u32 = 10;

/// Rolling avg fiyat penceresi (§9) — tek-tick manipülasyonunu öldürür.
pub const PRICE_WINDOW: usize = 5;

/// Fabrika sermayesi skor oranı (§9): `build_cost × NUM / DEN`.
/// Default 1/2 = yatırımın %50'si skora döner.
pub const FACTORY_SCORE_NUM: i64 = 1;
pub const FACTORY_SCORE_DEN: i64 = 2;

// =============================================================================
// Şehir mesafeleri
// =============================================================================

/// İstanbul ↔ Ankara — v3'te yarıya indi (3→2 tick).
pub const DIST_ISTANBUL_ANKARA: u32 = 2;
/// Ankara ↔ İzmir — en yakın çift, hâlâ 1 tick.
pub const DIST_ANKARA_IZMIR: u32 = 1;
/// İstanbul ↔ İzmir — deniz yolu (4→2 tick).
pub const DIST_ISTANBUL_IZMIR: u32 = 2;

// =============================================================================
// Bozulma
// =============================================================================

/// Un: bu kadar tick sonra fire başlar. v3'te yumuşatıldı (3→5, %100→%50)
/// — kervan en uzun rotası (İst↔İzm = 4 tick) varış sırasında kayıpsız geçsin
/// ama "Un'u uzun süre cebinde tutarsan zarar" mekaniği kalsın.
pub const PERISH_UN_TICKS: u32 = 5;
pub const PERISH_UN_LOSS_PCT: u32 = 50;

/// Zeytinyağı: 5 tick sonra %10 fire.
pub const PERISH_ZEYTINYAGI_TICKS: u32 = 5;
pub const PERISH_ZEYTINYAGI_LOSS_PCT: u32 = 10;

// =============================================================================
// NPC likidite
// =============================================================================

/// `MarketMaker` baz fiyat — ham madde (§10: 5-8₺ tipik).
pub const NPC_BASE_PRICE_RAW_LIRA: i64 = 6;
/// `MarketMaker` baz fiyat — bitmiş ürün (§10: 12-18₺ tipik).
/// Eski 15 → 18: `production_tick=3` ile Sanayici marjı yetmez, sezon sonu
/// negatif `PnL`. Marj 9 → 12 (%200) ile Sanayici dengeye gelir.
pub const NPC_BASE_PRICE_FINISHED_LIRA: i64 = 18;

/// `MarketMaker` markup — stok varsa bu yüzdeyle satar (base × 1.1).
pub const NPC_SELL_MARKUP_PCT: i64 = 110;
/// `MarketMaker` markdown — nakit varsa bu yüzdeyle alır (base × 0.9).
pub const NPC_BUY_MARKDOWN_PCT: i64 = 90;

/// NPC `OrderId` ofseti — insan oyuncu havuzu ile çakışmasın.
pub const NPC_ORDER_ID_OFFSET: u64 = 10_000_000_000;

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn factory_cost_table_has_entries() {
        assert_eq!(FACTORY_BUILD_COSTS_LIRA.len(), 5);
        assert_eq!(FACTORY_BUILD_COSTS_LIRA[0], 0); // starter bedava
        assert!(FACTORY_BUILD_COSTS_LIRA.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn caravan_cost_tables_monotonic_non_decreasing() {
        for w in CARAVAN_COSTS_SANAYICI_LIRA.windows(2) {
            assert!(w[0] <= w[1]);
        }
        for w in CARAVAN_COSTS_TUCCAR_LIRA.windows(2) {
            assert!(w[0] <= w[1]);
        }
    }

    #[test]
    fn tuccar_caravan_capacity_exceeds_sanayici() {
        assert!(CARAVAN_CAPACITY_TUCCAR > CARAVAN_CAPACITY_SANAYICI);
    }

    #[test]
    fn news_tick_costs_are_monotonic() {
        assert_eq!(NEWS_TICK_COST_FREE_LIRA, 0);
        assert!(NEWS_TICK_COST_FREE_LIRA < NEWS_TICK_COST_BRONZE_LIRA);
        assert!(NEWS_TICK_COST_BRONZE_LIRA < NEWS_TICK_COST_SILVER_LIRA);
        assert!(NEWS_TICK_COST_SILVER_LIRA < NEWS_TICK_COST_GOLD_LIRA);
    }

    #[test]
    fn tuccar_news_costs_are_discounted() {
        // Tüccar her tier'da normalden ucuz (rol avantajı korunur).
        assert!(NEWS_TICK_COST_BRONZE_TUCCAR_LIRA < NEWS_TICK_COST_BRONZE_LIRA);
        assert!(NEWS_TICK_COST_SILVER_TUCCAR_LIRA < NEWS_TICK_COST_SILVER_LIRA);
        assert!(NEWS_TICK_COST_GOLD_TUCCAR_LIRA < NEWS_TICK_COST_GOLD_LIRA);
    }

    #[test]
    fn news_leads_are_monotonic() {
        assert_eq!(NEWS_LEAD_FREE, 0);
        assert!(NEWS_LEAD_BRONZE <= NEWS_LEAD_SILVER);
        assert!(NEWS_LEAD_SILVER < NEWS_LEAD_GOLD);
    }

    #[test]
    fn event_lead_covers_max_news_lead() {
        // Altın aboneler `event_tick - NEWS_LEAD_GOLD` görür; underflow olmasın.
        assert!(EVENT_LEAD_TICKS >= NEWS_LEAD_GOLD);
    }

    #[test]
    fn event_probabilities_ascend_with_season() {
        assert!(EVENT_PROB_EARLY_PCT < EVENT_PROB_MID_PCT);
        assert!(EVENT_PROB_MID_PCT < EVENT_PROB_LATE_PCT);
    }

    #[test]
    fn shock_percentages_scale_with_severity() {
        assert!(SHOCK_MINOR_PCT < SHOCK_MAJOR_PCT);
        assert!(SHOCK_MAJOR_PCT < SHOCK_MACRO_PCT);
    }

    #[test]
    fn loan_interest_is_positive_and_reasonable() {
        // Not harici kural: 0 < faiz < %100 (yıkıcı oran olmasın).
        assert!(LOAN_INTEREST_RATE_PERCENT > 0);
        assert!(LOAN_INTEREST_RATE_PERCENT < 100);
    }

    #[test]
    fn factory_score_ratio_is_half() {
        // §9: yatırımın %50'si skora döner.
        assert_eq!(FACTORY_SCORE_NUM * 2, FACTORY_SCORE_DEN);
    }

    #[test]
    fn npc_markup_above_markdown() {
        assert!(NPC_SELL_MARKUP_PCT > NPC_BUY_MARKDOWN_PCT);
    }
}
