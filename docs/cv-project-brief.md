# MoneyWar — Proje ve Teknik Özet (CV)

## Proje Özeti

**MoneyWar**, tick-tabanlı bir ekonomi simülasyon oyunudur. Piyasa fiyat keşfi, NPC karar motoru, LAN multiplayer ve web platformunu kapsayan tam yığın bir Rust projesidir. Sıfırdan saf fonksiyonel bir oyun motoru, terminal TUI ve gerçek zamanlı web arayüzü yazılmıştır.

| | |
|---|---|
| **Dil** | Rust (100%) |
| **Versiyon** | v0.5.36 |
| **Commit** | 289 |
| **Kod hacmi** | ~36 000 satır Rust |
| **Test sayısı** | ~534 unit + integration testi |
| **Repo** | [github.com/firatege/MoneyWar](https://github.com/firatege/MoneyWar) |

---

## Teknik Mimari

Proje Rust workspace olarak 8 crate'e bölünmüştür. Her katmanın dışarıya bağımlılığı açıkça kısıtlanmıştır:

```
moneywar-domain   (6 078 satır)  — saf veri tipleri, sıfır bağımlılık
moneywar-engine   (8 068 satır)  — tick motoru, sadece domain'e bağlı
moneywar-npc      (6 399 satır)  — NPC davranış ağacı
moneywar-net        (331 satır)  — LAN protokol katmanı (postcard wire)
moneywar-server   (1 235 satır)  — Tokio TCP yetkili sunucu + lobi
moneywar-cli     (10 144 satır)  — Terminal TUI (ratatui + crossterm)
moneywar-sim        (722 satır)  — Başsız deterministik simülasyon koşucusu
moneywar-web      (1 526 satır)  — Web platform (actix-web + WebSocket)
```

**Katman kuralı:** `domain` hiçbir şeye bağlı değil. `engine` sadece `domain` — Tokio yasak, I/O yasak. Bu sayede motor saf fonksiyon, replay ve property test mümkün.

---

## Öne Çıkan Teknik Kararlar

### Deterministik Tick Motoru

```
advance_tick(state, commands) → (new_state, report)
```

Motor bir saf fonksiyondur; I/O ve global state yoktur. RNG `ChaCha8Rng` ile `(room_id, tick)` çiftinden türetilir — aynı girdi her zaman bit-perfect aynı çıktıyı üretir. Bu özellik LAN multiplayer'ın yetkili sunucu modelini mümkün kılar: client'lar yerel görsel, sunucu gerçek durum yönetir.

### Finansal Aritmetik

Tüm para işlemleri `Money(i64)` tipiyle kuruş cinsinden tam sayı olarak yapılır. Float kullanılmaz — kesinlik garantisi ve deterministik davranış sağlanır.

### Deterministik Koleksiyonlar

`HashMap` yasaktır; tüm haritalar `BTreeMap` kullanır. NPC kararları ve clearing iterasyon sırası böylece seed'den bağımsız ama tekrarlanabilir kalır.

### Order-Book Aware NPC Pricing

Her NPC rolü `best_bid`/`best_ask` okuyarak marketable fiyat hesaplar. Rol bazlı `CrossPolicy` ile kimin pasif, kimin agresif emir vereceği belirlenir:

| Rol | Davranış |
|---|---|
| Çiftçi | Stok kritik eşiği aşınca cross (acil eritme) |
| Alıcı | Her zaman cross (tüketici talep baskısı) |
| Sanayici | Fabrika ham açlığında cross |
| Tüccar | Arbitraj kâr eşiği aşıldığında cross |

### Walras Tâtonnement (Asimetrik Fiyat Keşfi)

Her clearing sonunda `price_baseline` talep/arz dengesine göre kayar:

- Yukarı: `+%0.2/tick` (yavaş — talep birikim)  
- Aşağı: `−%1.0/tick` (hızlı — arz fazlası, panik satış)  
- Stok bazlı ek drift: stok eşiği aşıldığında `−%0.3..%0.7/tick`  
- Clamp: `initial × [%60, %160]` (fiyat kaçışını durdurur)

### Anti-Deadlock: Patience Erosion

Art arda eşleşme olmayan oyuncunun fiyat bandı genişler (`no_match_streak`). 15 tick + sezon ilerleme = maks %30 yumuşama. Matematiksel garanti: fiyat donması imkânsız.

### Pay-as-Bid Batch Clearing

Emirler tick boyunca toplanır, tick sonunda toplu eşleştirilir. Her eşleşen lot kendi fiyatından işlem görür. Eşleşme ve tâtonnement kümülatif clearing raporuna yazılır.

---

## Ekonomi Sistemi

| Parametre | Değer |
|---|---|
| Pazar bucket | 18 (3 şehir × 6 ürün) |
| Şok event türü | 4 (Drought / Strike / BumperHarvest / NewMarket) |
| NPC rol arketipi | 5 (Çiftçi, Alıcı, Sanayici, Tüccar, Spekülatör) |
| Kontrat türü | 2 yönlü (ham madde + mamul) |
| Tick lifecycle adımı | 8 (Dispatch → Events → Production → Transport → Contracts → Loans → Economy → Clearing) |

---

## Multiplayer Mimarisi

LAN multiplayer `tokio` TCP üzerine yetkili sunucu modeliyle çalışır:

- Wire format: `postcard` (binary, serde uyumlu)
- Sunucu: her tick `advance_tick` çağırır, diff'i tüm client'lara yayar  
- Client: yalnızca komut gönderir ve render eder, state hesaplamaz  
- Lobi: rol seçimi (`t` Tüccar / `s` Sanayici) + ready gate  

---

## Web Platformu

`moneywar-web` crate'i `actix-web` tabanlı bir backend sunar:

- `GET /api/snapshot` — anlık oyun durumu JSON  
- `GET /api/series` — fiyat zaman serisi  
- `WS /ws` — canlı tick snapshot broadcast (tokio broadcast kanalı)  
- Güvenlik header'ları: CSP, HSTS, X-Frame-Options, Referrer-Policy  
- React frontend statik dosya olarak aynı origin'den servis edilir  

---

## Test ve Kalite

- **~534 test** — unit, integration ve doc-test dahil  
- **Motor invariantları** (proptest ile):  
  - Para korunumu: `Σ(nakit + escrow + banka) = sabit`  
  - Determinizm: aynı input → bin kez aynı output  
  - Escrow non-negatif  
  - Kervan varış garantisi  
- **Clippy profili:** `clippy::all deny + clippy::pedantic warn` — tüm uyarılar CI'da hata sayılır  
- **MSRV:** Rust 1.85 (stable, sabit — CI sürpriz farklarından korunmak için `dtolnay/rust-toolchain` ile pin'lendi)  
- `unsafe_code = "forbid"` — workspace genelinde unsafe yasak  

---

## CI / Yayınlama

GitHub Actions:

| Job | İçerik |
|---|---|
| `ci.yml` | `cargo fmt --check` + `cargo clippy -D warnings` + `cargo test --workspace` |
| `release.yml` | `v*` tag ile tetiklenir; 4 platform için cross-compile + GitHub Release'e upload |

Desteklenen platformlar: macOS ARM, macOS x86-64, Linux x86-64, Windows x86-64.

---

## Kullanılan Teknolojiler

| Kategori | Teknoloji |
|---|---|
| Dil | Rust (edition 2024) |
| Terminal TUI | ratatui 0.29, crossterm 0.28 |
| Async runtime | tokio 1 (multi-thread) |
| Web backend | actix-web, actix-files |
| Gerçek zamanlı | WebSocket (tokio broadcast) |
| Serializasyon | serde, serde_json, postcard (binary wire) |
| RNG | rand_chacha (ChaCha8Rng) |
| Hata yönetimi | thiserror, anyhow |
| Gözlemlenebilirlik | tracing, tracing-subscriber |
| Yapılandırma | toml |

---

## Öğrenilen / Gösterilen Yetkinlikler

- **Saf fonksiyonel mimari**: I/O-free deterministik motor tasarımı
- **Domain-driven design**: her crate tek bir sorumluluk, katman sınırları tip sisteminde kodlanmış
- **Finansal simülasyon matematiği**: tâtonnement, order-book, batch clearing
- **Oyun AI / NPC tasarımı**: rol bazlı davranış ağaçları, CrossPolicy, patience erosion
- **Ağ programlama**: Tokio TCP, yetkili sunucu, binary protokol
- **Terminal UI**: ratatui ile çok panelli TUI, overlay, sparkline, wizard akışları
- **Web geliştirme**: actix-web REST + WebSocket, güvenlik header yönetimi
- **Test kültürü**: property-based testing, invariant kanıtı, 500+ test
- **DevOps**: GitHub Actions CI + çok platform release pipeline
