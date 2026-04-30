//! Spekülatör rule base — market maker, hem bid hem ask, spread daraltıcı.
//!
//! Kullanıcı şikayeti: "Spekülatör batıyor". Düzeltme: dengeli risk, olay
//! sinyaline tepki, sıkı spread sadece düşük rekabet altında.

use crate::engine::vars::build_standard_vars;
use crate::fuzzy::{Engine, Rule};

#[must_use]
pub fn build_engine() -> Engine {
    let mut e = Engine::new();
    for v in build_standard_vars() {
        e = e.add_var(v);
    }

    e
        // ── BUY (likidite ver — al tarafı) ──
        // 1. Nakit yüksek + rekabet düşük → al (likidite ver)
        .add_rule(
            Rule::new()
                .when("cash", "yuksek")
                .when("competition", "dusuk")
                .then("buy_score", 0.7),
        )
        // 2. Olay yaklaşıyor + momentum yüksek → front-run buy
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .when("momentum", "yuksek")
                .then("buy_score", 0.85),
        )
        // 3. Arbitraj yüksek → al (ucuz şehirde stok)
        .add_rule(
            Rule::new()
                .when("arbitrage", "yuksek")
                .then("buy_score", 0.75),
        )
        // 4. Fiyat ucuz + stok az → al
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "dusuk")
                .when("stock", "dusuk")
                .then("buy_score", 0.7),
        )
        // 5. Nakit az → alma (risk yönetimi)
        .add_rule(Rule::new().when("cash", "dusuk").then("buy_score", 0.1))
        // ── SELL (likidite ver — sat tarafı) ──
        // 6. Stok yüksek + fiyat pahalı → sat (kar al)
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("price_rel_avg", "yuksek")
                .then("sell_score", 0.9),
        )
        // 7. Olay yaklaşıyor + momentum düşük → sat (front-run sell)
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .when("momentum", "dusuk")
                .then("sell_score", 0.85),
        )
        // 8. Stok orta + rekabet düşük → sat (likidite ver)
        .add_rule(
            Rule::new()
                .when("stock", "orta")
                .when("competition", "dusuk")
                .then("sell_score", 0.65),
        )
        // ── Aggressiveness — spread daraltma ──
        // 9. Rekabet düşük → bid + ask agresif (likidite eksik)
        .add_rule(
            Rule::new()
                .when("competition", "dusuk")
                .then("bid_aggressiveness", 0.8),
        )
        .add_rule(
            Rule::new()
                .when("competition", "dusuk")
                .then("ask_aggressiveness", 0.8),
        )
        // 10. Rekabet yüksek → spread aç (risk al, oturma sat)
        .add_rule(
            Rule::new()
                .when("competition", "yuksek")
                .then("ask_aggressiveness", 0.4),
        )
        // ── İflas + dengeleyici ──
        // 11. İflas riski yüksek → tüm BUY kapat
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .then("buy_score", 0.0),
        )
        // 12. İflas riski yüksek → spread aç (risk azalt)
        .add_rule(
            Rule::new()
                .when("bankruptcy_risk", "yuksek")
                .then("bid_aggressiveness", 0.3)
                .then("ask_aggressiveness", 0.3),
        )
        // 13. Talep yok (dengesiz) + stok varsa → BID düşür (alıcı oluştur)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "dusuk")
                .then("bid_aggressiveness", 0.85),
        )
        // 14. Çok talep + arz az → ASK agresif (sat dengele)
        .add_rule(
            Rule::new()
                .when("bid_supply_ratio", "yuksek")
                .when("stock", "orta")
                .then("ask_aggressiveness", 0.9),
        )
        // 15. Sezon kalan kısa → genel risk azalt (BUY/SELL düşür)
        .add_rule(
            Rule::new()
                .when("season_remaining", "dusuk")
                .then("buy_score", 0.2),
        )
        // 16. Stok orta + fiyat orta + rekabet az → MARKET MAKE (her iki yönde de)
        // Bu Spekülatör'ün ana rolü — sürekli likidite ver, küçük spread kazan.
        .add_rule(
            Rule::new()
                .when("stock", "orta")
                .when("price_rel_avg", "orta")
                .when("competition", "dusuk")
                .then("buy_score", 0.7)
                .then("sell_score", 0.7),
        )
        // 17. Stok yüksek + fiyat orta → SAT (sermaye çevir)
        .add_rule(
            Rule::new()
                .when("stock", "yuksek")
                .when("price_rel_avg", "orta")
                .then("sell_score", 0.75),
        )
        // 18. Olay yüksek + nakit yüksek + stok orta → asıl front-run
        .add_rule(
            Rule::new()
                .when("event", "yuksek")
                .when("cash", "yuksek")
                .when("momentum", "yuksek")
                .then("buy_score", 0.95),
        )
        // 19. Stok DÜŞÜK + nakit yüksek → BUY (boş pozisyonu doldur, market maker
        //     mamulde de iki yönlü olsun). Trace'ten Spek mamulde sadece SAT
        //     ediyordu; bu rule mamul BID'i açar, spread daralır.
        .add_rule(
            Rule::new()
                .when("stock", "dusuk")
                .when("cash", "yuksek")
                .then("buy_score", 0.6),
        )
        // 20. Spread'i daraltıcı genel boost — fiyat orta + rekabet orta →
        //     ASK'i agresif yap (pahalı satmaktan vazgeç, match şansı artsın).
        .add_rule(
            Rule::new()
                .when("price_rel_avg", "orta")
                .when("competition", "orta")
                .then("ask_aggressiveness", 0.75),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::inputs::compute_inputs;
    use moneywar_domain::{
        CityId, GameState, Money, NpcKind, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn spek_state(cash: i64, stock: u32) -> GameState {
        let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "TestSpek",
            Role::Tuccar,
            Money::from_lira(cash).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Spekulator);
        if stock > 0 {
            let _ = p.inventory.add(CityId::Istanbul, ProductKind::Pamuk, stock);
        }
        s.players.insert(p.id, p);
        s
    }

    #[test]
    fn rich_spek_provides_liquidity_when_market_thin() {
        let s = spek_state(40_000, 0);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        let bid_aggro = out.get("bid_aggressiveness").copied().unwrap_or(0.0);
        assert!(buy > 0.3, "zengin Spek likidite verir (buy={buy})");
        assert!(bid_aggro > 0.3, "ince pazar → agresif bid ({bid_aggro})");
    }

    #[test]
    fn poor_spek_does_not_buy() {
        let s = spek_state(500, 0);
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let buy = out.get("buy_score").copied().unwrap_or(0.0);
        assert!(buy < 0.3, "fakir Spek risk almaz (buy={buy})");
    }

    #[test]
    fn full_stock_at_high_price_sells() {
        let mut s = spek_state(40_000, 150);
        // Baseline 10₺, current 20₺ → ratio 2.0 → price_rel_avg 1.0 → "yuksek".
        s.price_baseline
            .insert((CityId::Istanbul, ProductKind::Pamuk), Money::from_lira(10).unwrap());
        s.price_history.insert(
            (CityId::Istanbul, ProductKind::Pamuk),
            vec![
                (moneywar_domain::Tick::new(1), Money::from_lira(20).unwrap()),
                (moneywar_domain::Tick::new(2), Money::from_lira(20).unwrap()),
            ],
        );
        let inputs = compute_inputs(&s, PlayerId::new(100), CityId::Istanbul, ProductKind::Pamuk);
        let out = build_engine().evaluate(&inputs);
        let sell = out.get("sell_score").copied().unwrap_or(0.0);
        assert!(sell > 0.4, "stok dolu + pahalı → sat (sell={sell})");
    }
}
