//! NPC kişilik arketipleri ve utility AI ağırlıkları.
//!
//! Her arketip **gerçek finansal piyasa stratejisinden** ilham alır:
//! Aggressive (yüksek risk), Trend Follower (momentum), Mean Reverter
//! (contrarian), Arbitrageur (şehirler arası rant), Event Trader
//! (haber-reaktif), Hoarder (sabırlı stoklama), Cartel (manipülatör).
//!
//! Sezon başında her NPC'ye seed RNG ile bir arketip atanır → sezon boyu
//! sabit (replay safety). Aynı arketipli NPC'ler birbirini takip etme
//! eğiliminde — bandwagon/cluster davranışı için temel.

/// 7 strateji arketipi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Personality {
    /// ⚡ Atılgan — yüksek risk toleransı, hızlı yatırım, agresif fiyat
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

    /// Bu arketipin utility AI ağırlıkları. Ağırlıklar utility puanı
    /// hesabında çarpan görevi görür — örn. `Aggressive` yüksek `profit`
    /// ağırlığı ve düşük `risk_aversion` ile yüksek-getirili-yüksek-riskli
    /// aksiyonları sever.
    #[must_use]
    pub const fn weights(self) -> Weights {
        match self {
            Self::Aggressive => Weights {
                profit: 1.2,
                risk_aversion: 0.2,
                urgency: 1.5,
                momentum_bias: 0.3,
                patience: 0.1,
                arbitrage_bias: 0.5,
                event_response: 1.0,
            },
            Self::TrendFollower => Weights {
                profit: 1.0,
                risk_aversion: 0.5,
                urgency: 1.0,
                momentum_bias: 1.0,
                patience: 0.3,
                arbitrage_bias: 0.4,
                event_response: 0.8,
            },
            Self::MeanReverter => Weights {
                profit: 1.0,
                risk_aversion: 0.7,
                urgency: 0.6,
                momentum_bias: -0.8,
                patience: 0.9,
                arbitrage_bias: 0.5,
                event_response: 0.6,
            },
            Self::Arbitrageur => Weights {
                profit: 1.1,
                risk_aversion: 0.4,
                urgency: 1.2,
                momentum_bias: 0.0,
                patience: 0.4,
                arbitrage_bias: 1.5,
                event_response: 0.7,
            },
            Self::EventTrader => Weights {
                profit: 1.0,
                risk_aversion: 0.3,
                urgency: 1.8,
                momentum_bias: 0.5,
                patience: 0.4,
                arbitrage_bias: 0.6,
                event_response: 1.6,
            },
            Self::Hoarder => Weights {
                profit: 0.9,
                risk_aversion: 0.8,
                urgency: 0.4,
                momentum_bias: -0.3,
                patience: 1.0,
                arbitrage_bias: 0.3,
                event_response: 0.5,
            },
            Self::Cartel => Weights {
                profit: 1.3,
                risk_aversion: 0.3,
                urgency: 1.0,
                momentum_bias: 0.6,
                patience: 0.5,
                arbitrage_bias: 0.7,
                event_response: 0.8,
            },
        }
    }
}

/// Utility AI ağırlık vektörü. Her parametre 0..2 aralığında, default 1.0
/// (nötr). Negatif değer ters yönde ağırlık (örn. Mean Reverter momentum'a
/// negatif tepki).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    /// Beklenen kâr ağırlığı — yüksek = kâra duyarlı, düşük = kâra ilgisiz
    pub profit: f64,
    /// Risk kaçınma — yüksek = riskten kaçar, düşük = riski tolere eder
    pub risk_aversion: f64,
    /// Aciliyet tepkisi — yüksek = TTL bitmek üzere fırsatları sever
    pub urgency: f64,
    /// Momentum bias — pozitif = trend follower, negatif = contrarian
    pub momentum_bias: f64,
    /// Sabırlılık — yüksek = ucuz alıma kadar bekle, düşük = hızlı al
    pub patience: f64,
    /// Arbitraj bias — yüksek = şehirler arası rota fırsatlarını sever
    pub arbitrage_bias: f64,
    /// Olay tepkisi — yüksek = news-reactive, olay-driven aksiyonlara odaklı
    pub event_response: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_seven_archetypes_present() {
        assert_eq!(Personality::ALL.len(), 7);
    }

    #[test]
    fn emoji_unique_per_archetype() {
        let mut emojis: Vec<_> = Personality::ALL.iter().map(|p| p.emoji()).collect();
        emojis.sort_unstable();
        let n_total = emojis.len();
        emojis.dedup();
        assert_eq!(emojis.len(), n_total, "emojiler benzersiz olmalı");
    }

    #[test]
    fn label_unique_per_archetype() {
        let mut labels: Vec<_> = Personality::ALL.iter().map(|p| p.label()).collect();
        labels.sort_unstable();
        let n_total = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), n_total);
    }

    #[test]
    fn aggressive_has_low_risk_aversion() {
        let w = Personality::Aggressive.weights();
        assert!(w.risk_aversion < 0.5, "Aggressive risk almayı sever");
        assert!(w.urgency > 1.0, "Aggressive aciliyete pozitif tepki");
    }

    #[test]
    fn hoarder_high_patience_low_urgency() {
        let w = Personality::Hoarder.weights();
        assert!(w.patience >= 0.9);
        assert!(w.urgency < 0.5);
    }

    #[test]
    fn trend_follower_positive_momentum() {
        let w = Personality::TrendFollower.weights();
        assert!(w.momentum_bias > 0.5);
    }

    #[test]
    fn mean_reverter_negative_momentum() {
        let w = Personality::MeanReverter.weights();
        assert!(w.momentum_bias < -0.5);
    }

    #[test]
    fn arbitrageur_high_arbitrage_bias() {
        let w = Personality::Arbitrageur.weights();
        assert!(w.arbitrage_bias > 1.0);
    }

    #[test]
    fn event_trader_high_event_response() {
        let w = Personality::EventTrader.weights();
        assert!(w.event_response > 1.4);
    }
}
