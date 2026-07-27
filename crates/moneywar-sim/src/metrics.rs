//! Rol-ayrık yoğunlaşma / rekabet metrikleri — emergent monopol ölçümü (Faz 0+).
//!
//! Sanayici ve Tüccar farklı eksenlerden ölçülür:
//! - Sanayici → üretim hakimiyeti: fab sahipliği HHI, üretim bucket HHI,
//!   ham madde alım yoğunlaşması, toplam üretilen birim.
//! - Tüccar → lojistik hakimiyeti: kervan dispatch HHI, şehir-çifti akış
//!   HHI, taşınan toplam birim.
//!
//! Ortak metrikler: servet Gini, rol `PnL` tablosu.
//!
//! Saf yardımcılar (`hhi`, `gini`) ayrı tutulur ki test edilebilsin.

use std::collections::BTreeMap;

use moneywar_domain::{CityId, GameState, NpcKind, PlayerId, ProductKind};
use moneywar_engine::leaderboard;

// =============================================================================
// Saf matematik yardımcıları
// =============================================================================

/// Pay dağılımından Herfindahl–Hirschman Endeksi (0..10000).
/// Tek aktör → 10000 (tam monopol), n eşit aktör → 10000/n.
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

/// Gini katsayısı (0=tam eşit, 1=tek elde). Negatif değerler kaydırılır.
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
    let weighted: f64 = xs.iter().enumerate().map(|(i, x)| (i as f64 + 1.0) * x).sum();
    ((2.0 * weighted) / (n * sum) - (n + 1.0) / n).clamp(0.0, 1.0)
}

// =============================================================================
// Biriktirici (sim loop'unda her olay kaydedilir)
// =============================================================================

/// Koşum boyunca rol-ayrık metrikleri biriktirir.
#[derive(Debug, Default)]
pub struct MetricsAccumulator {
    // --- Sanayici ekseni ---
    /// Üretim: bucket → sanayici(id) → üretilen birim.
    production_by_bucket: BTreeMap<(CityId, ProductKind), BTreeMap<u64, u64>>,
    /// Ham madde alımı: sanayici(id) → satın alınan birim (tedarik yoğunlaşması).
    raw_buy_by_sanayici: BTreeMap<u64, u64>,

    // --- Tüccar ekseni ---
    /// Kervan taşıması: tüccar(id) → taşınan toplam birim.
    caravan_vol_by_tuccar: BTreeMap<u64, u64>,
    /// Şehir-çifti akış: (from, to) → tüccar(id) → taşınan birim.
    flow_by_route: BTreeMap<(CityId, CityId), BTreeMap<u64, u64>>,

    // --- Ortak ---
    /// Toplam eşleşme hacmi (tüm roller, iki taraf).
    total_vol: u64,
    /// Toplam kervan taşıma birimi.
    total_caravan_vol: u64,
}

impl MetricsAccumulator {
    /// `OrderMatched` olayı. `seller_kind` rolü sanayici ham alım tespiti için.
    pub fn record_match(
        &mut self,
        _city: CityId,
        product: ProductKind,
        buyer: PlayerId,
        buyer_kind: Option<NpcKind>,
        _seller: PlayerId,
        qty: u32,
    ) {
        let q = u64::from(qty);
        self.total_vol += q;

        // Sanayici ham madde alıyorsa → tedarik yoğunlaşması.
        if buyer_kind == Some(NpcKind::Sanayici) && product.is_raw() {
            *self.raw_buy_by_sanayici.entry(buyer.value()).or_default() += q;
        }
    }

    /// `ProductionCompleted` olayı — Sanayici üretim hakimiyeti.
    pub fn record_production(
        &mut self,
        city: CityId,
        product: ProductKind,
        owner: PlayerId,
        units: u32,
    ) {
        *self
            .production_by_bucket
            .entry((city, product))
            .or_default()
            .entry(owner.value())
            .or_default() += u64::from(units);
    }

    /// `CaravanArrived` olayı — Tüccar lojistik hakimiyeti.
    pub fn record_caravan(
        &mut self,
        from: CityId,
        to: CityId,
        owner: PlayerId,
        owner_kind: Option<NpcKind>,
        cargo_units: u32,
    ) {
        if owner_kind != Some(NpcKind::Tuccar) {
            return; // Sadece Tüccar kervan metrikleri burada
        }
        let q = u64::from(cargo_units);
        self.total_caravan_vol += q;
        *self.caravan_vol_by_tuccar.entry(owner.value()).or_default() += q;
        *self
            .flow_by_route
            .entry((from, to))
            .or_default()
            .entry(owner.value())
            .or_default() += q;
    }

    /// Koşum sonu rol-ayrık metrikler.
    #[must_use]
    pub fn finalize(&self, state: &GameState) -> Metrics {
        // ── Sanayici metrikleri ──────────────────────────────────────────────

        // Fabrika sahipliği HHI.
        let mut fac_by_owner: BTreeMap<u64, u64> = BTreeMap::new();
        for f in state.factories.values() {
            if state.players.get(&f.owner).and_then(|p| p.npc_kind)
                == Some(NpcKind::Sanayici)
            {
                *fac_by_owner.entry(f.owner.value()).or_default() += 1;
            }
        }
        let factory_hhi = hhi(&fac_by_owner.values().copied().collect::<Vec<_>>());
        let factory_counts: Vec<(String, u64)> = {
            let mut v: Vec<_> = fac_by_owner
                .iter()
                .map(|(id, cnt)| {
                    let name = state
                        .players
                        .get(&PlayerId::new(*id))
                        .map_or_else(|| format!("#{id}"), |p| p.name.clone());
                    (name, *cnt)
                })
                .collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            v
        };

        // Üretim HHI: bucket başına sanayici yoğunlaşması ortalaması.
        let prod_hhis: Vec<f64> = self
            .production_by_bucket
            .values()
            .filter(|m| !m.is_empty())
            .map(|m| hhi(&m.values().copied().collect::<Vec<_>>()))
            .collect();
        let production_hhi = avg(&prod_hhis);

        // Toplam üretilen birim.
        let total_produced: u64 = self
            .production_by_bucket
            .values()
            .flat_map(|m| m.values())
            .sum();

        // Ham madde alım HHI (tedarik zinciri hakimiyeti).
        let raw_supply_hhi = hhi(&self.raw_buy_by_sanayici.values().copied().collect::<Vec<_>>());

        // ── Tüccar metrikleri ────────────────────────────────────────────────

        // Kervan dispatch HHI (kim ne kadar taşıdı).
        let caravan_hhi =
            hhi(&self.caravan_vol_by_tuccar.values().copied().collect::<Vec<_>>());

        // Şehir-çifti (rota) HHI: hangi rota en yoğun ve kim domine ediyor.
        let route_hhis: Vec<f64> = self
            .flow_by_route
            .values()
            .filter(|m| !m.is_empty())
            .map(|m| hhi(&m.values().copied().collect::<Vec<_>>()))
            .collect();
        let route_dominance_hhi = avg(&route_hhis);

        // En aktif rota (taşınan birim).
        let top_route: Option<((CityId, CityId), u64)> = self
            .flow_by_route
            .iter()
            .map(|(route, owners)| (*route, owners.values().sum::<u64>()))
            .max_by_key(|(_, vol)| *vol);

        // ── Ortak ───────────────────────────────────────────────────────────

        let scores = leaderboard(state);
        // Gini yalnız **yarışan** roller arasında: hane halkı ve banka
        // dahilken sayı firmalar arası uçurumu değil rol farkını ölçüyordu.
        let wealth: Vec<f64> = scores
            .iter()
            .filter(|s| {
                state
                    .players
                    .get(&s.player_id)
                    .and_then(|p| p.npc_kind)
                    .is_none_or(NpcKind::is_competitor)
            })
            .map(|s| s.total.as_cents() as f64)
            .collect();
        let wealth_gini = gini(&wealth);

        // Rol bazlı toplam PnL.
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
            // Sanayici
            factory_hhi,
            factory_counts,
            production_hhi,
            total_produced,
            raw_supply_hhi,
            // Tüccar
            caravan_hhi,
            route_dominance_hhi,
            top_route,
            total_caravan_vol: self.total_caravan_vol,
            // Ortak
            wealth_gini,
            role_pnl,
        }
    }
}

fn avg(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

// =============================================================================
// Sonuç
// =============================================================================

/// Koşum sonu rol-ayrık yoğunlaşma metrikleri.
#[derive(Debug)]
pub struct Metrics {
    // ── Sanayici ──────────────────────────────────────────────────────────────
    /// Fabrika sahipliği HHI (sadece Sanayiciler). 3333 = 3 eşit Sanayici.
    pub factory_hhi: f64,
    /// Sanayici başına fabrika sayısı (isim, adet), azalan.
    pub factory_counts: Vec<(String, u64)>,
    /// Üretim bucket HHI ortalaması — üretim ne kadar tek elde.
    pub production_hhi: f64,
    /// Sezon boyunca toplam üretilen birim.
    pub total_produced: u64,
    /// Ham madde alım HHI — tedarik zincirini kim kontrol ediyor.
    pub raw_supply_hhi: f64,

    // ── Tüccar ────────────────────────────────────────────────────────────────
    /// Kervan taşıma HHI — lojistiği kim domine ediyor.
    pub caravan_hhi: f64,
    /// Şehir-çifti rota HHI ortalaması — rotalar ne kadar tekelleşmiş.
    pub route_dominance_hhi: f64,
    /// En aktif rota ve birimi.
    pub top_route: Option<((CityId, CityId), u64)>,
    /// Toplam kervan taşıma birimi.
    pub total_caravan_vol: u64,

    // ── Ortak ─────────────────────────────────────────────────────────────────
    /// Servet Gini (tüm roller, 0=eşit, 1=tek elde).
    pub wealth_gini: f64,
    /// Rol → toplam `PnL` (lira), azalan.
    pub role_pnl: Vec<(String, f64)>,
}

// =============================================================================
// Testler
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hhi_monopoly_is_max() {
        assert!((hhi(&[100]) - 10_000.0).abs() < 1e-9);
    }

    #[test]
    fn hhi_equal_split_is_inverse_n() {
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
        let g = gini(&[0.0, 0.0, 0.0, 100.0]);
        assert!(g > 0.7, "aşırı eşitsizlik yüksek Gini vermeli, got {g}");
    }

    #[test]
    fn gini_handles_negatives_via_shift() {
        let g = gini(&[-50.0, 0.0, 50.0, 100.0]);
        assert!((0.0..=1.0).contains(&g));
    }

    #[test]
    fn gini_single_value_is_zero() {
        assert_eq!(gini(&[42.0]), 0.0);
    }
}
