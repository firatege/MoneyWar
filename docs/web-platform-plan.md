# MoneyWar Web Platform — Canlı Ekonomi Dashboard

> **Durum:** Planlandı, başlanmadı
> **Tarih:** 2026-06-01
> **Hedef:** Terminal oyununu, NPC ekonomisinin sonsuza kadar loop'ta çalıştığı,
> insanların web'den canlı izlediği bir borsa/ekonomi dashboard'una çevirmek.

## Kararlar

| Konu | Karar |
|---|---|
| Backend | **actix-web** (yeni `moneywar-web` crate) |
| Loop modeli | Sezon döngüsü — 90 tick → yeni seed → otomatik yeni sezon (sonsuz) |
| Tick hızı | 2 sn/tick (≈3 dk/sezon) |
| Frontend | React + Vite + TS + lightweight-charts (borsa hissi) |
| Faz 1 | Spectator (izleme) |
| Faz 5 | İnteraktif müdahale (sonra) |

## Bağlam

Motor zaten ideal:
- `advance_tick(state, &[Command]) -> Result<(GameState, TickReport)>` — saf + deterministik
- `decide_all_npcs(state, rng, tick, difficulty) -> Vec<Command>` — NPC sürücüsü
- `GameState` tam serileştirilebilir (serde derive)
- tokio workspace'de var, **web framework yok**
- Deploy pipeline hazır: Docker → hub.umceko.com → k8s (feb) → `moneywar.byfeb.com`

Eksik: web sunucusu + DTO katmanı + frontend.

## Kritik Teknik Nokta: DTO Katmanı

GameState `BTreeMap<(CityId, ProductKind), V>` kullanıyor — **tuple key JSON'a
çevrilemez** (postcard bu yüzden var). Web için açık DTO struct'ları kurulacak,
GameState → DTO dönüşüm fonksiyonları yazılacak. İşin ana gövdesi bu.

---

## Faz 1: Backend — `moneywar-web` crate (actix)

**Yeni crate:** `crates/moneywar-web/`
**Bağımlılıklar:** `actix-web`, `actix-ws`, `tokio` (broadcast), `serde_json`, motor crate'leri.

**Bileşenler:**
- `SimDriver` — state + room_id + season_no + tick + difficulty sahibi.
  `step()`: tick < season_ticks ise `decide_all_npcs` + `advance_tick`;
  değilse leaderboard arşivle + yeni seed ile yeni sezon başlat.
- **Loop task** — `tokio::time::interval(2s)` → her tick `driver.step()` →
  snapshot üret → `broadcast::Sender`'a yolla + `Arc<RwLock<Snapshot>>` güncelle.
- **World builder reuse** — sim runner / CLI'daki dengeli NPC kurulumunu
  (3 Sanayici, 4 Tüccar, specialty shuffle) paylaşılan `new_season(seed)`
  fonksiyonuna çıkar. Divergence olmasın.

**Endpoint'ler:**
- `GET /api/snapshot` → mevcut tam snapshot (ilk yükleme + geç katılanlar)
- `GET /api/series?city=&product=` → fiyat zaman serisi
- `WS /ws` → her tick canlı snapshot push (broadcast subscribe)
- `GET /` + statik → frontend (Faz 4)

**DTO'lar (`dto.rs`):**
```
Snapshot { season, tick, season_ticks, players[], leaderboard[],
           prices[], factories[], caravans[], recent_events[] }
PriceCell { city, product, baseline, last, avg5, shock_pct }
PriceSeries { city, product, points: [{tick, price}] }
PlayerDto { id, name, role, npc_kind, cash, is_npc }
EventDto  { tick, kind, summary }   // LogEvent → okunabilir özet
```

**Test:** `cargo run -p moneywar-web` → `curl /api/snapshot`, `wscat /ws`.

---

## Faz 2: Frontend İskelet + Canlı Veri

**Yeni dizin:** `web/` (Vite + React + TS)
- `useGameSocket()` hook — WS bağlan, snapshot state tut, fiyat serilerine ekle,
  event feed'i sınırla (son ~50)
- `<SeasonHeader>` — sezon #, tick/90, geri sayım
- `<Leaderboard>` — canlı PnL sıralaması, rol rozetleri
- `<EventFeed>` — kayan olay şeridi (OrderMatched, FactoryBuilt, FactoryIdle, EventScheduled)

Faz sonu: ekonomi canlı izlenebilir (tablo + feed), grafik yok henüz.

---

## Faz 3: Grafikler + Tam Dashboard

- `<MarketChart>` — lightweight-charts, seçili (şehir, ürün) canlı çizgi/alan grafik
- `<PriceGrid>` — 5 şehir × 6 ürün ızgarası, mini sparkline + tıkla→büyük grafik
- `<CityPanel>` — şehir başına üretim/stok/fabrika durumu
- `<OrderBook>` — seçili bucket bid/ask derinlik görünümü
- Dark "trading terminal" estetiği (web tasarım kuralları: kasıtlı, template değil)

---

## Faz 4: Deploy

- **Dockerfile** çok-aşamalı: (A) Rust `moneywar-web` build, (B) Node `vite build`
  → dist, (C) runtime: binary + dist, actix `Files` ile serve, `:8080`
- **k8s** zaten `moneywar.byfeb.com` → `:8080`. Çalışan değişir (session-server → moneywar-web)
- Terminal oyunu istenirse `terminal.moneywar.byfeb.com`'a taşınabilir (opsiyonel)
- `deploy.sh` güncelle: web binary + frontend build

---

## Faz 5 (Sonra — Ayrı): İnteraktif Müdahale

İzleyiciler web'den ekonomiye etki eder: şok enjekte et (kıtlık/bolluk), event
tetikle, hatta oyuncu olarak emir ver. `advance_tick`'e dışarıdan `Command`
enjeksiyonu gerekir — backend loop'a "pending web commands" kuyruğu eklenir.
Şimdilik sadece tasarımdan haberdar, kodlanmayacak.

---

## Etkilenen / Yeni Dosyalar

| Yol | Tür |
|---|---|
| `crates/moneywar-web/` | Yeni crate (actix server, SimDriver, DTO, loop) |
| Kök `Cargo.toml` | members'a `moneywar-web` ekle |
| Paylaşılan `new_season(seed)` | sim/cli world builder'dan çıkar |
| `web/` | Yeni Vite+React+TS frontend |
| `Dockerfile` | Rust + Node + runtime aşamaları |
| `k8s.yaml`, `deploy.sh` | web binary'e yönlendir |

## Riskler

| Risk | Önlem |
|---|---|
| Tuple-key JSON | DTO katmanı (açık struct'lar) — planın merkezi |
| World builder divergence (cli vs sim) | Tek paylaşılan `new_season` fonksiyonu |
| actix-rt + tokio broadcast | actix-rt tokio üzerine kurulu, broadcast sorunsuz |
| Snapshot payload boyutu | 30 bucket ~30-50KB; birkaç izleyici için tam snapshot/tick yeterli, sonra delta |
| Frontend Docker build | Node base zaten var (session-server) |

## Veri Modeli Referansı

- **5 şehir:** İstanbul, Ankara, İzmir, Bursa, Konya
- **6 ürün:** Pamuk, Buğday, Zeytin (ham) · Kumaş, Un, Zeytinyağı (mamul)
- **30 bucket** = 5 şehir × 6 ürün
- `price_history: BTreeMap<(CityId, ProductKind), Vec<(Tick, Money)>>` — grafik için zaman serisi
- `leaderboard(state) -> Vec<PlayerScore>` — PnL sıralaması (cash + stok + fab + escrow - başlangıç)
- `TickReport.entries: Vec<LogEntry>` — canlı feed kaynağı (her tick olaylar)
