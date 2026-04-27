//! NPC kontrat davranışı — öner + kabul aksiyonları.
//!
//! Mevcut domain altyapısı: `Contract`, `ContractProposal`, `ListingKind`
//! (Public/Personal). NPC sadece **Public** kontrat önerir (pano ilanı,
//! ilk kapan alır), gelen Public ilanı utility ile kabul edebilir.
//!
//! Kişilik etkisi:
//! - 📦 Hoarder: kontrat sever (uzun-vadeli sabit gelir)
//! - 💀 Cartel: büyük miktar kontrat → manipülasyon
//! - ⚡ Aggressive: kontrat nadir öner (hızlı satış sever)
//! - 🎲 EventTrader: olay yaklaşırken sabit fiyat lock
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::doc_markdown
)]

use moneywar_domain::{
    CityId, Command, ContractId, ContractProposal, ContractState, GameState, ListingKind, Money,
    Personality, PlayerId, ProductKind, Tick,
};

use crate::dss::inputs::money_lira;
use crate::dss::utility::ActionCandidate;

/// Sat kontratı öneri adayları — NPC'nin stoğunda bulunan ürün × hedef şehir.
/// Her tick max 1-2 aday üretir (top-K filtreleme yapar caller).
///
/// Mantık:
/// - NPC'nin stoğu var (city, product) → bu stoğu kontratlamak makul
/// - delivery_tick = current + 5 (orta vadeli)
/// - unit_price = market × 1.05 (alıcı için biraz prim, kabul motivasyonu)
/// - kapora = total_value × 0.10 (her iki taraf %10 risk)
pub fn propose_contract_candidates(
    state: &GameState,
    pid: PlayerId,
    personality: Personality,
    tick: Tick,
) -> Vec<(Command, ActionCandidate)> {
    let Some(player) = state.players.get(&pid) else {
        return Vec::new();
    };

    // Kişilik bazlı filtre — Aggressive ve Momentum kontrat sevmez
    let kontrat_zaten_var = state.contracts.values().any(|c| {
        c.seller == pid && matches!(c.state, ContractState::Proposed | ContractState::Active)
    });
    if kontrat_zaten_var {
        return Vec::new(); // Aynı NPC bir kontrat aktifken yenisini önermez
    }

    // En çok stoğu olan (city, product)'u bul — kontrat için en uygun
    let mut entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, _, q)| *q >= 30) // en az 30 birim stok lazım
        .collect();
    entries.sort_by_key(|(_, _, q)| std::cmp::Reverse(*q));

    let mut out = Vec::new();
    for (city, product, qty) in entries.into_iter().take(2) {
        let market = state
            .price_history
            .get(&(city, product))
            .and_then(|v| v.last())
            .map(|(_, p)| *p)
            .or_else(|| state.effective_baseline(city, product))
            .unwrap_or(Money::from_cents(800));

        let unit_price_cents = (market.as_cents() * 105) / 100;
        let unit_price = Money::from_cents(unit_price_cents);

        let contract_qty: u32 = qty.min(50); // max 50 birim
        let total_value_cents = unit_price_cents.saturating_mul(i64::from(contract_qty));
        let deposit_cents = total_value_cents / 10; // %10 kapora

        // Nakit yetiyor mu (kapora için)?
        if player.cash.as_cents() < deposit_cents {
            continue;
        }

        let proposal = ContractProposal {
            seller: pid,
            listing: ListingKind::Public,
            product,
            quantity: contract_qty,
            unit_price,
            delivery_city: city,
            delivery_tick: tick.checked_add(5).unwrap_or(tick),
            seller_deposit: Money::from_cents(deposit_cents),
            buyer_deposit: Money::from_cents(deposit_cents),
        };

        // Utility hesabı:
        // - profit_lira: garantili kâr (qty × price - kapora riski)
        // - capital_lira: kapora kilidi
        // - risk: 0.3 (cayma riski)
        // - urgency: kişiliğe göre
        // - hold_pressure: 5 tick stok kilidi
        let revenue = money_lira(unit_price) * f64::from(contract_qty);
        let urgency = match personality {
            Personality::Hoarder => 0.9,
            Personality::Cartel => 0.7,
            Personality::EventTrader | Personality::MeanReverter => 0.6,
            Personality::Arbitrageur | Personality::TrendFollower => 0.3,
            Personality::Aggressive => 0.2,
        };

        let action = ActionCandidate {
            profit_lira: revenue * 0.5, // garantili gelir, ~yarısı net kâr
            capital_lira: money_lira(Money::from_cents(deposit_cents)),
            risk: 0.3,
            urgency,
            momentum: 0.0,
            arbitrage: 0.0,
            event: 0.0,
            hold_pressure: 0.6,
        };

        out.push((Command::ProposeContract(proposal), action));
    }
    out
}

/// Pano'daki Public kontratlardan kabul aday üret.
///
/// NPC alıcı tarafı olur — Public ilanı kabul ederek `ContractState::Active`
/// yapar. Avantaj: sabit fiyatla mal alır (gelecek teslim).
///
/// Kişilik:
/// - Hoarder/MeanReverter: kabul eder (sabırlı)
/// - Aggressive/TrendFollower: nadir kabul (anlık fırsat sever)
pub fn accept_contract_candidates(
    state: &GameState,
    pid: PlayerId,
    personality: Personality,
    _tick: Tick,
) -> Vec<(Command, ActionCandidate)> {
    let Some(player) = state.players.get(&pid) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (cid, contract) in &state.contracts {
        if contract.state != ContractState::Proposed {
            continue;
        }
        if !contract.listing.is_public() {
            continue; // sadece public, personal hedefli
        }
        if contract.seller == pid {
            continue; // kendi kontratımı kabul edemem
        }

        // Kabul için kapora yetiyor mu?
        if player.cash < contract.buyer_deposit {
            continue;
        }

        // Toplam ödeme yetiyor mu?
        let Ok(total_value) = contract.total_value() else {
            continue;
        };
        if player.cash < total_value {
            continue;
        }

        // Fiyat değerlendirmesi: market'tan ucuz mu?
        let market = state
            .effective_baseline(contract.delivery_city, contract.product)
            .unwrap_or(Money::from_cents(800));
        let ratio = if market.as_cents() > 0 {
            contract.unit_price.as_cents() as f64 / market.as_cents() as f64
        } else {
            1.0
        };

        // Ratio < 1.0 → kontrat fiyatı market'tan düşük (alıcı için iyi)
        // Ratio > 1.0 → market'tan yüksek (alıcı için kötü, kabul nadir)
        let value_signal = (1.0 - ratio).max(-0.5);

        // Kişilik kabul eğilimi
        let acceptance_bias = match personality {
            Personality::Hoarder => 0.8,
            Personality::MeanReverter => 0.7,
            Personality::EventTrader => 0.5,
            Personality::Cartel => 0.4,
            Personality::Arbitrageur | Personality::TrendFollower => 0.2,
            Personality::Aggressive => 0.1,
        };

        let action = ActionCandidate {
            profit_lira: money_lira(total_value) * value_signal * 0.5,
            capital_lira: money_lira(total_value),
            risk: 0.4, // satıcı caymabilir
            urgency: acceptance_bias,
            momentum: 0.0,
            arbitrage: 0.0,
            event: 0.0,
            hold_pressure: 0.3,
        };

        out.push((
            Command::AcceptContract {
                contract_id: ContractId::new(cid.value()),
                acceptor: pid,
            },
            action,
        ));
    }
    out
}
