//! Haber aboneliği — `SubscribeNews` komutu (§6).
//!
//! Üç abonelik tier'ı:
//! - **Bronz (bedava):** herkese açık, olay tick'inde duyurulur.
//! - **Gümüş:** 1 tick önceden bölgesel haber. Tüccar için **bedava**.
//! - **Altın:** 2 tick önceden, premium fiyat.
//!
//! Aktif bir abonelik oyuncunun minimum tier'ını set eder; role bazlı bedava
//! avantaj (Tüccar'ın Silver) `effective_news_tier` helper'ında birleşir.
//!
//! RNG ile olay tetikleme ve haber dağıtımı [`crate::events`] içindedir;
//! bu modül yalnız abonelik komutunu işler.

use moneywar_domain::{DomainError, GameState, NewsTier, Player, PlayerId, Role, Tick};

use crate::{
    error::EngineError,
    report::{LogEntry, TickReport},
};

/// Oyuncunun efektif haber tier'ı: abonelik tier'ı ile rol bazlı defaultun
/// büyüğü. Tüccar varsayılan olarak Gümüş alır.
#[must_use]
pub(crate) fn effective_news_tier(player: &Player, subscribed: Option<NewsTier>) -> NewsTier {
    let role_default = if matches!(player.role, Role::Tuccar) {
        NewsTier::Silver
    } else {
        NewsTier::Bronze
    };
    match subscribed {
        Some(t) if t > role_default => t,
        _ => role_default,
    }
}

/// `SubscribeNews` komutu — tier değişikliği, maliyet debit edilir.
///
/// Tüccar için Gümüş bedavadır (aynı cost sıfır sayılır). Ücret maliyeti
/// ödemeye nakit yetmiyorsa komut reddedilir.
pub(crate) fn process_subscribe_news(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
    player: PlayerId,
    tier: NewsTier,
) -> Result<(), EngineError> {
    let Some(p_ref) = state.players.get(&player) else {
        return Err(EngineError::Domain(DomainError::Validation(format!(
            "player {player} not found"
        ))));
    };

    // Tüccar için Silver bedava — diğer herkes tarife üstünden.
    let is_tuccar_free = matches!(p_ref.role, Role::Tuccar) && matches!(tier, NewsTier::Silver);
    let cost = if is_tuccar_free {
        moneywar_domain::Money::ZERO
    } else {
        tier.subscription_cost()?
    };

    let p_mut = state.players.get_mut(&player).expect("checked");
    if p_mut.cash < cost {
        return Err(EngineError::Domain(DomainError::InsufficientFunds {
            have: p_mut.cash,
            want: cost,
        }));
    }
    if !cost.is_zero() {
        p_mut.debit(cost)?;
    }
    state.news_subscriptions.insert(player, tier);

    report.push(LogEntry::news_subscribed(tick, player, tier, cost));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moneywar_domain::{Money, Player, PlayerId, Role, RoomConfig, RoomId};

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
    fn subscribe_gold_debits_cost_and_records_tier() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Sanayici, 10_000);
        process_subscribe_news(&mut s, &mut r, Tick::new(1), pid, NewsTier::Gold).unwrap();
        assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));
        // Gold cost = 2000₺
        assert_eq!(
            s.players[&pid].cash,
            Money::from_lira(10_000 - 2_000).unwrap()
        );
    }

    #[test]
    fn subscribe_silver_free_for_tuccar() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Tuccar, 100);
        process_subscribe_news(&mut s, &mut r, Tick::new(1), pid, NewsTier::Silver).unwrap();
        assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Silver));
        // Cash değişmedi.
        assert_eq!(s.players[&pid].cash, Money::from_lira(100).unwrap());
    }

    #[test]
    fn subscribe_silver_paid_for_sanayici() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Sanayici, 1_000);
        process_subscribe_news(&mut s, &mut r, Tick::new(1), pid, NewsTier::Silver).unwrap();
        // Silver cost = 500₺
        assert_eq!(s.players[&pid].cash, Money::from_lira(500).unwrap());
    }

    #[test]
    fn subscribe_insufficient_funds_rejected() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Sanayici, 100);
        let err =
            process_subscribe_news(&mut s, &mut r, Tick::new(1), pid, NewsTier::Gold).unwrap_err();
        assert!(err.to_string().contains("insufficient"));
        // Tier kaydı yapılmadı.
        assert!(!s.news_subscriptions.contains_key(&pid));
    }

    #[test]
    fn effective_tier_tuccar_gets_silver_by_default() {
        let p = Player::new(PlayerId::new(1), "T", Role::Tuccar, Money::ZERO, false).unwrap();
        assert_eq!(effective_news_tier(&p, None), NewsTier::Silver);
    }

    #[test]
    fn effective_tier_sanayici_defaults_to_bronze() {
        let p = Player::new(PlayerId::new(1), "S", Role::Sanayici, Money::ZERO, false).unwrap();
        assert_eq!(effective_news_tier(&p, None), NewsTier::Bronze);
    }

    #[test]
    fn effective_tier_gold_subscription_overrides_role_default() {
        let p = Player::new(PlayerId::new(1), "T", Role::Tuccar, Money::ZERO, false).unwrap();
        assert_eq!(
            effective_news_tier(&p, Some(NewsTier::Gold)),
            NewsTier::Gold
        );
    }

    #[test]
    fn effective_tier_lower_subscription_does_not_lower_tuccar() {
        let p = Player::new(PlayerId::new(1), "T", Role::Tuccar, Money::ZERO, false).unwrap();
        // Tuccar subscribes Bronze — effective is still Silver (role default).
        assert_eq!(
            effective_news_tier(&p, Some(NewsTier::Bronze)),
            NewsTier::Silver
        );
    }
}
