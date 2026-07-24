# NPC Karakter & Canlılık Planı

**Sorun:** Firmalar karakterlerinde değil, hepsi aynı davranıyor, oyunda canlılık yok.

**Kısıtlar (değişmez):** deterministik (aynı seed = aynı oyun), LLM yok, canlıda
gerçek-zamanlı hız, `Difficulty` seviyeleri korunur, synthetic baseline ekonomi
sağlık eşikleri (6/6) bozulmamalı.

---

## Kök nedenler (kod kanıtlı)

| # | Bulgu | Yer |
|---|-------|-----|
| 1 | 32 NPC'den 25'inin `personality: None` — Çiftçi/Alıcı/Spekülatör/Banka hiç kişilik almıyor | `moneywar-web/src/world.rs:89-135` |
| 2 | Kişilik sadece 3 ağırlık çarpanı; 14 sinyalli skorda kaybolur | `npc/behavior/personality.rs:44-82` |
| 3 | `update_traits` tüm beyinleri aynı 3 hedef vektöre sürüklüyor — kişiliğe çıpa yok | `npc/behavior/brain.rs:325-348` |
| 4 | Trait'ler fiyat/miktar/TTL/cross-policy'ye hiç inmiyor; hepsi sabit | `npc/behavior/roles/ciftci.rs:55,63-80` + tüm roller |
| 5 | `rival_threat_for` argümanlarını atıp global skaler dönüyor → kişisel rekabet imkânsız | `npc/behavior/brain.rs:260-270` |
| 6 | `chatter.rs` (600 satır kişilikli Türkçe replik) sadece CLI'da, web'e bağlı değil | `moneywar-cli/src/chatter.rs` |
| 7 | Tüm aynı-rol NPC'ler özdeş sermaye + stokla başlıyor | `moneywar-web/src/world.rs:119-129` |

---

## Faz 1 — Karakteri gerçekten dağıt (temel; bunsuz diğerleri anlamsız)

**Hedef:** Her firmanın kalıcı, birbirinden ayrı bir kimliği olsun.

1. **Herkese kişilik.** `world.rs:seed_npcs` — Çiftçi, Alıcı, Spekülatör de
   `pick_personality(rng)` alsın. (Banka hariç; onun ayrı akışı var.)
   `moneywar-server/src/world.rs` ve `moneywar-sim` de aynı hizaya gelsin.

2. **Arketip → taban trait vektörü.** `PersonalityTraits::from_archetype(Personality)`:
   | Arketip | risk | aggression | patience | greed |
   |---------|------|-----------|----------|-------|
   | Aggressive | 0.80 | 0.90 | 0.15 | 0.55 |
   | Hoarder | 0.25 | 0.30 | 0.90 | 0.80 |
   | Arbitrageur | 0.70 | 0.55 | 0.35 | 0.60 |
   | TrendFollower | 0.60 | 0.50 | 0.30 | 0.50 |
   | MeanReverter | 0.45 | 0.35 | 0.80 | 0.65 |
   | EventTrader | 0.85 | 0.65 | 0.20 | 0.45 |
   | Cartel | 0.40 | 0.75 | 0.85 | 0.90 |

   `AgentBrain::new(personality, quirk_seed)` — `Default` yerine bununla kurulur.
   `BrainPool::sync_players` state'ten kişiliği okuyup geçirir.

3. **Kişisel tuhaflık (quirk).** `player_id` hash'inden her trait'e kalıcı
   ±0.12 ofset. İki Atılgan Çiftçi bile birbirinin kopyası olmasın.
   Deterministik — RNG yok, saf hash.

4. **Drift'i çıpala (en kritik tek değişiklik).** `update_traits` artık global
   hedefe değil, **kendi tabanına + deneyim sapmasına** sürüklensin:
   ```
   target = clamp(baseline + quirk + experience_delta, 0, 1)
   experience_delta ∈ [-0.20, +0.20]   // pnl_trend'den türetilir
   ```
   Karakter deneyimle *bükülür*, yerini başkasına bırakmaz. Kazanan Stoklayıcı
   hâlâ Stoklayıcıdır, sadece daha özgüvenli bir Stoklayıcı.

5. **Kişilik modülasyonunu güçlendir.** `apply_personality` 3 alan yerine
   ilgili tüm sinyallere dokunsun (Hoarder: `urgency×0.3, stock ters, patience
   üzerinden TTL`; Cartel: `partner_trust×2, competition×2` vb.).

**Test:** Sezon sonunda (t=350) trait vektörlerinin arketip bazında ayrışması —
aynı arketipten NPC'ler arası ortalama mesafe < farklı arketipler arası mesafe.
Bu bir birim testi olarak yazılabilir ve regresyonu yakalar.

---

## Faz 2 — Karakter fiyata, miktara, zamana insin

**Hedef:** "standart gidiyor" hissini bitiren faz. Trait'ler emrin kendisini
şekillendirsin, sadece hangi emrin seçildiğini değil.

`pricing.rs`'e ortak yardımcı: `TraitPricing { floor_pct, ceiling_pct, qty_frac, policy, ttl }`

| Trait | Neyi sürer |
|-------|-----------|
| `greed` | SELL tabanı %75 (panik) ↔ %115 (inatçı); BUY tavanı tersi |
| `patience` | `CrossPolicy` eşiği + TTL (sabırlı: Passive + uzun TTL; sabırsız: Cross + kısa) |
| `risk` | Emir miktarı stoğun %20'si ↔ %60'ı |
| `aggression` | Rekabetli bucket'ta best_ask'ı kırma marjı (undercut) |

Tüm roller (`ciftci`, `sanayici`, `tuccar`, `alici`, `spekulator`) sabit
merdivenleri bırakıp bu yardımcıyı kullanır.

**Risk:** Ekonomi dengesi burada bozulabilir. Bu faz bittiğinde synthetic
baseline + behavioral sim koşulup 6/6 sağlık eşiği doğrulanacak. Trait
aralıkları eşikleri bozarsa genlik daraltılır (%75-115 → %85-108 gibi), yön
korunur.

---

## Faz 3 — Rekabeti kişiselleştir (drama)

1. **`rival_threat_for`'ı gerçekten hesapla.** Şu an `city/product/player_id`
   argümanları atılıyor. Bucket bazlı, rakip bazlı tehdit haritası kurulacak.

2. **`nemesis: Option<PlayerId>`** — en güçlü bucket'ımda bana en çok zarar
   veren firma. `AgentBrain`'de yaşar.

3. **`Goal::PriceWar { city, product, target: PlayerId }`** — hedef artık bir
   bucket değil, *bir firma*. O firmanın fiyatının altına gir, o çekilince bitir.

4. **`grudge: BTreeMap<PlayerId, f64>`** — sönümlenen kin. `partner_trust`
   sinyalini negatife çevirir: düşmanınla ticaret yapmazsın, dostuna indirim
   verirsin. (Mevcut `relations`/trust altyapısının üstüne oturur.)

---

## Faz 4 — Canlılık: anlatı yüzeyi (en görünür faz)

1. **`chatter.rs`'i paylaşılan crate'e taşı** (`moneywar-engine` veya yeni
   `moneywar-narrative`) ve **web event feed'ine bağla.** 600 satırlık kişilikli
   Türkçe replik bankası zaten yazılmış, sadece CLI'da kilitli.

2. **Anlatı `LogEvent`'leri:** `RivalryDeclared`, `PriceWarStarted`,
   `MonopolyFormed`, `FirmBankrupt`, `GoalChanged`, `NemesisCrushed`.
   Mekanik `OrderMatched` akışının yanına hikâye akışı.

3. **Firma sayfası** (`web/src/pages/FirmPage.tsx`): arketip rozeti + emoji,
   trait radar grafiği, güncel hedef, nemesis kartı, "bu firmanın hikâyesi"
   zaman çizelgesi. Şu an sadece PnL grafiği + fabrika listesi var.

4. **Leaderboard:** isim yanında kişilik emojisi, hedef çipi (CORNER/RETREAT),
   trend oku. `PlayerDto` zaten `goal` ve `traits` taşıyor — kullanılmıyor.

---

## Faz 5 — Heterojen dünya

- Başlangıç sermayesi seed'li ±%40 sapma → büyük holding vs. çırpınan küçük firma.
- Çiftçi stok dağılımı: bazıları tek üründe uzman, bazıları dağınık.
- Ara sıra 2× sermayeli bir "dev" firma — herkesin hedefi.

---

## Faz 6 — Olaylar ve şoklar

- Firma iflası → varlıkların piyasaya tasfiyesi (fiyat şoku + fırsat).
- Düşman firma yutma / varlık devralma.
- Sezon anlatı dönüm noktaları ticker'da.

---

## Sıralama ve gerekçe

**Faz 1 → 2 → 4 → 3 → 5 → 6.**

Faz 1+2 gerçek düzeltme ve ucuz (birkaç yüz satır). Faz 4 en görünür ama
1+2'siz sadece makyaj olur — chatter "Atılgan" der ama firma herkesle aynı
davranmaya devam eder. Faz 3 dramayı derinleştirir ama 1+2'nin üstüne oturur.
