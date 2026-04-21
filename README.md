# 💰 MoneyWar

Tick-tabanlı ekonomi simülasyon oyunu. **Rust workspace** olarak yazıldı; saf motor + terminal TUI.

> Oyun tasarım dokümanı: [`docs/game-design.md`](docs/game-design.md)
> Mimari plan: [`docs/architecture.md`](docs/architecture.md)
> Public API cheat-sheet: [`docs/API.md`](docs/API.md)

---

## Hızlı başlangıç

```bash
# Test + build
cargo test --workspace

# CLI'ı aç (renkli TUI, tek ekran)
cargo run -p moneywar-cli
```

İçeri girince **rol seç** (`1` Sanayici / `2` Tüccar), oyunu başlat. `?` ile komutlar, `i` ile nasıl oynanır açılır.

---

## Proje yapısı

```
MoneyWar/
├── crates/
│   ├── moneywar-domain/   # Saf veri tipleri (Money, Player, GameState, Command, ...)
│   ├── moneywar-engine/   # Tick motoru — advance_tick pure fn + batch auction
│   ├── moneywar-npc/      # NPC davranışları (NpcBehavior trait + MarketMaker)
│   ├── moneywar-server/   # (Faz 9) axum + WebSocket
│   └── moneywar-cli/      # Terminal TUI (ratatui + crossterm)
├── docs/
│   ├── game-design.md     # Oyun tasarım kararları (§0-§12)
│   ├── architecture.md    # Mimari + faz planı
│   └── API.md             # Public API cheat-sheet
└── .github/workflows/     # CI (fmt + clippy + test)
```

**Katman kuralları:**
- `domain` hiçbir crate'e bağlı değil.
- `engine` sadece `domain` — tokio/I/O yok, saf fonksiyonlar, deterministik.
- `npc` domain + engine.
- `server` hepsi + tokio/axum/sqlx.
- `cli` domain + engine + npc (server'a dokunmaz).

---

## Faz durumu

| Faz | Konu | Durum |
|---|---|---|
| 0 | Workspace setup | ✅ |
| 1 | Domain tipleri | ✅ |
| 2 | Saf tick motoru iskeleti | ✅ |
| 3A | Order book submit/cancel | ✅ |
| 3B | Batch auction matching | ✅ |
| 3C | Settlement + saturation + price history | ✅ |
| 4A | Fabrika + üretim pass | ✅ |
| 4B | Kervan + taşıma pass | ✅ |
| 5 | Kontrat + escrow + settlement | ✅ |
| 5.5 | NPC banka kredi + auto-default | ✅ |
| 6 | Haber abonelik + RNG olay motoru | ✅ |
| 7 | Skor + leaderboard (§9) | ✅ |
| 8 | NPC iskeleti (MarketMaker) | ✅ |
| **CLI playtest** | ratatui TUI + komut sistemi | ✅ |
| 9 | Server + oda yönetimi (axum + WS) | ⏳ |
| 10 | PostgreSQL persistence + kariyer | ⏳ |
| 11 | Frontend (TS/React + ts-rs) | ⏳ |
| 12 | Polish + E2E smoke | ⏳ |

---

## Balance tuning

Oyun dengesi tek dosyada: [`crates/moneywar-domain/src/balance.rs`](crates/moneywar-domain/src/balance.rs).

Her parametre `pub const`. Örn:
```rust
pub const LOAN_INTEREST_RATE_PERCENT: u32 = 15;
pub const FACTORY_BUILD_COSTS_LIRA: [i64; 5] = [0, 10_000, 15_000, 22_000, 30_000];
pub const EVENT_PROB_LATE_PCT: u32 = 20;
```

**Workflow:**
1. `balance.rs`'i aç, değeri değiştir.
2. `cargo test --workspace` — invariantlar / integration testler geçiyor mu?
3. `cargo run -p moneywar-cli` — tam sezon izle, leaderboard'a bak.
4. Commit.

---

## CLI kullanımı

**Tuşlar:**

| Tuş | İşlev |
|---|---|
| `Space` | Bir tick ilerlet (bekleyen komutlar + NPC'ler) |
| `s` | Auto-sim aç/kapa |
| `:` | Komut modu (metin yaz, Enter ile gönder) |
| `?` / `h` | Yardım (tüm komutlar) |
| `i` | Nasıl oynanır |
| `q` / `Esc` | Çık |

**Örnek komutlar:**

```
:buy istanbul pamuk 20 7           # 20 pamuk @7₺ alım emri
:sell istanbul kumas 10 18         # satım emri
:build istanbul kumas              # kumaş fabrikası kur (Sanayici)
:caravan istanbul                  # kervan al
:ship 1 istanbul ankara pamuk 20   # kervan #1 ile 20 pamuk Ankara'ya
:loan 10000 30                     # 10k₺ kredi, 30 tick vade
:news gold                         # altın abonelik
```

---

## Deterministik motor

`advance_tick(state, commands) → (new_state, report)` saf fonksiyon. RNG `(room_id, tick)`'ten türetilir — aynı input, bit-perfect aynı output. Replay + property testler buna dayanıyor.

**Test komutları:**

```bash
cargo test --workspace
cargo test -p moneywar-engine        # motor unit + proptest
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo doc --workspace --open         # HTML API docs
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
