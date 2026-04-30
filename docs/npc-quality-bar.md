# NPC Kalite Kapısı (Quality Bar) — Difficulty başına ayrı eşikler

> **Amaç:** Easy/Medium/Hard zorluk seviyelerinin **her biri için ayrı**
> kalite kapısı. Her seviyenin kendi doğru davranışı var — Easy'de NPC'ler
> nazik, Hard'da agresif. Aynı eşik 3 seviyeye uymaz.
>
> **Yöntem:** `cargo run -p moneywar-sim -- --multi-seed --difficulty <X>`
> ile baseline rapor üret, ilgili difficulty tablosuna göre değerlendir.

---

## 🟢 EASY — "Yeni başlayan dostu, NPC nazik"

**Tasarım niyeti:** İnsan oyuncu pasif kalsa bile rahat para kazansın.
NPC'ler aktif ama agresif değil. Salaklaştırılmış DSS davranışı.

| # | Metrik | Eşik | v04 |
|---|---|---|---|
| E1 | Hiçbir NPC iflas etmesin | 0 NPC < 100₺ | ❌ Alıcı 214₺ |
| E2 | NPC PnL dengeli (büyük kazanan/kaybeden yok) | abs(PnL) ≤ 30K | ⚠ Sanayici +31K |
| E3 | Match verimliliği | ≥ %1.5 | ✅ %2.1 |
| E4 | İnsan oyuncu pasif senaryoda kâr | İnsan PnL ≥ 0 | ✅ +0 |
| E5 | Stale emir | ≤ 5 | ❓ |
| E6 | Tüccar/Sanayici aktif (en az 1 aksiyon/3 tick) | 30+ aksiyon/sezon | ✅ |

**v04 Easy puan:** 4/6 ✅✅⚠✅❓✅

### Easy rol beklentileri
| Rol | Final PnL aralığı | Davranış |
|---|---|---|
| Sanayici | +5K - +20K | 0-1 fabrika, yavaş üretim |
| Tüccar | +1K - +5K | 0-1 kervan, az arbitraj |
| Esnaf | +5K - +20K | yavaş satış, az bid |
| Alıcı | -50K civarı (hayatta) | rahat alıcı, batma yok |
| Spekülatör | -10K - +5K | dar spread, hayatta kal |

---

## 🟡 MEDIUM — "Dengeli rekabet, gerçek oyun"

**Tasarım niyeti:** İnsan oyuncu strateji yapmazsa kaybeder. NPC'ler
çalışkan ama insanı boğmaz. Default difficulty.

| # | Metrik | Eşik | v04 |
|---|---|---|---|
| M1 | Hiçbir NPC iflas etmesin | 0 NPC < 100₺ | ❌ Alıcı 25₺ |
| M2 | NPC PnL gradient pozitif | Sanayici/Tüccar/Esnaf > 0 | ✅ |
| M3 | Match verimliliği | ≥ %2.5 | ⚠ %2.2 |
| M4 | Aktivite ortalaması (NPC başı emir/sezon) | ≥ 50 | ✅ |
| M5 | Stale emir | ≤ 3 | ❓ |
| M6 | Spekülatör break-even ± %20 | -8K ≤ PnL ≤ +8K | ❌ -38K |

**v04 Medium puan:** 2/6 ✅✅⚠✅❓❌

### Medium rol beklentileri
| Rol | Final PnL aralığı | Davranış |
|---|---|---|
| Sanayici | +20K - +60K | 1 fabrika, dolu üretim |
| Tüccar | +5K - +15K | 1-2 kervan, arbitraj aktif |
| Esnaf | +10K - +25K | düzenli satış + bid |
| Alıcı | -70K civarı (hayatta) | aktif alıcı, sezonun sonunda %30 cash |
| Spekülatör | -8K - +8K | spread daraltıyor, break-even |

---

## 🔴 HARD — "Agresif rakip, oyuncuya tehdit"

**Tasarım niyeti:** NPC'ler oyuncuyu geçer. İnsan strateji yapmazsa kaybeder,
yapsa bile zorlanır. Negatif threshold + max aksiyon.

| # | Metrik | Eşik | v04 |
|---|---|---|---|
| H1 | NPC'ler aktif kazansın | Tüccar+Sanayici PnL toplam ≥ 80K | ✅ +90K (9.7+55+35) |
| H2 | Match verimliliği | ≥ %2.0 | ❌ %1.8 |
| H3 | İnsan pasif senaryoda kayıp eğilimi | İnsan PnL ≤ +5K | ✅ +0 |
| H4 | Spekülatör pozitif PnL | > 0 | ❌ -28K |
| H5 | Stale emir | ≤ 3 | ❓ |
| H6 | Alıcı NPC %20+ cash hayatta | Alıcı son cash ≥ 20K | ❌ 622₺ |

**v04 Hard puan:** 2/6 ✅❌✅❌❓❌

### Hard rol beklentileri
| Rol | Final PnL aralığı | Davranış |
|---|---|---|
| Sanayici | +40K - +80K | 1-2 fabrika, %80+ üretim aktif |
| Tüccar | +10K - +25K | 2 kervan, sürekli arbitraj |
| Esnaf | +15K - +35K | her tick aktif, agresif fiyat |
| Alıcı | en az 20K kalsın | regen mekanizması ya da satış kolu |
| Spekülatör | +5K - +20K | sıkı spread, market dominance |

---

## Genel Skor Tablosu

| Difficulty | Geçer | v04 Mevcut | Eksik |
|---|---|---|---|
| 🟢 Easy | 6/6 | 4/6 | E1 (Alıcı), E2 (Sanayici çok kazanıyor) |
| 🟡 Medium | 6/6 | 2/6 | M1 (Alıcı), M3 (verim), M6 (Spek) |
| 🔴 Hard | 6/6 | 2/6 | H2 (verim), H4 (Spek), H6 (Alıcı) |

**Toplam: 8/18.** Faz 2-7 boyunca bu skoru 18/18'e taşımak hedef.

---

## Ortak Sorunlar (3 zorluk için de fix gerekli)

1. **Alıcı NPC iflas** — `npc-demand-fix-plan.md` Faz 1: enjeksiyon veya satış kolu.
2. **Spekülatör batıyor** — Faz 4 rule base ile düzeltilecek (spread mantığı).
3. **Stale emir ölçümü yok** — sim raporu'na ekle (kolay).

## Sonraki Adım

Fuzzy plana (`fuzzy-ai-test-platform-plan.md`) Faz 2'den devam.
Her faz sonrası bu 3 tabloya bak, skor 18/18'e yaklaşıyor mu kontrol et.
