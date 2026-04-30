//! Esnaf rule base — saf satıcı + dükkan rotasyonu (mamul stok eşiği altında bid).

use crate::engine::vars::build_standard_vars;
use crate::fuzzy::{Engine, Rule};

#[must_use]
pub fn build_engine() -> Engine {
    let mut e = Engine::new();
    for v in build_standard_vars() {
        e = e.add_var(v);
    }

    e
        // ── SELL (saf satıcı) ──
        // 1. Stok dolu → sat
        .add_rule(Rule::new().when("stock", "yuksek").then("sell_score", 0.95))
        // 2. Stok orta + fiyat pahalı → sat
        .add_rule(
            Rule::new()
                .when("stock", "orta")
                .when("price_rel_avg", "yuksek")
                .then("sell_score", 0.85),
        )
        // 3. Sezon sonu + stok varsa → likidite
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.9),
        )
        // ── BUY (dükkan rotasyonu — mamul stok düşükse bid) ──
        // 4. Stok düşük + fiyat ucuz → al
        .add_rule(
            Rule::new()
                .when("stock", "dusuk")
                .when("price_rel_avg", "dusuk")
                .then("buy_score", 0.85),
        )
        // 5. Stok düşük + nakit yüksek → al
        .add_rule(
            Rule::new()
                .when("stock", "dusuk")
                .when("cash", "yuksek")
                .then("buy_score", 0.7),
        )
        // 6. Nakit az → alma
        .add_rule(Rule::new().when("cash", "dusuk").then("buy_score", 0.1))
        // 7. Stok zaten dolu → alma
        .add_rule(Rule::new().when("stock", "yuksek").then("buy_score", 0.05))
        // ── Aggressiveness ──
        // 8. Sezon sonu → ask agresif (likidite için)
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .then("ask_aggressiveness", 0.85),
        )
        // 9. Rekabet yüksek + stok dolu → ask agresif (kapış)
        .add_rule(
            Rule::new()
                .when("competition", "yuksek")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.9),
        )
        // ── İflas + talep koruması ──
        // 10. İflas riski yüksek → tüm BUY kapat
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .then("buy_score", 0.0),
        )
        // 11. Talep yok (bid yok) + stok dolu → ASK düşürme anlamsız
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "dusuk")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.15),
        )
        // 12. Talep yüksek + stok dolu → ASK agresif (alıcı yarışı)
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
        CityId, GameState, Money, NpcKind, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn esnaf_state(stock: u32, cash: i64) -> GameState {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "TestEsnaf",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Esnaf);
        if stock > 0 {
            let _ = p.inventory.add(CityId::Istanbul, ProductKind::Kumas, stock);
        }
        s.players.insert(p.id, p);
        s
    }

    #[test]
    fn full_stock_wants_to_sell() {
        let s = esnaf_state(150, 10_000);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Kumas);
        let out = build_engine().evaluate(&inputs);
        let sell = out.get("sell_score").copied().unwrap_or(0.0);
        assert!(sell > 0.5, "stok dolu → sat (sell={sell})");
    }

    #[test]
    fn empty_stock_wants_to_buy_finished() {
        let s = esnaf_state(0, 30_000);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Kumas);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        assert!(buy > 0.5, "stok boş + nakit var → al (buy={buy})");
    }
}
