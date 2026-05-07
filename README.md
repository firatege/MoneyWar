# 💰 MoneyWar

Tick-tabanlı ekonomi simülasyon oyunu. **Rust workspace** olarak yazıldı; saf motor + terminal TUI + LAN multiplayer.

> Oyun tasarım dokümanı: [`docs/game-design.md`](docs/game-design.md)
> Mimari plan: [`docs/architecture.md`](docs/architecture.md)
> Public API cheat-sheet: [`docs/API.md`](docs/API.md)

**Şu anki sürüm**: `v0.4.0-pre1` — ekonomi modeli yeniden inşa (order-book aware pricing, tâtonnement, kontrat sistemi). Detay: aşağıdaki [Sürüm notları](#-sürüm-notları-v040-pre1).

---

## 🎮 Oyunu dene

### Seçenek 1 — Hazır binary (Rust gerekmez)

[GitHub Releases](https://github.com/firatege/MoneyWar/releases) sayfasından indir:

| Sistem | İndir | Çalıştır |
|---|---|---|
| macOS (Apple Silicon) | `moneywar-aarch64-apple-darwin.tar.gz` | `./moneywar` |
| macOS (Intel) | `moneywar-x86_64-apple-darwin.tar.gz` | `./moneywar` |
| Linux (x64) | `moneywar-x86_64-unknown-linux-gnu.tar.gz` | `./moneywar` |
| Windows (x64) | `moneywar-x86_64-pc-windows-msvc.zip` | `moneywar.exe` |

### Seçenek 2 — Kaynaktan derle

```bash
git clone https://github.com/firatege/MoneyWar.git
cd MoneyWar
./mw solo            # tek-oyunculu TUI
```

`./mw` proje köküne eklenmiş kısayol scripti:

```bash
./mw help            # tüm komutlar
./mw solo            # tek-oyunculu TUI
./mw server          # LAN sunucu (port 7878)
./mw connect 192.168.1.42:7878 Ali   # LAN client
./mw build           # release build
./mw test            # cargo test --workspace
./mw check           # fmt + clippy (CI ile aynı)
```

---

## 🛠 Tek-oyunculu CLI

`./mw solo` ile aç. Startup ekranında:
- **Preset** seç: `Hızlı` (90 tick) / `Standart` (150 tick) / `Uzun` (350 tick)
- **Difficulty** seç: `Easy` (NPC fiyat marjı %15 cömert, bol likidite) / `Hard` (saf rekabet)
- **İsim** gir, Enter ile başla

İçeride:

| Tuş | İşlev |
|---|---|
| `Space` | Bir tick ilerlet |
| `t` | Auto-sim aç/kapa (300ms tick) |
| `?` / `h` | Yardım (tüm komutlar) |
| `i` | Nasıl oynanır |
| **`b`/`s`** | **Buy / Sell wizard** — pazar paneli (`bid`/`ask`/`last`) + `M` tuşu marketable fiyat + canlı kâr/maliyet hesabı |
| `f` | BuildFactory (Sanayici) |
| `c` | BuyCaravan |
| `d` | Dispatch (kervan gönder) |
| `o` | Offer (kontrat öner) |
| `y` | Kontrat panel (Public liste + benim aktif/geçmiş) |
| `r` | Pazar verileri (sparkline + %delta) |
| **`L`** | **Leaderboard popup** (Sıra/Oyuncu/Rol/Nakit/Stok/PnL Δ) |
| **`F`** | **Reddedilenler overlay** (CommandRejected + FillRejected listesi) |
| `m` | Varlıklarım (envanter + fab + kervan + kontrat) |
| `n` | Haber kutusu |
| `e` | Son eşleşmeler (kim kime ne sattı) |
| `j` | NPC söylenti akışı (chatter) |
| `:` | Komut modu (gelişmiş) |
| `q` / `Esc` | Çık |

---

## 🌐 LAN multiplayer

İki kişi aynı ağda gerçek zamanlı oynayabilir.

```bash
# Bir kişi (host):
./mw server                    # default port 7878

# Diğer kişi (her client kendi terminalinde):
./mw connect <host-ip>:7878 Ali
./mw connect <host-ip>:7878 Veli
```

**Lobide:** `t` Tüccar, `s` Sanayici, `r` ready toggle. Herkes Ready basınca oyun başlar.

> **Not:** Multiplayer şu an stdout-mode (TUI yok). Sprint 4'te ratatui'ye entegre edilecek. Tek-oyunculuda zengin TUI mevcut.

**LAN dışından:** [Tailscale](https://tailscale.com), [ZeroTier](https://www.zerotier.com), [Hamachi](https://vpn.net) gibi virtual LAN aracı (5 dk setup, NAT/router sorunu yok).

---

## 📦 Proje yapısı

```
MoneyWar/
├── crates/
│   ├── moneywar-domain/   # Saf veri tipleri (Money, Player, GameState, Command, ...)
│   ├── moneywar-engine/   # Tick motoru — advance_tick + batch auction + tâtonnement
│   ├── moneywar-npc/      # NPC davranışları (behavior/ + difficulty + 7 kişilik arketipi)
│   ├── moneywar-net/      # LAN protokolü (postcard wire format)
│   ├── moneywar-server/   # Tokio TCP authoritative server (lobby + game loop)
│   ├── moneywar-cli/      # Terminal TUI (ratatui) + LAN client
│   └── moneywar-sim/      # Headless deterministic simulation runner
├── docs/                  # Game design + architecture + API
├── mw                     # Kısayol scripti (./mw help)
└── .github/workflows/     # CI (fmt + clippy + test) + release
```

**Katman kuralları:**
- `domain` hiçbir crate'e bağlı değil.
- `engine` sadece `domain` — tokio/I/O yok, saf fonksiyonlar, deterministik.
- `npc` domain + engine.
- `net` domain + serde + postcard.
- `server` hepsi + tokio + tracing.
- `cli` domain + engine + npc + net.

---

## 🧬 Ekonomi mekaniği (v0.4)

### Order-book aware pricing
NPC'ler `state.best_bid`/`best_ask` üzerinden **marketable** fiyat hesaplar. Her rolün kendi `CrossPolicy`'si var:

| Rol | Davranış |
|---|---|
| **Çiftçi** | CROSS at stok>500 (acil eritme), pasif altı |
| **Alıcı** | CROSS always (tüketici talep) |
| **Sanayici** | CROSS fab var ise (ham açlığı) / PASSIVE değilse |
| **Tüccar** | CROSS arbitraj kâr eşiği aşıldığında |
| **Spek** | (v0.4.0'da emekli) |

### Patience erosion + season drift
Bir bucket'ta art arda match olmayan player'ın `floor`'u düşer / `ceiling`'i yükselir. 15 tick streak + sezon ilerleme = max %30 yumuşama → **deadlock-proof**.

### Walras tâtonnement (asimetrik)
Her clearing sonunda `price_baseline` talep/arz dengesine göre kayar:
- Yukarı: `+%0.2/tick` (yavaş — talep birikim)
- Aşağı: `-%1.0/tick` (hızlı — arz fazlası, panik düşüş)
- Stok-bazlı drift: bucket stok > threshold ise ek `-%0.3..%0.7/tick`
- Initial × `[%60, %160]` clamp (runaway durdurma)

### Şok event'leri
Sezon ilerlemesine göre artar (early/mid/late = `%12/%18/%28`). 4 tip:
- **Drought** — ham fiyat ↑
- **Strike** — arz ↓ → ham fiyat ↑
- **BumperHarvest** — Çiftçi'lere ek stok → fiyat ↓
- **NewMarket** — mamul talep patlaması → fiyat ↑

### Kontrat sistemi
İki yönlü kontrat altyapısı (Tüccar ham, Sanayici mamul). v0.4.0'da NPC propose şu an kapalı (engine stok escrow eksik) — sadece insan oyuncu wizard `o` ile öner. NPC accept tarafı aktif.

### Difficulty parametreleri
| Mode | top_k | min_score | softener_pct |
|---|---|---|---|
| Easy | 8 | 0.05 | %15 (NPC cömert) |
| Medium | 6 | 0.10 | %5 |
| Hard | 12 | 0.0 | %0 |

---

## 🎯 Faz durumu

| Faz | Konu | Durum |
|---|---|---|
| 0–8 | Workspace, domain, engine, NPC, kontrat, kredi, haber, skor | ✅ |
| CLI playtest | ratatui TUI + komut sistemi | ✅ |
| **Ekonomi v2** | Order-book pricing + tâtonnement + 2-yönlü kontrat | ✅ (v0.4.0) |
| **MP Sprint 0–3** | Net protokol + TCP server + lobby + tick loop | ✅ |
| MP Sprint 4 | TUI integration + state_hash + reconnect | ⏳ |
| 10 | PostgreSQL persistence + kariyer | ⏳ |
| 11 | Frontend (TS/React) | ⏳ |
| 12 | Polish + E2E smoke | ⏳ |

---

## ⚖️ Balance tuning

Oyun dengesi tek dosyada: [`crates/moneywar-domain/src/balance.rs`](crates/moneywar-domain/src/balance.rs).

Her parametre `pub const`:
```rust
pub const SATURATION_BASE: u32 = 250;
pub const EVENT_PROB_EARLY_PCT: u32 = 12;
pub const FACTORY_SCORE_NUM: i64 = 3;  // %75
pub const FACTORY_SCORE_DEN: i64 = 4;
pub const CARAVAN_CAPACITY_SANAYICI: u32 = 500;
pub const CARAVAN_CAPACITY_TUCCAR: u32 = 1200;
```

**Workflow:**
1. `balance.rs`'i aç, değeri değiştir.
2. `./mw test` — invariantlar / integration testler geçiyor mu?
3. `cargo run --bin sim --release` — headless 90 tick PnL özeti.
4. `./mw solo` — TUI'de tam sezon izle.
5. Commit.

---

## 🔬 Deterministik motor

`advance_tick(state, commands) → (new_state, report)` saf fonksiyon. RNG `(room_id, tick)`'ten türetilir — aynı input, bit-perfect aynı output. LAN multiplayer authoritative server bu sayede çalışıyor.

**Test komutları:**

```bash
./mw test              # cargo test --workspace (450+ test)
./mw check             # fmt --check + clippy -D warnings
cargo doc --workspace --open    # HTML API docs

# Headless ekonomi simülasyonu (90 tick Hard, PnL + price discovery rapor):
cargo run --bin sim --release
```

---

## 🔄 Tick lifecycle

Her `advance_tick` çağrısında sırasıyla:

1. **Dispatch** — oyuncu + NPC komutları (`SubmitOrder`, `BuildFactory`, `ProposeContract`, ...)
2. **Events** — RNG ile şok tetikle (Drought/Strike/BumperHarvest/NewMarket)
3. **Production** — biten batch'ler envantere
4. **Transport** — kervanlar hedef envanterlere
5. **Contracts** — `delivery_tick`'i gelenler fulfill/breach
6. **Loans** — auto-settle
7. **Economy tick** — Çiftçi mahsul + Alıcı tüketim (player offset ile dağılmış), wages, maintenance, World fab
8. **Clearing** — pay-as-bid continuous matching + tâtonnement baseline güncelleme + patience erosion

---

## 📋 Sürüm notları (v0.4.0-pre1)

Ana yenilikler:

**Ekonomi**
- Order-book aware pricing: `marketable_ask`/`bid` + `CrossPolicy`
- Patience erosion (`no_match_streak`) + season drift
- Walras tâtonnement (asimetrik %0.2↑/%1.0↓)
- Tâtonnement initial × [%60, %160] clamp
- Stok-bazlı baseline aşağı drift

**NPC**
- Spek emekli (kompozisyon 0)
- Sanayici kervan kapasitesi 200 → 500
- Sanayici fab scoring %50 → %75
- Periyodik consume/harvest player offset (ritim dağılımı)
- Easy mode redesign: bol likidite + %15 fiyat softener

**Kontrat**
- 2-yönlü kontrat altyapısı (Tüccar ham, Sanayici mamul)
- NPC propose şu an kapalı (stok escrow eksik) — sadece insan
- NPC accept aktif

**TUI**
- Leaderboard popup overlay (`L` tuşu)
- Reddedilenler overlay (`F` tuşu)
- Wizard pre-flight engel kontrolü
- Pazar paneli: bid/ask/last + `M` marketable
- Sparkline yanına gerçek %delta
- Sezon sonu reveal yenilendi (Sıra/Rol/Nakit/Stok/PnL Δ)

---

## 📜 Lisans

UNLICENSED (v1 — dev aşaması).
