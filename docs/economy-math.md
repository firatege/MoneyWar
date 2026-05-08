# MoneyWar Ekonomi Matematiği — v0.4 Borsa Mekaniği

> Bu döküman MoneyWar v0.4'ün ekonomi modelini matematiksel olarak anlatır.
> Hedef: piyasa neden donmuyor, fiyatlar nasıl keşfediliyor, NPC'ler nasıl
> rasyonel kararlar veriyor.

## İçindekiler

1. [Yapıtaşları](#1-yapıtaşları)
2. [Tick lifecycle](#2-tick-lifecycle)
3. [NPC pricing — order-book aware](#3-npc-pricing--order-book-aware)
4. [Patience erosion + season drift](#4-patience-erosion--season-drift)
5. [Walras tâtonnement](#5-walras-tâtonnement)
6. [Stok-bazlı baseline drift](#6-stok-bazlı-baseline-drift)
7. [Şok event sistemi](#7-şok-event-sistemi)
8. [NPC rol davranışları](#8-npc-rol-davranışları)
9. [Para akış döngüsü](#9-para-akış-döngüsü)
10. [Difficulty parametreleri](#10-difficulty-parametreleri)
11. [Anti-deadlock kanıtı](#11-anti-deadlock-kanıtı)

---

## 1. Yapıtaşları

### Bucket
Bir `(şehir, ürün)` çifti tek bir piyasa bucket'ıdır. 3 şehir × 6 ürün = **18 bucket**.

### Order book
Her bucket'ta `Vec<MarketOrder>`:
```
order_book[(Istanbul, Pamuk)] = [
  MarketOrder { side: Buy,  price: 4.62₺, qty: 25, ttl: 3, player: ... },
  MarketOrder { side: Sell, price: 4.83₺, qty: 30, ttl: 3, player: ... },
  ...
]
```
Tick içinde emirler **anında match edilmez**, sadece toplanır. Tick sonu batch clearing.

### Fiyat referansları
| Sembol | Tanım | Kaynak |
|---|---|---|
| `baseline(c, p)` | Sezon başı çapası, kayar | `state.price_baseline` |
| `initial(c, p)` | Sezon başı snapshot, sabit | `state.price_baseline_initial` |
| `rolling_avg(c, p, 5)` | Son 5 clearing ortalaması | `state.price_history` |
| `effective_baseline(c, p)` | `baseline × shock_multiplier` | `effective_baseline()` |
| `reference_price(c, p)` | `rolling_avg ?? effective_baseline` | NPC'lerin temel referansı |

### Money tipi
Tüm fiyatlar `Money` (cent cinsinden `i64`). 4.62₺ = 462 cents. Bütün hesaplar tam sayı — float yok, deterministik.

---

## 2. Tick lifecycle

`advance_tick(state, commands) → (new_state, report)` saf fonksiyon. RNG `(room_id, tick)`'ten türetilir.

```
1. Dispatch       — komutları işle (SubmitOrder, BuildFactory, ...)
2. Events         — RNG ile şok tetikle (Drought, BumperHarvest, ...)
3. Production     — biten batch'ler envantere
4. Transport      — kervanlar hedef şehre
5. Contracts      — delivery_tick'i gelenler fulfill/breach
6. Loans          — vadesi gelenler
7. Economy tick   — Çiftçi mahsul, Alıcı tüketim, wages, World fab
8. Clearing       — pay-as-bid match + tâtonnement + patience erosion
```

---

## 3. NPC pricing — order-book aware

### CrossPolicy

Her NPC rolü kendi davranış kimliğine göre fiyatlandırır:

```rust
enum CrossPolicy {
  Cross,    // karşı tarafa yetiş (best_bid/best_ask + 1 jiton)
  Passive,  // kendi limit fiyatında bekle
}
```

| Rol | Cross koşulu |
|---|---|
| Çiftçi | `stok ≥ 500` (acil eritme) |
| Alıcı | `daima` (tüketici talep) |
| Sanayici BUY | `fab_var` (ham açlığı) |
| Sanayici SELL | `stok ≥ 150` (mamul birikti) |
| Tüccar | arbitraj kâr eşiği aşıldığında |

### marketable_ask / marketable_bid

NPC'nin SELL fiyat hesabı (`marketable_ask`):

```
urgency_pct = patience_streak + season_drift + market_softener
softened_floor = stock_floor × (1 - urgency_pct / 100)

if policy == Cross AND best_bid ≥ softened_floor:
    target = best_bid                    # bid'a yetiş
else:
    target = softened_floor              # kendi tabanında dur

final_price = jitter(target, tick, city, product, side, player_id)
```

BUY tarafı simetrik:

```
softened_ceiling = cash_ceiling × (1 + urgency_pct / 100)

if policy == Cross AND best_ask ≤ softened_ceiling:
    target = best_ask                    # ask'a yetiş
else:
    target = softened_ceiling            # kendi tavanında dur
```

### Jitter (NPC-spesifik)

`apply_jitter` fiyata ±%3 NPC-spesifik gürültü ekler:

```
h = tick × 2654435761
h ^= city_idx × 7
h ^= product_idx × 13
h ^= side_idx × 17
h ^= player_id × 31              # v0.4.1: NPC-spesifik
h × 2654435761
pct = (h mod 7) - 3              # -3..+3
final = price × (100 + pct) / 100
```

**Niye player_id?** Aynı bucket'ta 4 Tüccar aynı baseline okur — eğer jitter NPC-spesifik olmazsa 4'ü de **aynı fiyat** verir ("bot army"). Player_id ile her NPC kendine has %3'lük sapma alır.

---

## 4. Patience erosion + season drift

### Patience erosion (`no_match_streak`)

Her clearing sonu:
```
for player in bucket_participants:
  if player matched in this tick:
      no_match_streak[(player, city, product)] = 0
  else:
      no_match_streak[(player, city, product)] += 1   # cap MAX_NO_MATCH_STREAK = 15
```

NPC pricing'de:
```
patience = min(no_match_streak, 15)        # 0..15
```

### Season drift

Sezon ilerleme `progress = current_tick / season_ticks ∈ [0, 1]`:
```
drift = progress × 15                       # 0..15
```

### Difficulty softener

Easy mode: `state.market_softener_pct = 15` (NPC fiyatları human lehine)
```
softener = market_softener_pct.clamp(0, 15)
```

### Toplam urgency

```
urgency_pct = min(patience + drift + softener, 45)
```

**Etki**: SELL `floor`'u %45'e kadar düşürür / BUY `ceiling`'i %45'e kadar yükseltir.

---

## 5. Walras tâtonnement

Her clearing sonunda `price_baseline` talep/arz dengesine göre kayar.

### İmbalance

```
imbalance_milli = (BUY_qty - SELL_qty) × 1000 / (BUY_qty + SELL_qty)
                 ∈ [-1000, +1000]
```

### Asimetrik kayma

```
if imbalance > 0:                          # talep fazlası
    factor_milli = 1000 + imbalance × 2 / 1000   # +%0.2/tick (yavaş)
else:                                      # arz fazlası
    factor_milli = 1000 + imbalance × 10 / 1000  # -%1.0/tick (hızlı)
```

**Asimetri gerekçesi**: Reel piyasa "fiyat hızla düşer (panik), yavaş yükselir (talep birikim)" davranışı. 8 Alıcı'nın sürekli pozitif imbalance'ına karşı denge için.

### Clamp

```
new_baseline = baseline × factor_milli / 1000
clamped = clamp(new_baseline, initial × 0.6, initial × 1.6)
```

Sezon boyu kümülatif `baseline ∈ [%60, %160] × initial`. Patlama imkansız.

---

## 6. Stok-bazlı baseline drift

Tâtonnement'a ek mekanizma: bucket'ta **toplam stok** yüksekse fiyat aşağı drift.

```
total_stock = Σ player.inventory[(c, p)]   # bütün oyuncuların bu bucket'taki toplamı

high_threshold = 400 (raw) | 100 (mamul)

if total_stock > high_threshold:
    excess = min(total_stock - high_threshold, 2000)
    extra_down = 3 + excess / 500          # 3..7 milli
    factor_milli -= extra_down              # -%0.3..-%0.7/tick ek
```

**Mantık**: "Arz fazlası → fiyat düşer." Reel ekonomide depo dolu → satıcı indirime mecbur.

Birleşik etki tâtonnement + stok drift:
- BUY > SELL + stok düşük → +%0.2/tick (talep)
- BUY > SELL + stok yüksek → +%0.2 - %0.5 = -%0.3/tick (talep var ama arz daha çok)
- BUY < SELL + stok yüksek → -%1.0 - %0.5 = -%1.5/tick (panik düşüş)

---

## 7. Şok event sistemi

### Tetikleme

Her tick `roll_event` çağrılır:
```
prob = match season_progress:
  early (<%50)  → 12%
  mid   (50-80) → 18%
  late  (>%80)  → 28%

if rng.random_pct() < prob:
    severity = if late { Macro } elif mid { Major } else { Minor }
    pick random event_type ∈ {Drought, Strike, BumperHarvest, NewMarket}
```

### Event tipleri

| Tip | Hedef | Etki |
|---|---|---|
| Drought | (city, raw) | `multiplier = +8/+18/+30 %` (Minor/Major/Macro) |
| Strike | (city, raw) | `multiplier = +8/+18/+30 %` |
| BumperHarvest | (city, raw) | Çiftçi'ye `qty × (50/100/150 %)` ek stok |
| NewMarket | (city, finished) | `multiplier = +25/+40/+50 %` (talep patlaması) |

### Active shocks

```
state.active_shocks: BTreeMap<(CityId, ProductKind), ActiveShock>
ActiveShock { multiplier_pct: i32, expires_at: Tick, source: GameEvent }

effective_baseline(c, p) = baseline(c, p) × (1 + shock_multiplier / 100)
```

Şok aktifken NPC pricing `reference_price`'ı şokla çarpılmış değerden okur → fiyat şoka tepki verir.

---

## 8. NPC rol davranışları

### Çiftçi
Mahsul (`HARVEST_PERIOD = 8` tick, player offset ile dağılmış):
```
for each (city, raw):
  prime_qty   = rand(120..240)            # uzmanlık şehri tam
  secondary   = prime_qty / 4              # ikincil %25
  demand      = prime_qty / 8              # talep %12.5
```

Pricing: `marketable_ask(stock_floor, policy=Cross_if_stock≥500_else_Passive)`. Stok arttıkça `stock_floor_pct` agresifleşir:
```
qty 0-499:    100% baseline
qty 500-999:  80% (panik indirimi başlar)
qty 1000+:    65% (kriz)
```

### Sanayici
Üç aksiyon: BuildFactory, BUY ham, SELL mamul.

BUY ham:
- `cash_ceiling = baseline × 1.10` (fab açlığı için yüksek tavan)
- Cross policy: `fab_var` ise CROSS
- needed_raws: `factory.product.raw_input()` setinden

SELL mamul:
- Stok-tier: 0-49 = %95, 50-149 = %88, 150-299 = %78, 300+ = %70
- Cross at stok≥150 (mamul birikti, eritme zamanı)

Fab build cost (kümülatif):
```
build_cost(0) = 0
build_cost(1) = 4_000₺
build_cost(2) = 10_000₺
build_cost(3) = 18_000₺
build_cost(4+) = 30_000₺
```

### Tüccar
Şehirler arası arbitraj. Her tick:
1. **Kervan al** — eğer arbitraj fırsatı + cash/3 ≥ next_caravan_cost
2. **Dispatch** — idle kervan + stok varsa, en kârlı (product, to_city)'e yolla
3. **BUY/SELL** — pazar emirleri (cheap_city BUY, off-cheap SAT)

### Alıcı
Tüketim (`CONSUME_PERIOD = 8` tick, player offset):
```
for finished_product in inventory:
  consumed = qty × 25%
  inventory -= consumed                    # tüketim, satılmaz
```

Pricing: `marketable_bid(cash_ceiling = baseline × (100..110%))`, daima CROSS.
- Stok dolu (30+) → ceiling 100%
- Stok orta (15) → 105%
- Stok 0 → 110% (kıtlık primi)

### Banka
Komut emit etmez. Engine `tick_banks` ile loan akışı.

---

## 9. Para akış döngüsü

```
Sanayici cash → wages (her 10 tick) → Alıcı cash → market BUY → Sanayici cash
       ↓                                                          ↑
       └─→ build_cost (engine "yok eder")                          │
       └─→ maintenance (engine "yok eder")                         │
                                                                   │
Çiftçi mahsul → market SELL → Sanayici/Tüccar BUY → cash → ...─────┘
                                                                   │
Tüccar BUY ucuz şehir + kervan + SELL pahalı şehir → arbitraj kâr ─┘
                                                                   │
Banka → loan verir (cash inject), loan geri öder (cash sink) ──────┘
                                                                   │
World fab → engine baseline mamul satar (mamul kıtlık önler)       │
```

### Closed-loop koşulu

```
Σ player.cash + Σ contract_escrow + Σ loan_principal = SABIT (modulo build_cost + maintenance + tax)
```

Para sızıntıları:
- `build_cost`: Sanayici → engine (sistem dışı)
- `maintenance` (her 10 tick): Sanayici → engine
- `transaction_tax %2`: trade'de buyer pays + seller alır eksi tax → engine
- `cancel_penalty %2`: emir iptali → engine

Para girişi:
- `wages`: Sanayici → Alıcı (yeni para basılmaz, sadece transfer)
- `World fab gelir`: engine fab satar → cash gelir Sanayici/Tüccar/Alıcı'ya (yeni para girişi)

---

## 10. Difficulty parametreleri

| Param | Easy | Medium | Hard |
|---|---|---|---|
| `top_k` | 8 | 6 | 12 |
| `silence_per_10` | 0 | 1 | 0 |
| `noise` | 0.05 | 0.10 | 0.05 |
| `min_score` | 0.05 | 0.10 | 0.0 |
| `market_softener_pct` | 15 | 5 | 0 |

`top_k`: NPC'nin her tick max kaç aday emit etmesi.
`silence_per_10`: Tick atlama olasılığı (5 = %50 sessiz).
`noise`: Skor hesabına eklenen rastgele gürültü ölçeği.
`min_score`: Aday emit eşiği — bunun altı düşer.
`market_softener_pct`: NPC pricing urgency'sine ek (Easy human lehine).

---

## 11. Anti-deadlock kanıtı

**İddia**: Bir bucket'ta 15 tick içinde garantili match olur (donmuş market imkansız).

### Senaryo

Çiftçi tek (stok 100, cross koşulu yok → pasif), Sanayici tek (fab var → cross). Initial baseline 4.0₺.

| Tick | Çiftçi ASK | Sanayici BID | Crossed? |
|---|---|---|---|
| 0 | 4.00 (floor) | 3.80 (ceiling) | ❌ ASK > BID |
| 1 | 3.99 (patience+1) | 3.81 | ❌ |
| ... | | | |
| 15 | 4.00 × (1-%15) = 3.40 | 3.80 × (1+%15) = 4.37 | **✅ BID > ASK** |

15 tick içinde:
- Çiftçi `floor` `urgency × 1%` indirir → 3.40
- Sanayici `ceiling` `urgency × 1%` yükseltir → 4.37
- Cross alanı: `4.37 - 3.40 = 0.97₺` → match olur

Bunlara ek olarak tâtonnement her tick `imbalance × ±0.2..1.0%` ile baseline kaydırır → `floor`/`ceiling` hızla yakınlaşır.

Sezon drift sezon sonunda `+15` ekleyerek toplam urgency'yi `45`'e çıkarır → cross garantili.

---

## Ek: Görsel akış

```
                    SubmitOrder komutları
                            ↓
                    state.order_book.push
                            ↓
                    Tick sonu: clear_bucket
                            ↓
            ┌──────────────────────────────┐
            │  shuffle (RNG room+tick)      │
            │  match_continuous (pay-bid)   │
            │  settle_fills (cash + envant) │
            │  patience_erosion update      │
            │  tâtonnement: baseline kayar  │
            │  stok_drift: bucket stok yük  │
            └──────────────────────────────┘
                            ↓
                    MarketCleared event
                            ↓
            sparkline + leaderboard güncellenir
```

---

## Referans tablosu (sabitler)

| Konstant | Değer | Açıklama |
|---|---|---|
| `MAX_NO_MATCH_STREAK` | 15 | Patience erosion cap |
| `TRANSACTION_TAX_PCT` | 2 | Trade tax |
| `WAGE_PER_FACTORY_LIRA` | 500 | Sanayici → Alıcı transfer |
| `MAINTENANCE_PER_FACTORY_LIRA` | 250 | Sistem dışı |
| `HARVEST_PERIOD` | 8 | Çiftçi mahsul cycle |
| `CONSUME_PERIOD` | 8 | Alıcı tüketim cycle |
| `WORLD_FAB_PERIOD` | 2 | Engine fab üretim |
| `EVENT_PROB_EARLY/MID/LATE` | 12/18/28 | Şok prob % |
| `PRICE_CLAMP_LOW/HIGH_PCT` | 25 / 175 | Vic3 fiyat clamp |
| `FACTORY_SCORE_NUM/DEN` | 3/4 | Fab değer skora dönüş %75 |
| `CARAVAN_CAPACITY_SANAYICI` | 500 | Sanayici kervan |
| `CARAVAN_CAPACITY_TUCCAR` | 1200 | Tüccar kervan |

---

## Kaynak kod referansları

- `crates/moneywar-engine/src/market.rs` — clear_bucket + tâtonnement
- `crates/moneywar-npc/src/behavior/pricing.rs` — marketable_ask/bid + jitter
- `crates/moneywar-npc/src/behavior/roles/` — rol-spesifik enumerate fonksiyonları
- `crates/moneywar-domain/src/state.rs` — best_bid/best_ask/midpoint helpers
- `crates/moneywar-engine/src/events.rs` — şok event tetikleme
- `crates/moneywar-engine/src/economy.rs` — wages/maintenance/harvest/consume
- `crates/moneywar-domain/src/balance.rs` — tüm sabitler

---

*v0.4.1, 2026-05-08*
