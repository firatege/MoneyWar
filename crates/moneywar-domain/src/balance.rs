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
pub const FACTORY_BATCH_SIZE: u32 = 10;

/// Batch başlatıldıktan kaç tick sonra biter (§4, tentatif).
pub const FACTORY_PRODUCTION_TICKS: u32 = 2;

// =============================================================================
// Fabrika
// =============================================================================

/// `§10` kurulum maliyet tablosu — `existing_count` index'i ile oku.
/// İlk fabrika bedava (starter), sonra artan maliyet.
/// `existing_count >= len()` için son eleman kullanılır (5+ sabit 30k).
pub const FACTORY_BUILD_COSTS_LIRA: [i64; 5] = [0, 10_000, 15_000, 22_000, 30_000];

// =============================================================================
// Kervan
// =============================================================================

/// Sanayici kervanı kapasitesi (§10).
pub const CARAVAN_CAPACITY_SANAYICI: u32 = 20;

/// Tüccar kervanı kapasitesi — daha büyük (§10).
pub const CARAVAN_CAPACITY_TUCCAR: u32 = 50;

/// Sanayici kervan maliyet tablosu (§10).
pub const CARAVAN_COSTS_SANAYICI_LIRA: [i64; 3] = [0, 5_000, 10_000];

/// Tüccar kervan maliyet tablosu (§10). Tüccar 4'e kadar alır, ucuz.
pub const CARAVAN_COSTS_TUCCAR_LIRA: [i64; 4] = [0, 6_000, 10_000, 15_000];

// =============================================================================
// Piyasa
// =============================================================================

/// Doygunluk eşiği formülü (§10):
/// `threshold = SATURATION_BASE + (player_count - SATURATION_MIN_PLAYERS) × SATURATION_PER_PLAYER`.
pub const SATURATION_BASE: u32 = 40;
/// Her ek oyuncu başına doygunluk eşiği artışı.
pub const SATURATION_PER_PLAYER: u32 = 10;
/// Doygunluk formülünün alt sınırı (bu sayının altında formül devreye girmez).
pub const SATURATION_MIN_PLAYERS: u8 = 2;

// =============================================================================
// Haber
// =============================================================================

/// Bronz tier aylık/sezonluk ücreti — bedava (herkese açık).
pub const NEWS_COST_BRONZE_LIRA: i64 = 0;
/// Gümüş abonelik ücreti (Tüccar için bedava, diğerleri için).
pub const NEWS_COST_SILVER_LIRA: i64 = 500;
/// Altın abonelik — premium.
pub const NEWS_COST_GOLD_LIRA: i64 = 2_000;

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

/// İstanbul ↔ Ankara (§3) tick cinsinden.
pub const DIST_ISTANBUL_ANKARA: u32 = 3;
/// Ankara ↔ İzmir — en yakın çift.
pub const DIST_ANKARA_IZMIR: u32 = 2;
/// İstanbul ↔ İzmir — deniz yolu, en uzun.
pub const DIST_ISTANBUL_IZMIR: u32 = 4;

// =============================================================================
// Bozulma
// =============================================================================

/// Un: bu kadar tick sonra %100 kaybolur (§4).
pub const PERISH_UN_TICKS: u32 = 3;
pub const PERISH_UN_LOSS_PCT: u32 = 100;

/// Zeytinyağı: 5 tick sonra %10 fire.
pub const PERISH_ZEYTINYAGI_TICKS: u32 = 5;
pub const PERISH_ZEYTINYAGI_LOSS_PCT: u32 = 10;

// =============================================================================
// NPC likidite
// =============================================================================

/// `MarketMaker` baz fiyat — ham madde (§10: 5-8₺ tipik).
pub const NPC_BASE_PRICE_RAW_LIRA: i64 = 6;
/// `MarketMaker` baz fiyat — bitmiş ürün (§10: 12-18₺ tipik).
pub const NPC_BASE_PRICE_FINISHED_LIRA: i64 = 15;

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
    fn news_costs_are_monotonic() {
        assert!(NEWS_COST_BRONZE_LIRA < NEWS_COST_SILVER_LIRA);
        assert!(NEWS_COST_SILVER_LIRA < NEWS_COST_GOLD_LIRA);
    }

    #[test]
    fn news_leads_are_monotonic() {
        assert!(NEWS_LEAD_BRONZE < NEWS_LEAD_SILVER);
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
