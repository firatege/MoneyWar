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
    /// Hard — agresif, sessizlik az, eşik düşük.
    pub const HARD: Self = Self {
        top_k: 5,
        silence_per_10: 0,
        noise: 0.05,
        min_score: 0.0,
    };

    /// Medium — dengeli rekabet.
    pub const MEDIUM: Self = Self {
        top_k: 3,
        silence_per_10: 1,
        noise: 0.10,
        min_score: 0.10,
    };

    /// Easy — yavaş, seçici, eşik yüksek.
    pub const EASY: Self = Self {
        top_k: 1,
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
