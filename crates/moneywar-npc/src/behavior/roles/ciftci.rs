//! Çiftçi rol davranışı — hammadde üreticisi, sell-only.
//!
//! Çiftçi her HARVEST_PERIOD (8) tick'te `harvest_ciftci_stock` ile envantere
//! mahsul alır. Davranışı sade: stoğunu pazara satar, "ne zaman ne kadar?"
//! kararı utility skor üzerinden.
//!
//! # Aday üretim kuralı
//!
//! Envanterindeki her `(city, raw_product, qty)` için bir Sell adayı:
//! - quantity = stoğun yarısı (max 100, min 1)
//! - unit_price = `effective_baseline(city, product)`
//! - skor → orchestrator hesaplar (Çiftçi `Weights`'i ile)
//!
//! # Çiftçi `Weights` mantığı (`personality.rs`'te)
//!
//! - `stock` +1.0 → stok varsa sat
//! - `urgency` +0.5 → sezon sonu agresifleş
//! - `local_raw_advantage` +0.4 → uzmanlık şehrini önceliklendir
//! - `price_rel_avg` +0.3 → pahalıyken sat
//! - `competition` -0.2 → rakip baskısı varsa bekle
//! - `cash` -0.3 → düşük cash → motive et

use moneywar_domain::{GameState, Money, OrderSide, Player, ProductKind};

use crate::behavior::candidates::ActionCandidate;

/// Çiftçi'nin bu tick için olası satış adayları.
#[must_use]
pub fn enumerate(state: &GameState, player: &Player) -> Vec<ActionCandidate> {
    let mut out = Vec::new();
    for (city, product, qty) in player.inventory.entries() {
        if !product.is_raw() || qty == 0 {
            continue;
        }
        let quantity = (qty / 2).max(1).min(100);
        let unit_price = state.effective_baseline(city, product).unwrap_or_else(|| {
            // Baseline yoksa fallback — sim her zaman init eder, prod CLI de.
            Money::from_lira(default_raw_price(product)).unwrap_or(Money::ZERO)
        });
        if unit_price.as_cents() <= 0 {
            continue;
        }
        out.push(ActionCandidate::SubmitOrder {
            side: OrderSide::Sell,
            city,
            product,
            quantity,
            unit_price,
        });
    }
    out
}

const fn default_raw_price(_product: ProductKind) -> i64 {
    moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CityId, NpcKind, PlayerId, ProductKind, Role, RoomConfig, RoomId,
    };

    fn ciftci_with_stock(stock: u32) -> (GameState, Player) {
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "ciftci",
            Role::Tuccar,
            Money::from_lira(8_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Ciftci);
        if stock > 0 {
            p.inventory
                .add(CityId::Istanbul, ProductKind::Pamuk, stock)
                .unwrap();
        }
        (s, p)
    }

    #[test]
    fn empty_inventory_yields_no_candidates() {
        let (s, p) = ciftci_with_stock(0);
        assert!(enumerate(&s, &p).is_empty());
    }

    #[test]
    fn raw_stock_yields_sell_at_half_qty() {
        let (s, p) = ciftci_with_stock(200);
        let cands = enumerate(&s, &p);
        assert_eq!(cands.len(), 1);
        let ActionCandidate::SubmitOrder { side, quantity, .. } = &cands[0] else {
            panic!("expected SubmitOrder");
        };
        assert_eq!(*side, OrderSide::Sell);
        assert_eq!(*quantity, 100);
    }

    #[test]
    fn finished_stock_skipped() {
        let s = GameState::new(RoomId::new(1), RoomConfig::hizli());
        let mut p = Player::new(
            PlayerId::new(100),
            "ciftci",
            Role::Tuccar,
            Money::from_lira(8_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Ciftci);
        // Çiftçi'nin elinde mamul olamaz normalde, ama enumerate skip etmeli.
        p.inventory
            .add(CityId::Istanbul, ProductKind::Kumas, 50)
            .unwrap();
        let cands = enumerate(&s, &p);
        assert!(cands.is_empty(), "Çiftçi mamul satmamalı");
    }

    #[test]
    fn quantity_caps_at_100() {
        let (s, p) = ciftci_with_stock(500);
        let cands = enumerate(&s, &p);
        let ActionCandidate::SubmitOrder { quantity, .. } = &cands[0] else {
            panic!()
        };
        // 500/2 = 250, ama cap 100.
        assert_eq!(*quantity, 100);
    }

    #[test]
    fn quantity_floor_at_1_for_tiny_stock() {
        let (s, p) = ciftci_with_stock(1);
        let cands = enumerate(&s, &p);
        let ActionCandidate::SubmitOrder { quantity, .. } = &cands[0] else {
            panic!()
        };
        assert_eq!(*quantity, 1);
    }
}
