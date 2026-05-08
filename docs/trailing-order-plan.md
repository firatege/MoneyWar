# Trailing Order — Gelecek Feature

> **Durum**: planlanmış, henüz uygulanmadı.
> **Neden**: User feedback (2026-05-08): "TTL'de kalsa bile fiyatlar değişiyor,
> orderi değiştiremiyorum". Uzun TTL emirler tâtonnement'la fiyat farkına
> takıldığı için faydasız.

## Sorun

Mevcut `MarketOrder.unit_price` sabit. Tâtonnement her tick `price_baseline`'i
%0.2-1.0 kaydırır. 50 tick'lik bir emir verildiğinde:

```
t1:   BUY 1000 @ 7.00₺ (baseline 7.00 — fiyat doğru)
t10:  baseline 7.20 → senin 7.00 emrin alt tarafta
t30:  baseline 7.80 → emir tamamen geride, kimse satmaz
t50:  TTL bitti, çoğu match olmadı
```

Sonuç: insan oyuncu **passive playstyle** ("bir kerede al, fab işlesin")
istiyorsa mevcut sistemde çalışmıyor. Sürekli cancel + resubmit gerekli.

## Önerilen çözüm: Trailing Order

`MarketOrder.price_mode` enum'u ekle:

```rust
pub enum PriceMode {
    /// Klasik sabit limit fiyat (mevcut davranış).
    Fixed(Money),
    /// Baseline'a göre kayar fiyat — engine her tick yeniden hesaplar.
    Trailing {
        /// `effective_price = baseline × (1 + offset_pct / 100)`
        /// BUY için negatif (-5 → baseline'ın %5 altı), SELL için pozitif.
        offset_pct: i32,
    },
}
```

Engine clearing'de:
```rust
let effective_price = match order.price_mode {
    PriceMode::Fixed(p) => p,
    PriceMode::Trailing { offset_pct } => {
        let base = state.effective_baseline(city, product).unwrap_or(Money::ZERO);
        Money::from_cents(base.as_cents() * (100 + offset_pct as i64) / 100)
    }
};
```

## Avantajlar

- Insan oyuncu tek emir verir, sezon boyu pasif çalışır
- "Baseline'ın %5 üstüne ham al" → otomatik takip
- Reel piyasa **trailing stop** mantığı

## Dezavantajlar / Risk

- `MarketOrder` struct değişir → serde format breaking change
- Match logic'inde her order için price hesabı (mevcut: pre-calculated)
- TUI'de yeni wizard adımı (Fixed vs Trailing seçim)
- Replay/test'lerde determinizm korunmalı (baseline tâtonnement deterministik
  zaten — sorun yok)

## Implementasyon planı

### Faz 1: Domain
- `MarketOrder.price_mode: PriceMode` (eski `unit_price` deprecate, `Fixed` wrap)
- Migration: mevcut state'lerde `Fixed(unit_price)` ile init
- Test: `MarketOrder::new` overload (Fixed) + `new_trailing(offset)`

### Faz 2: Engine
- `match_continuous` → `effective_price` hesabı her loop iterasyonunda
- Pay-as-bid clearing: trailing emirde de `incoming.effective_price` kullan
- Patience erosion: trailing emirler de aynı sayaca dahil

### Faz 3: TUI
- Wizard PriceLira adımına 2 mod:
  - `F` (Fixed) — eski davranış, sabit fiyat
  - `T` (Trailing) — `% offset` gir (örn -5 BUY için, +3 SELL için)
- Holdings panelinde trailing emir varsa "🌊 trailing -5%" badge
- Status mesajı: "BUY 1000 Pamuk, baseline-5% trailing, TTL 50"

### Faz 4: NPC (opsiyonel)
- Sanayici/Çiftçi default davranış zaten her tick taze emir veriyor
  (kendi pricing helper'ları), trailing'e ihtiyaç duymaz
- Belki sezon-uzun stratejik insan-benzeri Tüccar AI için kullanılır

## Test stratejisi

```rust
#[test]
fn trailing_order_follows_baseline() {
    let mut s = setup();
    s.price_baseline.insert((Ist, Pamuk), Money::from_lira(7).unwrap());
    let order = MarketOrder::new_trailing(..., Buy, qty=100, offset=-5, ttl=10);
    s.order_book.insert(...);
    // baseline 7 → effective 6.65
    assert_match_at(price=6.65);
    // tâtonnement: baseline 7 → 7.20
    advance_tick(...);
    // emir hala kitapta, effective 7.20 × 0.95 = 6.84
    assert_match_at(price=6.84);
}
```

## Ne zaman?

v0.5.0 hedefli. v0.4.x içinde **kontrat sistemi** (mevcut) bu eksiği
geçici doldurur — insan oyuncu kontrat ile passive alım yapar.

## Referans

- `crates/moneywar-domain/src/order.rs` — MarketOrder struct
- `crates/moneywar-engine/src/market.rs::match_continuous` — clearing
- `crates/moneywar-cli/src/main.rs::FieldKind::PriceLira` — wizard

---

*Not alındı: 2026-05-08, kullanıcı talebi*
