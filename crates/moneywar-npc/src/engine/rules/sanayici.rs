//! Sanayici rule base — fabrika sahibi, ham madde alıcı, mamul satıcı.
//!
//! Sanayici akışı:
//! - Hammadde stoğu düşükse + nakit varsa → buy
//! - Mamul stoğu yüksekse + fiyat pahalıysa → sat
//! - Olay yaklaşıyorsa + fiyat düşükse → fırsat alımı
//! - Sezon sonu + stok varsa → likidite

use crate::engine::vars::build_standard_vars;
use crate::fuzzy::{Engine, Rule};

#[must_use]
pub fn build_engine() -> Engine {
    let mut e = Engine::new();
    for v in build_standard_vars() {
        e = e.add_var(v);
    }

    e
        // ── BUY kuralları ──
        // 1. Fabrikası VAR + ham madde stok az → çok agresif al (üretim sürekliliği)
        .add_rule(
            Rule::new()
                .when("factory_count", "orta")
                .when("stock", "dusuk")
                .then("buy_score", 0.95),
        )
        // 1b. Fabrikası YOK → çok az al (zaten üretmeyecek)
        .add_rule(
            Rule::new()
                .when("factory_count", "dusuk")
                .then("buy_score", 0.15),
        )
        // 1c. Stok az + nakit yüksek → al (default)
        .add_rule(
            Rule::new()
                .when("stock", "dusuk")
                .when("cash", "yuksek")
                .then("buy_score", 0.7),
        )
        // 2. Ucuz fırsat — fiyat düşük + stok az → al
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "dusuk")
                .when("stock", "dusuk")
                .then("buy_score", 0.8),
        )
        // 3. Olay yaklaşıyor + fiyat düşük → al (haber-reaktif)
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .when("momentum", "dusuk")
                .then("buy_score", 0.85),
        )
        // 4. Nakit az → alma
        .add_rule(Rule::new().when("cash", "dusuk").then("buy_score", 0.1))
        // 5. Stok dolu → buy yok
        .add_rule(Rule::new().when("stock", "yuksek").then("buy_score", 0.05))
        // ── SELL kuralları ──
        // 6. Stok dolu + fiyat pahalı → sat
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("price_rel_avg", "yuksek")
                .then("sell_score", 0.95),
        )
        // 7. Stok dolu + momentum düşüyor → panik sat
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("momentum", "dusuk")
                .then("sell_score", 0.85),
        )
        // 8. Sezon sonu + stok varsa → likidite
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.8),
        )
        // 9. Olay yaklaşıyor + momentum yüksek → tepe sat
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .when("momentum", "yuksek")
                .then("sell_score", 0.9),
        )
        // 10. Stok az + fiyat ucuz → satma
        .add_rule(
            Rule::new()
                .when("stock", "dusuk")
                .when("price_rel_avg", "dusuk")
                .then("sell_score", 0.05),
        )
        // ── Aggressiveness ──
        // 11. Olay yaklaşıyor → bid agresif
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .then("bid_aggressiveness", 0.9),
        )
        // 12. Rekabet yüksek → ask agresif (kapış)
        .add_rule(
            Rule::new()
                .when("competition", "yuksek")
                .then("ask_aggressiveness", 0.85),
        )
        // ── Build factory ──
        // 13. Nakit yüksek + sezon başı → fabrika kur
        .add_rule(
            Rule::new()
                .when("cash", "yuksek")
                .when("urgency", "dusuk")
                .then("build_factory_score", 0.9),
        )
        // ── Contract ──
        // 14. Arbitraj yüksek → kontrat öner
        .add_rule(
            Rule::new()
                .when("arbitrage", "yuksek")
                .then("contract_score", 0.7),
        )
        // ── İflas + ROI koruması ──
        // 15. İflas riski yüksek → tüm BUY ve fabrika kapat
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .then("buy_score", 0.0)
                .then("build_factory_score", 0.0),
        )
        // 16. İflas riski yüksek → SAT zorunlu
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .then("sell_score", 0.85),
        )
        // 17. Fabrika 3+ ve sezon yarı → yeni fabrika kurma (ROI ödemiyor)
        .add_rule(
            Rule::new()
                .when("factory_count", "yuksek")
                .then("build_factory_score", 0.05),
        )
        // 18. Sezon kalan kısa → fabrika kurma (amortisman süresi yok)
        .add_rule(
            Rule::new()
                .when("season_remaining", "dusuk")
                .then("build_factory_score", 0.05),
        )
        // ── Talep yok-ise ASK düşürme ──
        // 19. Talep yok (bid_supply_ratio düşük) + stok yüksek → ASK agresif AZALT
        // (Kullanıcı şikayeti: "fiyat düşürdüm kimse almadı" — fiyat indirme anlamsız)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "dusuk")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.2),
        )
        // 20. Talep yüksek + stok yüksek → ASK agresif (alıcı yarışı)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "yuksek")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.95),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::inputs::compute_inputs;
    use moneywar_domain::{
        CityId, GameState, Money, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn npc_state(cash: i64) -> GameState {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let p = Player::new(
            PlayerId::new(100),
            "TestSan",
            Role::Sanayici,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap();
        s.players.insert(p.id, p);
        s
    }

    #[test]
    fn rich_with_no_stock_wants_to_buy() {
        let s = npc_state(40_000);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        let sell = out.get("sell_score").copied().unwrap_or(0.0);
        assert!(buy > sell, "yüksek nakit + boş stok → buy_score > sell_score (buy={buy}, sell={sell})");
    }

    #[test]
    fn poor_npc_does_not_want_to_buy() {
        let s = npc_state(500);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        assert!(buy < 0.5, "fakir NPC az alır (buy={buy})");
    }

    #[test]
    fn rich_npc_at_season_start_wants_to_build() {
        let s = npc_state(50_000);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let build = out.get("build_factory_score").copied().unwrap_or(0.0);
        assert!(build > 0.5, "yüksek nakit + sezon başı → fabrika kur (build={build})");
    }

    #[test]
    fn deterministic_for_same_state() {
        let s = npc_state(20_000);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let e1 = build_engine().evaluate(&inputs);
        let e2 = build_engine().evaluate(&inputs);
        assert_eq!(e1, e2);
    }
}
