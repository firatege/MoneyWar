//! Toptancı (eski Esnaf) rule base — aracı katman.
//!
//! Plan v4 Faz 4:
//! - Çiftçi'den ham madde alır (BUY raw)
//! - Sanayici'ye / Alıcı'ya satar (SELL raw / mamul stok varsa)
//! - Stok arbitrajı: ucuz al, normal/üst marjla sat
//! - Likidite kolu: cash düşükse alım kıs, satışı agresifleştir
//!
//! Action permission matrix `decide.rs` içinde — burada sadece skor üretiriz;
//! gate ham/mamul ayrımını yapar.

use crate::engine::vars::build_standard_vars;
use crate::fuzzy::{Engine, Rule};

#[must_use]
pub fn build_engine() -> Engine {
    let mut e = Engine::new();
    for v in build_standard_vars() {
        e = e.add_var(v);
    }

    e
        // ── BUY (Çiftçi'den ham al — temel görev) ──
        // 1. Fiyat ucuz + nakit var → al (stok kursun)
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "dusuk")
                .when("cash", "yuksek")
                .then("buy_score", 0.95),
        )
        // 2. Fiyat ucuz + nakit orta → orta agresiflik
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "dusuk")
                .when("cash", "orta")
                .then("buy_score", 0.75),
        )
        // 3. Stok düşük + arz yüksek → fırsat
        .add_rule(
            Rule::new()
                .when("stock", "dusuk")
                .when("ask_supply_ratio", "yuksek")
                .then("buy_score", 0.85),
        )
        // 3b. Tuning v6: arz yüksek + cash orta → al (Çiftçi'nin satışını yakala)
        .add_rule(
            Rule::new()
                .when("ask_supply_ratio", "yuksek")
                .when("cash", "orta")
                .then("buy_score", 0.7),
        )
        // 3c. Tuning v6: arz orta + stok orta → orta agresif al
        .add_rule(
            Rule::new()
                .when("ask_supply_ratio", "orta")
                .when("stock", "orta")
                .then("buy_score", 0.55),
        )
        // 3d. Tuning v6: fiyat orta + cash orta + stok düşük → al (default piyasa)
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "orta")
                .when("cash", "orta")
                .when("stock", "dusuk")
                .then("buy_score", 0.6),
        )
        // 3e. Tuning v6.5: lokal uzmanlık → şehir specialty raw'ı yüksek öncelik
        // Bu tie-break sorununu çözer: aynı buy_score'lu adaylar arasında
        // Esnaf-X İzmir Zeytin'i, Esnaf-Y Ankara Bugday'ı, Esnaf-Z Istanbul Pamuk'u tercih eder.
        .add_rule(
            Rule::new()
                .when("local_raw_advantage", "yuksek")
                .when("cash", "orta")
                .then("buy_score", 0.9),
        )
        // 4. Stok düşük + nakit yüksek → al (raf doldur)
        .add_rule(
            Rule::new()
                .when("stock", "dusuk")
                .when("cash", "yuksek")
                .then("buy_score", 0.7),
        )
        // 5. Cash dusuk → BUY tamamen kapan
        .add_rule(Rule::new().when("cash", "dusuk").then("buy_score", 0.05))
        // 6. Stok zaten dolu → BUY kıs
        .add_rule(Rule::new().when("stock", "yuksek").then("buy_score", 0.1))
        // 7. İflas riski → tüm BUY iptal
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .then("buy_score", 0.0),
        )
        // ── SELL (markup ile aktar) ──
        // 8. Stok dolu → sat
        .add_rule(Rule::new().when("stock", "yuksek").then("sell_score", 0.9))
        // 9. Stok orta + fiyat pahalı → sat (markup yakala)
        .add_rule(
            Rule::new()
                .when("stock", "orta")
                .when("price_rel_avg", "yuksek")
                .then("sell_score", 0.85),
        )
        // 10. Talep yüksek + stok orta → sat
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "yuksek")
                .when("stock", "orta")
                .then("sell_score", 0.8),
        )
        // 11. Sezon sonu likiditesi
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.95),
        )
        // ── Bid/Ask aggressiveness ──
        // 12. Bid: pahalı şehir → bid az agresif (markup koruması)
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "yuksek")
                .then("bid_aggressiveness", 0.3),
        )
        // 13. Bid: ucuz şehir → bid agresif (fırsatı yakala)
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "dusuk")
                .then("bid_aggressiveness", 0.75),
        )
        // ASK formülü: market × (1 - (ask_aggro - 0.5) × 0.7 × aggr).
        // ask_aggro 0.85 → ASK %32 ALTINDA market! Bu ZARARA SATIŞ.
        // Toptancı markup için ask_aggro DÜŞÜK olmalı (ASK yüksek = kâr).
        // Esnaf PnL -5K idi çünkü BUY 6₺ → SELL ~5₺ ile zararına satıyordu.
        //
        // 14. Ask: rekabet yüksek + stok dolu → markup KORU (panik satma).
        .add_rule(
            Rule::new()
                .when("competition", "yuksek")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.3),
        )
        // 15. Ask: talep yüksek + stok dolu → markup yumuşat (yarış ama kârlı)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "yuksek")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.45),
        )
        // 16. Ask: talep yok + stok dolu → ASK çek (kimse almıyorsa fiyatı tut)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "dusuk")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.2),
        )
        // 17. Sezon sonu → likidite, ama hâlâ markup koru (panik satma yok)
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .then("ask_aggressiveness", 0.5),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::inputs::compute_inputs;
    use moneywar_domain::{
        CityId, GameState, Money, NpcKind, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn toptanci_state(stock: u32, cash: i64) -> GameState {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "TestToptanci",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Esnaf);
        if stock > 0 {
            let _ = p.inventory.add(CityId::Istanbul, ProductKind::Pamuk, stock);
        }
        s.players.insert(p.id, p);
        s
    }

    #[test]
    fn full_stock_wants_to_sell() {
        let s = toptanci_state(150, 10_000);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let sell = out.get("sell_score").copied().unwrap_or(0.0);
        assert!(sell > 0.5, "stok dolu → sat (sell={sell})");
    }

    #[test]
    fn empty_stock_with_cash_wants_to_buy_raw() {
        let s = toptanci_state(0, 30_000);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        assert!(buy > 0.4, "stok boş + nakit var → al (buy={buy})");
    }

    #[test]
    fn low_cash_blocks_buy() {
        let s = toptanci_state(0, 100);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        assert!(buy < 0.3, "nakit yok → buy kıs (buy={buy})");
    }
}
