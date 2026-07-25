# MoneyWar Bitirme Planı — "Entrika İzleme Oyunu"

> Bu plan `docs/npc-character-plan.md`'nin yerine geçer (o dokümandan Faz 1
> kişilik dağıtımının küçültülmüş hali buraya emildi). Onay: 2026-07-25.

**Oyunun yeni tanımı:** Seyirci olarak açıp, şirketlerin birbirine kazık
attığını, fiyat kırdığını, pazarı tekelleştirdiğini harita üstünde izlediğin
bir simülasyon.

**Kuzey yıldızı:** İzleyen kişi **"kim ne yapıyor"ı zahmetsiz takip
edebilmeli.** Her fazın kabul sorusu: *bu değişiklik izleyicinin takibini
kolaylaştırıyor mu?* Olay isimli, failli ve mağdurlu olmalı ("Vural Holding,
Efes'te un pazarını ele geçirdi") — anonim istatistik değil.

**Korunan kısıtlar:** deterministik (aynı seed = aynı oyun), LLM yok, canlıda
gerçek zamanlı hız. **Gevşetilen kısıt:** ekonomi sağlığı artık hedef değil
fren — drama dengesizlikten çıkar, dengesizliğe izin var; sadece felaket
(toptan çöküş, para basımı) engellenir.

**Tüccar kararı:** donduruldu. AI'ına dokunulmaz, tuning ve drama
skorkartından muaf; kervanları kalır (haritanın hareketli kanı). Kervanların
tekelci firmalara devri v2.

---

## Faz 0 — Drama skorkartı (başarı ölçütünü ters çevir)

Şimdiye kadar "ekonomi sağlıklı mı" ölçüldü, o yüzden oyun sıkıcılaştı.
Yeni ölçüt: sezon başına hikâyelik olay sayısı.

- `LogEvent`'e anlatı variant'ları eklenir (emisyon Faz 1-2-5'te):
  `MonopolyFormed/Broken`, `UndercutCampaign`, `PriceWarDeclared/Won`,
  `FirmBankrupt`, `GrudgeFormed`, `SupplyChoke`, `CartelFormed/Betrayed`.
- `moneywar-sim`'e `DramaScorecard`: olayları sayar, sim özetine DRAMA
  bölümü basar.
- Hedef: sezon başına **≥ 8 hikâyelik olay** (Faz 1 bitince aktifleşen test).
- Felaket frenleri (eski 6/6'nın yerini alan 3 kontrol):
  1. Eşleşme hacmi > taban (piyasa tamamen ölmedi),
  2. İflas ≤ sezon başına 3 (oyun 50. tick'te bitmesin),
  3. Para korunumu (motor zaten koruyor; skorkart raporlar).

## Faz 1 — Tekel + fiyat kırma motoru (sim'in kalbi)

1. **Pazar hâkimiyeti takibi.** (şehir, ürün) başına kayan pencerede satış
   payı; pay > %60 → tekelci statüsü → `MonopolyFormed`. Düşünce → `MonopolyBroken`.
2. **Tekelin ödülü.** Tekelci fiyat şişirir (sömürü fazı) → diğerlerine
   "tekeli kır" fırsat sinyali → kendiliğinden tekel-kırma dinamiği.
3. **Kişi hedefli Goal.** `PriceWar { city, product, target: PlayerId }` —
   rakibin fiyatının altına in, o çekilene/iflas edene dek → `PriceWarDeclared/Won`.
4. **Undercut tespiti + kin.** 3 tick üst üste altına inilme →
   `UndercutCampaign` + mağdurda `grudge` (sönümlenen) → `GrudgeFormed`;
   kin karşı-saldırı ya da kaçış kararını besler.
5. **Minimum kişilik.** Tek soru: bu firma *tekelci mi, kırıcı mı, fırsatçı
   mı?* 3-4 entrika arketipi, `player_id` hash'inden deterministik, kalıcı.

## Faz 2 — Üretim zinciri derinliği + SupplyChoke

- `raw_input() -> Option<Self>` yerine `recipe() -> &[(ProductKind, oran)]`
  (1-3 girdi). Girdi eksikse fabrika **girdi açlığı** (görünür durum).
- Katmanlar: yeni hammaddeler (Boya, Üzüm) → ara ürünler (+Şarap) →
  2 parçalı (Elbise = Kumaş+Boya; Ekmek = Un+Zeytinyağı) → **3 parçalı son
  ürün** (tema kararı: Ziyafet Sofrası = Ekmek+Şarap+Zeytinyağı).
  Derin katman = yüksek marj + kırılgan tedarik.
- Alıcı talep sepetine yeni ürünler; zengin şehirlerde son ürün talebi.
- **SupplyChoke olayı:** girdi pazarı tekelcisi rakibin fabrikasını aç
  bırakınca damgalanır. Sanayici tepkisi: dikey entegrasyon
  (`BuildPrivateFarm`) ya da tekelciyle `ProposeContract`.

## Faz 3 — Anlatı olay akışı

- Anlatı LogEvent'leri DTO + API'ye çıkar.
- `chatter.rs` (CLI'daki 600 satır kişilikli Türkçe replik) paylaşılan
  crate'e taşınır, web ticker'ına bağlanır.
- Kuzey yıldızı testi: ticker'daki her satır fail + fiil + mağdur/mekân içerir.

## Faz 4 — Harita frontend'i

- Stilize SVG harita, 5 şehir sabit koordinat.
- Firma rozetleri hâkimiyet payına göre boyutlanır; tekelde taç.
- Kervanlar rotalarda hareket eden noktalar.
- Olay katmanı: fiyat savaşı animasyonu, iflasta solup dağılma, tekelde
  şehir renklenmesi. Ticker ↔ harita senkron (olaya tıkla → odaklan).
- Şehir tıkla → yerel pazar paneli; firma tıkla → hikâye zaman çizelgesi.

## Faz 5 — Kartel + ihanet (kesilebilir sigorta)

- Aynı pazarda iki güçlü firma kırmayı keser → fiyat birlikte yükselir →
  `CartelFormed`. Biri gizlice kırıp payı kapar → `CartelBetrayed` →
  kalıcı kin → fiyat savaşı.

---

## Sıra ve durum

| Faz | İçerik | Durum |
|-----|--------|-------|
| 0 | Drama skorkartı + felaket frenleri | ✅ 459e7a4 |
| 1 | Tekel + fiyat kırma motoru | ✅ 459e7a4 |
| 2 | Üretim zinciri + SupplyChoke | ✅ 3cde156 |
| 3 | Anlatı akışı → web feed + IntrigueDto | ✅ cae9915 |
| 4 | Harita frontend'i | ✅ 549f4ec |
| 5 | Kartel + ihanet | açık (kesilebilir) |

### Faz 3 notu — chatter

Plan `chatter.rs`'i (CLI'daki 600 satır kişilikli replik bankası) paylaşılan
crate'e taşımayı öngörüyordu. Bunun yerine `engine::story_headline()` yazıldı:
anlatı olayının Türkçe manşeti tek kaynakta, sim + web + harita aynı metni
okuyor. Chatter farklı bir olay kümesi için renk metni; kuzey yıldızı olan
"kim ne yapıyor" netliğine manşet formatı daha doğrudan hizmet ediyor.
Chatter CLI'da duruyor, istenirse ayrıca bağlanabilir.

### Sezon başına ölçülen (350 tick, 3 koşum)

- ~52-73 isimli hikâye olayı; tekel 11-30 kuruluş / 9-23 kırılma
- 22-43 undercut kampanyası, 4-14 fiyat savaşı, 5-12 tedarik boğması
- Felaket frenleri temiz (piyasa canlı, iflas 0)
- 7 üretilen ürünün hepsi gerçekten üretiliyor (Ziyafet dahil)

### Açık uçlar

- **İflas hiç olmuyor** (0/sezon). Banka kurtarması ve iflas eşiği
  (nakit < 1₺ + envanter boş + varlık yok) çok katı. Drama için firmaların
  gerçekten batabilmesi gerek — Faz 5 öncesi bakılmalı.
- **Fiyat savaşları çoğu kez sönüyor** (4-14 ilan, 0-2 zafer). Saldırgan
  baskıyı sürdüremiyor; `PRICE_WAR_FIZZLE_TICKS` ve savaş fiyat tabanı
  ayarlanabilir.
- **Mamul pazarlarında tekel çekişmesiz**: Sanayici sayısı 15 mamul
  bucket'a göre az, çoğu pazarda tek üretici var. Kompozisyon veya
  fabrika maliyeti ayarı gerekebilir.
- Tüccar PnL'i diğer rollerin ~5 katı (dondurulmuş rol, tuning yok).

## Riskler

- **YÜKSEK — bilinçli dengesizlik:** tekelciye doğal fren = şişen fiyatın
  yarattığı tekel-kırma iştahı; ayrıca iflas ≤ 3/sezon felaket freni.
- **ORTA — determinizm:** yeni karar katmanları hash tabanlı, RNG'siz.
- **ORTA — kapsam:** Faz 5 kesilebilir; Faz 1-4 = "oyun bitti" çizgisi.
- **DÜŞÜK — harita performansı:** 5 şehir ~30 firma, SVG rahat taşır.
