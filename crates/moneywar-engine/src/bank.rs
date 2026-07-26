//! Banka NPC akışı — Plan v4 Faz 8.
//!
//! # İki ürün: kurtarma ve yatırım
//!
//! Banka başlangıçta yalnız **batma riski olana** kredi açıyordu. Denge
//! çalışmaları iflası ortadan kaldırınca bu iş de bitti: ölçümde sezon
//! boyunca 0 kredi, 0 temerrüt ve Banka kârı tam 0. İşi "batmak üzere
//! olana borç vermek" olan bir bankanın, kimsenin batmadığı bir ekonomide
//! yapacak işi yok.
//!
//! Bu yüzden ikinci ürün eklendi: **yatırım kredisi**. Gerçek bankanın asıl
//! işi de budur — üreten ama sermayesi yetmeyen firmayı finanse etmek.
//! Ekonomide bunun karşılığı var: fabrikaların yarısından çoğu girdi
//! açlığı çekiyor ve firmalar tarla/fabrika kuracak nakdi biriktirene
//! kadar bekliyor.
//!
//! Kurtarma kredisi duruyor (emniyet ağı), yatırım kredisi onun yanına
//! geldi. İkisi de aynı kapalı döngüyü kullanır.
//!
//! Closed loop:
//! - Banka kasasından principal çıkar → borçlunun cash'ine eklenir.
//! - Vade tick'inde principal + faiz Banka'ya geri döner (`auto_settle`
//!   loans.rs'te `lender` set'ine göre Banka'ya kredilendirilir).
//!
//! Faiz oranı sabit `BANK_INTEREST_PCT` (v4 için %15, v5'te dinamik).
//! Sadece NPC'lere kredi verir — oyuncu hâlâ klasik `TakeLoan` komutu
//! kullanır (sistem bankası).

use moneywar_domain::{GameState, Loan, LoanId, Money, NpcKind, PlayerId, Tick};

use crate::report::{LogEntry, TickReport};

/// Banka kredi tarama periyodu (her N tick'te bir).
const BANK_LEND_PERIOD: u32 = 12;
// Banka tuning (Faz F3): NPC'lerin çoğu sezon boyu 3K altına iniyordu →
// Banka her cycle 2 kredi açıyor → sezon boyu 30+ kredi → Banka batıyordu.
// DISTRESS 3000 → 1000 (sadece gerçek iflas riski), MAX_LOANS 2 → 1.
/// Borçlu cash eşiği (lira) — bu altına düşen NPC'ye kurtarma kredisi açılır.
const DISTRESS_THRESHOLD_LIRA: i64 = 1_000;

/// Yatırım kredisi tavanı (lira). Nakdi bunun altında kalan **üretici**
/// firma sermaye kısıtlı sayılır: fabrikası var, büyütecek parası yok.
///
/// Değer, firmanın kendi kendine tarla kuramayacağı düzeye ayarlı — NPC
/// tarla için nakdinin 22.500₺'yi geçmesini bekliyor.
const INVESTMENT_CEILING_LIRA: i64 = 20_000;

/// Yatırım kredisi için en az bu kadar fabrikası olmalı: banka fikre değil,
/// çalışan işletmeye kredi verir.
const INVESTMENT_MIN_FACTORIES: usize = 1;
/// Bir Banka tek tick'te en fazla kaç kredi açar (panik önler).
const MAX_LOANS_PER_BANK_PER_TICK: usize = 1;
/// Standart kredi miktarı (lira).
const LOAN_PRINCIPAL_LIRA: i64 = 8_000;
/// Vade (tick).
const LOAN_DURATION_TICKS: u32 = 25;
/// Faiz (tam sayı yüzde).
const BANK_INTEREST_PCT: u32 = 15;
/// Banka tek seferde kasasından en fazla bu kadarını krediye yönlendirir
/// (kendi tampon koruması).
const BANK_MAX_OUTLAY_RATIO_PCT: i64 = 60;

/// Bir NPC yatırım kredisine uygun mu?
///
/// Koşullar bilinçli olarak dar: **üretici** olacak (kredinin karşılığı
/// gerçek bir yatırım olsun), çalışan fabrikası olacak (banka fikre değil
/// işletmeye kredi verir) ve nakdi tavanın altında olacak (kendi kendini
/// finanse edebilen firmaya kredi gereksiz).
fn is_investment_candidate(state: &GameState, id: PlayerId) -> bool {
    let Some(p) = state.players.get(&id) else {
        return false;
    };
    if p.npc_kind != Some(NpcKind::Sanayici) {
        return false;
    }
    if p.cash.as_cents() >= INVESTMENT_CEILING_LIRA.saturating_mul(100) {
        return false;
    }
    state.factories.values().filter(|f| f.owner == id).count() >= INVESTMENT_MIN_FACTORIES
}

/// `advance_tick` içinden çağrılır — Banka NPC'lerin kredi akışını işler.
pub(crate) fn tick_banks(state: &mut GameState, report: &mut TickReport, tick: Tick) {
    let t = tick.value();
    if t == 0 || !t.is_multiple_of(BANK_LEND_PERIOD) {
        return;
    }

    // Banka ve distress NPC listelerini önden topla — borrow checker.
    let banks: Vec<(PlayerId, i64)> = state
        .players
        .iter()
        .filter(|(_, p)| p.npc_kind == Some(NpcKind::Banka))
        .map(|(id, p)| (*id, p.cash.as_cents()))
        .collect();
    if banks.is_empty() {
        return;
    }

    let threshold_cents = DISTRESS_THRESHOLD_LIRA.saturating_mul(100);
    let principal_cents = LOAN_PRINCIPAL_LIRA.saturating_mul(100);

    // Kurtarma önce: batma riski olan, yatırım adayının önüne geçer.
    let distressed: Vec<PlayerId> = state
        .players
        .iter()
        .filter(|(_, p)| {
            p.is_npc && p.npc_kind != Some(NpcKind::Banka) && p.cash.as_cents() < threshold_cents
        })
        .map(|(id, _)| *id)
        .collect();

    // Yatırım adayları — kurtarma listesinde olanlar tekrarlanmaz.
    let investment: Vec<PlayerId> = state
        .players
        .keys()
        .copied()
        .filter(|id| !distressed.contains(id) && is_investment_candidate(state, *id))
        .collect();

    if distressed.is_empty() && investment.is_empty() {
        return;
    }

    // Borçlu kuyruğunu deterministik sırada (PlayerId BTreeMap zaten sıralı).
    let mut queue = distressed.into_iter().chain(investment);

    // Borçluların hâli hazırda açık banka kredisi varsa atla (ikilenme önle).
    let already_borrowed: std::collections::BTreeSet<PlayerId> = state
        .loans
        .values()
        .filter(|l| !l.repaid && l.lender.is_some())
        .map(|l| l.borrower)
        .collect();

    // Bankayı bir kez yakan firmaya ikinci kredi yok. Bu kısıt olmadan
    // temerrüt sonsuz can veriyordu: borç silinir → sonraki döngüde yeni
    // kredi → firma asla batmaz.
    let blacklisted: std::collections::BTreeSet<PlayerId> = state
        .intrigue
        .loan_defaults
        .keys()
        .copied()
        .chain(state.intrigue.bankrupt.iter().copied())
        .collect();

    for (bank_id, mut bank_cash) in banks {
        let max_outlay = bank_cash * BANK_MAX_OUTLAY_RATIO_PCT / 100;
        let mut spent = 0i64;
        let mut issued = 0usize;

        while issued < MAX_LOANS_PER_BANK_PER_TICK {
            // Bütçe kalmadı?
            if spent.saturating_add(principal_cents) > max_outlay {
                break;
            }
            if bank_cash < principal_cents {
                break;
            }
            let Some(borrower_id) = queue.next() else {
                break;
            };
            if already_borrowed.contains(&borrower_id) || blacklisted.contains(&borrower_id) {
                continue;
            }

            // Loan kaydı oluştur.
            let loan_id = LoanId::new(state.counters.next_loan_id);
            let due_tick = match tick.checked_add(LOAN_DURATION_TICKS) {
                Ok(t) => t,
                Err(_) => break,
            };
            let principal = Money::from_cents(principal_cents);
            let Ok(loan) = Loan::new(
                loan_id,
                borrower_id,
                principal,
                BANK_INTEREST_PCT,
                tick,
                due_tick,
            ) else {
                break;
            };
            let loan = loan.with_lender(bank_id);
            let Ok(total_due) = loan.total_due() else {
                break;
            };

            // Banka kasasından çek.
            let Some(bank) = state.players.get_mut(&bank_id) else {
                break;
            };
            if bank.debit(principal).is_err() {
                break;
            }

            // Borçluya kredilendir.
            let Some(borrower) = state.players.get_mut(&borrower_id) else {
                // Borçlu bulunamadı, Banka kasasına geri yatır.
                if let Some(bank) = state.players.get_mut(&bank_id) {
                    let _ = bank.credit(principal);
                }
                break;
            };
            if borrower.credit(principal).is_err() {
                if let Some(bank) = state.players.get_mut(&bank_id) {
                    let _ = bank.credit(principal);
                }
                break;
            }

            // Loan kaydı.
            state.counters.next_loan_id = state.counters.next_loan_id.saturating_add(1);
            state.loans.insert(loan_id, loan);

            report.push(LogEntry::loan_taken(
                tick,
                borrower_id,
                loan_id,
                principal,
                BANK_INTEREST_PCT,
                due_tick,
                total_due,
            ));

            spent = spent.saturating_add(principal_cents);
            bank_cash = bank_cash.saturating_sub(principal_cents);
            issued += 1;
        }
    }
}
