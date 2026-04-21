# MoneyWar — API Cheat Sheet

Tek bakışta her crate'in public API'si. `cargo doc --open` ile renkli HTML olarak da gezilebilir.

---

## 🎛️ Balance — oyun dengesi

**Dosya:** [`crates/moneywar-domain/src/balance.rs`](../crates/moneywar-domain/src/balance.rs)

Tüm tunable sayısal parametreler tek yerde `pub const`. Bir değeri değiştirip `cargo build` ile yeniden derleyince motor yeni denge ile çalışır.

Bölümler:
- **Zaman:** `FACTORY_BATCH_SIZE`, `FACTORY_PRODUCTION_TICKS`
- **Fabrika:** `FACTORY_BUILD_COSTS_LIRA`
- **Kervan:** `CARAVAN_CAPACITY_*`, `CARAVAN_COSTS_*_LIRA`
- **Piyasa:** `SATURATION_BASE`, `SATURATION_PER_PLAYER`
- **Haber:** `NEWS_COST_*_LIRA`, `NEWS_LEAD_*`
- **Olay:** `EVENT_LEAD_TICKS`, `EVENT_PROB_*_PCT`, `SHOCK_*_PCT`
- **Kredi:** `LOAN_INTEREST_RATE_PERCENT`
- **Skor:** `IDLE_FACTORY_THRESHOLD`, `PRICE_WINDOW`, `FACTORY_SCORE_{NUM,DEN}`
- **Mesafe:** `DIST_ISTANBUL_ANKARA`, `DIST_ANKARA_IZMIR`, `DIST_ISTANBUL_IZMIR`
- **Bozulma:** `PERISH_*_TICKS`, `PERISH_*_LOSS_PCT`
- **NPC:** `NPC_BASE_PRICE_*_LIRA`, `NPC_{SELL_MARKUP,BUY_MARKDOWN}_PCT`

---

## 📦 `moneywar-domain` — saf veri modeli

**Public API:** [`crates/moneywar-domain/src/lib.rs`](../crates/moneywar-domain/src/lib.rs)

### Tipler

| Tip | Dosya | Açıklama |
|---|---|---|
| `Money` | `money.rs` | `i64` cent, overflow-safe aritmetik |
| `Tick`, `SeasonProgress` | `time.rs` | Oyun içi zaman |
| `PlayerId`, `RoomId`, `OrderId`, `ContractId`, `FactoryId`, `CaravanId`, `NewsId`, `EventId`, `LoanId` | `ids.rs` | Newtype ID'ler |
| `CityId`, `DemandLevel` | `city.rs` | 3 şehir + talep profili |
| `ProductKind`, `ProductClass`, `Perishability` | `product.rs` | 6 ürün, ham/bitmiş, bozulma |
| `Role`, `Player`, `Inventory` | `player.rs` | Oyuncu + envanter |
| `Factory`, `FactoryBatch` | `factory.rs` | Fabrika + üretim kuyruğu |
| `Caravan`, `CaravanState`, `Cargo`, `CargoSpec` | `caravan.rs` | Kervan + kargo |
| `MarketOrder`, `OrderSide` | `order.rs` | Hal Pazarı emri |
| `Contract`, `ContractState`, `ContractProposal`, `ListingKind` | `contract.rs` | Anlaşma Masası |
| `Loan` | `loan.rs` | NPC kredi |
| `NewsTier`, `NewsItem` | `news.rs` | Haber sistemi |
| `GameEvent`, `EventSeverity` | `event.rs` | Olay tipleri |
| `RoomConfig`, `Preset` | `config.rs` | Oda ayarları |
| `GameState`, `IdCounters` | `state.rs` | Oyun durumu kökü |
| `Command` | `command.rs` | Oyuncunun motoru yönlendirdiği komutlar |
| `DomainError` | `error.rs` | Validation/overflow/insufficient errors |

### Komutlar (`Command` enum'u)

Oyuncunun motora gönderebileceği tüm aksiyonlar:

| Variant | Amacı |
|---|---|
| `SubmitOrder(MarketOrder)` | Hal Pazarı'na limit emri |
| `CancelOrder { order_id, requester }` | Emri geri çek |
| `ProposeContract(ContractProposal)` | Kontrat önerisi |
| `AcceptContract { contract_id, acceptor }` | Kontratı kabul et |
| `CancelContractProposal { contract_id, requester }` | Öneriyi geri çek |
| `BuildFactory { owner, city, product }` | Fabrika kur (Sanayici tekeli) |
| `BuyCaravan { owner, starting_city }` | Kervan al |
| `DispatchCaravan { caravan_id, from, to, cargo }` | Kervanı yola çıkar |
| `SubscribeNews { player, tier }` | Haber aboneliğini değiştir |
| `TakeLoan { player, amount, duration_ticks }` | NPC bankasından kredi al |
| `RepayLoan { player, loan_id }` | Krediyi manuel öde |

---

## ⚙️ `moneywar-engine` — tick motoru

**Public API:** [`crates/moneywar-engine/src/lib.rs`](../crates/moneywar-engine/src/lib.rs)

### Ana giriş noktası

```rust
pub fn advance_tick(
    state: &GameState,
    commands: &[Command],
) -> Result<(GameState, TickReport), EngineError>
```

Saf fonksiyon: state + komutlar → yeni state + rapor. Determinism: aynı input → aynı output (RNG `room_id + tick`'ten türetilir).

**Tick lifecycle** (`tick.rs` içinde):

1. Komut dispatch
2. `events::advance_events` — RNG ile olay tetikle + haber dağıt
3. `production::advance_production` — batch tamamla + yeni batch başlat
4. `transport::advance_caravans` — varışta cargo boşalt
5. `contracts::advance_contracts` — delivery_tick fulfill/breach
6. `loans::advance_loans` — vadesi gelen auto-settle
7. `market::clear_markets` — batch auction + settlement

### Public API

| Öğe | Dosya | Açıklama |
|---|---|---|
| `advance_tick` | `tick.rs` | Motor girişi |
| `TickReport`, `LogEntry`, `LogEvent` | `report.rs` | Analitik log |
| `EngineError` | `error.rs` | Motor hataları |
| `seed_for`, `rng_for` | `rng.rs` | Deterministik `ChaCha8Rng` |
| `PlayerScore`, `score_player`, `leaderboard` | `scoring.rs` | §9 skor formülü |
| `IDLE_FACTORY_THRESHOLD`, `PRICE_WINDOW` | `scoring.rs` | Skor sabitleri (balance'tan) |

### `LogEvent` variant'ları

Analitik/audit için motor tarafından emit edilen semantic event'ler:

- `CommandAccepted`, `CommandRejected` — raw komut logu
- `OrderMatched`, `MarketCleared`, `FillRejected` — piyasa
- `FactoryBuilt`, `ProductionStarted`, `ProductionCompleted`, `FactoryIdle` — üretim
- `CaravanBought`, `CaravanDispatched`, `CaravanArrived` — taşıma
- `ContractProposed`, `ContractAccepted`, `ContractCancelled`, `ContractSettled` — kontrat
- `LoanTaken`, `LoanRepaid`, `LoanDefaulted` — kredi
- `NewsSubscribed`, `EventScheduled` — haber + olay

Serde tag: `{"kind": "snake_case_variant", ...}` — DB indexing için stabil.

---

## 🤖 `moneywar-npc` — NPC davranışları

**Public API:** [`crates/moneywar-npc/src/lib.rs`](../crates/moneywar-npc/src/lib.rs)

```rust
pub trait NpcBehavior {
    fn decide(&self, state: &GameState, self_id: PlayerId, rng: &mut ChaCha8Rng, tick: Tick) -> Vec<Command>;
}

pub struct MarketMaker;  // v1 iskelet impl

pub fn decide_all_npcs(state: &GameState, rng: &mut ChaCha8Rng, tick: Tick) -> Vec<Command>;
```

Persona-genişletilebilir: `SanayiciNpc`, `TuccarNpc`, `AggressiveSpeculator` gelecek fazlarda eklenecek.

---

## 🖥️ `moneywar-cli` — terminal TUI

**Çalıştırma:** `cargo run -p moneywar-cli`

Tuşlar: `Space` = bir tick, `s` = auto-sim, `q` = çık.

Panellar: Oyuncu (skor+stok), Pazar (fiyat+Δ), Haber (tier), Son Tick, Leaderboard.

---

## 🧪 Test çalıştırma

```bash
cargo test --workspace            # tüm testler
cargo test -p moneywar-engine     # tek crate
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo doc --workspace --open      # tüm public API HTML olarak
```

---

## 🎮 Balance workflow

1. [`balance.rs`](../crates/moneywar-domain/src/balance.rs)'i aç
2. İstediğin değeri değiştir (ör. `LOAN_INTEREST_RATE_PERCENT = 20`)
3. `cargo test --workspace` — invariantlar geçiyor mu?
4. `cargo run -p moneywar-cli` — canlı sezon izle
5. Mutlu musun? Commit.
