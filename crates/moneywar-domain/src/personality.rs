//! NPC kişilik arketipleri — domain-level enum.
//!
//! Kişilik utility AI ağırlıklarını shape'ler. Ağırlıkların kendisi
//! `moneywar-npc` crate'inde (domain → engine bağımlılığı yok).
//!
//! Sezon başında her NPC'ye seed RNG ile bir arketip atanır → sezon boyu
//! sabit (replay safety). Aynı arketipli NPC'ler birbirini takip etme
//! eğilimindeler — bandwagon davranışı için temel.

use serde::{Deserialize, Serialize};

/// 7 strateji arketipi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Personality {
    /// ⚡ Atılgan — yüksek risk, hızlı yatırım, agresif fiyat
    Aggressive,
    /// 📈 Trend Follower — fiyat momentum'unu kovala
    TrendFollower,
    /// 🔄 Mean Reverter — aşırı fiyatı tersine çevir, contrarian
    MeanReverter,
    /// 🛣️ Arbitrageur — şehirler arası fiyat farkı, sürekli kervan
    Arbitrageur,
    /// 🎲 Event Trader — haber/olay-reaktif, Gold tier abone
    EventTrader,
    /// 📦 Hoarder — sezon erken stokla, Hasat'ta sat
    Hoarder,
    /// 💀 Cartel — piyasayı manipüle et, dump-and-pump
    Cartel,
}

impl Personality {
    /// Tüm arketipler — seed RNG için iter sırası (deterministik).
    pub const ALL: [Self; 7] = [
        Self::Aggressive,
        Self::TrendFollower,
        Self::MeanReverter,
        Self::Arbitrageur,
        Self::EventTrader,
        Self::Hoarder,
        Self::Cartel,
    ];

    /// Tek karakterlik emoji — leaderboard ve chatter'da görsel ipucu.
    #[must_use]
    pub const fn emoji(self) -> &'static str {
        match self {
            Self::Aggressive => "⚡",
            Self::TrendFollower => "📈",
            Self::MeanReverter => "🔄",
            Self::Arbitrageur => "🛣",
            Self::EventTrader => "🎲",
            Self::Hoarder => "📦",
            Self::Cartel => "💀",
        }
    }

    /// İnsan-okur Türkçe etiket.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Aggressive => "Atılgan",
            Self::TrendFollower => "Momentum",
            Self::MeanReverter => "Tersçi",
            Self::Arbitrageur => "Arbitrajcı",
            Self::EventTrader => "Fırsatçı",
            Self::Hoarder => "Stoklayıcı",
            Self::Cartel => "Kartel",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_seven_archetypes_present() {
        assert_eq!(Personality::ALL.len(), 7);
    }

    #[test]
    fn emoji_unique() {
        let mut e: Vec<_> = Personality::ALL.iter().map(|p| p.emoji()).collect();
        e.sort_unstable();
        let n = e.len();
        e.dedup();
        assert_eq!(e.len(), n);
    }

    #[test]
    fn serde_roundtrip() {
        let p = Personality::Aggressive;
        let json = serde_json::to_string(&p).unwrap();
        let back: Personality = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
