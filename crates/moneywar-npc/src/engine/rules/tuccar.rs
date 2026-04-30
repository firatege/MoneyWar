//! Tüccar rule base — arbitraj + kervan + uzun vadeli alış kontratı.

use crate::engine::vars::build_standard_vars;
use crate::fuzzy::{Engine, Rule};

#[must_use]
pub fn build_engine() -> Engine {
    let mut e = Engine::new();
    for v in build_standard_vars() {
        e = e.add_var(v);
    }

    e
        // ── BUY (al) ──
        // 1. Arbitraj yüksek + nakit yüksek → al (full agresif)
        .add_rule(
            Rule::new()
                .when("arbitrage", "yuksek")
                .when("cash", "yuksek")
                .then("buy_score", 0.95),
        )
        // 1b. Arbitraj orta + nakit var → orta agresif al (Tuning v6)
        .add_rule(
            Rule::new()
                .when("arbitrage", "orta")
                .when("cash", "orta")
                .then("buy_score", 0.7),
        )
        // 1c. Fiyat ucuz şehir + nakit orta → al (Tüccar arbitraj çekirdek)
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "dusuk")
                .when("cash", "orta")
                .then("buy_score", 0.8),
        )
        // 2. Fiyat ucuz + stok az → fırsat alımı
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "dusuk")
                .when("stock", "dusuk")
                .then("buy_score", 0.85),
        )
        // 3. Olay yaklaşıyor + momentum düşük → al (haber-reaktif)
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .when("momentum", "dusuk")
                .then("buy_score", 0.8),
        )
        // 4. Nakit az → alma
        .add_rule(Rule::new().when("cash", "dusuk").then("buy_score", 0.1))
        // 5. Rekabet dolu + arbitraj yok → fırsat dar, alma
        .add_rule(
            Rule::new()
                .when("competition", "yuksek")
                .when("arbitrage", "dusuk")
                .then("buy_score", 0.2),
        )
        // ── SELL (sat) ──
        // 6. Stok yüksek + fiyat pahalı → sat
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("price_rel_avg", "yuksek")
                .then("sell_score", 0.95),
        )
        // 7. Stok yüksek + momentum düşüyor → panik sat
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("momentum", "dusuk")
                .then("sell_score", 0.8),
        )
        // 8. Nakit az + stok varsa → likidite sat
        .add_rule(
            Rule::new()
                .when("cash", "dusuk")
                .when("stock", "yuksek")
                .then("sell_score", 0.85),
        )
        // 9. Sezon sonu + stok varsa → likidite
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.85),
        )
        // ── Aggressiveness ──
        // 10. Arbitraj yüksek → ask agresif
        .add_rule(
            Rule::new()
                .when("arbitrage", "yuksek")
                .then("ask_aggressiveness", 0.9),
        )
        // 11. Olay yaklaşıyor → bid agresif
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .then("bid_aggressiveness", 0.85),
        )
        // ── Contract ──
        // 12. Arbitraj yüksek + nakit yüksek → buyer-request kontratı
        .add_rule(
            Rule::new()
                .when("arbitrage", "yuksek")
                .when("cash", "yuksek")
                .then("contract_score", 0.75),
        )
        // ── İflas koruması ──
        // 13. İflas riski yüksek → tüm BUY kapat
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .then("buy_score", 0.0),
        )
        // 14. İflas riski yüksek + stok varsa → SAT zorunlu
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.95),
        )
        // ── Kervan satın kararı ──
        // 15. Kervan yok + arbitraj yüksek + nakit yüksek + sezon var → KERVAN AL
        // (Output: dispatch_score'u kervan satın için kullanıyoruz; orchestrator ayırt eder)
        .add_rule(
            Rule::new()
                .when("caravan_count", "dusuk")
                .when("arbitrage", "yuksek")
                .when("cash", "yuksek")
                .when("season_remaining", "yuksek")
                .then("buy_caravan_score", 0.95),
        )
        // 16. Kervan yok + nakit yüksek → KERVAN AL (default)
        .add_rule(
            Rule::new()
                .when("caravan_count", "dusuk")
                .when("cash", "yuksek")
                .then("buy_caravan_score", 0.6),
        )
        // 17. 3+ kervan + sezon yarı → kervan alma (amortisman ödemiyor)
        .add_rule(
            Rule::new()
                .when("caravan_count", "yuksek")
                .then("buy_caravan_score", 0.0),
        )
        // ── Dispatch (arbitraj-bazlı taşıma) ──
        // 18. Stok var + arbitraj yüksek → DISPATCH (esas tetik: kâr fırsatı)
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("arbitrage", "yuksek")
                .then("dispatch_score", 0.9),
        )
        // 18b. Stok var + arbitraj orta → DISPATCH (orta agresif)
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("arbitrage", "orta")
                .then("dispatch_score", 0.65),
        )
        // 18c. Yerel bid yok + stok var → DISPATCH (yerel pazar zayıf, taşı)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "dusuk")
                .when("stock", "yuksek")
                .then("dispatch_score", 0.55),
        )
        // 19. Hedef pazarda bid yüksek + stok varsa → DISPATCH (sekonder sinyal)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "yuksek")
                .when("stock", "yuksek")
                .then("dispatch_score", 0.7),
        )
        // ── Talep yok ise ASK düşürme anlamsız ──
        // 20. Talep yok + stok yüksek → ASK agresif AZALT (fiyat düşürme boşa)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "dusuk")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.15),
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
            "TestTuc",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap();
        s.players.insert(p.id, p);
        s
    }

    #[test]
    fn poor_tuccar_with_stock_sells_for_liquidity() {
        let mut s = npc_state(500);
        s.players
            .get_mut(&PlayerId::new(100))
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 80)
            .unwrap();
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let sell = out.get("sell_score").copied().unwrap_or(0.0);
        assert!(sell > 0.5, "fakir tüccar elindeki stoğu satmalı (sell={sell})");
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
