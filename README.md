# 💰 MoneyWar

Tick-tabanlı ekonomi simülasyon oyunu. **Rust workspace** olarak yazıldı; saf motor + terminal TUI + LAN multiplayer.

> Oyun tasarım dokümanı: [`docs/game-design.md`](docs/game-design.md)
> Mimari plan: [`docs/architecture.md`](docs/architecture.md)
> Public API cheat-sheet: [`docs/API.md`](docs/API.md)

---

## 🎮 Oyunu dene

### Seçenek 1 — Hazır binary (Rust gerekmez)

[GitHub Releases](https://github.com/firatege/MoneyWar/releases) sayfasından indir, aç, çalıştır:

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

`./mw` proje köküne eklenmiş kısayol scripti. Uzun `cargo run -p ...` komutlarını saklar:

```bash
./mw help            # tüm komutlar
./mw solo            # tek-oyunculu TUI
./mw server          # LAN sunucu (port 7878)
./mw connect 192.168.1.42:7878 Ali   # LAN client
./mw build           # release build (binary'ler target/release/'e)
./mw test            # cargo test --workspace
./mw check           # fmt + clippy (CI ile aynı)
```

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

**Oyunda (stdin komutları):**

| Komut | İşlev |
|---|---|
| `i` | Durumum (cash + envanter) |
| `l` | Skor tablosu |
| `b ist pamuk 50 6.0` | BUY 50 pamuk @ 6₺ İstanbul |
| `s ank kumas 30 18.0` | SELL 30 kumaş @ 18₺ Ankara |
| `f ist kumas` | BuildFactory (Sanayici) |
| `c izm` | BuyCaravan, İzmir'den başla |
| `?` | Yardım |
| `q` | Çık |

Şehirler: `ist`/`izm`/`ank`. Ürünler: `pamuk`/`bugday`/`zeytin`/`kumas`/`un`/`yag`.

> **Not:** Multiplayer şu an stdout-mode (TUI yok). Sprint 4'te ratatui'ye entegre edilecek. Tek-oyunculuda zengin TUI mevcut — `./mw solo`.

**LAN dışından oynamak için:** [Tailscale](https://tailscale.com), [ZeroTier](https://www.zerotier.com) veya [Hamachi](https://vpn.net) gibi virtual LAN aracı kurun (5 dakikalık setup, NAT/router sorunu yok).

---

## 🛠 Tek-oyunculu CLI

`./mw solo` ile aç. Rol seç (`1` Sanayici / `2` Tüccar). İçeride:

| Tuş | İşlev |
|---|---|
| `Space` | Bir tick ilerlet |
| `t` | Auto-sim aç/kapa (300ms tick) |
| `?` / `h` | Yardım (tüm komutlar) |
| `i` | Nasıl oynanır |
| `b/s/f/c/d/l/o/a/y/...` | Tek-tuş wizard'lar (Buy/Sell/Build/Caravan/Dispatch/Loan/Offer/Accept/...) |
| `:` | Komut modu (gelişmiş) |
| `q` / `Esc` | Çık |

---

## Proje yapısı

```
MoneyWar/
├── crates/
│   ├── moneywar-domain/   # Saf veri tipleri (Money, Player, GameState, Command, ...)
│   ├── moneywar-engine/   # Tick motoru — advance_tick pure fn + batch auction
│   ├── moneywar-npc/      # NPC davranışları (DSS + 7 kişilik arketipi)
│   ├── moneywar-net/      # LAN protokolü (postcard wire format, ClientMsg/ServerMsg)
│   ├── moneywar-server/   # Tokio TCP authoritative server (lobby + game loop)
│   └── moneywar-cli/      # Terminal TUI (ratatui) + LAN client
├── docs/
│   ├── game-design.md     # Oyun tasarım kararları (§0-§12)
│   ├── architecture.md    # Mimari + faz planı
│   └── API.md             # Public API cheat-sheet
├── mw                     # Kısayol scripti (./mw help)
└── .github/workflows/     # CI (fmt + clippy + test) + release (4 platform binary)
```

**Katman kuralları:**
- `domain` hiçbir crate'e bağlı değil.
- `engine` sadece `domain` — tokio/I/O yok, saf fonksiyonlar, deterministik.
- `npc` domain + engine.
- `net` domain + serde + postcard.
- `server` hepsi + tokio + tracing.
- `cli` domain + engine + npc + net (TUI tarafı server'a dokunmaz).

---

## Faz durumu

| Faz | Konu | Durum |
|---|---|---|
| 0–8 | Workspace, domain, engine, NPC, kontrat, kredi, haber, skor | ✅ |
| CLI playtest | ratatui TUI + komut sistemi | ✅ |
| **MP Sprint 0** | `moneywar-net` protokol iskeleti | ✅ |
| **MP Sprint 1** | TCP server + `--connect` heartbeat | ✅ |
| **MP Sprint 2** | Lobby + GameStart broadcast | ✅ |
| **MP Sprint 3** | Tick döngüsü + emir gönderme (gerçek MP) | ✅ |
| MP Sprint 4 | TUI integration + state_hash + reconnect | ⏳ |
| 10 | PostgreSQL persistence + kariyer | ⏳ |
| 11 | Frontend (TS/React) | ⏳ |
| 12 | Polish + E2E smoke | ⏳ |

---

## Balance tuning

Oyun dengesi tek dosyada: [`crates/moneywar-domain/src/balance.rs`](crates/moneywar-domain/src/balance.rs).

Her parametre `pub const`. Örn:
```rust
pub const FACTORY_BATCH_SIZE: u32 = 100;
pub const FACTORY_BUILD_COSTS_LIRA: [i64; 5] = [0, 15_000, 25_000, 40_000, 60_000];
pub const SATURATION_BASE: u32 = 250;
```

**Workflow:**
1. `balance.rs`'i aç, değeri değiştir.
2. `./mw test` — invariantlar / integration testler geçiyor mu?
3. `./mw solo` — tam sezon izle, leaderboard'a bak.
4. Commit.

---

## Deterministik motor

`advance_tick(state, commands) → (new_state, report)` saf fonksiyon. RNG `(room_id, tick)`'ten türetilir — aynı input, bit-perfect aynı output. LAN multiplayer authoritative server bu sayede çalışıyor: server `advance_tick`, client'lar mirror.

**Test komutları:**

```bash
./mw test              # cargo test --workspace --all-targets (446+ test)
./mw check             # fmt --check + clippy -D warnings (CI ile aynı)
cargo doc --workspace --open    # HTML API docs
```

---

## Tick lifecycle

Her `advance_tick` çağrısında sırasıyla:

1. **Dispatch** — oyuncu + NPC komutları işlenir (`SubmitOrder`, `BuildFactory`, vb).
2. **Events** — RNG ile olay tetikle, abonelere haber dağıt.
3. **Production** — biten batch'ler envantere, yeni batch başlat.
4. **Transport** — varış zamanı gelen kervanlar hedef envanterlere boşalır.
5. **Contracts** — `delivery_tick`'i gelenler fulfill/breach.
6. **Loans** — vadesi gelen krediler auto-settle.
7. **Clearing** — Hal Pazarı uniform batch auction + settlement + price history.

---

## Lisans

UNLICENSED (v1 — dev aşaması).
