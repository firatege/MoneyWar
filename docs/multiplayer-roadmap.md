# MoneyWar LAN MP — v0.1.5+ Roadmap

> Hazırlandığı tarih: 2026-04-28
> Mevcut sürüm: v0.1.5 (Sprint 0-4 tamam, MP TUI çalışıyor)

İki direkt sorulan özelliği cevaplayıp, sonra "yapabileceklerimiz" listesi. Karar matrisi + sprint kırılımı + açık karar soruları sonda.

---

## 1. Doğrudan istenen iki özellik

### 1a. Zorluk seçimi (Easy / Hard / Expert)

**Mimari özet.** Şu an `crates/moneywar-server/src/game.rs` içinde `const SERVER_DIFFICULTY: Difficulty = Difficulty::Hard;` sabit. Bunu lobi config'ine taşıyıp host'un seçmesi gerekiyor. Engine deterministik olduğu için NPC kararları client'a state olarak yansır — protokol tarafında zorluk kelimesini iletmek bile gerekmiyor aslında, ama UI'da göstermek için lobi'ye eklemek mantıklı (oyuncu kime karşı oynadığını bilsin).

**Veri modeli değişiklikleri.**

- `Lobby` struct'ına `pub difficulty: Difficulty` ekle (`crates/moneywar-server/src/lobby.rs:30`).
- `ClientMessage::SetLobbyConfig { difficulty: Option<Difficulty>, season_length: Option<SeasonPreset>, npc_count: Option<u8> }` — tek mesaj, opsiyonel alanlarla. Sadece host'tan kabul (server `lobby.host == player_id` kontrolü). Postcard additive olduğu için `PROTOCOL_VERSION = 1` kalabilir.
- `ServerMessage::LobbyState`'e `difficulty: Difficulty` alanı `#[serde(default)]` ile.
- `GameStart` mesajına da difficulty eklensin — client UI başlık'ta gösterir ("Hard mode — 3 insan vs 9 NPC").

**Edge case'ler / riskler.**

- Mid-game değişmemeli — `lobby.game_started == true` ise `SetLobbyConfig` reddedilsin.
- Host disconnect → host migration zaten var (`remove_player` host devrediyor), yeni host config'i değiştirebilir.
- Default'u `Hard` bırak (mevcut deneyim) ama UI'da varsayılan olarak vurgulanmasın — host bilinçli seçsin.
- `Difficulty::Easy` için `npc/lib.rs`'te SmartTrader var ama agresiflik downgrade'i tam belirsizse: kâr çarpanı düşürülsün, üretim hedefleri %30 azaltılsın gibi konkre param-tweak gerekebilir. Eğer Easy ile Hard arasında davranış zaten yeterince ayrışıyorsa ekstra iş yok, sadece flag yetiyor.

**İş yükü.** ~3-4 saat.

**Öneri.** Yap, ilk sırada. Kullanıcı doğrudan istedi, riski sıfır, başka şeylere bağımlılık yok.

---

### 1b. Tick rewind / undo

İki ayrı mesele var — karıştırma:

**Mesele A: "Yanlış emir gönderdim, geri alayım" (komut iptali).**

Şu anki durum: emir submit edilir, server pending queue'ya alır, **tick advance olunca uygulanır**. Tick işlenmeden önceki pencerede iptal mümkün. `Command::CancelOrder { order_id }` zaten domain'de var; server-side: `pending_commands` queue'sundan o oyuncunun son N emrini çıkarmak için `ClientMessage::WithdrawPending { count: u8 }` ekle. Bu **rewind değil**, sadece henüz işlenmemiş niyeti silmek. Ucuz, MP-friendly, hiç kimsenin başına bir şey gelmiyor.

**İş yükü:** ~1 saat.

**Mesele B: Gerçek tick rewind (state'i geri sar).**

Mimari zor. Engine deterministik (iyi haber): `Vec<GameState>` snapshot tutarsan eski state'e dönmek `mem::replace` kadar kolay. Kötü haber: MP'de **bir oyuncu undo basarsa diğerlerinin kararları siliniyor** — bu bir "savaş suçu". Üç gerçek seçenek:

1. **Vote-required rewind.** Host `RequestRewind { ticks: 1..=5 }` yollar, tüm oyuncular kabul ederse server snapshot'a döner ve bunu `RewindApplied { new_tick }` ile broadcast eder. Adil ama UX yorucu — herkesin onaylaması lazım, lobide vote dialog'u patlıyor.
2. **Host-only emergency rewind.** Sadece host basabilir, ama bu sosyal sözleşmeye dayanıyor (host arkadaşsa OK, halka açık LAN'da değil).
3. **Hayır, rewind yok — sadece pause + son-tick raporu.** Pause'la zaten "panik anında dur" çözülüyor; raporda neyin neye dönüştüğünü göster ki bir dahaki tick için akıllı emir verilsin.

**Veri modeli (rewind yapılırsa).**

- Server tarafında `state_history: VecDeque<GameState>` (cap 10). Her tick advance'te `push_back`, `len > 10` ise `pop_front`.
- `ClientMessage::RequestRewind { ticks: u8 }` (host-only ya da vote'lu).
- `ServerMessage::RewindVoteOpen { initiator, ticks, deadline_tick }` + `ClientMessage::RewindVote { accept: bool }` + `ServerMessage::RewindApplied { state, new_tick }`.

**Riskler.**

- Snapshot bellek: 30KB × 10 = 300KB / oyun. Sorun değil.
- Rewind sırasında uçuşan emirler: server `pending_commands.clear()` + `advance_pending.clear()` zorunlu, yoksa state-tick mismatch olur.
- Eğlence riski: rewind oyunu uyutur. Arkadaşlar 5 dakikada bir "şunu geri alalım abi" der.

**İş yükü.**

- Sadece pending withdraw (Mesele A): ~1 saat.
- Host-only rewind (Mesele B basit form): ~5-6 saat.
- Vote-required rewind: ~2 gün (mesaj + UI vote dialog + edge case).

**Öneri.**

- **Şimdi: Mesele A'yı yap** (pending withdraw). Çünkü ucuz ve gerçekten faydalı.
- **Mesele B'yi yapma.** Yerine **Pause + "son tick raporu paneli"** koy — undo'nun verdiği "ne oldu burada?" hissini cover ediyor, MP etiğini bozmuyor.

---

## 2. Diğer önerilen özellikler

| # | Özellik | Karmaşıklık | Değer | Not |
|---|---------|----|---|---|
| 2 | **Pause/Resume** (host) | 2 | 4 | `ClientMessage::SetPaused`, sadece host. Auto loop'ta `tokio::select!` ile cancel branch, manual'de zaten kendiliğinden duruyor. Session tutuluyor. |
| 3 | **Save/Load** (sezon dump) | 2 | 4 | `GameState` zaten `Serialize` — `--save game.bin` ile postcard yaz, `--load game.bin` ile state'i lobby skip ederek yükle. Solo için killer feature, MP'de host kaydeder. |
| 4 | **Replay** (izleme) | 3 | 3 | Server her tick `Vec<Command>`'ı log'lar (`replay.bin`), `--replay` modu CLI'de tick tick simüle eder. Engine deterministik olduğu için seed + commands → state 1:1. |
| 5 | **Spectator mode** | 2 | 3 | `ClientMessage::JoinAsObserver`, `LobbySlot`'a `is_observer: bool`. Sadece state alır, komut yollayamaz. Stream/öğretmek için. |
| 6 | **Late-join (snapshot)** | 4 | 3 | Şu an `JoinError::GameAlreadyStarted` reddediyor. Yerine: yeni gelen Spectator olarak girer, ya da boş slot varsa direkt katılır. |
| 7 | **Chat** (in-game text) | 1 | 4 | `ClientMessage::Chat { text }` → broadcast. CLI'de `T` ile chat moduna gir, ratatui altta scroll panel. ~2 saat iş, sosyal değer büyük. |
| 8 | **Reconnect** (resume token) | 3 | 4 | `Welcome`'a `resume_token: u64` ekle, client kaydet. Disconnect olunca server slot'u 30sn `Disconnected { resume_token }` durumunda tutsun. |
| 9 | **Sezon uzunluğu config** | 1 | 3 | `SeasonPreset` zaten kodda (90/150/350). `SetLobbyConfig` mesajına eklendi mi tek satır iş. |
| 10 | **NPC sayısı config** | 1 | 3 | Aynı mesajda `npc_count: u8` (0-10). `world::build_initial_state` parametre alsın. |
| 11 | **state_hash desync detect** | 2 | 5 | Sprint 4'te zaten plandaydı. `blake3(postcard(state))` her tick. Olmazsa olmaz. |
| 12 | **Achievements / maç sonu özet** | 2 | 4 | Sezon bittiğinde `ServerMessage::SeasonEnded { summary: SeasonSummary }`. En kârlı emir, en büyük breach, lider grafiği. |
| 13 | **Custom RNG seed** | 1 | 2 | `SetLobbyConfig` mesajına `seed: Option<u64>`. Reproducible matchler — speedrun/turnuva. |
| 14 | **AI difficulty per-NPC (chaos mode)** | 2 | 2 | NPC'ler karışık seviyede. Eğlenceli ama gimmick. |
| 15 | **Adaptive difficulty** | 4 | 3 | İnsan lider olunca NPC'ler agresifleşsin. Sprint 4 plan'ında vardı. Büyük iş — V3'e ertele. |
| 16 | **Vote-kick** | 3 | 2 | AFK/troll için. LAN/arkadaş ortamında nadiren gerekir. |

---

## 3. Karar matrisi

| Özellik | Karmaşıklık | Değer | Öncelik |
|---|---|---|---|
| 1a. Zorluk seçimi (lobi config) | 2 | 5 | **NOW** |
| 7. Chat | 1 | 4 | **NOW** |
| 2. Pause/Resume | 2 | 4 | **NOW** |
| 1b-A. Pending withdraw (komut iptali) | 1 | 4 | **NOW** |
| 9+10+13. SetLobbyConfig kompleksi (sezon/NPC/seed) | 1+1+1 | 3+3+2 | **NOW** (1a ile aynı mesajda) |
| 3. Save/Load | 2 | 4 | NEXT |
| 8. Reconnect | 3 | 4 | NEXT |
| 11. state_hash desync | 2 | 5 | NEXT |
| 12. Maç sonu özet | 2 | 4 | NEXT |
| 5. Spectator | 2 | 3 | LATER |
| 6. Late-join (snapshot) | 4 | 3 | LATER |
| 4. Replay | 3 | 3 | LATER |
| 1b-B. Tick rewind (host-only / vote) | 4-5 | 2 | **SKIP** (Pause + maç sonu özetiyle ihtiyaç kalmıyor) |
| 14. Chaos mode NPC | 2 | 2 | LATER |
| 15. Adaptive difficulty | 4 | 3 | LATER (V3) |
| 16. Vote-kick | 3 | 2 | SKIP |

---

## 4. Sprint kırılımı

### Sprint 5 — "Lobi'ye söz hakkı" (3-4 gün)

Tek tema: host gerçekten host olsun. Hepsi tek `SetLobbyConfig` mesajı etrafında dönüyor.

**Bitiş kriteri:** 4 oyunculu LAN maçta host D tuşunda zorluk değiştirip Easy/Hard/Expert, sezon 90/150/350, NPC 4-10, seed sabit — hepsi seçebiliyor. Lobi UI'da herkes config'i görüyor. Game başlayınca config kilitleniyor.

**Yapılacaklar:**

1. `ClientMessage::SetLobbyConfig` (host-only) + `ServerMessage::LobbyState` extension
2. `Lobby` struct'a `LobbyConfig { difficulty, season, npc_count, seed }` alanı
3. Server-side: handler + `game_started` kontrolü + non-host reject
4. `world::build_initial_state` config'ten okuyacak (npc_count, seed, season_ticks)
5. `game::SERVER_DIFFICULTY` const'unu kaldır, lobby'den gelsin
6. CLI lobi ekranı: D=difficulty, S=season, N=npc_count cycle (host olmayan oyuncularda görüntü-only)
7. Tests: `select_role_resets_ready` benzeri `host_only_can_set_config`, `config_locked_after_game_start`

### Sprint 6 — "Sosyal + güvenlik ağı" (3-4 gün)

Chat + Pause + pending withdraw + maç sonu özet. Hepsi hafif, MP deneyimini büyük geliştiriyor.

**Bitiş kriteri:** Chat altta panelde scroll, host P ile pause/resume yapabiliyor, oyuncu son göndereceği emrini Backspace ile silebiliyor (henüz tick işlenmediyse), sezon bitince `SeasonSummary` ekran tüm oyuncularda aynı.

### Sprint 7 — "Kalıcılık + bütünlük" (1 hafta)

Save/Load + state_hash desync detect + Reconnect.

**Bitiş kriteri:** Host `--save match.bin` ile çıktığında dosya yazılıyor, `--load match.bin` ile sezon kaldığı yerden devam ediyor. Disconnect olan oyuncu 30sn içinde aynı slot'a geri dönüyor. Desync uyarısı log'a düşüyor.

### Sprint 8+ — Spectator, late-join, replay

Yetiştirilirse. v0.2 sürümüne yığ.

---

## 5. Sıradaki sprint için somut adım listesi (Sprint 5)

1. `crates/moneywar-net/src/lib.rs`: `LobbyConfig` struct ekle, `ClientMessage::SetLobbyConfig`, `LobbyState` mesajına `config: LobbyConfig` alanı, `GameStart`'a aynısı.
2. `crates/moneywar-server/src/lobby.rs`: `Lobby` struct'a `pub config: LobbyConfig` ve default impl. `set_config(&mut self, player_id, cfg) -> Result<(), ConfigError>` host kontrolüyle.
3. `crates/moneywar-server/src/game.rs`: `SERVER_DIFFICULTY` const'unu sil, `decide_all_npcs(&snapshot, &mut npc_rng, next_tick, state.lobby.config.difficulty)` parametreyle çağır.
4. `crates/moneywar-server/src/world.rs`: `build_initial_state(&lobby)` zaten lobby alıyor — config'ten `npc_count`, `season_ticks`, `seed` çek.
5. `crates/moneywar-server/src/main.rs`: `handle_client_message`'e `SetLobbyConfig` case + reject path + `broadcast_lobby_state`.
6. `crates/moneywar-cli/src/mp_session.rs`: `MpEvent::LobbyConfigUpdated` ekle, `MpCommand::SetConfig` ekle.
7. `crates/moneywar-cli/src/main.rs`: lobi ekranı `Mode::MpLobby` içinde D/S/N tuşları (host ise), config render.
8. Tests: `tests/lobby_config.rs` integration test — host config değiştir, non-host reddedilir, `game_started` sonrası reddedilir.
9. `--difficulty` CLI argümanını solo mode'da koru (ileri uyumluluk).
10. Manuel smoke test: 2 client + server, host Hard→Expert→Easy döngüsü, NPC davranışı gerçekten değişiyor mu.

---

## 6. Açık karar soruları

### Soru 1: Tick rewind skip — onaylanıyor mu?

Önerilen: rewind yok, yerine **Pause + Pending Withdraw + Maç Sonu Özet**. Yani yanlış emir gönderirsen tick işlenmeden Backspace ile geri alırsın, kötü gidiyorsa host pause basar konuşulur, bitince ne olduğunu özette görürsünüz. Gerçek tick rewind (state'i 5 tick geri sarma) **skip**.

**Default: evet, skip.**

### Soru 2: Sprint 5 kapsamı — 4 ayar birden mi?

Önerilen: difficulty + season + npc_count + seed hepsi tek `SetLobbyConfig` mesajında, host-only.

Alternatif: sadece zorluk + sezon yap, NPC sayısı + seed Sprint 8'e bırak.

**Default: hepsi Sprint 5'te** (mesajı bir kere yazmak iki ay sonra geri dönmekten ucuz).

---

## İlgili dosyalar

- `crates/moneywar-net/src/lib.rs`
- `crates/moneywar-server/src/main.rs`
- `crates/moneywar-server/src/game.rs`
- `crates/moneywar-server/src/lobby.rs`
- `crates/moneywar-server/src/world.rs`
- `crates/moneywar-cli/src/main.rs`
- `crates/moneywar-cli/src/mp_session.rs`
- `crates/moneywar-npc/src/lib.rs` (Difficulty enum)
