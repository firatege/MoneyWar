# MoneyWar — Feature Roadmap

> **Son güncelleme**: 2026-05-09
> **Şu anki sürüm**: `v0.4.1`
> Bekleyen feature'lar, statü, öncelik ve dependency'ler.

## 📚 Statü göstergeleri

- ✅ **Tamam** — implementasyon bitmiş, master'da
- 🚧 **Devam** — aktif geliştirme
- 📋 **Plan** — döküman var, kod yok
- 💭 **Vizyon** — fikir aşaması, henüz plan yok
- ⏸ **Beklemede** — bağımlılık veya karar bekliyor

---

## v0.4.1 (mevcut)

| Feature | Statü | Not |
|---|---|---|
| Order-book aware pricing | ✅ | `marketable_ask`/`bid` + `CrossPolicy` |
| Walras tâtonnement (asimetrik) | ✅ | `+%0.2/-1.0` per tick |
| Patience erosion | ✅ | `MAX_NO_MATCH_STREAK = 15` |
| Ürün-spesifik üretim | ✅ | Un/Kumaş/Zeytinyağı farklı verim+süre |
| NPC-spesifik jitter | ✅ | `player_id` hash'e karıştırıldı |
| Spek emekli | ✅ | Kompozisyon 0, altyapı korundu |
| Çiftçi starter stok | ✅ | Sezon başı 200 birim prime ham |
| Wizard pre-flight engel | ✅ | Cooldown/cash/stok kontrolü |
| Leaderboard popup (`L`) | ✅ | İnsan + Sanayici + Tüccar filtre |
| Reddedilenler overlay (`F`) | ✅ | Son 50 reject sebebi |
| Şok event sıklığı artırıldı | ✅ | early/mid/late = 12/18/28 % |
| TTL cap genişletildi | ✅ | `100/200` (önce `10/50`) |

---

## v0.5.0 — Sıradaki Sprint

### 🚧 Sprint A — Yeni şehirler (öncelik: yüksek)
**Hedef**: 3 → 5 şehir (Bursa + Konya).
**Süre tahmini**: 1-1.5 saat.

- `CityId` enum genişlet (Bursa, Konya)
- `distance_to()` 5×5 matrix (Türkiye coğrafyası)
- `demand_for()`, `cheap_raw()` yeni şehirler için
- NPC kompozisyonu: Çiftçi 6 → 10 (şehir başına 2)
- TUI Market panel responsive layout

**Yan etki**: 18 → 30 bucket. Performans sorun yok (`O(bucket × order)`).
**Tetikleyici user feedback**: "daha fazla şehir" (2026-05-09).

---

### 🚧 Sprint B — Yeni ham/mamul zincirleri (öncelik: yüksek)
**Hedef**: 6 → 10 ürün. İki yeni tek-girdi zincir.
**Süre tahmini**: 2 saat.

| Yeni ham | Yeni mamul | Verim | Süre | Karakter |
|---|---|---|---|---|
| Süt | Peynir | %70 | 3 tick | Bozulur (perish 2t/50%) |
| Şeker pancarı | Şeker | %60 | 2 tick | Lüks tüketim |

- `ProductKind` enum 4 yeni variant
- `output_ratio_pct`, `production_ticks`, `base_price_lira`, `raw_input` genişlet
- Şok event'ler yeni ham'ları desteklesin (BumperHarvest/Drought)
- TUI ürün renkleri/emojiler

**Yan etki**: Sprint A ile birlikte 50 bucket toplam.
**Tetikleyici**: "biraz daha hammadde olabilir mi" (2026-05-09).

---

### 📋 Trailing order — `PriceMode` enum
**Statü**: Plan var (`docs/trailing-order-plan.md`).
**Süre**: 2-3 saat.
**Tetikleyici**: "TTL'de kalsa bile fiyatlar değişiyor" (2026-05-08).

- `MarketOrder.price_mode: PriceMode { Fixed | Trailing }`
- Engine her tick `effective_price = baseline × (1 + offset/100)` hesaplar
- TUI wizard'da Fixed/Trailing toggle (`F`/`T` tuşu)

**Avantaj**: Insan oyuncu tek emir verir, sezon boyu pasif takip.
**Risk**: `MarketOrder` struct değişir → serde format breaking.

---

### 📋 Multi-shock support
**Statü**: Plan var (sade).
**Süre**: 1 saat.
**Tetikleyici**: "şoklar üst üste binebilmeli" (2026-05-08).

- `state.active_shocks` `BTreeMap<key, ActiveShock>` → `BTreeMap<key, Vec<ActiveShock>>`
- `effective_baseline` çoklu şok multiplier'larını çarpar
- TUI'de aktif şok şeridinde stack gösterimi

**Beklenen**: aynı bucket'a Drought + Strike üst üste gelirse fiyat etkisi birikir.

---

### 📋 Stok escrow
**Statü**: Plan var.
**Süre**: 3-4 saat.
**Tetikleyici**: kontrat NPC propose %78 breach sorunu (2026-05-08).

- `Inventory.escrowed: BTreeMap<(City, Product), u32>`
- Kontrat accept'inde stok escrow'a kilitlenir
- Market emirinde escrow'lu stok satılamaz
- Settlement'ta serbest bırakılır

**Avantaj**: NPC kontrat propose'u tekrar açılır (şu an kapalı), %0 breach.

---

### 💭 EconomyTuning struct
**Statü**: Vizyon (mevcut sezon-uzun parametreler hardcoded).
**Süre**: 1-2 saat.

```rust
pub struct EconomyTuning {
    tatonnement_up_pct: u32,    // *1000
    tatonnement_down_pct: u32,
    event_prob_early/mid/late: u32,
    contract_delivery_offset: u32,
    harvest_period: u32,
    consume_period: u32,
}
impl EconomyTuning {
    pub fn for_preset(preset: Preset) -> Self { ... }
}
```

**Hedef**: Hızlı/Standart/Uzun preset'lere göre otomatik tuning.

---

## v0.6.0 — Orta vadeli

### 📋 Sprint C — Çoklu girdi mamul (tier-2)
**Statü**: Plan var.
**Süre**: 3-4 saat (büyük refactor).
**Tetikleyici**: "karmaşık mamul sistemi olabilir mi" (2026-05-09).

| Mamul | Girdi (units/100) | Verim | Süre | Base ₺ |
|---|---|---|---|---|
| Pasta | Un (60) + Şeker (40) | %85 | 4 tick | 80 |
| Dondurma | Süt (50) + Şeker (50) | %75 | 3 tick | 70 |
| Yünlü Kumaş | Kumaş (70) + Yün (30) | %90 | 5 tick | 95 |

- `Recipe` struct: `Vec<RecipeInput>`
- `raw_input()` deprecate, `recipe()` çağrılır
- Engine production: tüm girdileri kontrol et + düş
- Partial batch: en kıt girdiye göre proportional

**Bağımlılık**: Sprint B (yeni ham'lar) önce gerek.

---

### 📋 MP Sprint 4 — Multiplayer TUI integration
**Statü**: Plan var (`docs/multiplayer-roadmap.md`).
**Süre**: 4-6 saat.

- ratatui TUI'yi multiplayer'a entegre et (şu an stdout-mode)
- `state_hash` validation (server vs client)
- Reconnect mekanizması
- Lobi → game transition smooth

**Bağımlılık**: MP Sprint 0-3 ✅ tamam.

---

### 💭 PostgreSQL persistence
**Statü**: Faz 10 (game-design.md'den).
**Süre**: sezon-uzun (8+ saat).

- `sqlx` ile schema (players, matches, contracts, leaderboards)
- Sezon arası kariyer (oyuncu profili, çoklu sezon istatistik)
- Replay sistemi (her tick state save)

---

## v0.7+ — Vizyon

### 💭 Frontend (TS/React)
**Faz 11** (game-design.md'den).
- WASM bridge (engine'i tarayıcıya getir)
- React ile zengin UI (canlı grafikler, leaderboard real-time)
- Mobile-friendly responsive

### 💭 Steam release hazırlık
**Faz 12**.
- Steam SDK entegrasyonu
- Cloud save
- Achievement sistemi
- Leaderboard global

### 💭 Mod desteği
- Kullanıcı `recipe.toml` ekleyebilir
- Custom ürün/şehir tanımı
- NPC AI behavior override

### 💭 E2E smoke + benchmark suite
- Headless full-sezon stress test
- Determinism benchmark (replay golden master)
- Performance regression detection

---

## Yapılmayacaklar (kasıtlı)

- ❌ **Cüzdan tabanlı micro-transaction** — gerçek para entegrasyonu hedef değil
- ❌ **PvP combat** — bu ekonomi sim, savaş değil
- ❌ **Random emir bot** — NPC'lerin AI'sı zaten bu rolü yapıyor
- ❌ **Voice chat** — multiplayer'da metin yeter

---

## Karar matrisi (öncelik için)

| Feature | User talebi var mı? | Etki büyüklüğü | Risk | Süre |
|---|---|---|---|---|
| Sprint A (şehir) | ✓ | büyük | düşük | 1-1.5 saat |
| Sprint B (yeni ürün) | ✓ | büyük | düşük | 2 saat |
| Trailing order | ✓ | orta | orta (refactor) | 2-3 saat |
| Multi-shock | ✓ | orta | düşük | 1 saat |
| Stok escrow | ✓ | büyük | yüksek (engine refactor) | 3-4 saat |
| Sprint C | ✓ | çok büyük | yüksek | 3-4 saat |
| MP Sprint 4 | — | büyük | yüksek | 4-6 saat |
| EconomyTuning | — | küçük | düşük | 1-2 saat |

**Önerilen sıra v0.5.0 için**:
1. Sprint A (şehir) — kolay başlangıç, ekonomi zenginleşir
2. Multi-shock — küçük, kalan user feedback'i bitirir
3. Sprint B (yeni ürün) — ekonomik çeşitlilik
4. Trailing order — passive playstyle desteği
5. Stok escrow — kontrat sistemi tam çalışır

---

## Geçmiş kararlar

| Tarih | Karar | Sebep |
|---|---|---|
| 2026-05-07 | Esnaf emekli (composition 0) | %78 breach, mismatch |
| 2026-05-07 | Spek emekli (composition 0) | -304K para sızıntısı, 0 SELL match |
| 2026-05-08 | NPC kontrat propose kapatıldı | Stok escrow yok → %78 breach |
| 2026-05-08 | TTL cap 50 → 200 | User passive playstyle |
| 2026-05-08 | Sezon: sadece Hızlı (90 tick) | User: "Standart/Uzun gerek yok" |
| 2026-05-09 | Ürün-spesifik üretim | User: "gerçekçi yapalım" |

---

## Referanslar

- `docs/architecture.md` — sistem mimarisi
- `docs/game-design.md` — oyun tasarım kararları
- `docs/economy-math.md` — borsa matematiği detay
- `docs/multiplayer-roadmap.md` — MP sprint planları
- `docs/trailing-order-plan.md` — trailing order detay
- `docs/contracts-bidirectional-plan.md` — kontrat sistemi planı
- `docs/npc-quality-bar.md` — NPC kalite kriterleri

---

*Bu dosya canlıdır. Yeni feature istekleri bu dokümana eklenir, statü güncellenir.*
