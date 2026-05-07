//! Oda yapılandırması (game-design.md §0).
//!
//! Preset tablosu:
//!
//! | Preset    | Tick süresi | Sezon    | NPC |
//! |-----------|-------------|----------|-----|
//! | Hızlı     | 60 sn       | 90 tick  | 3   |
//! | Standart  | 30 dk       | 150 tick | 4   |
//! | Uzun      | 1 saat      | 350 tick | 5   |
//! | Custom    | Serbest     | Serbest  | Serbest |
//!
//! Piyasa doygunluk eşiği §10 formülü: `40 + (player_count - 2) * 10`.

use serde::{Deserialize, Serialize};

use crate::DomainError;

/// Oda preseti etiketi (UI + log için).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Preset {
    Hizli,
    Standart,
    Uzun,
    Custom,
}

impl Preset {
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Hizli => "Hızlı",
            Self::Standart => "Standart",
            Self::Uzun => "Uzun",
            Self::Custom => "Custom",
        }
    }
}

impl std::fmt::Display for Preset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

/// NPC kompozisyonu — kaç Sanayici + Tüccar + Alıcı + Esnaf + Spekülatör +
/// Çiftçi + Banka spawn edilsin.
///
/// - **Sanayici**: fabrika kurar, ham → finished üretir
/// - **Tüccar**: arbitraj (al-sat şehirler arası)
/// - **Alıcı**: saf alıcı — sadece buy emri (`AliciNpc` davranışı)
/// - **Esnaf**: toptancı (eski adı), aracı katman — Çiftçi'den ham al,
///   Sanayici/Alıcı'ya sat
/// - **Spekülatör**: her tick aynı (city, product) için hem bid hem ask
///   emir verir. Market making — spread daraltır, "mallar bekliyor" sorununu
///   doğrudan çözer.
/// - **Çiftçi**: hammadde üreticisi (v4). Periyodik mahsul + SELL only —
///   Sanayici fabrikalarının ham kaynağı. Bu olmadan Pamuk/Buğday/Zeytin
///   pazarları sezon ortasında ölür.
/// - **Banka**: likidite (v4). Komut emit etmez; loan akışı engine'de.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NpcComposition {
    pub sanayici: u8,
    pub tuccar: u8,
    pub alici: u8,
    pub esnaf: u8,
    #[serde(default)]
    pub spekulator: u8,
    /// v4 hammadde üreticisi. Eski configlerde yok → field-level default ile
    /// `default_const().ciftci`'ye düşer (yoksa pazar sezon ortasında ölür).
    #[serde(default = "default_ciftci")]
    pub ciftci: u8,
    /// v4 likidite sağlayıcı. Eski configlerde yok → field-level default.
    #[serde(default = "default_banka")]
    pub banka: u8,
}

const fn default_ciftci() -> u8 {
    NpcComposition::default_const().ciftci
}
const fn default_banka() -> u8 {
    NpcComposition::default_const().banka
}

impl NpcComposition {
    /// Default: 5 Sanayici, 4 Tüccar, 8 Alıcı, 0 Esnaf, 3 Spekülatör,
    /// 6 Çiftçi (2/ham), 3 Banka = 29 NPC. v8.19: Esnaf emekli edildi —
    /// matematik paradoksu (60K cash ama 429K BUY accept; engine
    /// `settle_segment` buyer_ok bypass'ı), tedarik zinciri Çiftçi (ham) →
    /// Sanayici (mamul) → Alıcı (tüketim) zaten net. Esnaf middleman
    /// katmanı oyun mantığı katmıyordu, sadece para sızıntısı + gürültü.
    #[must_use]
    pub const fn default_const() -> Self {
        Self {
            sanayici: 5,
            tuccar: 4,
            alici: 8,
            esnaf: 0,
            // v8.24: Spek emekli — Hard mode'da -304K zarar (3 NPC × -100K).
            // SELL match=0 (ASK %101 baseline hiç eşleşmiyor), ham depoda
            // birikiyor. Para sızıntısı: Spek → Tüccar/Çiftçi. Çıkarınca
            // ham BUY tarafı %30 azalır ama Sanayici cross + tâtonnement
            // ile dengelenir. NPC altyapısı korundu (kompozisyon 0).
            spekulator: 0,
            ciftci: 6,
            banka: 3,
        }
    }

    #[must_use]
    pub const fn total(&self) -> u8 {
        self.sanayici
            .saturating_add(self.tuccar)
            .saturating_add(self.alici)
            .saturating_add(self.esnaf)
            .saturating_add(self.spekulator)
            .saturating_add(self.ciftci)
            .saturating_add(self.banka)
    }
}

impl Default for NpcComposition {
    fn default() -> Self {
        Self::default_const()
    }
}

/// Runtime dengeleme knob'ları — `moneywar.toml` ile override edilebilir.
///
/// `balance.rs`'deki `pub const`'lar compile-time sabit kalır; burası sadece
/// **kullanıcı ayarlaması beklenen** değerler (TTL, cancel penalty, cooldown,
/// NPC kompozisyonu).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameBalance {
    /// Emir wizard'da pre-fill edilecek default TTL.
    pub default_order_ttl: u32,
    /// Kullanıcının seçebileceği en uzun TTL.
    pub max_order_ttl: u32,
    /// Erken çekme cezası yüzdesi (`notional × pct × remaining_ticks / ttl_ticks`).
    /// Faz 2'de devreye girer — şimdilik sadece config'de tutuluyor.
    pub cancel_penalty_pct: u32,
    /// Emir bittikten sonra aynı `(player, city, product)` için
    /// yeni emir kabul edilmeden önce geçmesi gereken tick sayısı.
    /// Faz 2'de devreye girer.
    pub relist_cooldown_ticks: u32,
    /// NPC kompozisyonu (kaç Sanayici/Tüccar/Alıcı spawn edilsin).
    pub npcs: NpcComposition,
}

impl GameBalance {
    /// TTL alt sınırı — emirler en az 1 clear pass'e katılmalı.
    pub const MIN_ORDER_TTL: u32 = 1;
    /// TTL üst sınırı (hardcoded sanity cap).
    pub const MAX_ORDER_TTL_HARD: u32 = 50;

    #[must_use]
    pub const fn default_const() -> Self {
        Self {
            default_order_ttl: 3,
            max_order_ttl: 10,
            cancel_penalty_pct: 2,
            relist_cooldown_ticks: 2,
            npcs: NpcComposition::default_const(),
        }
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.default_order_ttl < Self::MIN_ORDER_TTL
            || self.default_order_ttl > self.max_order_ttl
        {
            return Err(DomainError::Validation(format!(
                "default_order_ttl must be in [{}, max_order_ttl={}], got {}",
                Self::MIN_ORDER_TTL,
                self.max_order_ttl,
                self.default_order_ttl
            )));
        }
        if self.max_order_ttl > Self::MAX_ORDER_TTL_HARD {
            return Err(DomainError::Validation(format!(
                "max_order_ttl must be ≤ {}, got {}",
                Self::MAX_ORDER_TTL_HARD,
                self.max_order_ttl
            )));
        }
        if self.cancel_penalty_pct > 100 {
            return Err(DomainError::Validation(format!(
                "cancel_penalty_pct must be ≤ 100, got {}",
                self.cancel_penalty_pct
            )));
        }
        if self.npcs.total() > RoomConfig::MAX_NPC {
            return Err(DomainError::Validation(format!(
                "npc composition total must be ≤ {}, got {}",
                RoomConfig::MAX_NPC,
                self.npcs.total()
            )));
        }
        Ok(())
    }
}

impl Default for GameBalance {
    fn default() -> Self {
        Self::default_const()
    }
}

/// Oda ayarları. Motor bu config'i okuyup tick mantığını ona göre işler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomConfig {
    pub preset: Preset,
    /// Gerçek dünya saniyesi — tick başı süre.
    pub tick_seconds: u32,
    /// Sezon toplam tick sayısı.
    pub season_ticks: u32,
    pub npc_count: u8,
    pub max_human_players: u8,
    /// Runtime knob'ları (TTL, ceza, cooldown).
    #[serde(default)]
    pub balance: GameBalance,
}

impl RoomConfig {
    /// Tick süresi üst sınırı (1 saat).
    pub const MAX_TICK_SECONDS: u32 = 3_600;
    /// Tick süresi alt sınırı (10 saniye — motor throttling).
    pub const MIN_TICK_SECONDS: u32 = 10;
    /// Sezon alt sınırı (anlamlı oyun için).
    pub const MIN_SEASON_TICKS: u32 = 10;
    /// Sezon üst sınırı (makul üst limit).
    pub const MAX_SEASON_TICKS: u32 = 10_000;
    /// İnsan oyuncu alt sınırı.
    pub const MIN_HUMAN_PLAYERS: u8 = 2;
    /// İnsan oyuncu üst sınırı (v1 ölçeği).
    pub const MAX_HUMAN_PLAYERS: u8 = 5;
    /// NPC üst sınırı (piyasa likiditesi için makul).
    pub const MAX_NPC: u8 = 40;

    /// Hızlı preset — 2-3 arkadaş 1.5 saatte sezon oynar.
    #[must_use]
    pub const fn hizli() -> Self {
        Self {
            preset: Preset::Hizli,
            tick_seconds: 60,
            season_ticks: 90,
            npc_count: 3,
            max_human_players: 5,
            balance: GameBalance::default_const(),
        }
    }

    /// Standart preset — ~3 gün, günde 3-5 kez giriş.
    #[must_use]
    pub const fn standart() -> Self {
        Self {
            preset: Preset::Standart,
            tick_seconds: 30 * 60,
            season_ticks: 150,
            npc_count: 4,
            max_human_players: 5,
            balance: GameBalance::default_const(),
        }
    }

    /// Uzun preset — ~14 gün, günde 1-2 kez giriş.
    #[must_use]
    pub const fn uzun() -> Self {
        Self {
            preset: Preset::Uzun,
            tick_seconds: 60 * 60,
            season_ticks: 350,
            npc_count: 5,
            max_human_players: 5,
            balance: GameBalance::default_const(),
        }
    }

    /// Custom — kullanıcı tarafından manuel kurulmuş config.
    pub fn custom(
        tick_seconds: u32,
        season_ticks: u32,
        npc_count: u8,
        max_human_players: u8,
    ) -> Result<Self, DomainError> {
        let cfg = Self {
            preset: Preset::Custom,
            tick_seconds,
            season_ticks,
            npc_count,
            max_human_players,
            balance: GameBalance::default_const(),
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Mevcut config'in üstüne yeni `balance` takar (preset/tick değişmez).
    #[must_use]
    pub const fn with_balance(mut self, balance: GameBalance) -> Self {
        self.balance = balance;
        self
    }

    /// Config doğrulaması. Tüm aralıklar ve invariantlar.
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.tick_seconds < Self::MIN_TICK_SECONDS || self.tick_seconds > Self::MAX_TICK_SECONDS
        {
            return Err(DomainError::Validation(format!(
                "tick_seconds must be in [{}, {}], got {}",
                Self::MIN_TICK_SECONDS,
                Self::MAX_TICK_SECONDS,
                self.tick_seconds
            )));
        }
        if self.season_ticks < Self::MIN_SEASON_TICKS || self.season_ticks > Self::MAX_SEASON_TICKS
        {
            return Err(DomainError::Validation(format!(
                "season_ticks must be in [{}, {}], got {}",
                Self::MIN_SEASON_TICKS,
                Self::MAX_SEASON_TICKS,
                self.season_ticks
            )));
        }
        if self.max_human_players < Self::MIN_HUMAN_PLAYERS
            || self.max_human_players > Self::MAX_HUMAN_PLAYERS
        {
            return Err(DomainError::Validation(format!(
                "max_human_players must be in [{}, {}], got {}",
                Self::MIN_HUMAN_PLAYERS,
                Self::MAX_HUMAN_PLAYERS,
                self.max_human_players
            )));
        }
        if self.npc_count > Self::MAX_NPC {
            return Err(DomainError::Validation(format!(
                "npc_count must be ≤ {}, got {}",
                Self::MAX_NPC,
                self.npc_count
            )));
        }
        Ok(())
    }

    /// Piyasa doygunluk eşiği: formül [`crate::balance`]'tan — default
    /// `SATURATION_BASE + (player_count - SATURATION_MIN_PLAYERS) × SATURATION_PER_PLAYER`
    /// = `40 + (n - 2) × 10` (§10).
    ///
    /// Aşan miktar %50 fiyattan satılır (motor Faz 3C'de uygular).
    /// `player_count` = insan + NPC toplamı.
    #[must_use]
    pub fn saturation_threshold(&self, player_count: u8) -> u32 {
        let above_min =
            u32::from(player_count.saturating_sub(crate::balance::SATURATION_MIN_PLAYERS));
        crate::balance::SATURATION_BASE + above_min * crate::balance::SATURATION_PER_PLAYER
    }

    /// Toplam katılımcı kapasitesi (insan + NPC).
    #[must_use]
    pub const fn total_participants(&self) -> u8 {
        self.max_human_players.saturating_add(self.npc_count)
    }
}

impl Default for RoomConfig {
    fn default() -> Self {
        Self::hizli()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hizli_preset_matches_design() {
        let c = RoomConfig::hizli();
        assert_eq!(c.preset, Preset::Hizli);
        assert_eq!(c.tick_seconds, 60);
        assert_eq!(c.season_ticks, 90);
        assert_eq!(c.npc_count, 3);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn standart_preset_matches_design() {
        let c = RoomConfig::standart();
        assert_eq!(c.tick_seconds, 30 * 60);
        assert_eq!(c.season_ticks, 150);
        assert_eq!(c.npc_count, 4);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn uzun_preset_matches_design() {
        let c = RoomConfig::uzun();
        assert_eq!(c.tick_seconds, 60 * 60);
        assert_eq!(c.season_ticks, 350);
        assert_eq!(c.npc_count, 5);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn custom_rejects_too_short_tick() {
        let err = RoomConfig::custom(1, 100, 3, 5).expect_err("too short");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn custom_rejects_too_long_tick() {
        let err = RoomConfig::custom(3_601, 100, 3, 5).expect_err("too long");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn custom_rejects_too_short_season() {
        let err = RoomConfig::custom(60, 5, 3, 5).expect_err("short season");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn custom_rejects_too_few_humans() {
        let err = RoomConfig::custom(60, 100, 3, 1).expect_err("solo");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn custom_rejects_too_many_humans() {
        let err = RoomConfig::custom(60, 100, 3, 6).expect_err("too many");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn custom_rejects_excessive_npcs() {
        let err = RoomConfig::custom(60, 100, 41, 5).expect_err("too many npc");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn custom_accepts_valid_values() {
        let c = RoomConfig::custom(120, 60, 2, 4).unwrap();
        assert_eq!(c.preset, Preset::Custom);
        assert_eq!(c.tick_seconds, 120);
    }

    #[test]
    fn saturation_formula_matches_design() {
        let c = RoomConfig::hizli();
        // 250 + (n-2) × 50 (anti-snowball, hacim 10× ölçek):
        // 2 players → 250
        // 3 players → 300
        // 4 players → 350
        // 5 players → 400
        // 12 players → 750
        assert_eq!(c.saturation_threshold(2), 250);
        assert_eq!(c.saturation_threshold(3), 300);
        assert_eq!(c.saturation_threshold(4), 350);
        assert_eq!(c.saturation_threshold(5), 400);
        assert_eq!(c.saturation_threshold(12), 750);
    }

    #[test]
    fn saturation_handles_below_min() {
        let c = RoomConfig::hizli();
        // player_count < SATURATION_MIN_PLAYERS → base eşiğe sabitlenir.
        assert_eq!(c.saturation_threshold(0), 250);
        assert_eq!(c.saturation_threshold(1), 250);
    }

    #[test]
    fn default_is_hizli_preset() {
        assert_eq!(RoomConfig::default(), RoomConfig::hizli());
    }

    #[test]
    fn total_participants_sums() {
        let c = RoomConfig::hizli();
        assert_eq!(c.total_participants(), 8); // 5 humans + 3 NPC
    }

    #[test]
    fn preset_display_names() {
        assert_eq!(Preset::Hizli.to_string(), "Hızlı");
        assert_eq!(Preset::Standart.to_string(), "Standart");
    }

    #[test]
    fn serde_roundtrip() {
        let c = RoomConfig::uzun();
        let back: RoomConfig = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn npc_composition_total_sums_parts() {
        let c = NpcComposition {
            sanayici: 2,
            tuccar: 2,
            alici: 4,
            esnaf: 2,
            spekulator: 1,
            ciftci: 1,
            banka: 1,
        };
        assert_eq!(c.total(), 13);
    }

    #[test]
    fn npc_composition_default_is_sim_aligned_26_npc() {
        // v8.24: Spek emekli (spekulator=0). Toplam 29 → 26.
        let c = NpcComposition::default_const();
        assert_eq!(c.sanayici, 5);
        assert_eq!(c.tuccar, 4);
        assert_eq!(c.alici, 8);
        assert_eq!(c.esnaf, 0);
        assert_eq!(c.spekulator, 0);
        assert_eq!(c.ciftci, 6);
        assert_eq!(c.banka, 3);
        assert_eq!(c.total(), 26);
    }

    #[test]
    fn game_balance_rejects_excessive_npc_composition() {
        let b = GameBalance {
            default_order_ttl: 3,
            max_order_ttl: 10,
            cancel_penalty_pct: 2,
            relist_cooldown_ticks: 2,
            // Toplam 41 > MAX_NPC=40 → reject.
            npcs: NpcComposition {
                sanayici: 10,
                tuccar: 10,
                alici: 10,
                esnaf: 5,
                spekulator: 6,
                ciftci: 0,
                banka: 0,
            },
        };
        assert!(b.validate().is_err());
    }
}
