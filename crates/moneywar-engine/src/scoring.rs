//! Skor ve leaderboard hesaplayıcısı (§9).
//!
//! # Formül
//!
//! ```text
//! Skor = Nakit
//!      + Σ (stok_i × son5tick_ortalama_fiyat_i)   // stok değeri
//!      + Σ (fabrika_kurulum_maliyeti_j × 0.5)     // atıl fabrika = 0
//!      + Σ (aktif_kontrat_escrow_k)               // kendi kaporan
//! ```
//!
//! - **Son 5 tick ortalama fiyat** → tek-tick manipülasyonu öldürür.
//! - **Atıl fabrika** → son 10 tick'te üretim yapmadı → değer sıfır.
//! - **Kontrat escrow** → hem `Proposed` (satıcının kaporası) hem `Active`
//!   (her iki taraf) kilitli paralar skorlanır.
//!
//! Bu modül **saf** — state okur, mutasyon yok. `advance_tick` içine
//! entegre edilmez; CLI / UI / server on-demand çağırır.

use moneywar_domain::{ContractState, Factory, GameState, Money, PlayerId, Tick};
use serde::{Deserialize, Serialize};

/// Skor formülündeki tek-tek kalemler. Toplam = `cash + stock + factory + escrow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerScore {
    pub player_id: PlayerId,
    pub cash: Money,
    pub stock_value: Money,
    pub factory_value: Money,
    pub escrow_value: Money,
    pub total: Money,
}

/// Atıl fabrika eşiği (§9) — son 10 tick'te üretim yoksa skora 0.
pub const IDLE_FACTORY_THRESHOLD: u32 = 10;

/// Skor hesaplaması için rolling avg penceresi (son 5 tick, §9).
pub const PRICE_WINDOW: usize = 5;

/// Verilen oyuncunun mevcut durumdaki skorunu hesapla.
///
/// Oyuncu state'te yoksa tüm kalemleri sıfır olan dolgu döner (sessiz
/// default — leaderboard agregasyonu bozulmasın).
#[must_use]
pub fn score_player(state: &GameState, player_id: PlayerId) -> PlayerScore {
    let Some(player) = state.players.get(&player_id) else {
        return PlayerScore {
            player_id,
            cash: Money::ZERO,
            stock_value: Money::ZERO,
            factory_value: Money::ZERO,
            escrow_value: Money::ZERO,
            total: Money::ZERO,
        };
    };

    let cash = player.cash;
    let stock_value = compute_stock_value(state, player_id);
    let factory_value = compute_factory_value(state, player_id, state.current_tick);
    let escrow_value = compute_escrow_value(state, player_id);

    // Toplam — saturating (overflow edge case'i skoru "sonsuz" yapmasın).
    let total_cents = cash
        .as_cents()
        .saturating_add(stock_value.as_cents())
        .saturating_add(factory_value.as_cents())
        .saturating_add(escrow_value.as_cents());

    PlayerScore {
        player_id,
        cash,
        stock_value,
        factory_value,
        escrow_value,
        total: Money::from_cents(total_cents),
    }
}

/// Tüm oyuncular için skor hesapla, toplamdan büyüğe sırala.
/// Tie-break: `player_id` ASC (deterministik).
#[must_use]
pub fn leaderboard(state: &GameState) -> Vec<PlayerScore> {
    let mut scores: Vec<PlayerScore> = state
        .players
        .keys()
        .map(|&pid| score_player(state, pid))
        .collect();
    scores.sort_by(|a, b| {
        b.total
            .cmp(&a.total)
            .then_with(|| a.player_id.cmp(&b.player_id))
    });
    scores
}

/// Stok değeri: her (şehir, ürün) için `miktar × son 5 tick ortalama fiyatı`.
/// Tarihçe yoksa o kalem 0.
fn compute_stock_value(state: &GameState, player_id: PlayerId) -> Money {
    let Some(player) = state.players.get(&player_id) else {
        return Money::ZERO;
    };
    let mut total: i64 = 0;
    for (city, product, qty) in player.inventory.entries() {
        let Some(avg) = state.rolling_avg_price(city, product, PRICE_WINDOW) else {
            continue;
        };
        let line = avg.as_cents().saturating_mul(i64::from(qty));
        total = total.saturating_add(line);
    }
    Money::from_cents(total)
}

/// Fabrika değeri: atıl olmayan fabrikaların `build_cost × 0.5` toplamı.
/// Build cost her oyuncunun fabrika sırasına göre `§10` tablosundan gelir.
fn compute_factory_value(state: &GameState, player_id: PlayerId, current: Tick) -> Money {
    // Sahibinin fabrikaları ID sırasına (kurulum sırası) göre.
    let mut owned: Vec<&Factory> = state
        .factories
        .values()
        .filter(|f| f.owner == player_id)
        .collect();
    owned.sort_by_key(|f| f.id);

    let mut total: i64 = 0;
    for (idx, factory) in owned.iter().enumerate() {
        if factory.is_atil(current, IDLE_FACTORY_THRESHOLD) {
            continue;
        }
        let build_cost = Factory::build_cost(u32::try_from(idx).unwrap_or(u32::MAX));
        let half = build_cost.as_cents() / 2;
        total = total.saturating_add(half);
    }
    Money::from_cents(total)
}

/// Aktif kontratlardaki kilitli para. `Proposed`'ta satıcı kaporası,
/// `Active`'te her iki taraf kendi payını görür.
fn compute_escrow_value(state: &GameState, player_id: PlayerId) -> Money {
    let mut total: i64 = 0;
    for c in state.contracts.values() {
        match c.state {
            ContractState::Proposed => {
                if c.seller == player_id {
                    total = total.saturating_add(c.seller_deposit.as_cents());
                }
            }
            ContractState::Active => {
                if c.seller == player_id {
                    total = total.saturating_add(c.seller_deposit.as_cents());
                }
                if c.accepted_by == Some(player_id) {
                    total = total.saturating_add(c.buyer_deposit.as_cents());
                }
            }
            ContractState::Fulfilled | ContractState::Breached { .. } => {
                // Settled — contract state'ten zaten silinmiş olmalı;
                // defensive no-op.
            }
        }
    }
    Money::from_cents(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{
        CityId, Contract, ContractId, ContractProposal, ListingKind, Money, Player, PlayerId,
        ProductKind, Role, RoomConfig, RoomId,
    };

    fn state() -> GameState {
        GameState::new(RoomId::new(1), RoomConfig::hizli())
    }

    fn add_player(state: &mut GameState, id: u64, role: Role, cash_lira: i64) -> PlayerId {
        let p = Player::new(
            PlayerId::new(id),
            format!("P{id}"),
            role,
            Money::from_lira(cash_lira).unwrap(),
            false,
        )
        .unwrap();
        state.players.insert(p.id, p);
        PlayerId::new(id)
    }

    #[test]
    fn cash_only_scores_as_cash() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Tuccar, 1_000);
        let sc = score_player(&s, pid);
        assert_eq!(sc.cash, Money::from_lira(1_000).unwrap());
        assert_eq!(sc.stock_value, Money::ZERO);
        assert_eq!(sc.factory_value, Money::ZERO);
        assert_eq!(sc.escrow_value, Money::ZERO);
        assert_eq!(sc.total, Money::from_lira(1_000).unwrap());
    }

    #[test]
    fn stock_without_price_history_contributes_zero() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Tuccar, 0);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 50)
            .unwrap();
        let sc = score_player(&s, pid);
        assert_eq!(sc.stock_value, Money::ZERO);
    }

    #[test]
    fn stock_with_price_history_uses_rolling_avg() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Tuccar, 0);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 50)
            .unwrap();
        // 3 tick history: 10, 12, 14 → avg = 12.
        let hist = s
            .price_history
            .entry((CityId::Istanbul, ProductKind::Pamuk))
            .or_default();
        hist.push((Tick::new(1), Money::from_lira(10).unwrap()));
        hist.push((Tick::new(2), Money::from_lira(12).unwrap()));
        hist.push((Tick::new(3), Money::from_lira(14).unwrap()));
        let sc = score_player(&s, pid);
        // 50 × 12₺ = 600₺
        assert_eq!(sc.stock_value, Money::from_lira(600).unwrap());
    }

    #[test]
    fn factory_value_uses_half_of_build_cost_and_skips_idle() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Sanayici, 0);
        // 3 fabrika: cost table 0 / 10k / 15k → skor katkı 0 / 5k / 7.5k = 12.5k.
        for i in 1..=3u64 {
            let fid = moneywar_domain::FactoryId::new(i);
            let mut f = Factory::new(fid, pid, CityId::Istanbul, ProductKind::Kumas).unwrap();
            // Son üretim tick 5 varsayalım, current 10 → 5 tick geçti, atıl değil.
            f.last_production_tick = Some(Tick::new(5));
            s.factories.insert(fid, f);
        }
        s.current_tick = Tick::new(10);
        let sc = score_player(&s, pid);
        // Expected: 0 + 5k + 7.5k = 12500
        assert_eq!(sc.factory_value, Money::from_lira(12_500).unwrap());
    }

    #[test]
    fn idle_factory_contributes_zero() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Sanayici, 0);
        // 2. fabrika (10k cost → 5k katkı), ama son üretim yok + 20 tick geçmiş = atıl.
        for i in 1..=2u64 {
            let fid = moneywar_domain::FactoryId::new(i);
            let mut f = Factory::new(fid, pid, CityId::Istanbul, ProductKind::Kumas).unwrap();
            f.last_production_tick = Some(Tick::new(5));
            s.factories.insert(fid, f);
        }
        s.current_tick = Tick::new(20); // 20 - 5 = 15 tick atıl (≥ 10)
        let sc = score_player(&s, pid);
        assert_eq!(sc.factory_value, Money::ZERO);
    }

    #[test]
    fn escrow_sums_active_contract_deposits() {
        let mut s = state();
        let seller = add_player(&mut s, 1, Role::Sanayici, 10_000);
        let buyer = add_player(&mut s, 2, Role::Tuccar, 10_000);

        // Proposed kontrat, seller_deposit 200.
        let proposal = ContractProposal {
            seller,
            listing: ListingKind::Public,
            product: ProductKind::Kumas,
            quantity: 10,
            unit_price: Money::from_lira(20).unwrap(),
            delivery_city: CityId::Istanbul,
            delivery_tick: Tick::new(10),
            seller_deposit: Money::from_lira(200).unwrap(),
            buyer_deposit: Money::from_lira(200).unwrap(),
        };
        let cid = ContractId::new(1);
        let contract = Contract::propose(
            cid,
            proposal.seller,
            proposal.listing,
            proposal.product,
            proposal.quantity,
            proposal.unit_price,
            proposal.delivery_city,
            proposal.delivery_tick,
            Tick::new(1),
            proposal.seller_deposit,
            proposal.buyer_deposit,
        )
        .unwrap();
        s.contracts.insert(cid, contract);

        // Satıcı escrow görür.
        let ss = score_player(&s, seller);
        assert_eq!(ss.escrow_value, Money::from_lira(200).unwrap());
        // Alıcı henüz kabul etmedi; escrow yok.
        let sb = score_player(&s, buyer);
        assert_eq!(sb.escrow_value, Money::ZERO);

        // Accept edince buyer escrow 200'e çıkar.
        s.contracts.get_mut(&cid).unwrap().accept(buyer).unwrap();
        let sb2 = score_player(&s, buyer);
        assert_eq!(sb2.escrow_value, Money::from_lira(200).unwrap());
        // Satıcı hala 200 görmeli.
        let ss2 = score_player(&s, seller);
        assert_eq!(ss2.escrow_value, Money::from_lira(200).unwrap());
    }

    #[test]
    fn leaderboard_sorts_desc_by_total_with_player_id_tiebreak() {
        let mut s = state();
        add_player(&mut s, 1, Role::Tuccar, 500);
        add_player(&mut s, 2, Role::Tuccar, 1_000);
        add_player(&mut s, 3, Role::Tuccar, 500); // tie with player 1

        let board = leaderboard(&s);
        assert_eq!(board.len(), 3);
        // Total sıralaması: 1000, 500, 500 (tie → smaller player_id first)
        assert_eq!(board[0].player_id, PlayerId::new(2));
        assert_eq!(board[1].player_id, PlayerId::new(1));
        assert_eq!(board[2].player_id, PlayerId::new(3));
    }

    #[test]
    fn total_equals_sum_of_parts() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Tuccar, 500);
        s.players
            .get_mut(&pid)
            .unwrap()
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 10)
            .unwrap();
        s.price_history
            .entry((CityId::Istanbul, ProductKind::Pamuk))
            .or_default()
            .push((Tick::new(1), Money::from_lira(5).unwrap()));

        let sc = score_player(&s, pid);
        let expected = sc.cash.as_cents()
            + sc.stock_value.as_cents()
            + sc.factory_value.as_cents()
            + sc.escrow_value.as_cents();
        assert_eq!(sc.total.as_cents(), expected);
    }

    #[test]
    fn missing_player_returns_zero_score() {
        let s = state();
        let sc = score_player(&s, PlayerId::new(999));
        assert_eq!(sc.total, Money::ZERO);
    }
}
