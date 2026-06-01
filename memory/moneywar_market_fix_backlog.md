---
name: moneywar-market-fix-backlog
description: Log analizinden çıkan piyasa anomali düzeltmeleri — uygulanmayı bekliyor
metadata:
  type: project
---

# Piyasa Düzeltme Backlog'u

Kaynak: 90-tick log analizi (session 2026-05-31).
Durum: Listelenmiş, uygulanmadı. Kullanıcı "ne yapıyoruz" deyince uygula.

---

## Fix 1 — Sanayici: city-specific raw demand (satır 102-150, sanayici.rs)

**Sorun:** `needed_raws` tüm fabrikaların ham maddelerini tek listede topluyor.
`fab_cities × needed_raws` cross-product → Ankara'da Zeytinyağı fab varken
Buğday da alınmaya çalışıyor (Ankara'da Un fab yok).

**Log kanıtı:**
- Sanayici Ankara Buğday BUY: 85 emir → Ankara'da Un fab: 0
- Sanayici Konya Buğday BUY: 3 emir → Konya'da Un fab: 0
- İstanbul Buğday stoku: 4979, Sanayici İstanbul'da Buğday BUY: 0

**Çözüm:** Her `city` iterasyonunda `needed_raws` yerine `city_raws` kullan:
```rust
let city_raws: BTreeSet<ProductKind> = state.factories.values()
    .filter(|f| f.owner == player.id && f.city == city)
    .filter_map(|f| f.product.raw_input())
    .collect();
```
`bucket_cash` da buna göre güncellenmeli (cross-product sayısı değil, gerçek
(city, raw) çifti sayısı).

---

## Fix 2 — Tüccar-Sanayici combo engelleri (tuccar.rs + sanayici.rs)

**Sorun:** Tüccar ucuz şehirden hammadde alıp fabrika şehrine götüremiyor.
5 engel:

1. **Arbitraj sinyali çakmıyor:** Ham madde fiyatları şehirler arası düz
   (hepsi ~5₺) → spread < %15 → `tuccar.rs:173` atlıyor.

2. **min_qty çok yüksek:** `caravan.capacity / 2` = 600 birim bekliyor.
   Tüccar 25 birim/tick alıyor → 24 tick bekliyor. `tuccar.rs:130`.

3. **TTL mismatch:** Sanayici BUY TTL = 3 tick. Kervan yolculuğu 2-3 tick.
   Kervan varınca alıcı expire olmuş.

4. **Fiyat penceresi çakışmıyor:** Tüccar SELL taban = maliyet × 1.10,
   Sanayici BUY tavan = baseline × 1.15. Tüccar'ın satış fiyatı
   Sanayici'nin alış tavanını geçiyor.

5. **Koordinasyon sinyali yok:** İki NPC birbirinin ne istediğini bilmiyor.

**Çözümler (öncelik sırasına göre):**
- Sanayici BUY ceiling: `× 1.15` → `× 1.30` (palyatif ama hızlı)
- min_qty eşiğini düşür: `capacity / 2` → `capacity / 4`
- Tüccar için "fabrika şehri talep sinyali": richest_city yerine
  Sanayici fabrikalarının olduğu şehirlerdeki best_bid'e ağırlık ver

---

## Fix 3 — Un fabrikası yetersizliği (pick_factory_target, sanayici.rs)

**Sorun:** 3 Sanayici'nin 9 fabrikasından sadece 1'i Un, 4'ü Zeytinyağı.
Zeytinyağı margin yüksek (60₺) → Un (17₺) hiç çekilmiyor.

**Log:** ProductionStarted Un: sadece İzmir, 18 kez. FactoryIdle: 638 toplam.

**Çözüm:** `pick_factory_target` içinde slot bazlı kota:
- İlk fab: mevcut gibi (player_id ile dağıt)
- İkinci fab: Un veya Kumaş zorunlu (Zeytinyağı zaten var)
- Üçüncü fab: kalan en kârlı

Ya da `competition_factor` Un için ayrıcalıklı ağırlık:
`same_product_global` çarpanını Un için 1×, diğerleri için 3× yap.

---

## Fix 4 — Buğday baseline eriyor (market.rs, Walras)

**Sorun:** Buğday bucket'ında her tick BUY = 0 → imbalance = -1000 →
baseline -%0.3/tick. 90 tickte %27 erime. reference_price = baseline
(fill yok) → NPC teklifleri de eriyorla beraber kayıyor → döngü.

**Çözüm:** Walras'a "alıcısız bucket" freni ekle:
- Eğer son 5 tickte bu bucket'ta hiç fill olmadıysa Walras adımını
  yarıya indir (`factor_milli` değişimini / 2 yap).
- Ya da minimum imbalance floor: `imbalance.max(-300)` (tam -1000 değil).

---

## Uygulama Önceliği

1. Fix 1 (Sanayici city-specific raw) — en temiz, en az yan etki
2. Fix 3 (Un fabrikası kotası) — Fix 1 ile birlikte Buğday talebini açar
3. Fix 2 kısmi (ceiling × 1.30 + min_qty düşür) — Tüccar comboyu açar
4. Fix 4 (Walras freni) — diğerleri sonrası kalibrasyon
