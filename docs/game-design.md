# MoneyWar — Oyun Tasarım Notları

Bu dosya beyin fırtınası sırasında netleşen tasarım kararlarını tutar.
Her karar "neden" gerekçesiyle birlikte yazılır — sonradan değiştirmek istersek
hangi varsayımın yıkıldığını görelim.

---

## 0. Ölçek ve Dağıtım Modeli

### Karar: Oda-bazlı, 2-5 oyuncu, NPC dolgulu

**v1 hedef kitlesi: 2–5 kişilik arkadaş grupları.** MMO sandbox değil, board-game
ruhunda özel oda sessions'ları.

- Bir oyuncu oda açar, arkadaşları katılır
- Oda sahibi preset seçer (kurallar, süre, NPC sayısı)
- Sezon biter, skorlar kalıcı kariyer istatistiklerine işlenir
- Yeni oda, sıfırdan başlar

### Neden sandbox değil

- 5 kişilik sandbox = ölü şehir. Piyasa boş, kimse kimseyi bulmuyor.
- Balance 5-kişi ile 500-kişi arasında aynı kalamaz → erken optimizasyon tuzağı.
- Oda modeli hedef kitleye biçilmiş kaftan: küçük arkadaş grubu, özel deneyim.

### Sandbox modu: v3'e bırakıldı

Oyun 50-100+ aktif oyuncuya ulaşırsa ayrı bir mod olarak sandbox açılır. Oda mod
ile aynı motoru kullanır, sadece "dünya" büyük ve kalıcıdır. Şimdilik yol haritasında.

### NPC dolgusu

Küçük odalarda piyasa canlılığı için NPC Sanayici + NPC Tüccar karakterler ekonomiye
dahil olur. Basit kurallarla alım/satım yaparlar, likidite sağlarlar, leaderboard'da
görünmezler. **Detaylı tasarımı ayrı bir oturumda ele alınacak.**

### Oda konfigürasyonu (preset + custom)

Oda sahibi preset seçer:

| Preset | Tick süresi | Sezon | NPC |
|---|---|---|---|
| **Hızlı** (v1 varsayılan) | 1 dk | ~1.5 saat (90 tick) | 3 |
| **Standart** | 30 dk | ~3 gün (~150 tick) | 4 |
| **Uzun** | 1 saat | ~14 gün (~350 tick) | 5 |
| **Custom** | Serbest | Serbest | Serbest |

Teknik maliyet küçük: tek `RoomConfig` struct'ı, tick motoru config okuyor.
Validation kuralları runtime'da (tick > 0, sezon > 10 tick vb.)

### v1 odağı: Hızlı preset

İlk versiyon "Hızlı" preset üstüne kurulur. 2-3 arkadaş 1-2 saatte tam bir sezon
oynar. Eğlence burada test edilir. Diğer presetler çalışır ama ayarlama odaksız.

---

## 1. Zaman Modeli: Tick Bazlı

### Karar

Oyun **tick bazlı** ilerler. Gerçek zamanlı piyasa yoktur. Her tick bir "oyun günü"ne karşılık gelir.

- **Tick uzunluğu ve sezon uzunluğu oda config'inden gelir** (§0'daki preset tablosu)
- v1 Hızlı preset: tick 1 dakika, sezon ~90 tick (~1.5 saat)
- Motor hangi tick süresiyle çalışırsa çalışsın aynı mantıkla işler (config-driven)

### Neden

1. **Browser MMO'nun hayatta kalma şartı.** Gerçek zamanlı piyasa, online olmayan
   oyuncuyu rekabetten düşürür. 24 saat bağımlılık = 3. günde herkes yorulur.
2. **Anlaşmalar ancak tick modelinde anlamlı.** "3 tick sonra teslimat" bir
   bağlayıcı söz yaratır; gerçek zamanda bu kavram yok.
3. **Strateji oyunu olsun, tepki oyunu değil.** Oyuncu günde 3–5 kez girer,
   emir verir, sonucu bekler. Virtonomics / Travian / OGame modeli — on beş yıl
   boyunca binlerce oyuncuyu tuttu.
4. **Dopamin kaynağı "reveal" anıdır.** Tick sınırında emirler açılır, fiyatlar
   oynar, kimin ne yaptığı belli olur. Bu an oyunun zirvesidir.

### Nasıl Çalışır

Her tick bir **atomik işlem bloğu**:

1. **Faz 1 — Emir toplama:** Oyuncular tick süresi boyunca alım/satım/kontrat
   emri koyar. Emirler kilitli; tick açılana kadar karşı taraf göremez
   (bluff alanı).
2. **Faz 2 — Eşleştirme:** Tick sınırında motor tüm emirleri topluca işler.
   Fiyat arz/talep ile belirlenir.
3. **Faz 3 — Olaylar:** Rastgele olaylar (kıtlık, grev, patlama) bu aşamada
   tetiklenir. Bazıları sadece belirli oyunculara "haber" olarak gider.
4. **Faz 4 — Teslimat & raporlama:** Üretim biter, kontratlar yürür, oyunculara
   özet rapor gönderilir.

### Önemli Sonuçlar

- Oyuncu offline iken de ekonomisi işler (üretim devam eder, kontratlar yürür).
- "Son dakika panik satışı" diye bir şey yok — emir verirsin, tick açılınca görürsün.
- Motor sürekli fiyat hesaplamaz, sadece tick sınırında hesaplar. Bu
  **mühendislik açısından da kritik bir basitleşme** — piyasa sürekli değil,
  tick başına 1 kez açılıyor.

---

## 2. Ticaret Motoru

İki ayrı piyasa katmanı var. Biri günlük alım-satım, diğeri ileri tarihli anlaşmalar.

### Katman 1 — Hal Pazarı (günlük alım-satım)

Her tick bir "sabah hal açılışı" gibi çalışır.

**İşleyiş:**
1. Tick süresince oyuncular alım/satım emri yazar ("100 kilo domates, 50 liradan satarım")
2. Emirler kilitli, kimse diğerinin teklifini göremez (bluff alanı)
3. Tick kapanınca motor tüm emirleri açar, sıralar
4. Arz ve talebin kesiştiği **tek bir takas fiyatı** çıkarır
5. O fiyatta buluşan emirler eşleşir, diğerleri çöpe

**Neden sürekli açık piyasa değil (örn. borsa):**
- Tick modelimiz zaten atomik bloklar halinde çalışıyor, sürekli eşleştirmeye ihtiyaç yok
- Kod ~10 kat basit
- Latency/hız oyunu yok → saniyede 1000 emir atanla eşit koşullardasın
- Bluff hâlâ var (hayalet emir koyup çekebiliyorsun, tick açılana kadar)

### Katman 2 — Anlaşma Masası (ileri tarihli kontratlar)

İki oyuncu arasında bağlayıcı söz. **Kapora sistemi** ile motor zorla uygular.

**İşleyiş:**
- Kontrat alanları: alıcı, satıcı, ürün, miktar, fiyat, teslimat tick'i, kapora miktarı
- İmza anında motor her iki tarafın kaporasını kasaya kilitler (escrow)
- Teslimat tick'i gelince: şartlar sağlanırsa kaporalar serbest + mal el değiştirir
- Biri cayarsa: onun kaporası yanar, diğer tarafa tazminat gider

**İki format:**
- **Kişiye özel:** "Sana özel teklif" — sadece seçilen oyuncu kabul edebilir
- **İlan:** Panoya asılır, ilk kapan alır

### Güven puanı (reputation) sistemi YOK

Bazı oyunlarda "ihanet edersen itibar düşer" sistemi vardır. Bizde olmayacak.

**Neden:**
- Sezon 2 hafta sonra sıfırlanıyor; itibar biriktirmenin anlamı yok
- Kapora yanması matematiksel yeterli caydırıcı — ihanet edersen kaybedersin
- Rep sistemi karmaşıklık ekliyor, az kazandırıyor

**İhanet alanı nerede kaldı?**
- Kartel üyesi gizlice farklı fiyattan satabilir (kontrat yoksa kapora da yok)
- "Laf olsun" gizli anlaşmalar motor tarafından takip edilmez — ihanet serbest
- Public kontrat yerine kişiye özel kontrat tercih etmek "görünmez anlaşma" imkanı verir

### Katman 3 — Türev piyasası (v2, sonraya)

Fiyat yönüne bahis, opsiyon, short pozisyon. v1'de Katman 2 üstüne "yönlü kontrat"
yazarak benzer şeyler yapılabiliyor. Türev motorunu sonraya bırakıyoruz.

---

## 3. Şehirler

### Karar

**3 şehir:** İstanbul, Ankara, İzmir. Her şehir kendi spot piyasasına sahip.
Oyuncular şehirler arası mal taşıyarak fiyat farklarından yararlanır.

### 4 farklılaşma boyutu (v1)

#### 1. Doğal uzmanlaşma (her şehir bir şeyde iyi)
- **İstanbul:** tekstil ucuz üretir (ham pamuk bol)
- **Ankara:** buğday/gıda ucuz üretir (tarım bölgesi)
- **İzmir:** zeytinyağı/meyve ucuz üretir (Ege iklimi)

Üretim yeri ucuz, tüketim yeri pahalı → doğal ticaret yönü oluşur.

#### 2. Mesafe asimetrisi (yollar eşit değil)
- İstanbul ↔ Ankara: **3 tick** + orta maliyet
- Ankara ↔ İzmir: **2 tick** + düşük maliyet
- İstanbul ↔ İzmir: **4 tick** + yüksek maliyet (deniz yolu)

Yakın şehirlere sık küçük sevkiyat, uzak şehirlere nadir büyük sevkiyat.

#### 3. Talep profili (her şehir farklı şey tüketir)
- **İstanbul:** lüks mal talebi yüksek (metropol)
- **Ankara:** temel gıda talebi yüksek
- **İzmir:** orta seviye dengeli talep

#### 4. Bölgesel olaylar (rastgele şoklar)
- Örn: "İzmir'de grev → zeytinyağı üretimi durdu" → fiyat fırlar
- Örn: "Ankara'da kuraklık → buğday arzı düştü"
- Olaylar **haber servisi alan oyunculara 1 tick önce** gider → bilgi asimetrisi
- Diğer oyuncular 1 tick sonra öğrenir, ama o zamana kadar haberi olan pozisyon almıştır

### 5. Şehir kapasitesi (v2, sonraya)

Her şehir tick başına sınırlı mal soğurabilir. Aşırı arz = fiyat çöker.
Kartel oyunu için altın fikir ama v1'e ağır, sonraya bıraktık.

---

## 4. Ürünler ve Üretim

### Karar: 3 zincir, 6 ürün

Her şehir bir ham madde üretir, Sanayici fabrikada ham maddeyi bitmiş ürüne çevirir.

| Ham madde | Bitmiş ürün | Ucuz üretim yeri |
|---|---|---|
| Pamuk | Kumaş | İstanbul |
| Buğday | Un | Ankara |
| Zeytin | Zeytinyağı | İzmir |

6 ürün × 3 şehir = 18 fiyat noktası. Yeterince fırsat, çok fazla değil.
10+ ürün olsa oyuncu kafası karışır, fiyatlar takip edilemez.

### Üretim mekaniği

- **1 ham madde → 1 bitmiş ürün** (basit oran)
- Üretim süresi: **2 tick** (tentatif — test edilecek)
- Bitmiş ürün hem daha değerli, hem talep profili yüksek şehirlerde (İstanbul lüks) ekstra pahalıdır
- Sanayici kararı: fabrikayı ham madde kaynağına mı (girdi ucuz) yoksa talep merkezine mi (çıktı pahalı) kurar?

### Taşıma mekaniği

**Kayıp riski yok, süre riski var.**

- Tüccar kervanı gönderir, X tick sonra **kesinlikle** varır
- Yolda mal kaybı/haydut/kaza **yok** — oyuncu sıçma hissi yaşamasın
- Asıl risk: taşıma süresi içinde fiyat oynaması
- **Sistemik olaylar** rotayı kapatabilir ("İzmir-Ankara yolu kapalı, 2 tick gecikme") — ama bu şans değil haber, haber servisi olan önce öğrenir

**Kapasite asimetrisi:**
- **Tüccar:** tek kervanda yüksek miktar, birim başına düşük maliyet
- **Sanayici (ve diğerleri):** kervan çalıştırabilir ama düşük kapasite, yüksek maliyet
- Büyük hacim = Tüccar ile kontrat yapmak doğal seçim

### Mevsim/döngü fiyatları (v1'de açık)

Her ham maddenin doğal ritmi var, oyuncu öğrenir ve planlar:
- **Buğday:** sezon başı pahalı, hasat tick'inde fiyat çöker, sonra yavaş çıkar
- **Zeytin:** 2–3 tick'lik hasat penceresi, kaçırırsan 1 hafta bekle
- **Pamuk:** yıl boyu stabil ama iklim olaylarında oynar

### Bozulma (v1'de kısmi)

Bazı mallar uzun bekletilirse kaybolur:
- **Zeytinyağı:** depoda 5 tick sonra %10 fire
- **Un:** 3 tick sonra bozulur
- **Pamuk, Kumaş, Buğday, Zeytin:** dayanıklı, fire yok

Bu mekanik "elimde biriktir, fiyat çıkınca satarım" stratejisini tüm ürünler için geçerli olmaktan çıkarır. Karar derinliği eklenir.

---

## 5. Meslek Sistemi

### Karar

- **v1'de sadece 2 meslek çalışır:** Sanayici + Tüccar
- Diğer 3 meslek (Spekülatör, Banker, Kartel) sonraki sürümlerde eklenir
- Her oyuncu sezon başında meslek seçer, sezon içinde değişemez

### Neden Sanayici + Tüccar ile başlıyoruz

- Bu ikisi olmadan ekonomi yok. Biri üretir, diğeri taşır/satar. Temel iskelet.
- Diğer 3 meslek bu ikisinin üstünde yaşar:
  - Banker üretilen malı finanse eder
  - Spekülatör ticaret fiyatı üstüne bahis oynar
  - Kartel işleyen bir piyasayı manipüle eder
- Üretim + ticaret çalışmadan diğer meslekler anlamsız
- Mekanik basit: fabrika, ham madde, mal, taşıma, satış. Kaldıraç/türev gerekmiyor.

### Rol farklılaşma seviyesi: "Her rolün 1 kilit yeteneği"

Üç olası yaklaşım:

1. **Sadece istatistik farkı** (Sanayici %20 ucuz üretir) — sığ, kimlik yok
2. **Her rolün kendine özel 1 yeteneği** ← SEÇİLEN
3. **Her rol tamamen farklı oyun** — aşırı ağır, 5 ayrı oyun yazmak gerekir

Seviye 2'de **temel alım-satımı herkes yapar**, ama her rolün tanımlayıcı bir
tekelci yeteneği vardır.

### Sanayici

- **Tekelci yetenek:** Fabrika kurabilen tek rol. Ham madde → ürün dönüşümünü
  sadece Sanayici yapar.
- **Oynayış:** Fabrika kur, ham madde al, üret, sat. Pasif gelirli, yavaş ama kesin.
- **Risk profili:** Düşük. Sermaye uzun süre bağlanır ama büyüme güvenli.
- **Becerisi:** Tedarik zinciri yönetimi, kapasite planlama, lokasyon seçimi.
- **Zayıf yönü:** Sermaye bağlı olduğu için ani fiyat çöküşlerinde çaresiz.

### Tüccar

- **Tekelci yetenek:** "Ticaret filosu" — taşıma kapasitesi yüksek, maliyeti düşük.
  Haber servisi Gümüş katmanını içeride bedava alır (1 tick önceden bölgesel olay haberi).
- **Oynayış:** Piyasadan alır, başka şehre taşır, orada satar. Aktif, gezen, fırsatçı.
- **Risk profili:** Orta. Beceri odaklı.
- **Becerisi:** Fiyat okuma, rota zamanlaması, stok yönetimi, haber yorumlama.
- **Zayıf yönü:** Üretim gücü yok, her zaman başkasının malına bağımlı.

### Roller arası geçirgenlik (sınırlı)

Roller kilitli ama **tam kilit değil, verimsiz geçirgenlik**:

| Aktivite | Sanayici | Tüccar |
|---|---|---|
| Fabrika kurmak | ✅ Tekel yeteneği | ❌ Hiç yapamaz |
| Kervan çalıştırmak | ✅ Küçük kapasite, yüksek maliyet | ✅ Tekel avantajı, büyük kapasite |
| Haber Gümüş | Parayla satın alır | İçeride bedava |

**Tasarım gerekçesi:**
- Sanayici kendi küçük hacmini kendi taşıyabilir → yalnız oyuncuya tolerans
- Büyük hacim için Tüccar ile kontrat şart → iki rolün doğal eşleşmesi
- Fabrika tekeli Sanayici'de → kimlik net kalır
- Tüccar Sanayici'ye her zaman muhtaç → sosyal olmaya zorlanır, kontrat yapar

**Bilinçli asimetri:** Sanayici kendi başına hayatta kalabilir, Tüccar kalamaz. Bu bug değil özellik — iki rolün ruhu farklılaşır.

### Kota ve sezon dengesi

- **Serbest seçim.** Kota yok. Herkes istediği rolü seçer.
- **2-5 kişilik oda modelinde** dengesizlik kaçınılmaz olabilir (örn. 4 Sanayici 1 Tüccar).
  Buna karşı: NPC dolgusu piyasayı canlı tutar, roller arası fark eksik olanı yakalar.
- Dengesizlik olursa (örn. 4 Sanayici) pazar kendini düzeltir: ürün bolluğu fiyatları düşürür → 1 Tüccar NPC'lerle birlikte kârlı olur.
- Kota v2+ rolleri (Kartel, Banker) eklenince ve sandbox modunda yeniden değerlendirilir.

### Tasarım prensipleri

- **Meslek sezon içinde değişmez** — bağlayıcılık olmazsa strateji derinliği kaybolur
- **Meslekler arası hiyerarşi yok** — her meslekten sezon şampiyonu çıkabilmeli
- **Skill tavanları farklı olmalı** — Sanayici öğrenmesi kolay, Tüccar daha aktif/fırsatçı
- **Rolsüz seçenek yok** — her oyuncu kimlik taşır

### Gelecek meslekler (v2+, taslak)

- **Spekülatör:** Kaldıraçla oynar, fiyat yönüne bahis. Tavan yok, taban iflas.
- **Banker:** Borç verir, faiz toplar. Müşteri iflas ederse batar.
- **Kartel Ağı:** Anlaşma çevirir, bilgi satar, ittifak + ihanet. Meta oyuncu.

### Ekosistem vizyonu (5 meslek tamamlanınca)

```
Sanayici üretir
    ↓
Tüccar taşır ve piyasaya sürer
    ↓
Spekülatör fiyat yönüne bahis oynar
    ↓
Banker herkesi finanse eder
    ↓
Kartel hepsinin üstünden oynar, bilgi ve ittifak kurar
```

---

## 6. Haber Servisi ve Olaylar

### Haber katmanları

Bilgi bir kaynak. Üç katman:

| Katman | Kim alır | Ne verir |
|---|---|---|
| **Bronz** | Herkese açık, ücretsiz | Tick açılınca genel olay duyurusu (herkesle aynı anda) |
| **Gümüş** | Abonelik (Tüccar bedava, diğerleri parayla) | 1 tick önceden bölgesel olay haberi |
| **Altın** | Pahalı abonelik, herkese açık | 2 tick önceden + olay olasılık tahminleri ("Ankara'da kuraklık riski %40") |

Haber servisi = kurnazlığın ana kaynağı. Okuyan + hızlı karar veren kazanır.

### Olay tipleri

**Negatif (arz/talep şoku):**
- Kuraklık, grev, salgın, yangın → üretim düşer, fiyat fırlar
- Yol kapanması → kervan gecikir, rota fiyat farkı genişler

**Pozitif (fırsat):**
- Bereketli hasat → arz bol, fiyat düştü, alım zamanı
- Yeni pazar açıldı → geçici talep patlaması

**Olay zincirleri (narrative):**
- Tick 30: "Ankara'da kuraklık" → buğday arzı düştü
- Tick 35: "Ankara göç veriyor" → İzmir'de gıda talebi arttı
- Tick 40: "İşçi sıkıntısı" → fabrika verimi -%20

Oyuncu ilk haberi okurken zinciri tahmin eder. Okuyabilen kazanır.

---

## 7. Kontrat Derinliği

Anlaşma Masası'nın (§2 Katman 2) temel üstünde kullanım kalıpları:

### Kervan kiralama kontratı (v1'in kilit kontrat tipi)

> "Sanayici, 5 tick boyunca haftada şu kadar ürünümü Tüccar'a taşıması için sabit ücret öder."

- Tüccar aktif rota garantisi kazanır
- Sanayici lojistik güvencesi kazanır
- **Sanayici + Tüccar doğal eşleşmesi** — mekanik olarak ittifak
- Bozarsan kapora yanar

### Gelecek kontrat tipleri (v2+)

- **Ortaklık kontratı:** Kâr paylaşımlı iş ("sen üret ben sat, %60-%40") — Kartel rolü gelince
- **Tekel kontratı:** Fiyat disiplini ("kimseye 80 altından düşük satma") — Kartel rolü gelince
- **NPC borç:** Oyun bankasından sabit faizle kredi — kaldıraç iştahlı oyuncu için (belki v1'in sonunda)

---

## 8. Snowball Kontrolü ve Denge

### Problem

Ekonomi oyunlarında para bileşik büyür: önde giden otomatik uzaklaşır. Sezonun
yarısında kimse lideri yakalayamazsa diğerleri bırakır → oyun ölür.

### Küçük oda ölçeğinde (2-5 oyuncu)

100 kişilik sandbox'ta snowball istatistiksel bir sorun. 5 kişilik odada **sosyal bir sorun**.
5 kişide "4'ü birleşip 1'i bitirme" doğal olarak var → mekanik snowball kurtarıcı daha az
gerekli. Ama yine de iki mekanizma v1'de devrede:

### Mekanizma 1: Piyasa doygunluğu

Her şehrin her üründe tick başına **soğurma kapasitesi** var.

- İstanbul tick başına ~500 kumaş soğurur normal fiyata
- Eğer bir Sanayici 800 kumaş dökerse: 500'ü normal fiyattan, 300'ü yarı fiyattan satılır
- Lider için dolaylı tavan: çok ürettiğin kadar satamazsın

Bu mekanizma şehir başına, ürün başına ölçeklenebilir — oda kapasitesi küçülünce
doygunluk eşiği de küçülür (config'e bağlı).

### Mekanizma 2: Geç sezon volatilitesi

Olay motoru sezon boyunca ritim değiştirir:

```
Sezonun ilk %50'si:   Normal olaylar, seyrek, küçük-orta etki
Sezonun %50-80'i:     Olay sıklığı artar, büyüklük büyür
Sezonun son %20'si:   Makro şoklar, tüm piyasayı vuran büyük olaylar
```

Son %20'de lider tek yanlış pozisyonla ciddi kaybedebilir. Comeback penceresi
doğal olarak açılır — yapay catch-up değil, narrative geç sezon kriz.

### Reddedilen mekanizmalar

- **Top N şeffaflığı:** 5 kişide herkes zaten Top N'de, anlamsız. Sandbox'ta (v3) yeniden gelebilir.
- **Rubber-banding / Mario Kart mavi kabuk:** İyi oynayanı cezalandırır, oyunun ruhuna ters.
- **Zenginden vergi:** Kapitalist oyun kurallarına ters, oyuncu sinirlenir.

### Küçük oda için ek doğal dengeleyici

2-5 kişide **sosyal koalisyon** snowball'un kendi kendine çözücüsü:
- Lider öne çıkarsa diğerleri kontrat üstünden birleşip onu kıstırır
- Tekel kontratı (v2) geldiğinde bu daha güçlü mekanik olur
- Kartel rolü (v2) bu sosyal dinamiği mekanikleştirir

v1'de kontrat sistemi zaten koalisyonu mümkün kılıyor — mekanik olarak yasak yok.

---

## 9. Kazanma Koşulu ve Skor

### Karar: Çok-yollu tek skor

Tek bir sayı şampiyonu belirler, ama bu sayıya **4 farklı kalemden** ulaşılabilir.
Oyuncu tek şeye odaklanmak zorunda değil — kendi tarzında strateji seçer.

### Formül

```
Skor = Nakit
     + Σ(stok_i × son5tick_ortalama_fiyat_i)
     + Σ(fabrika_kurulum_maliyeti_j × 0.5)      [atıl fabrika = 0]
     + Σ(escrow_kapora_k)
```

### Kalem detayları

**1. Nakit**
Oyuncunun cebindeki para. Direkt.

**2. Stok değeri**
Her ürünün miktarı × o ürünün kendi şehrindeki son 5 tick ortalama fiyatı.
- Son 5 tick ortalaması tek-tick manipülasyonunu öldürür
- Satmak/satmamak arasında skor farkı yok (satınca nakit, satmayınca aynı değerde stok)
- **Panik satışı motivasyonu kalmaz** — son tick'te stok boşaltma yarışı yok

**3. Fabrika sermayesi**
Fabrika kurulum maliyeti × 0.5
- Yatırımın %50'si skora döner → fabrika kurmak kısmen risk, kısmen kazanç
- Tamamen kayıp değil → Sanayici cezalanmıyor
- **Atıl fabrika kuralı:** Son 10 tick'te hiç üretim yapmadıysa değeri 0. Fabrika kurup unutmayı engeller.

**4. Escrow paran**
Aktif kontratlardaki kilitli kaporan. Senin paran, kasada bekliyor — skorlanır.
- v1'de kontratın beklenen kâr/zararı (cari piyasayla karşılaştırma) skorlanmaz
- Sadece kendi kaporan. Sade, manipülasyona kapalı.

### Çok-yollu strateji — aynı 100k skor, 4 farklı kompozisyon

| Yol | Nakit | Stok | Fabrika | Escrow |
|---|---|---|---|---|
| Saf Sanayici | 30k | 40k | 30k | 0 |
| Saf Tüccar | 70k | 25k | 0 | 5k |
| Karma | 40k | 30k | 20k | 10k |
| Kontrat Ağı (v2) | 40k | 20k | 0 | 40k |

Hiçbirini "doğru yol" yapmaz. Oyuncu kendi tarzını seçer, formül cezalandırmaz.

### Leaderboard görünürlüğü: sıralama var, rakam yok

**Oyun boyunca:**
- Top 5 sıralaması canlı görünür ("1. Ali, 2. Ayşe, 3. Sen, ...")
- Aralarındaki **sayısal fark gizli** — %10 geride mi %50 mi bilinmez
- Umut canlı, stres düşük

**Sezon sonu:**
- Tüm rakamlar açılır, kalem bazlı döküm herkese görünür
- Oyuncu nereden kazandığını/kaybettiğini görür — öğrenme anı
- Reveal dramatik, final dopamin burada

### Kalıcılık: kariyer profili + rozetler

Her oyuncu, odalardan bağımsız, **kalıcı bir kariyer profili** taşır.

**İstatistikler:**
- Toplam oynanan oda sayısı
- Toplam şampiyonluk
- En yüksek tek-oda skoru (all-time high)
- Rol bazında dağılım ("12 Sanayici seansı, 4 şampiyonluk")

**Rozet örnekleri:**
- İlk Şampiyonluk
- 100k Kulübü (tek odada 100k skor)
- 10 Oda Ustası
- Dengeli Oyuncu (tüm kalemlerden eşit skor)
- Saf Sanayici (stok + fabrika %80'i)
- Saf Tüccar (nakit %80'i)

**Neden kalıcılık:** Oda biter ama kariyer devam eder. Oyuncu yeni oda açmaya
motive olur — istatistiklerini büyütür, rozet biriktirir. Sezonluk lig sistemi
yok (her oda zaten kendi sezonu), all-time rekorlar ve kariyer çubukları var.

---

## 10. Ekonomi Parametreleri (v1 Hızlı preset başlangıç değerleri)

> Bu rakamlar **v1 başlangıç değerleri**. Playtest'te ayarlanacak. Önemli olan
> oranlar ve sistem, mutlak sayılar değil. Preset değiştikçe (Standart/Uzun)
> değerler de ölçeklenebilir.

### Başlangıç paketi (eşit toplam değer, farklı yapı)

| Rol | Nakit | Starter ekipman | Toplam değer |
|---|---|---|---|
| Sanayici | 8k | 1 fabrika (seçilen şehirde, Seviye 1) + 1 küçük kervan (kap. 20) | ~15k |
| Tüccar | 13k | 1 büyük kervan (kap. 50) | ~15k |

Rol seçimi para miktarıyla değil **oynayış yapısıyla** tanımlanır. Sanayici üretimle,
Tüccar likit nakitle başlar — aynı toplam değer, farklı silah.

### Fabrika sistemi

**v1'de seviye yok, çoklu fabrika var.** Basit sebep: 1.5 saatlik Hızlı preset'te
L3'e ulaşma zamanı olmaz, kullanılmaz feature olur. Çoklu fabrika aynı büyüme
hissini daha basit verir.

| Sıra | Maliyet | Üretim |
|---|---|---|
| 1. fabrika | Bedava (starter) | 10 unit/tick |
| 2. | 10k | 10 unit/tick |
| 3. | 15k | 10 unit/tick |
| 4. | 22k | 10 unit/tick |
| 5. | 30k | 10 unit/tick |

Her fabrika **bir şehirde** kurulur. Lokasyon seçimi = tedarik zinciri kararı
(hammadde ucuz yeri mi, talep yüksek yeri mi?).

**Artan maliyet + piyasa doygunluğu** iki mekanizma üst üste → snowball'un
doğal tavanı.

### Kervan sistemi

**Çoklu kervan, paralel çalışır.** Her kervan bağımsız operasyon yapabilir
(aynı anda farklı rotalarda). Kervan rotadayken meşgul, teslimata kadar başka
iş yapamaz.

**Sanayici kervanları (küçük, yakın mesafe):**

| Sıra | Maliyet | Kapasite |
|---|---|---|
| 1. (starter) | Bedava | 20 |
| 2. | 5k | 20 |
| 3. | 10k | 20 |

**Tüccar kervanları (büyük, uzak mesafe):**

| Sıra | Maliyet | Kapasite |
|---|---|---|
| 1. (starter) | Bedava | 50 |
| 2. | 6k | 50 |
| 3. | 10k | 50 |
| 4. | 15k | 50 |

Kervan yönetimi = Tüccar'ın mikro-oyunu: hangi kervan nerede, hangi rotada,
ne zaman dönüş.

### Piyasa doygunluk eşiği

Oyuncu sayısına bağlı ölçeklenir (NPC'ler dahil):

```
eşik = 40 + (oyuncu_sayısı - 2) × 10
```

| Oyuncu sayısı | Şehir başına eşik (birim/tick/ürün) |
|---|---|
| 2 | 40 |
| 3 | 50 |
| 4 | 60 |
| 5 | 70 |

**Aşılınca:** aşan miktar **%50 fiyattan** satılır. Yumuşak tavan — "satamıyorsun"
değil "daha ucuza satıyorsun". Oyuncu hâlâ seçim yapıyor: dökmem veya yarı fiyatla.

### Fiyat skalası

| Kalem | Değer |
|---|---|
| Ham madde baz fiyat | 5–8 lira/unit (şehir bazlı farklılık) |
| Bitmiş ürün baz fiyat | 12–18 lira/unit |
| Marj ham→bitmiş | ~2x (üretici kârı) |
| Taşıma maliyeti | mesafe × 0.5–1 lira/unit |
| Ham madde doğal arzı | şehir başına ~30 unit/tick (NPC üreticiler sağlıyor) |

**Örnek tick ekonomisi:**
- Sanayici 1 fabrika: 10 unit × 10 lira marj = ~100 lira/tick kâr
- Tüccar 1 rota: 50 unit × 5 lira arbitraj = 250 gross → ~150 net
- 10k fabrika yatırımı ~100 tick'te geri döner → Hızlı preset'te (90 tick) 1. fabrika zar zor geri ödenir, 2. fabrika risktir

Bu orantı **"ilk sezon temkinli, sonraki sezonlar cesur"** öğrenme eğrisi yaratır.

### Para birimi

**Tek para birimi (lira).** Şehirler arası kur riski yok. Basitlik için v1'de bu karar.

---

## 11. Tasarım Pusulası

Her kararı bu üç prensibe göre süz:

### Prensip 1: Strateji oyunu, tepki oyunu değil

Oyuncu günde 3–5 kez girer, düşünür, karar verir, çıkar. 24 saat online kalmak
gerekmesin. Tick modeli bunu garantiliyor.

### Prensip 2: Karmaşıklık matematikte değil, kararlarda

- **Kötü:** "Deponun %42'sine stoğunun %17.3'ünü koy" — mikro hesap kazıma
- **İyi:** "Elimde 3 seçenek var, hangisi?" — anlamlı seçim

Bunu garantilemek için:
- Ürün sayısı az (5–7, 30 değil)
- Şehir sayısı az (3, 10 değil)
- Her tick 3–5 anlamlı karar yeterli

### Prensip 3: Dopamin kaynağı = reveal + kurnazlık

- **Reveal:** Tick kapanınca fiyatlar açılır, kazananlar/kaybedenler belli olur
- **Kurnazlık:** Bilgi asimetrisi (haber servisi), bluff (kilitli emirler), gizli anlaşmalar (public olmayan kontratlar)
- Bu iki kaynak kuru kalırsa oyuncu 3. günde sıkılır

---

## 12. Açık Kalan Sorular

**Kaldığımız yer:** Son oturumda §10 (Ekonomi Parametreleri) çözüldü. Kalan ana
konular: NPC tasarımı + UI/onboarding detayları. İskelet büyük ölçüde hazır,
kod yazımına başlayabiliriz.

Sonraki oturumda karar vermemiz gerekenler:

### Kalan ekonomi detayı
- [ ] **NPC borç v1'e girsin mi?** Kaldıraç iştahı için hafif versiyon. Oyun bankasından sabit faizle kredi.

### Onboarding ve UX
- [ ] **Onboarding:** Yeni oyuncu girince ilk 5 tick'te ne yapar? Tutorial var mı, yoksa "at denize" mi?
- [ ] **Oda oluşturma UI:** Preset seçimi + custom ayarlar ekranı ne gösterecek?
- [ ] **Leaderboard gösterimi:** Top 5 sıralama UI'da nasıl gösterilir (her tick güncellenir mi, oda sonunda mı reveal)?
- [ ] **Kariyer profili sayfası:** Rozet galerisi, istatistik dashboard — hangi bilgiler öne çıkar?

### NPC (ayrı oturumda)
- [ ] **NPC davranış kuralları:** Basit ucuz-al-pahalı-sat mı, yoksa farklı "kişilikler" mi (muhafazakar NPC, riskli NPC)?
- [ ] **NPC rol dağılımı:** Kaç NPC Sanayici, kaç NPC Tüccar? Preset'e göre değişir mi?
- [ ] **NPC sermaye ölçeği:** İnsan oyuncuyla eşit mi başlar, daha mı zayıf?
