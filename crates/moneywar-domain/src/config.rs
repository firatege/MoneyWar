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
    pub const MAX_NPC: u8 = 20;

    /// Hızlı preset — 2-3 arkadaş 1.5 saatte sezon oynar.
    #[must_use]
    pub const fn hizli() -> Self {
        Self {
            preset: Preset::Hizli,
            tick_seconds: 60,
            season_ticks: 90,
            npc_count: 3,
            max_human_players: 5,
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
        };
        cfg.validate()?;
        Ok(cfg)
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

    /// Piyasa doygunluk eşiği: `40 + (player_count - 2) × 10` (§10).
    ///
    /// Aşan miktar %50 fiyattan satılır (motor Faz 3'te uygular).
    /// `player_count` = insan + NPC toplamı.
    #[must_use]
    pub fn saturation_threshold(&self, player_count: u8) -> u32 {
        let above_min = u32::from(player_count.saturating_sub(2));
        40 + above_min * 10
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
        let err = RoomConfig::custom(60, 100, 21, 5).expect_err("too many npc");
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
        // §10 tablo:
        // 2 players → 40
        // 3 players → 50
        // 4 players → 60
        // 5 players → 70
        assert_eq!(c.saturation_threshold(2), 40);
        assert_eq!(c.saturation_threshold(3), 50);
        assert_eq!(c.saturation_threshold(4), 60);
        assert_eq!(c.saturation_threshold(5), 70);
    }

    #[test]
    fn saturation_handles_below_min() {
        let c = RoomConfig::hizli();
        // player_count=0 should saturate_sub(2) = 0 → threshold = 40
        assert_eq!(c.saturation_threshold(0), 40);
        assert_eq!(c.saturation_threshold(1), 40);
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
}
