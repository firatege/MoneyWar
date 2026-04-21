# MoneyWar — Mimari ve Faz Planı

> Oyun tasarım kararları: [`game-design.md`](./game-design.md)
> Bu dosya: crate iskeleti + mimari kararlar + faz sırası.

---

## 1. Onaylanan Mimari Kararlar (2026-04-21)

| Karar | Seçim | Gerekçe |
|---|---|---|
| Workspace yapısı | 5 crate (domain / engine / npc / server / cli) | Test izolasyonu + derleme hızı + katman net |
| Engine saflığı | Saf fonksiyon, `tokio` yok, I/O yok | Determinism → property test + replay |
| Sayı tipi | `Money(i64)` cent cinsinden | Float hassasiyet sorunu yok |
| Koleksiyon | `BTreeMap` (HashMap yasak) | Deterministik iterasyon |
| RNG | `rand_chacha::ChaCha8Rng`, seed = hash(tick, room) | Seed'den türeyen deterministik rastgelelik |
| Frontend | TypeScript + React + `ts-rs` | Ekosistem baskın, tip duplikasyonu sıfır |
| Persistence | PostgreSQL (sqlx) başından | v2 migration derdi yok |
| Real-time | WebSocket (axum + tokio-tungstenite) | Reveal anı push gerektirir |
| NPC borç | v1'de var (Faz 5.5) | Kaldıraç iştahı v1'den itibaren |

---

## 2. Crate İskeleti

```
MoneyWar/
├── Cargo.toml                    # workspace manifesti (pure virtual)
├── rustfmt.toml
├── .cargo/config.toml            # cargo alias'ları
├── .github/workflows/ci.yml      # fmt + clippy + test
├── crates/
│   ├── moneywar-domain/          # saf veri tipleri
│   ├── moneywar-engine/          # saf tick motoru
│   ├── moneywar-npc/             # NPC davranışları
│   ├── moneywar-server/          # HTTP/WS server (axum), persistence
│   └── moneywar-cli/             # terminal simülasyon (motor playtest)
├── web/                          # TS + React frontend (Faz 11+)
└── docs/
    ├── game-design.md
    └── architecture.md
```

### Katman kuralları

- `domain` → hiçbir crate'e bağlı değil
- `engine` → sadece `domain`'e bağlı (tokio yasak)
- `npc` → `domain` + `engine`
- `server` → `domain` + `engine` + `npc` + tokio/axum/sqlx
- `cli` → `domain` + `engine` + `npc` (server'a dokunmaz)

---

## 3. Faz Sırası (v1 iskelet)

| Faz | Konu | Tahmini |
|---|---|---|
| 0 | Workspace setup | 0.5 gün |
| 1 | Domain tipleri + RoomConfig | 1.5 gün |
| 2 | Saf tick motoru iskeleti | 1.5 gün |
| 3 | Hal Pazarı batch auction | 2 gün |
| 4 | Üretim + Taşıma | 1.5 gün |
| 5 | Kontrat + Escrow | 1.5 gün |
| 5.5 | NPC borç (banka kredi) | 0.5 gün |
| 6 | Haber + Olaylar | 1 gün |
| 7 | Skor + Leaderboard | 1 gün |
| 8 | NPC iskeleti (basit davranış) | 1 gün |
| 9 | Server + oda yönetimi | 2 gün |
| 10 | PostgreSQL persistence + kariyer | 1.5 gün |
| 11 | Frontend minimum (TS/React) | 3 gün |
| 12 | Polish + E2E smoke | 1 gün |
| **Toplam** |  | **~19.5 gün** |

### Ara milestone

**Faz 8 sonu** = CLI'dan oyun oynanabilir (frontend beklemeden motoru playtest edebiliriz).

---

## 4. Test Stratejisi

- **Unit:** `#[cfg(test)]` modülleri, AAA pattern, `rstest` parametreli
- **Property-based:** `proptest` ile engine invariantları (para korunumu, determinism, escrow non-negatif)
- **Integration:** `tests/` dizini (server REST/WS, full-season simülasyon)
- **E2E:** Playwright (Faz 11+)
- **Coverage:** `cargo-llvm-cov`, hedef %80+ (domain %90+, engine %85+)

### Engine invariant listesi (proptest)

1. Para korunumu: `Σ(cash + escrow + bank) == sabit` (yakma hariç, yakma explicit)
2. Stok korunumu: `Σ(product_total) == produced - decayed`
3. Determinism: aynı input → bin kez aynı output
4. Saturation monotonluk: eşik üstünde fiyat ≤ eşik altı
5. Atıl fabrika: 10 tick üretimsiz iff skora 0 katkı
6. Escrow non-negatif: aktif kontrat kaporası hiç eksi değil
7. Kervan varış: gönderilen kervan X tick sonra kesin varır (road closure hariç)

---

## 5. Kritik Riskler + Mitigasyon

| Risk | Mitigasyon |
|---|---|
| Batch auction tie-break non-determinism | Seed'den türetilmiş RNG + stabil sort (id + timestamp) |
| Float fiyat hassasiyeti | `Money(i64)` cent, float yasak |
| HashMap iterasyon non-determinism | `BTreeMap` zorla, kod review'da yakalanır |
| Escrow race (iki kabul aynı tick) | Sıralı işlem, tie-break seed'den |
| Snapshot corrupt | Atomic rename, komut log replay |
| Engine'e I/O sızar | `engine` crate `tokio` almaz, `lib.rs` docstring'de belirtilir |
| Tokio timer drift | `MissedTickBehavior::Delay`, tick sayacı sistem saatinden bağımsız |

---

## 6. Geliştirme Komutları

```bash
cargo chk              # cargo check --workspace --all-targets
cargo lint             # clippy -D warnings
cargo fmt-check        # fmt check (CI ile aynı)
cargo t                # test --workspace
cargo lint-pedantic    # pedantic warning'ler (opsiyonel disiplin)
```

---

## 7. Sürüm Hedefleri

- **v1 skeleton (~19 gün):** Faz 0→12, oyun oynanabilir, 2 meslek + NPC borç
- **v1.1:** Gelişmiş NPC kişilikleri, onboarding flow, UI polish
- **v2:** Spekülatör + Banker + Kartel rolleri, türev piyasası, fabrika seviye
- **v3:** Sandbox modu (50-100 oyuncu tek dünya)
