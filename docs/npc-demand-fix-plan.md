# NPC Talep & Alım Davranışı Düzeltmeleri

**Durum:** Plan — onay sonrası uygulanacak
**Bağlam:** Oyuncu 36+ tick oynadığında talep çöküyor, piyasa altı listing bile esnaf/tüccar tarafından alınmıyor.

---

## Tespit Edilen Sorunlar

### S1 — `AliciNpc` nakit tükeniyor, talep ölüyor
**Dosya:** `crates/moneywar-npc/src/lib.rs:459-511`

- `AliciNpc` tek yönlü nakit akışı: sadece alıyor, hiç satmıyor
- 100.000₺ başlangıç → 3 emir × ~150 birim × 16₺ ≈ 7.200₺/tick
- ~14 tick'te bütçe kritik, ~36 tick'te demand çöküyor
- Decay/regen mekanizması yok, geri dönüş yolu yok

### S2 — `EsnafNpc` çok kısıtlı
**Dosya:** `crates/moneywar-npc/src/lib.rs:525-630`

- `SHELF_THRESHOLD = 100` — stok 100+ ise hiç almıyor
- Bütçe = nakit × %30 → 10K nakit'te sadece 3K bid bütçesi
- Bid fiyatı = market × %93 → çoğu listing eşleşmiyor
- %30 silence — her tick'in %30'unda zaten pasif
- Stale `price_history` referansı

### S3 — `Tüccar` (SmartTrader) "ucuz" görmez
**Dosya:** `crates/moneywar-npc/src/lib.rs:281-446`

- Sadece arbitraj farkına bakıyor (`profit > 25 cents`)
- "Ucuz fiyat" konsepti yok — sadece şehirler arası fark
- Stoğu varsa hiç buy emri vermiyor, direkt dispatch
- Kervan cap = 2; ikisi yoldaysa alım yok

---

## Düzeltme Planı (Sıralı)

### Faz 1 — `AliciNpc` Nakit Döngüsü (öncelik: YÜKSEK)
**Hedef:** Talebin tick 36+ sonrası ayakta kalması.

**Seçenek A (basit):** Periyodik nakit enjeksiyonu
- Her 10 tick'te `AliciNpc` cash += 5.000₺ (max cap 200K)
- Veya: başlangıç nakdi 100K → 300K
- Test: 90 tick boyunca demand sıfırlanmamalı

**Seçenek B (gerçekçi):** Satış kolu ekle
- AliciNpc stoğu birikince Esnaf/Spekülatör'e satar
- Tek yönlü nakit akışını kapatır
- Daha karmaşık, gerçek ekonomik döngü

**Tercih:** A ile başla (1 saat), sonra B'ye geç.

---

### Faz 2 — `EsnafNpc` Bütçesi & Eşikleri (öncelik: YÜKSEK)
**Hedef:** Esnaf'ın aktif kalması, ucuz listing'leri görmesi.

- Başlangıç nakdi: 10K → 50K (`crates/moneywar-cli/src/main.rs:~1625`, `crates/moneywar-server/src/world.rs:~143`)
- `SHELF_THRESHOLD`: 100 → 50
- Bid oranı: market × 93% → market × 98%
- `silence_ratio` erken sezon: 30% → 10%

---

### Faz 3 — `Tüccar` "Ucuz Fırsat" Kolu (öncelik: ORTA)
**Hedef:** Piyasa altı listing'leri tüccar yakalasın.

- Mevcut arbitraj kolu korunur
- **Yeni kol:** Eğer şehirde mal `here_price × 0.85` altında listeli ise spekülatif buy yapar (kervan idle olmasa bile, sonraki tick için stok)
- Cap: tüccar nakit × %20 ile sınırlı
- Stok cap: ürün başına şehirde max 200 birim

---

### Faz 4 — Genel İyileştirmeler (öncelik: DÜŞÜK)
- Kervan cap 2 → 3 (Tüccar)
- `price_history` boşken `base_price` fallback'ini güçlendir
- Demand decay/regen sinyali (haber sistemine bağlı)

---

## Test Stratejisi

Her faz sonrası:

1. **Birim testler:** Yeni davranış için unit test ekle (`crates/moneywar-npc/src/lib.rs` test modülü)
2. **Determinism testi:** `crates/moneywar-engine/tests/determinism_proptest.rs` hala geçmeli
3. **Manuel oyun testi:** 90 tick CLI oyunu, log'da:
   - Demand 36+ tick sonrası > 0 mı?
   - Esnaf alım emirleri her tick var mı?
   - Tüccar piyasa altı listing'e eşleşiyor mu?

---

## Dosya Değişiklik Listesi

| Faz | Dosya | Değişiklik Türü |
|-----|-------|-----------------|
| 1 | `crates/moneywar-npc/src/lib.rs` (AliciNpc) | Davranış genişletme |
| 1 | `crates/moneywar-cli/src/main.rs` (seed) | Sabit değişikliği |
| 1 | `crates/moneywar-server/src/world.rs` (seed) | Sabit değişikliği |
| 2 | `crates/moneywar-npc/src/lib.rs` (EsnafNpc) | Eşik değişiklikleri |
| 2 | Aynı seed dosyaları | Esnaf nakdi |
| 3 | `crates/moneywar-npc/src/lib.rs` (decide_tuccar) | Yeni kol |

---

## Onay Bekleyen Sorular

1. Faz 1'de A (enjeksiyon) mı, B (satış kolu) mı?
2. Esnaf nakdi 50K mı yoksa 100K mı olsun?
3. Tüccar "ucuz fırsat" eşiği 0.85 mi (15% indirim) yoksa 0.90 mı?
4. Test: önce hepsini yapıp tek seferde mi test edelim, yoksa her faz sonrası 90-tick run mı?
