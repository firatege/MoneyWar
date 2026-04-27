//! NPC kişilik arketipleri için utility AI ağırlıkları.
//!
//! `Personality` enum domain'de (`moneywar_domain::Personality`). Burada
//! sadece her arketipin **utility ağırlık vektörü** (Weights) hesaplanır.

use moneywar_domain::Personality;

/// Utility AI ağırlık vektörü. Her parametre 0..2 aralığında (default 1.0).
/// Negatif değer ters yönde ağırlık (Mean Reverter momentum'a negatif tepki).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    pub profit: f64,
    pub risk_aversion: f64,
    pub urgency: f64,
    pub momentum_bias: f64,
    pub patience: f64,
    pub arbitrage_bias: f64,
    pub event_response: f64,
}

/// Kişiliğin utility AI ağırlıkları.
#[must_use]
pub const fn weights_for(personality: Personality) -> Weights {
    match personality {
        Personality::Aggressive => Weights {
            profit: 1.2,
            risk_aversion: 0.2,
            urgency: 1.5,
            momentum_bias: 0.3,
            patience: 0.1,
            arbitrage_bias: 0.5,
            event_response: 1.0,
        },
        Personality::TrendFollower => Weights {
            profit: 1.0,
            risk_aversion: 0.5,
            urgency: 1.0,
            momentum_bias: 1.0,
            patience: 0.3,
            arbitrage_bias: 0.4,
            event_response: 0.8,
        },
        Personality::MeanReverter => Weights {
            profit: 1.0,
            risk_aversion: 0.7,
            urgency: 0.6,
            momentum_bias: -0.8,
            patience: 0.9,
            arbitrage_bias: 0.5,
            event_response: 0.6,
        },
        Personality::Arbitrageur => Weights {
            profit: 1.1,
            risk_aversion: 0.4,
            urgency: 1.2,
            momentum_bias: 0.0,
            patience: 0.4,
            arbitrage_bias: 1.5,
            event_response: 0.7,
        },
        Personality::EventTrader => Weights {
            profit: 1.0,
            risk_aversion: 0.3,
            urgency: 1.8,
            momentum_bias: 0.5,
            patience: 0.4,
            arbitrage_bias: 0.6,
            event_response: 1.6,
        },
        Personality::Hoarder => Weights {
            profit: 0.9,
            risk_aversion: 0.8,
            urgency: 0.4,
            momentum_bias: -0.3,
            patience: 1.0,
            arbitrage_bias: 0.3,
            event_response: 0.5,
        },
        Personality::Cartel => Weights {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggressive_low_risk_aversion() {
        let w = weights_for(Personality::Aggressive);
        assert!(w.risk_aversion < 0.5);
        assert!(w.urgency > 1.0);
    }

    #[test]
    fn hoarder_high_patience_low_urgency() {
        let w = weights_for(Personality::Hoarder);
        assert!(w.patience >= 0.9);
        assert!(w.urgency < 0.5);
    }

    #[test]
    fn trend_follower_positive_momentum() {
        assert!(weights_for(Personality::TrendFollower).momentum_bias > 0.5);
    }

    #[test]
    fn mean_reverter_negative_momentum() {
        assert!(weights_for(Personality::MeanReverter).momentum_bias < -0.5);
    }

    #[test]
    fn arbitrageur_high_arbitrage_bias() {
        assert!(weights_for(Personality::Arbitrageur).arbitrage_bias > 1.0);
    }

    #[test]
    fn event_trader_high_event_response() {
        assert!(weights_for(Personality::EventTrader).event_response > 1.4);
    }
}
