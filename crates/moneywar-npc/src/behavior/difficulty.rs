//! Difficulty parametre seti — top-K, silence, noise, min_score.
//!
//! Eski fuzzy `DifficultyModulator`'ın yerini alır. Daha sade çünkü utility
//! motoru aggressiveness multiplier'ına ihtiyaç duymuyor (ağırlık tablosu zaten
//! kişilik etkisini taşır).

/// Davranış motoru zorluk parametreleri. `decide_behavior` bunları okur:
/// - `top_k`: aday listesinden seçilecek max aksiyon sayısı
/// - `silence_per_10`: tick atlama olasılığı (5 → %50 sessiz)
/// - `noise`: skora ek rastgele gürültü ölçeği (gerçekçilik için)
/// - `min_score`: aday emit eşiği — bunun altı düşer
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BehaviorDifficulty {
    pub top_k: u32,
    pub silence_per_10: u32,
    pub noise: f64,
    pub min_score: f64,
}

impl BehaviorDifficulty {
    /// Hard — odaklı emit. NPC'ler en iyi K aksiyona indirgenir; daha az ama
    /// daha doğru emir → kitap dolar gürültü düşer, match verim artar.
    /// Önceki 32 ile Spekülatör 36 aday geliyordu, kitap kaynatıyordu;
    /// 12 ile en kıymetli 12 aksiyon (Sanayici Build/BUY/SAT seti, Tüccar
    /// arbitraj çiftleri, vs.) emit edilir.
    pub const HARD: Self = Self {
        top_k: 12,
        silence_per_10: 0,
        noise: 0.05,
        min_score: 0.0,
    };

    /// Medium — yarısı kadar aksiyon, hafif sessizlik.
    pub const MEDIUM: Self = Self {
        top_k: 6,
        silence_per_10: 1,
        noise: 0.10,
        min_score: 0.10,
    };

    /// Easy — Hard'ın opposite'i: bol likidite (top_k yüksek), AMA NPC
    /// fiyatları human lehine cömert (bkz. `state.market_softener_pct=15`,
    /// pricing helper'larda Easy mode'da floor düşer / ceiling yükselir).
    /// Eski Easy (top_k=2, silence=3) piyasayı donuklaştırıyordu — human
    /// emir veriyor ama karşı taraf yok hissi. Şimdi NPC'ler aktif emir
    /// basıyor (likidite), sadece marjları yumuşak (human cömertlik).
    pub const EASY: Self = Self {
        top_k: 8,
        silence_per_10: 0,
        noise: 0.05,
        min_score: 0.05,
    };
}

impl Default for BehaviorDifficulty {
    fn default() -> Self {
        Self::MEDIUM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easy_has_high_liquidity() {
        // v8.22: Easy artık top_k yüksek (likidite bol), silence yok.
        // Cömertlik state.market_softener_pct ile ayrı → Difficulty
        // sadece akış kontrol parametresi.
        assert!(BehaviorDifficulty::EASY.top_k >= 8);
        assert_eq!(BehaviorDifficulty::EASY.silence_per_10, 0);
    }

    #[test]
    fn hard_has_max_aggression() {
        // Hard her tick max top_k emit, sıfır sessizlik.
        assert_eq!(BehaviorDifficulty::HARD.top_k, 12);
        assert_eq!(BehaviorDifficulty::HARD.silence_per_10, 0);
    }

    #[test]
    fn default_is_medium() {
        assert_eq!(BehaviorDifficulty::default(), BehaviorDifficulty::MEDIUM);
    }
}
