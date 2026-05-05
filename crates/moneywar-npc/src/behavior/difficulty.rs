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

    /// Easy — sadece en iyi aksiyon, çoğu tick sessiz.
    pub const EASY: Self = Self {
        top_k: 2,
        silence_per_10: 3,
        noise: 0.20,
        min_score: 0.20,
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
    fn presets_are_monotonic_by_top_k() {
        // Easy < Medium < Hard
        assert!(BehaviorDifficulty::EASY.top_k < BehaviorDifficulty::MEDIUM.top_k);
        assert!(BehaviorDifficulty::MEDIUM.top_k < BehaviorDifficulty::HARD.top_k);
    }

    #[test]
    fn easy_silences_more_than_hard() {
        assert!(BehaviorDifficulty::EASY.silence_per_10 > BehaviorDifficulty::HARD.silence_per_10);
    }

    #[test]
    fn min_score_threshold_descends_with_difficulty() {
        // Easy seçici (yüksek eşik), Hard hep emit (düşük eşik)
        assert!(BehaviorDifficulty::EASY.min_score > BehaviorDifficulty::MEDIUM.min_score);
        assert!(BehaviorDifficulty::MEDIUM.min_score > BehaviorDifficulty::HARD.min_score);
    }

    #[test]
    fn default_is_medium() {
        assert_eq!(BehaviorDifficulty::default(), BehaviorDifficulty::MEDIUM);
    }
}
