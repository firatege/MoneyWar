//! Haber aboneliği — `SubscribeNews` komutu + tick başı recurring debit.
//!
//! Dört abonelik tier'ı:
//! - **Free** (default): bedava, sadece "var/yok" + rolling avg fiyat. Olay
//!   haberi gelmez.
//! - **Bronze**: kategorik miktar + ask/bid bandı. Tick ücreti var; **Tüccar
//!   için bedava** (rol avantajı).
//! - **Silver**: 5'e yuvarlı miktar + ask/bid 5 kuruşa yuvarlı + 1-tick lead.
//!   Tüccar indirimli.
//! - **Gold**: tam veri + 2-tick lead. Tüccar indirimli.
//!
//! `process_subscribe_news` bir tier'a geçişi kaydeder ve **ilk tick** ücretini
//! hemen düşer. Sonraki her tick'te `charge_news_subscriptions` (`advance_tick`
//! içinden çağrılır) recurring fee'yi keser. Cash yetmezse 1 tick uyarı, ertesi
//! tick yine yetmezse `NewsTier::Free`'ye düşürülür.

use moneywar_domain::{DomainError, GameState, NewsTier, Player, PlayerId, Tick};

use crate::{
    error::EngineError,
    report::{LogEntry, TickReport},
};

/// Oyuncunun efektif haber tier'ı.
///
/// Mantık:
/// - Subscribe'lı tier varsa onu kullan (Free dahil — oyuncu açıkça iptal etti).
/// - Yoksa rol-default (yeni oyuncular için Bronze).
///
/// Tüccar avantajı artık `default_news_tier` üstünden değil,
/// `tick_cost(role)` indirimi üstünden ifade edilir.
#[must_use]
pub(crate) fn effective_news_tier(player: &Player, subscribed: Option<NewsTier>) -> NewsTier {
    subscribed.unwrap_or_else(|| player.role.default_news_tier())
}

/// `SubscribeNews` komutu — sadece tier'ı kaydeder. Debit'i aynı tick'te
/// `charge_news_subscriptions` yapar (single source of truth → çift sayım yok).
/// Ücret oyuncunun nakdine sığmıyorsa kayıt yine yapılır; o tick'in sonundaki
/// charge döngüsü uyarı bayrağı atar.
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
    let cost = tier.tick_cost(p_ref.role)?;

    // Free dahil tüm tier'lar map'te explicit tutulur — default Bronze
    // davranışı sadece map'te hiç kayıt yoksa devreye girer.
    state.news_subscriptions.insert(player, tier);
    // Yeni tier seçimi cleanstate — varsa eski uyarıyı kaldır.
    state.news_payment_warned.remove(&player);

    // Cost'u log'a yine yazıyoruz — UX bilgisi (kullanıcı maliyeti görür);
    // gerçek debit charge_news_subscriptions'da olacak.
    report.push(LogEntry::news_subscribed(tick, player, tier, cost));
    Ok(())
}

/// `advance_tick` içinden çağrılır — her oyuncuya effective tier'ına göre tick fee uygular.
///
/// Tüm oyuncular iter edilir; subscription map'inde yoksa rol-default kullanılır
/// (yeni oyuncular otomatik Bronze öder).
///
/// Ödeme akışı:
/// - cost = 0 (Free): no-op.
/// - cash >= cost: debit, varsa uyarı bayrağı temizlenir.
/// - cash <  cost ve daha önce uyarılmamış: bayrağı set et, log uyarı.
/// - cash <  cost ve **zaten uyarılmış**: tier'ı Free'ye düşür.
pub(crate) fn charge_news_subscriptions(
    state: &mut GameState,
    report: &mut TickReport,
    tick: Tick,
) {
    let entries: Vec<(PlayerId, NewsTier)> = state
        .players
        .iter()
        .map(|(id, p)| {
            let sub = state.news_subscriptions.get(id).copied();
            (*id, effective_news_tier(p, sub))
        })
        .collect();

    for (player_id, tier) in entries {
        let Some(player) = state.players.get(&player_id) else {
            continue; // defansif
        };
        let cost = match tier.tick_cost(player.role) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if cost.is_zero() {
            // Free tier — debit yok, uyarı varsa temizle.
            state.news_payment_warned.remove(&player_id);
            continue;
        }

        let player_cash = player.cash;
        if player_cash >= cost {
            let p_mut = state.players.get_mut(&player_id).expect("checked");
            p_mut.debit(cost).expect("pre-flight checked cash >= cost");
            state.news_payment_warned.remove(&player_id);
            report.push(LogEntry::news_tick_charged(tick, player_id, tier, cost));
        } else if state.news_payment_warned.contains(&player_id) {
            // İkinci shortfall → Free'ye düşür.
            state.news_subscriptions.insert(player_id, NewsTier::Free);
            state.news_payment_warned.remove(&player_id);
            report.push(LogEntry::news_downgraded(
                tick,
                player_id,
                tier,
                NewsTier::Free,
            ));
        } else {
            state.news_payment_warned.insert(player_id);
            report.push(LogEntry::news_payment_warning(tick, player_id, tier, cost));
        }
    }
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
    fn subscribe_records_tier_without_debit() {
        // Debit charge_news_subscriptions'da olur; subscribe sadece kayıt.
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Sanayici, 1_000);
        process_subscribe_news(&mut s, &mut r, Tick::new(1), pid, NewsTier::Gold).unwrap();
        assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));
        // Cash henüz değişmedi.
        assert_eq!(s.players[&pid].cash, Money::from_lira(1_000).unwrap());
    }

    #[test]
    fn subscribe_low_cash_succeeds_charge_decides() {
        // Pre-flight cash check kalktı — yetersiz cash kayıt durdurmaz, charge
        // döngüsü uyarı bayrağı atar.
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Sanayici, 5);
        process_subscribe_news(&mut s, &mut r, Tick::new(1), pid, NewsTier::Gold).unwrap();
        assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));
    }

    #[test]
    fn subscribe_to_free_records_explicit_opt_out() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Sanayici, 1_000);
        process_subscribe_news(&mut s, &mut r, Tick::new(1), pid, NewsTier::Gold).unwrap();
        process_subscribe_news(&mut s, &mut r, Tick::new(2), pid, NewsTier::Free).unwrap();
        // Free explicit yazılır (Bronze default'a düşmez).
        assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Free));
    }

    #[test]
    fn effective_tier_tuccar_floors_at_bronze() {
        let p = Player::new(PlayerId::new(1), "T", Role::Tuccar, Money::ZERO, false).unwrap();
        assert_eq!(effective_news_tier(&p, None), NewsTier::Bronze);
    }

    #[test]
    fn effective_tier_sanayici_defaults_to_bronze() {
        // v3: herkes Bronze'da başlar.
        let p = Player::new(PlayerId::new(1), "S", Role::Sanayici, Money::ZERO, false).unwrap();
        assert_eq!(effective_news_tier(&p, None), NewsTier::Bronze);
    }

    #[test]
    fn effective_tier_subscription_overrides_role_default() {
        let p = Player::new(PlayerId::new(1), "T", Role::Tuccar, Money::ZERO, false).unwrap();
        assert_eq!(
            effective_news_tier(&p, Some(NewsTier::Gold)),
            NewsTier::Gold
        );
    }

    #[test]
    fn effective_tier_explicit_free_overrides_default() {
        // Oyuncu açıkça Free'ye geçtiyse rol-default Bronze devreye girmez.
        let p = Player::new(PlayerId::new(1), "S", Role::Sanayici, Money::ZERO, false).unwrap();
        assert_eq!(
            effective_news_tier(&p, Some(NewsTier::Free)),
            NewsTier::Free
        );
    }

    #[test]
    fn charge_debits_each_tick_for_active_subscriber() {
        let mut s = state();
        let mut r = TickReport::new(Tick::new(1));
        let pid = add_player(&mut s, 1, Role::Sanayici, 1_000);
        process_subscribe_news(&mut s, &mut r, Tick::new(1), pid, NewsTier::Gold).unwrap();
        // process_subscribe debit etmedi (cash hâlâ 1000).
        assert_eq!(s.players[&pid].cash, Money::from_lira(1_000).unwrap());
        // Aynı tick'in sonunda charge çağrılır → 960.
        let mut r2 = TickReport::new(Tick::new(1));
        charge_news_subscriptions(&mut s, &mut r2, Tick::new(1));
        assert_eq!(s.players[&pid].cash, Money::from_lira(960).unwrap());
        assert!(r2.entries.iter().any(|e| matches!(
            e.event,
            crate::report::LogEvent::NewsTickCharged { .. }
        )));
    }

    #[test]
    fn charge_zero_cost_path_for_free_tier() {
        // Free tier explicit subscribe — debit yok.
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Tuccar, 100);
        s.news_subscriptions.insert(pid, NewsTier::Free);
        let mut r2 = TickReport::new(Tick::new(2));
        charge_news_subscriptions(&mut s, &mut r2, Tick::new(2));
        assert_eq!(s.players[&pid].cash, Money::from_lira(100).unwrap());
    }

    #[test]
    fn charge_warns_on_first_shortfall() {
        let mut s = state();
        // Gold = 40₺/tick. Cash 30₺ — yetmez.
        let pid = add_player(&mut s, 1, Role::Sanayici, 100);
        // Manuel olarak Gold'a koy (process_subscribe debit etmesin diye cash'i sonra düşürelim).
        s.news_subscriptions.insert(pid, NewsTier::Gold);
        s.players.get_mut(&pid).unwrap().cash = Money::from_lira(30).unwrap();

        let mut r2 = TickReport::new(Tick::new(2));
        charge_news_subscriptions(&mut s, &mut r2, Tick::new(2));

        assert!(s.news_payment_warned.contains(&pid));
        // Tier hâlâ Gold (henüz düşmedi).
        assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));
        assert!(r2.entries.iter().any(|e| matches!(
            e.event,
            crate::report::LogEvent::NewsPaymentWarning { .. }
        )));
    }

    #[test]
    fn charge_downgrades_to_free_on_second_shortfall() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Sanayici, 30);
        s.news_subscriptions.insert(pid, NewsTier::Gold);
        s.news_payment_warned.insert(pid);

        let mut r = TickReport::new(Tick::new(2));
        charge_news_subscriptions(&mut s, &mut r, Tick::new(2));

        // Free'ye düştü — map'te artık Free olarak kayıtlı (silmek yerine).
        assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Free));
        assert!(!s.news_payment_warned.contains(&pid));
        assert!(r.entries.iter().any(|e| matches!(
            e.event,
            crate::report::LogEvent::NewsDowngraded { .. }
        )));
    }

    #[test]
    fn charge_clears_warning_when_payment_resumes() {
        let mut s = state();
        let pid = add_player(&mut s, 1, Role::Sanayici, 1_000);
        s.news_subscriptions.insert(pid, NewsTier::Gold);
        s.news_payment_warned.insert(pid);

        let mut r = TickReport::new(Tick::new(2));
        charge_news_subscriptions(&mut s, &mut r, Tick::new(2));

        // Ödeme yapıldı → uyarı temizlendi, abone hâlâ Gold.
        assert!(!s.news_payment_warned.contains(&pid));
        assert_eq!(s.news_subscriptions.get(&pid), Some(&NewsTier::Gold));
    }
}
