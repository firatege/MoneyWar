//! Çiftçi rule base — hammadde üreticisi (sell-only).
//!
//! Çiftçi'nin tek görevi: stok'u pazara satmak. Periyodik mahsul refill
//! `engine::economy::tick_ciftci_harvest` ile gelir; rule base sadece **ne zaman
//! ne kadar agresif sat** kararını verir.

use crate::engine::vars::build_standard_vars;
use crate::fuzzy::{Engine, Rule};

#[must_use]
pub fn build_engine() -> Engine {
    let mut e = Engine::new();
    for v in build_standard_vars() {
        e = e.add_var(v);
    }

    e
        // ── SELL kuralları (sell-only role) ──
        // Çiftçi mahsul biriktirmeden satmalı — fire/vergi yer. Tuning v6.5:
        // Default ASK 0.7 (market'in altı, hızlı satış). Pazar dolaşımı kapısı.

        // 1. Stok DOLU + fiyat ORTA → SAT (rotasyon, market civarı agresif)
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("price_rel_avg", "orta")
                .then("sell_score", 0.95)
                .then("ask_aggressiveness", 0.75),
        )
        // 1b. Tuning v6.5: lokal uzmanlık şehirde her zaman SAT
        // (top-K tie-break sorununu çöz: Çiftçi specialty şehrine odaklansın)
        .add_rule(
            Rule::new()
                .when("local_raw_advantage", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.99)
                .then("ask_aggressiveness", 0.8),
        )
        // 1c. Lokal uzmanlık + stok orta → yine SAT (sürekli akış)
        .add_rule(
            Rule::new()
                .when("local_raw_advantage", "yuksek")
                .when("stock", "orta")
                .then("sell_score", 0.85)
                .then("ask_aggressiveness", 0.75),
        )
        // 2. Stok DOLU + fiyat PAHALI → AGRESİF SAT (zirvede)
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("price_rel_avg", "yuksek")
                .then("sell_score", 0.95)
                .then("ask_aggressiveness", 0.7),
        )
        // 3. Stok DOLU + fiyat UCUZ → SAT yine (mahsul biriktirmesin, fire riski)
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("price_rel_avg", "dusuk")
                .then("sell_score", 0.85)
                .then("ask_aggressiveness", 0.65),
        )
        // 4. Sezon SONU + stok varsa → MAX AGRESİF (sezon kayıp riski)
        .add_rule(
            Rule::new()
                .when("urgency", "yuksek")
                .when("stock", "yuksek")
                .then("sell_score", 0.95)
                .then("ask_aggressiveness", 0.9),
        )
        // 5. Stok DÜŞÜK → SAT durur (mahsul beklemede)
        .add_rule(
            Rule::new()
                .when("stock", "dusuk")
                .then("sell_score", 0.05),
        )
        // 6. Stok ORTA + cash dusuk → SAT (likidite ihtiyacı)
        .add_rule(
            Rule::new()
                .when("stock", "orta")
                .when("cash", "dusuk")
                .then("sell_score", 0.85)
                .then("ask_aggressiveness", 0.8),
        )
        // 7. Talep DOLU + stok varsa → premium fiyatla sat (alıcı yarışı)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "yuksek")
                .when("stock", "yuksek")
                .then("ask_aggressiveness", 0.95),
        )
        // Tuning v6.5: rule 7 eski "bid_dusuk → ask 0.2" kaldırıldı. Bu kural
        // Çiftçi'yi sezon başı (henüz NPC'ler bid yokken) pasif yapıyordu.
        // Çiftçi'nin görevi mahsulü pazara dökmek; bid yoksa bile market civarı
        // ASK ile bekleme yapsın.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::inputs::compute_inputs;
    use moneywar_domain::{
        CityId, GameState, Money, NpcKind, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn ciftci_state(stock: u32) -> GameState {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "TestCiftci",
            Role::Tuccar, // Çiftçi role::Tuccar (üretici-tipi NPC)
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Ciftci);
        if stock > 0 {
            let _ = p.inventory.add(CityId::Istanbul, ProductKind::Pamuk, stock);
        }
        s.players.insert(p.id, p);
        s
    }

    #[test]
    fn full_stock_wants_to_sell() {
        let s = ciftci_state(150);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let sell = out.get("sell_score").copied().unwrap_or(0.0);
        assert!(sell > 0.5, "stok dolu Çiftçi → SAT (sell={sell})");
    }

    #[test]
    fn empty_stock_no_action() {
        let s = ciftci_state(0);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let sell = out.get("sell_score").copied().unwrap_or(0.0);
        assert!(sell < 0.3, "stoğu yok Çiftçi → SAT yok (sell={sell})");
    }
}
