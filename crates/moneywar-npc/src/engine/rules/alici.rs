//! Alıcı rule base — saf alıcı + likidite kolu (cash bitince satar).
//!
//! Kullanıcı şikayeti: "Alıcılar doygunluğa giriyor" — fix: nakit azalınca
//! sadece alma yerine sat da, döngü kapanır.

use crate::engine::vars::build_standard_vars;
use crate::fuzzy::{Engine, Rule};

#[must_use]
pub fn build_engine() -> Engine {
    let mut e = Engine::new();
    for v in build_standard_vars() {
        e = e.add_var(v);
    }

    e
        // ── BUY (saf alıcı) ──
        // 1. Nakit yüksek + fiyat ucuz → al
        .add_rule(
            Rule::new()
                .when("cash", "yuksek")
                .when("price_rel_avg", "dusuk")
                .then("buy_score", 0.95),
        )
        // 2. Nakit yüksek + olay yaklaşıyor → al
        .add_rule(
            Rule::new()
                .when("cash", "yuksek")
                .when("event", "yuksek")
                .then("buy_score", 0.85),
        )
        // 3. Nakit orta + fiyat ucuz → al
        .add_rule(
            Rule::new()
                .when("cash", "orta")
                .when("price_rel_avg", "dusuk")
                .then("buy_score", 0.7),
        )
        // 4. Sezon sonu + nakit yüksek → likiditeye dön (al ki stok değer)
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .when("cash", "yuksek")
                .then("buy_score", 0.6),
        )
        // 5. Nakit az → alma
        .add_rule(Rule::new().when("cash", "dusuk").then("buy_score", 0.05))
        // 6. Stok dolu → alma (doygunluk)
        .add_rule(Rule::new().when("stock", "yuksek").then("buy_score", 0.1))
        // 6b. Default alıcı davranışı: cash var + stok yok/az → genel buy fırsatı.
        // Fiyat henüz veri yokken (sezon başı) bu yine alıcının ana motivasyonu.
        .add_rule(
            Rule::new()
                .when("cash", "yuksek")
                .when("stock", "dusuk")
                .then("buy_score", 0.7),
        )
        // ── SELL (likidite kolu — kullanıcı şikayet fix'i) ──
        // 7. Nakit az + stok yüksek → sat (cash regen)
        .add_rule(
            Rule::new()
                .when("cash", "dusuk")
                .when("stock", "yuksek")
                .then("sell_score", 0.9),
        )
        // 8. Nakit az + stok orta → sat
        .add_rule(
            Rule::new()
                .when("cash", "dusuk")
                .when("stock", "orta")
                .then("sell_score", 0.7),
        )
        // 9. Sezon sonu + stok varsa → likidite
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.8),
        )
        // 10. Stok yüksek + fiyat pahalı → sat (kar fırsatı)
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("price_rel_avg", "yuksek")
                .then("sell_score", 0.85),
        )
        // ── Aggressiveness ──
        // 11. Olay yaklaşıyor → bid agresif (kapışmak için)
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .then("bid_aggressiveness", 0.8),
        )
        // ── Talep dengeleme ──
        // 12. Bu pazarda zaten çok bid var (rekabet) → BID agresif (yarış)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "yuksek")
                .when("cash", "yuksek")
                .then("bid_aggressiveness", 0.9),
        )
        // 13. Az bid var + arz dolu → BID rahat (yarış yok, agresif olma)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "dusuk")
                .then("bid_aggressiveness", 0.3),
        )
        // 14. İflas riski yüksek → BUY tamamen kapat
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .then("buy_score", 0.0),
        )
        // 15. İflas riski yüksek + stok varsa → SAT zorunlu (likidite)
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.95),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::inputs::compute_inputs;
    use moneywar_domain::{
        CityId, GameState, Money, NpcKind, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn alici_state(cash: i64, stock: u32) -> GameState {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "TestAlici",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        if stock > 0 {
            let _ = p.inventory.add(CityId::Istanbul, ProductKind::Pamuk, stock);
        }
        s.players.insert(p.id, p);
        s
    }

    #[test]
    fn rich_empty_stock_wants_to_buy() {
        let s = alici_state(50_000, 0);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        assert!(buy > 0.4, "zengin alıcı boş stokla almalı (buy={buy})");
    }

    #[test]
    fn poor_with_stock_wants_to_sell_for_liquidity() {
        // Kullanıcı şikayeti: "Alıcılar doygunluğa giriyor" → bu kural fix
        let s = alici_state(500, 80);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let sell = out.get("sell_score").copied().unwrap_or(0.0);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        assert!(
            sell > buy,
            "fakir + stoğu var → sell > buy (sell={sell}, buy={buy})"
        );
    }

    #[test]
    fn poor_alici_does_not_buy() {
        let s = alici_state(200, 0);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        assert!(buy < 0.3, "fakir alıcı az alır (buy={buy})");
    }
}
