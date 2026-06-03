//! Yoğunlaşma / rekabet metrikleri — emergent monopol ölçümü (Faz 0).
//!
//! Sim koşumu sırasında eşleşmeler biriktirilir, sonunda `finalize` ile
//! bucket bazlı arz yoğunlaşması (HHI), en büyük firma payı, fabrika sahipliği
//! yoğunlaşması, servet eşitsizliği (Gini) ve rol bazlı PnL hesaplanır.
//!
//! Saf yardımcılar (`hhi`, `gini`) ayrı tutulur ki test edilebilsin.

use std::collections::BTreeMap;

use moneywar_domain::{CityId, GameState, PlayerId, ProductKind};
use moneywar_engine::leaderboard;

/// Pay dağılımından Herfindahl–Hirschman Endeksi (0..10000).
/// `shares` mutlak değerler (toplama normalize edilir). Tek aktör → 10000
/// (tam monopol), eşit dağılım → ~10000/n.
#[must_use]
pub fn hhi(shares: &[u64]) -> f64 {
    let total: u64 = shares.iter().sum();
    if total == 0 {
        return 0.0;
    }
    let total = total as f64;
    shares
        .iter()
        .map(|&s| {
            let frac = s as f64 / total;
            frac * frac
        })
        .sum::<f64>()
        * 10_000.0
}

/// Gini katsayısı (0=tam eşit, 1=tam eşitsiz). Negatif değerler en küçük
/// değer 0 olacak şekilde kaydırılır (servet skoru negatif olabilir).
#[must_use]
pub fn gini(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let shift = if min < 0.0 { -min } else { 0.0 };
    let mut xs: Vec<f64> = values.iter().map(|v| v + shift).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = xs.len() as f64;
    let sum: f64 = xs.iter().sum();
    if sum <= 0.0 {
        return 0.0;
    }
    // Sıralı formül: G = (2·Σ i·x_i)/(n·Σx) − (n+1)/n,  i 1-tabanlı.
    let weighted: f64 = xs.iter().enumerate().map(|(i, x)| (i as f64 + 1.0) * x).sum();
    ((2.0 * weighted) / (n * sum) - (n + 1.0) / n).clamp(0.0, 1.0)
}

/// Koşum boyunca eşleşme hacmini biriktirir.
#[derive(Debug, Default)]
pub struct MetricsAccumulator {
    /// Bucket → satıcı(PlayerId.value) → eşleşen birim (arz tarafı hakimiyeti).
    sell_by_bucket: BTreeMap<(CityId, ProductKind), BTreeMap<u64, u64>>,
    /// Oyuncu → toplam eşleşen hacim (her iki taraf).
    vol_by_player: BTreeMap<u64, u64>,
    total_vol: u64,
}

impl MetricsAccumulator {
    /// Bir OrderMatched olayını kaydet.
    pub fn record_match(
        &mut self,
        city: CityId,
        product: ProductKind,
        buyer: PlayerId,
        seller: PlayerId,
        qty: u32,
    ) {
        let q = u64::from(qty);
        *self
            .sell_by_bucket
            .entry((city, product))
            .or_default()
            .entry(seller.value())
            .or_default() += q;
        *self.vol_by_player.entry(seller.value()).or_default() += q;
        *self.vol_by_player.entry(buyer.value()).or_default() += q;
        self.total_vol += q;
    }

    /// Koşum sonu metrikleri — final state ile birlikte.
    #[must_use]
    pub fn finalize(&self, state: &GameState) -> Metrics {
        // Arz HHI: hacimli her bucket için satıcı yoğunlaşması, ortalama.
        let bucket_hhis: Vec<f64> = self
            .sell_by_bucket
            .values()
            .filter(|sellers| !sellers.is_empty())
            .map(|sellers| {
                let shares: Vec<u64> = sellers.values().copied().collect();
                hhi(&shares)
            })
            .collect();
        let supply_hhi = if bucket_hhis.is_empty() {
            0.0
        } else {
            bucket_hhis.iter().sum::<f64>() / bucket_hhis.len() as f64
        };

        // En büyük firma toplam hacim payı (%).
        let top_firm_share = if self.total_vol == 0 {
            0.0
        } else {
            let max = self.vol_by_player.values().copied().max().unwrap_or(0);
            // vol_by_player her eşleşmeyi iki kez sayar (alıcı+satıcı) → toplam 2×total_vol.
            max as f64 / (2.0 * self.total_vol as f64) * 100.0
        };

        // Fabrika sahipliği yoğunlaşması.
        let mut fac_by_owner: BTreeMap<u64, u64> = BTreeMap::new();
        for f in state.factories.values() {
            *fac_by_owner.entry(f.owner.value()).or_default() += 1;
        }
        let factory_hhi = hhi(&fac_by_owner.values().copied().collect::<Vec<_>>());

        // Servet Gini (leaderboard net değer skoru üzerinden).
        let scores = leaderboard(state);
        let wealth: Vec<f64> = scores.iter().map(|s| s.total.as_cents() as f64).collect();
        let wealth_gini = gini(&wealth);

        // Rol bazlı toplam PnL (lira).
        let mut role_pnl: BTreeMap<String, f64> = BTreeMap::new();
        for s in &scores {
            if let Some(p) = state.players.get(&s.player_id) {
                let label = p
                    .npc_kind
                    .map_or_else(|| "?".to_string(), |k| k.label().to_string());
                *role_pnl.entry(label).or_default() += s.total.as_cents() as f64 / 100.0;
            }
        }
        let mut role_pnl: Vec<(String, f64)> = role_pnl.into_iter().collect();
        role_pnl.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        Metrics {
            supply_hhi,
            top_firm_share,
            factory_hhi,
            wealth_gini,
            role_pnl,
        }
    }
}

/// Koşum sonu yoğunlaşma metrikleri.
#[derive(Debug)]
pub struct Metrics {
    /// Bucket bazlı arz HHI ortalaması (0..10000). >2500 yoğun, →10000 monopol.
    pub supply_hhi: f64,
    /// En büyük firmanın toplam hacim payı (%).
    pub top_firm_share: f64,
    /// Fabrika sahipliği HHI (0..10000).
    pub factory_hhi: f64,
    /// Servet Gini (0=eşit, 1=tek elde toplanmış).
    pub wealth_gini: f64,
    /// Rol → toplam PnL (lira), azalan.
    pub role_pnl: Vec<(String, f64)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hhi_monopoly_is_max() {
        assert!((hhi(&[100]) - 10_000.0).abs() < 1e-9);
    }

    #[test]
    fn hhi_equal_split_is_inverse_n() {
        // 4 eşit aktör → her biri 0.25, HHI = 4 × 0.0625 × 10000 = 2500.
        assert!((hhi(&[25, 25, 25, 25]) - 2_500.0).abs() < 1e-9);
    }

    #[test]
    fn hhi_empty_is_zero() {
        assert_eq!(hhi(&[]), 0.0);
        assert_eq!(hhi(&[0, 0]), 0.0);
    }

    #[test]
    fn gini_perfect_equality_is_zero() {
        assert!(gini(&[10.0, 10.0, 10.0, 10.0]) < 1e-9);
    }

    #[test]
    fn gini_extreme_inequality_near_one() {
        // Tek kişide tüm servet → Gini ≈ (n-1)/n.
        let g = gini(&[0.0, 0.0, 0.0, 100.0]);
        assert!(g > 0.7, "aşırı eşitsizlik yüksek Gini vermeli, got {g}");
    }

    #[test]
    fn gini_handles_negatives_via_shift() {
        // Negatif değerler kaydırılır; panik/NaN olmamalı.
        let g = gini(&[-50.0, 0.0, 50.0, 100.0]);
        assert!((0.0..=1.0).contains(&g));
    }

    #[test]
    fn gini_single_value_is_zero() {
        assert_eq!(gini(&[42.0]), 0.0);
    }
}
