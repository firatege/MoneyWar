# Fuzzy NPC Motoru + Test Ortamı + Kontrat AI + UI Cilası

> **Status:** ▶ Faz 0 başladı.
> **Hedef sürüm:** v0.3.0+
> **Tahmini efor:** 27-38 saat (5-7 commit)

## Niyet

Önce **gözlem aracı** kur, sonra fix'leri ölç. Tüm NPC tipleri (Sanayici, Tüccar,
Esnaf, Alıcı, Spekülatör) tek fuzzy motora bağlansın. Kontrat propose/accept
kararları da AI'ya. Difficulty 3 kademe (Easy/Medium/Hard, Expert yok).
Sezon sonu "neyi neden yaptı, kim battı, hangi mal piyasa altı kaldı"
detaylı rapor üretir. UI tarafında küçük cilalar (sade + az tuş + okunaklı).

## Faz Sırası

| Faz | İş | Süre | Risk |
|---|---|---|---|
| **0** | Test/observability ortamı (sim crate, snapshot, trace, rapor) | 5-7s | 🔴 ÖNCE |
| 1 | Difficulty Easy/Medium/Hard | 1-2s | 🟢 |
| 2 | NpcInputs modülü | 2s | 🟢 |
| 3 | Fuzzy variable library | 1s | 🟢 |
| 4 | Rol başına rule base (5 rol × 5-10 kural) | 3-5s | 🟡 |
| 5 | NpcDecisionEngine orchestrator | 3-4s | 🔴 |
| 6 | Tüm NPC'leri migrate (Esnaf/Alıcı/Spekülatör dahil) | 4-5s | 🔴 |
| 7 | Personality bias entegrasyon | 1-2s | 🟢 |
| 8 | Kontrat AI fuzzy | 2-3s | 🟡 |
| 9 | UI cilası (ayrı PR) | 2-3s | 🟢 |
| 10 | Test sürekli | 3-4s | 🟡 |

## Faz 0 — Test/Observability Ortamı

Yeni crate: `crates/moneywar-sim/`

### Bileşenler

```
crates/moneywar-sim/
├── Cargo.toml
├── src/
│   ├── lib.rs              # SimRunner public API
│   ├── runner.rs           # Headless deterministic run
│   ├── snapshot.rs         # TickSnapshot struct
│   ├── trace.rs            # NPC karar trace
│   ├── report.rs           # Markdown rapor üretici
│   └── scenario.rs         # İnsan oyuncu simülasyon scriptleri
├── tests/
│   └── baseline_v02.rs     # Mevcut sistemi baseline olarak kaydet
└── bin/
    └── sim.rs              # CLI: cargo run -p moneywar-sim --
```

### TickSnapshot

```rust
pub struct TickSnapshot {
    pub tick: u32,
    pub players: Vec<PlayerSnapshot>,
    pub open_orders: Vec<OrderSnapshot>,
    pub clearings: Vec<ClearingEntry>,
    pub events: Vec<EventLog>,
}
```

### NPC Karar Trace

Her NPC × her tick:
- Hangi inputs (cash_norm, stock_norm, vb.) — Faz 2 sonrası dolar.
- Hangi outputs (buy_score, sell_score, vb.) — Faz 4 sonrası dolar.
- Action emitted veya neden emit edilmedi.

Faz 0'da fuzzy yok → trace placeholder olarak commit count + emir özeti tutar.

### Rapor Soruları

1. Kim battı (cash < 100₺)? Hangi tick'te? Son aksiyonu?
2. Doygunluk: tick × toplam talep eğrisi
3. Piyasa altı emirler: 10+ tick bekleyen, market×0.85 altı SELL
4. NPC PnL evolution
5. Match/reject oranı
6. Fiyat trend per (city, product)
7. NPC × action_kind sayım

### Karşılaştırma

```bash
cargo run -p moneywar-sim -- \
    --baseline configs/v0_2_baseline.json \
    --variant configs/fuzzy_v3.json \
    --seeds 1,7,42 \
    --ticks 90 \
    --report-out artifacts/comparison.md
```

## Açık Sorular & Onaylar

Plan v2'de listelenen 12 soru için **default önerilerle başla** kararı alındı:

1. moneywar-sim **ayrı crate** ✓
2. Trace çıktı **JSONL + sezon sonu Markdown** ✓
3. Önce **headless**, in-game `:trace` sonra ✓
4. Fuzzy çıkış **utility (0-1)** ✓
5. Personality **multiplier olarak korunur** ✓
6. **Tek motor**, rol başına rule base ✓
7. `score_action` **personality adımında kalır** ✓
8. Performance **cache yok** (Sugeno hızlı) ✓
9. Difficulty **sadece oyun başlangıcı** ✓
10. Eski decide kodu **bir tur deprecated** ✓
11. Kontrat AI bu PR'da, **bidirectional ayrı PR** ✓
12. UI cilası **ayrı PR (Faz 9)** ✓

## Risk

- 🔴 **Test ortamı kurmadan fix yapma** — "iyileşti mi?" cevapsız.
  → Faz 0 ZORUNLU önce.
- 🔴 Davranış regression — sim ile yakalanır.
- 🟡 Kural ince ayarı — sim raporları ile iteratif.
- 🟡 Determinism — fuzzy zaten deterministic.

## Kalite Kapısı (Difficulty başına ayrı)

Her faz sonrası `cargo run -p moneywar-sim --multi-seed --difficulty <X>`
ile **3 zorluk seviyesinin her biri için ayrı 6 madde** kontrol et —
detay [`npc-quality-bar.md`](./npc-quality-bar.md):

| Difficulty | Skor hedefi | v04 Mevcut |
|---|---|---|
| 🟢 Easy | 6/6 | 4/6 |
| 🟡 Medium | 6/6 | 2/6 |
| 🔴 Hard | 6/6 | 2/6 |

**Toplam:** 18/18 yeşil olunca Faz 8 (kontrat AI) açılır. Mevcut: 8/18.

Ortak sorunlar (3 zorluk için de fix gerekli):
1. Alıcı NPC iflas (her zorlukta)
2. Spekülatör batıyor (3 zorlukta da negatif PnL)
3. Stale emir ölçümü sim raporunda yok

## Sonraki Adım

Faz 2: `NpcInputs` modülü — fuzzy/DSS için normalize sinyaller.
