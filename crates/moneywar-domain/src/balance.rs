//! 🎛️ **Oyun dengesi** — tek yerden ayarla.
//!
//! Bu dosya, motoru etkileyen tüm **sayısal** parametreleri `pub const`
//! olarak merkezileştirir. Bir değeri değiştirip `cargo build` ile yeniden
//! derlediğinizde tüm motor o değere göre çalışır. Deterministik kalır —
//! `GameState` içinde saklanmaz, derleme zamanı sabitidir.
//!
//! # Bölümler
//!
//! | Bölüm | İçerik |
//! |---|---|
//! | [Zaman](#zaman) | Fabrika üretim süresi, batch boyutu |
//! | [Fabrika](#fabrika) | Kurulum maliyet tablosu (§10) |
//! | [Kervan](#kervan) | Rol bazlı kapasite + maliyet |
//! | [Piyasa](#piyasa) | Doygunluk eşiği formülü (§10) |
//! | [Haber](#haber) | Tier ücretleri + lead-time (§6) |
//! | [Olay](#olay-motoru) | Olasılık, severity (§6) |
//! | [Kredi](#kredi) | NPC faizi (§7) |
//! | [Skor](#skor) | Atıl eşiği, rolling avg penceresi (§9) |
//! | [Mesafe](#ehir-mesafeleri) | Şehirler arası tick (§3) |
//! | [Bozulma](#bozulma) | Perishability kuralları (§4) |
//! | [NPC](#npc-likidite) | `MarketMaker` baz fiyatları |
//!
//! # Balance workflow
//!
//! 1. Parametreyi bu dosyada değiştir.
//! 2. `cargo test --workspace` — invariantlar ve integration testler
//!    yeni denge ile hala geçiyor mu?
//! 3. `cargo run -p moneywar-cli` — tam sezon simüle et, leaderboard'a bak.
//! 4. Anlamlı değişim = commit.
//!
//! # Dinamik config (v2 opsiyonu)
//!
//! v2'de oda-başı farklı denge isteniyorsa bu sabitler bir `GameBalance`
//! struct'ına taşınıp `GameState`'e eklenebilir. Şimdilik derleme zamanı
//! sabit — basit ve determinism için güvenli.

// =============================================================================
// Zaman
// =============================================================================

/// Bir batch'te tüketilen **ana girdi** miktarı — seviye 1, katman 1 fabrika
/// için taban değer (§10).
///
/// Gerçek batch boyutu bundan türer: `Factory::batch_size()` seviye çarpanını
/// (1× / 1.5× / 2×) ve [`ProductKind::batch_scale_pct`] katman ölçeğini
/// (tier 0-1 %100, tier 2 %60, tier 3 %40) uygular. Ek girdiler tarifteki
/// yüzdeye göre ayrıca tüketilir.
///
/// Tuning geçmişi: 10 → 100 (hacim 10× ölçeklendi), 100 → 50 (Çiftçi
/// kazandı ama Sanayici marjı eridi), 50 → 65 (Sanayici ROI dengesi).
///
/// [`ProductKind::batch_scale_pct`]: crate::ProductKind::batch_scale_pct
pub const FACTORY_BATCH_SIZE: u32 = 65;

/// **Reel üretim ölçeği** (yüzde). Fabrika batch'i, hasat ve özel çiftlik
/// çıktısının üçünü birden çarpar. 100 = bugünkü seviye.
///
/// # Neden tek bir ölçek
///
/// Ekonomi kalıcı kıtlıkta: emir defterinde talep arzın 3-22 katı. Bu
/// "fiyat ne olsun" oyununu "mal var mı" oyununa çeviriyor — kim önce
/// davranırsa kapıyor, fabrika girdisini tüketiciye kaptırıyor, zincirin
/// tepesi hiç dolmuyor (Ziyafet %13 karşılama).
///
/// Bu ölçek **yalnız malı** büyütür, parayı değil. Para arzı sabit kaldığı
/// için mal paraya göre bollaşır: fiyat düşer, bağlayıcı kısıt "stokta
/// var mı"dan "ne kadar ödemeye razısın"a kayar. Amaç kıtlığı yok etmek
/// değil, **nadirleştirmek** — asıl olay arz/talep dalgalanması olsun.
///
/// Ölçeği tek sabitte tutmak şart: üçü ayrı ayrı büyütülürse zincirin bir
/// halkası ötekini besleyemez hale gelir.
///
/// # Durum: kaldıraç bağlı ama henüz kullanılabilir değil
///
/// Ölçek büyütmenin **izlenebilirliği gerçekten düzelttiği** ölçüldü —
/// 350 tick, eşleşme büyüklüğü dağılımı:
///
/// ```text
///                  ×100    ×1000
///   1 birim       %15.3     %6.8
///   10-24 birim   %26.8    %33.7
///   Sanayici doluluk %29     %60
/// ```
///
/// Alıcı'nın emir **sayısı** ölçekle artmıyor (41.996 → 41.371): tavan
/// şehir × ürün ile sabit, ölçekle ilgisi yok.
///
/// Ölçeği kaldırmak için üç sızıntı kapatıldı ve hepsi gerçek hataydı
/// (ölçek 100'de zaten etkisizler):
///   1. depolama gideri birim başına mutlaktı → ölçekle çarpılıyordu
///      (×1000'de 480.904₺, para arzı -35.9%)
///   2. tohum maliyeti hasat miktarıyla çarpılıyordu → aynı sorun
///   3. Alıcı'nın istediği miktar 10'da sabitti → ölçek büyüdükçe tüketici
///      yuvarlama hatasına dönüşüyordu
///
/// Bunlardan sonra para arzı ×1000'de bile -6.9%'da kaldı. Ama denge hâlâ
/// bozuluyor: fiyat de ölçekle bölününce (world.rs) bu sefer üretici
/// eziliyor (×1000'de Sanayici -53K, Alıcı +64K).
///
/// Kalan engel: ekonomide **onlarca mutlak lira sabiti** var — ücret,
/// fabrika kurulum maliyeti, kredi anaparası, iflas eşiği, başlangıç
/// nakdi. Gerçek bir birim değişimi hepsini birlikte ölçeklemeyi
/// gerektiriyor; biri atlanınca denge kayıyor. Bu, tek sabitle çözülecek
/// bir iş değil, para birimini baştan tanımlamak.
pub const PRODUCTION_SCALE_PCT: u32 = 100;

/// Ölçek uygulanmış miktar. Sıfıra yuvarlamayı önler.
#[must_use]
pub const fn scaled_output(qty: u32) -> u32 {
    let out = qty * PRODUCTION_SCALE_PCT / 100;
    if out == 0 { 1 } else { out }
}

// =============================================================================
// Emek
// =============================================================================

/// Dünyadaki toplam işgücü. Firmalar bu havuzdan işçi tutar; havuz bitince
/// yeni fabrika kadro bulamaz ve eksik kadroyla düşük üretir.
///
/// **Kalibrasyon (2026-07-26).** 110, mevcut dünyada (~38 fabrika × 3 kişi =
/// 114 kadro ihtiyacı) emeği *ucu ucuna* kıt yapar. Daha dar havuzlar
/// ölçüldü ve pahalıya mal oluyor:
///
/// | havuz | adalet makası | girdisiz deneme | Sanayici `PnL` |
/// |---|---|---|---|
/// |  50 | 5.6× | %59 | 80K |
/// |  80 | 4.1× | %64 | 90K |
/// | 110 | 3.3× | %69 | 96K |
///
/// Sebep: oyunun bağlayıcı kısıtı emek değil **girdi**. Fabrikaların çoğu
/// zaten hammadde bulamıyor; üstüne emek kıtlığı koymak üretimi kısıyor ama
/// darboğazı çözmüyor. Havuz, girdi kıtlığı azaldıkça daraltılabilir —
/// o zaman emek gerçek bir tercih baskısı yaratır.
pub const LABOR_POOL_SIZE: u32 = 110;

/// Seviye 1 fabrikanın tam kadrosu. Seviye çarpanı [`FACTORY_BATCH_SIZE`]
/// ile aynı yönde: 1× / 1.5× / 2× → 3 / 4 / 6.
pub const EMPLOYEES_PER_FACTORY_L1: u32 = 3;
/// Seviye 2 tam kadro.
pub const EMPLOYEES_PER_FACTORY_L2: u32 = 4;
/// Seviye 3+ tam kadro.
pub const EMPLOYEES_PER_FACTORY_L3: u32 = 6;

/// Çalışan başına ücret (lira), `WAGE_PERIOD` tick'te bir ödenir.
///
/// Eski model **aktif fabrika** başına 300₺ ödüyordu (~20 aktif fab →
/// 6.000₺/periyot). Kadro modelinde ücret tüm çalışanlara ödenir, atıl
/// fabrikanınkine de — 114 kişi × 100₺ = 11.400₺ ile fatura neredeyse
/// ikiye katlanıyordu ve Sanayici `PnL`'i 102K → 80K'ya düşüyordu.
/// 60₺ eski toplam yükü gerçekten korur.
pub const WAGE_PER_EMPLOYEE_LIRA: i64 = 60;

/// Ücretin reel üretim ölçeğine göre düzeltilmiş hali.
///
/// [`PRODUCTION_SCALE_PCT`] yalnız malı büyütür. Ücret sabit kalırsa hane
/// halkı 2× malı aynı parayla almak zorunda kalır ve batar — ölçümde
/// ölçek ×200'de Alıcı -12K'dan -75K'ya düşmüştü. İşgücü havuzu
/// [`LABOR_POOL_SIZE`] ile sabit olduğu için istihdam da büyüyemiyor;
/// tek ayar noktası ücretin kendisi.
///
/// Ücret transferdir, basım değil: Sanayici öder, Alıcı alır. Para arzı
/// değişmez, yalnız üretimle tüketim aynı ölçekte kalır.
#[must_use]
pub const fn wage_per_employee_lira() -> i64 {
    WAGE_PER_EMPLOYEE_LIRA * PRODUCTION_SCALE_PCT as i64 / 100
}

/// Üreticinin bir batch'ten beklediği asgari kâr marjı (yüzde). Girdi
/// bütçesi = mamul geliri × `(100 − bu)`.
///
/// [`MIN_PRODUCTION_MARGIN_PCT`] ile birlikte çalışır ve ikisi tutarlı
/// olmak zorundadır (bkz. o sabitin dokümanı).
pub const FACTORY_TARGET_MARGIN_PCT: i64 = 30;

/// Mamul baseline'ının tarif maliyetinin üstünde tutulacağı asgari pay.
///
/// **Hedef marjdan türer, bağımsız seçilemez.** Üreticinin girdi bütçesi
/// `fiyat × (100 − FACTORY_TARGET_MARGIN_PCT)/100`; bu bütçenin maliyeti
/// karşılaması için fiyatın en az `maliyet / (1 − hedef_marj)` olması gerekir.
///
/// Bu ilişki kurulmadan önce taban %20, hedef marj %30 idi ve bileşke
/// `1.20 × 0.70 = 0.84` veriyordu: üretici kendi girdisinin ancak %84'ünü
/// karşılayabiliyor, yani **matematiksel olarak her zaman düşük teklif
/// vermek zorunda** kalıyordu. Ölçüm: Ziyafet fabrikası Şarap'a 94₺ verirken
/// tüketici 106₺ veriyor ve malın %83'ünü alıyordu.
///
/// `EXTRA` payı üreticiye rakibini geçebilecek küçük bir pay bırakır;
/// tam eşitlikte tavan piyasa fiyatına oturur, öne geçemez.
#[must_use]
pub const fn min_production_margin_pct() -> i64 {
    const EXTRA: i64 = 5;
    // fiyat = maliyet / (1 − t)  →  marj = t / (1 − t)
    FACTORY_TARGET_MARGIN_PCT * 100 / (100 - FACTORY_TARGET_MARGIN_PCT) + EXTRA
}

/// Batch başlatıldıktan kaç tick sonra biter (§4).
/// Eski yolculuk: 2 → 3 (Sanayici aşırı kârlı diye yavaşlatıldı), şimdi
/// 3 → 2 (NPC Sanayici sezon boyu hammadde bulamayıp 321/sezon `FactoryIdle`
/// veriyordu, %50 batch artışı dengeye getirir; Tüccar arbitrajı zaten
/// hacim 10× ölçekten kazandığı için fark Tüccar lehine değil).
pub const FACTORY_PRODUCTION_TICKS: u32 = 2;

// =============================================================================
// Fabrika
// =============================================================================

/// Kurulum maliyet tablosu — `existing_count` index'i ile oku.
///
/// İlk fabrika bedava, sonrakiler sabit 8K. Gittikçe zorlaşmıyor — sadece
/// nakit kısıtı sınır, sınırsız fabrika mümkün.
pub const FACTORY_BUILD_COSTS_LIRA: [i64; 2] = [
    0,      // 1. bedava
    8_000,  // 2.+ sabit 8K
];

// =============================================================================
// Kervan
// =============================================================================

/// Sanayici kervanı kapasitesi (§10).
/// 20 → 200 → 500: Sanayici fab kurmuş ham/mamul taşıma için kervan kullanır;
/// 200 yetersizdi (Tüccar 1200 ile karşılaştırıldığında 6× az). 500 ile fab
/// destekleyici lojistik makul, Tüccar yine 2.4× büyük (rol farkı korunur).
pub const CARAVAN_CAPACITY_SANAYICI: u32 = 500;

/// Tüccar kervanı kapasitesi — daha büyük (§10).
/// 50 → 500: hacim 10× ölçek revizyonu.
/// Faz F4: 500 → 800. Yeni tedarik zincirinde Tüccar şehirler arası mal
/// taşıyor (off-specialty bucket'ları besler). 4 Tüccar × 3 caravan ×
/// 800 = 9600 birim/dispatch, ölü bucket'lara daha çok arz akışı.
/// Faz F5: 800 → 1200 (Tüccar buff — Sanayici tekeli karşı denge).
pub const CARAVAN_CAPACITY_TUCCAR: u32 = 1080; // 1200→1080: %10 kısıtlama, arbitraj dengesi

/// Sanayici kervan maliyet tablosu (§10).
pub const CARAVAN_COSTS_SANAYICI_LIRA: [i64; 3] = [0, 5_000, 10_000];

/// Tüccar kervan maliyeti — sabit, gittikçe zorlaşmıyor.
/// 1. bedava, sonrakiler sabit 4K. Sınırsız kervan mümkün.
pub const CARAVAN_COSTS_TUCCAR_LIRA: [i64; 2] = [0, 4_000];

// =============================================================================
// Piyasa
// =============================================================================

/// Doygunluk eşiği formülü (§10):
/// `threshold = SATURATION_BASE + (player_count - SATURATION_MIN_PLAYERS) × SATURATION_PER_PLAYER`.
///
/// Eski değerler 40+10/oyuncu → 12 oyuncu için 140 birim. Pratikte tek bucket'ta
/// 140 birim eşleşme nadir → eşik tetiklenmiyordu (ölü kod). Şu an 25+5/oyuncu →
/// 12 oyuncu için 75 — tetiklenebilir, anti-snowball mekanizması canlı.
/// Faz 1: 250 → 2000. Dominant üretici tam fiyat alabilsin; monopol dinamiği
/// yapay olarak frenlenmemeli. Anti-snowball mekanizması fiilen devre dışı.
pub const SATURATION_BASE: u32 = 2000;
/// Her ek oyuncu başına doygunluk eşiği artışı.
/// 5 → 50: hacim 10× ölçek revizyonu.
pub const SATURATION_PER_PLAYER: u32 = 50;
/// Doygunluk formülünün alt sınırı (bu sayının altında formül devreye girmez).
pub const SATURATION_MIN_PLAYERS: u8 = 2;

/// İşlem vergisi (yüzde) — her settle'da alıcıdan **ek** olarak kesilir,
/// sistem dışına atılır (hard sink). EVE Online "broker fee + sales tax"
/// karşılığı. Closed-loop ekonomide tek gerçek para imha kanalı.
/// Kapatmak için 0 yap.
pub const TRANSACTION_TAX_PCT: i64 = 2;

/// Clearing fiyat clamp alt sınırı — taban fiyatın yüzdesi (Vic3 §market).
/// Fiyat tabanın %10'unun altına inemez. Ani arz patlamasında manyak dipleri
/// engeller, ama mevcut Walras tâtonnement baseline'ı kaydırırken clamp'ı
/// dar tutmamalı. v0.5.1: %25 → %10 (NPC fiyatlama'nın %50-200 marjını boğmasın).
pub const PRICE_CLAMP_LOW_PCT: i64 = 10;

/// Clearing fiyat clamp üst sınırı — taban fiyatın yüzdesi.
/// Faz 1: %500 → %1000. Fiyat savaşı + monopol dinamiği için daha geniş bant;
/// bir firma piyasayı köşeye sıkıştırırsa fiyat gerçek talebi yansıtsın.
pub const PRICE_CLAMP_HIGH_PCT: i64 = 1000;

// =============================================================================
// Haber (4-tier abonelik, recurring tick fee)
// =============================================================================
//
// 4 tier: Free < Bronze < Silver < Gold. Free herkese bedava (varsayılan).
// Tüccar her tier'da indirimli — bilgi onun mesleği. Bronze tüm Tüccar'lara
// bedava; daha üstü ucuz. Tüm ücretler **tick başına** kesilir; oyuncu cash'i
// yetmezse 1 tick uyarı, sonraki tick yine yetmezse Free'ye düşer.

/// Free — bedava, sadece "var/yok" + rolling avg.
pub const NEWS_TICK_COST_FREE_LIRA: i64 = 0;
/// Bronz — kategorik (yok/az/orta/bol) + ask/bid bandı.
pub const NEWS_TICK_COST_BRONZE_LIRA: i64 = 5;
/// Gümüş — 5'e yuvarlı miktar + ask/bid 5 kuruşa yuvarlı.
pub const NEWS_TICK_COST_SILVER_LIRA: i64 = 15;
/// Altın — tam veri + tüm olay haberleri.
pub const NEWS_TICK_COST_GOLD_LIRA: i64 = 40;

/// Tüccar — Bronze hafif indirimli (rol avantajı korunur ama bedava değil).
pub const NEWS_TICK_COST_BRONZE_TUCCAR_LIRA: i64 = 2;
/// Tüccar — Gümüş indirimli.
pub const NEWS_TICK_COST_SILVER_TUCCAR_LIRA: i64 = 5;
/// Tüccar — Altın indirimli.
pub const NEWS_TICK_COST_GOLD_TUCCAR_LIRA: i64 = 15;

/// Free: olay haberi yok (sürpriz şoklar).
pub const NEWS_LEAD_FREE: u32 = 0;
/// Bronz: olay tick'inde duyurulur.
pub const NEWS_LEAD_BRONZE: u32 = 0;
/// Gümüş: 1 tick önceden.
pub const NEWS_LEAD_SILVER: u32 = 1;
/// Altın: 2 tick önceden.
pub const NEWS_LEAD_GOLD: u32 = 2;

// =============================================================================
// Olay motoru
// =============================================================================

/// Motor olayı `current_tick + EVENT_LEAD_TICKS`'e zamanlar.
/// Minimum `NEWS_LEAD_GOLD` olmalı — Altın aboneler lead-time görebilsin.
pub const EVENT_LEAD_TICKS: u32 = 2;

/// Erken sezon (< %50 progress) olay olasılığı (yüzde).
/// v8.25: User "şoklar daha çok ve üst üste binebilmeli" — sezon başı %8 → %12.
pub const EVENT_PROB_EARLY_PCT: u32 = 12;
/// Mid sezon (%50–80) olasılığı. v8.25: 10 → 18.
pub const EVENT_PROB_MID_PCT: u32 = 18;
/// Geç sezon (%80+) olasılığı — makro şok penceresi. v8.25: 20 → 28.
pub const EVENT_PROB_LATE_PCT: u32 = 28;

/// Severity → fiyat şok yüzdeleri (§6, motor Faz 12'de kullanır).
pub const SHOCK_MINOR_PCT: u32 = 8;
pub const SHOCK_MAJOR_PCT: u32 = 18;
pub const SHOCK_MACRO_PCT: u32 = 35;

// =============================================================================
// Kredi
// =============================================================================

/// NPC bankasının sabit faiz oranı (§7, basit yüzde).
pub const LOAN_INTEREST_RATE_PERCENT: u32 = 15;

/// Ham madde başlangıç fiyatı çarpanı (yüzde). 100 = bugünkü seviye.
///
/// # Neden ayrı bir çarpan
///
/// Ham madde baseline'ları `world::seed_baselines` içinde elle gömülüydü
/// (uzmanlık şehri 4₺, Buğday 9₺, diğerleri 7₺) ve `base_price_lira()`
/// ham ürünler için hiç okunmuyordu — ham fiyatını değiştirmeye çalışmak
/// ekonomiyi zerre etkilemiyordu (dört farklı seviye birebir aynı sonucu
/// verdi, kablonun kopuk olduğu böyle çıktı).
///
/// # Ne işe yarıyor
///
/// Marj zincir boyunca ters: tier-1 %70-82, tier-3 %37. Sebep ham maddenin
/// mamule göre çok ucuz olması — dip bedavaya kâr ediyor, NPC rasyonel
/// davranıp oraya fabrika kuruyor, zincirin tepesi aç kalıyor.
///
/// Merdiveni **tepeden** düzeltmek denendi ve dengeyi yıktı: mamul fiyatını
/// yükseltmek değeri tüketiciden üreticiye aktarıyordu. Bu çarpan aynı
/// merdiveni **dipten** düzeltir — mamul fiyatına dokunmaz, dolayısıyla
/// Alıcı'nın harcaması sabit kalır.
///
/// # Denendi: işe yaramadı, 100'de bırakıldı
///
/// 30 oyun × 350 tick:
///
/// ```text
///        açlık  makas  Tüccar  Çiftçi  Sanayici  Spekül  Alıcı
/// ×100     %55   3.1x  239983  135247     91488   89721  -11967
/// ×150     %56   2.9x  208676  148276     80723   82591  -16790
/// ×200     %60   3.7x  184285  158212     66279   73180  -30587
/// ×250     %62   3.9x  155923  162271     54439   66395  -41905
/// ```
///
/// Girdi açlığı **düşmedi, arttı**. Yani "marj ters olduğu için NPC dibe
/// fabrika kuruyor, tepe aç kalıyor" hipotezi yanlış. Gerçek darboğaz
/// rekabet: fabrika ara malını tüketiciye kaptırıyor (Alıcı Un'un %81'ini
/// yiyor, bkz. `chain_probe`). Girdiyi pahalılaştırmak fabrikanın alım
/// gücünü daha da kısıyor.
///
/// Kaldıraç yerinde duruyor — artık gerçekten bağlı ve ölçülebilir; ama
/// bu sorunun cevabı burada değil, fabrikanın tüketiciye karşı teklif
/// gücünde (`derived_input_ceiling`).
pub const RAW_BASELINE_MULT_PCT: i64 = 100;

// =============================================================================
// Skor
// =============================================================================

/// Atıl fabrika eşiği (§9): son bu kadar tick'te üretim yoksa skora 0.
pub const IDLE_FACTORY_THRESHOLD: u32 = 10;

/// Rolling avg fiyat penceresi (§9) — tek-tick manipülasyonunu öldürür.
pub const PRICE_WINDOW: usize = 5;

/// Fabrika sermayesi skor oranı (§9): `build_cost × NUM / DEN`.
/// v8.24: 1/2 → 3/4 (yatırımın %50 → %75'i skora döner).
/// User: "Sanayici'ler tüm paralarını fab'a atıyor, scoring'de ceza
/// görüyorlar". %50'de Sanayici 15K fab kurarsa 7.5K direkt kayıp →
/// mamul kâr edemezse Sanayici hep dipte. %75 ile yatırımın çoğu skor
/// olur, Sanayici daha rekabetçi (kazanç hâlâ mamul satışından gelir).
pub const FACTORY_SCORE_NUM: i64 = 3;
pub const FACTORY_SCORE_DEN: i64 = 4;

// =============================================================================
// Şehir mesafeleri
// =============================================================================

/// İstanbul ↔ Ankara — v3'te yarıya indi (3→2 tick).
pub const DIST_ISTANBUL_ANKARA: u32 = 2;
/// Ankara ↔ İzmir — en yakın çift, hâlâ 1 tick.
pub const DIST_ANKARA_IZMIR: u32 = 1;
/// İstanbul ↔ İzmir — deniz yolu (4→2 tick).
pub const DIST_ISTANBUL_IZMIR: u32 = 2;

// =============================================================================
// Bozulma
// =============================================================================

/// Un: bu kadar tick sonra fire başlar. v3'te yumuşatıldı (3→5, %100→%50)
/// — kervan en uzun rotası (İst↔İzm = 4 tick) varış sırasında kayıpsız geçsin
/// ama "Un'u uzun süre cebinde tutarsan zarar" mekaniği kalsın.
pub const PERISH_UN_TICKS: u32 = 5;
pub const PERISH_UN_LOSS_PCT: u32 = 50;

/// Zeytinyağı: 5 tick sonra %10 fire.
pub const PERISH_ZEYTINYAGI_TICKS: u32 = 5;
pub const PERISH_ZEYTINYAGI_LOSS_PCT: u32 = 10;

// =============================================================================
// Çiftçi maliyeti
// =============================================================================

/// NPC emirlerinin varsayılan TTL'i — kaç tick boyunca kitapta yaşar.
/// Pay-as-bid + tick-shuffle borsa motorunda continuous matching tek tick'te
/// fill yapamayan emirleri kitapta tutmak gerek (ikinci/üçüncü tick'teki
/// yeni emirlere karşı eşleşsinler). TTL=1 → tek tick yaşam, çoğu emir
/// boşa düşer; TTL=3 → match verim 3× artar (yeni borsa modeline uygun).
pub const NPC_DEFAULT_ORDER_TTL: u32 = 3;

/// Çiftçi'nin her birim mahsul için ödediği tohum/işçilik maliyeti (lira).
/// Vic3 ilhamı: subsistence farms bile gübre/tohum tüketir.
/// `MoneyWar`'da Çiftçi sıfır maliyetle mahsul basıyor → açık faucet.
/// Bu sabit ile her `HARVEST_PERIOD` tick'te `qty × SEED_COST_PER_RAW_LIRA`
/// Çiftçi cebinden çıkar. Para yetmezse mahsul orantılı azalır.
/// Hedef: ham fiyatın ~%30'u → tipik mahsul fiyatı 4₺ ise 1₺ tohum maliyeti.
/// Çiftçi başlangıç cash 8K → ~50 tick yetecek runway.
pub const SEED_COST_PER_RAW_LIRA: i64 = 1;

// =============================================================================
// NPC likidite
// =============================================================================

/// `MarketMaker` baz fiyat — ham madde (§10: 5-8₺ tipik).
pub const NPC_BASE_PRICE_RAW_LIRA: i64 = 6;
/// `MarketMaker` baz fiyat — bitmiş ürün (§10: 12-18₺ tipik).
/// Eski 15 → 18: `production_tick=3` ile Sanayici marjı yetmez, sezon sonu
/// negatif `PnL`. Marj 9 → 12 (%200) ile Sanayici dengeye gelir.
pub const NPC_BASE_PRICE_FINISHED_LIRA: i64 = 18;

/// `MarketMaker` markup — stok varsa bu yüzdeyle satar (base × 1.1).
pub const NPC_SELL_MARKUP_PCT: i64 = 110;
/// `MarketMaker` markdown — nakit varsa bu yüzdeyle alır (base × 0.9).
pub const NPC_BUY_MARKDOWN_PCT: i64 = 90;

/// NPC `OrderId` ofseti — insan oyuncu havuzu ile çakışmasın.
pub const NPC_ORDER_ID_OFFSET: u64 = 10_000_000_000;

// =============================================================================
// World Fabrikaları (v8.11)
// =============================================================================

/// "Dünya Üretim" pseudo-player — engine-driven baseline mamul üretici.
/// Sanayici NPC fab dağılımı 9 mamul bucket'ı kapsayamadığında (5 NPC × 1-2
/// fab = 6-7 fab) 2-3 bucket fab'sız kalıyordu → talep var arz yok ölü pazar.
/// World Fab her tick periyodik olarak her (city, mamul) için küçük arz koyar
/// → 9/9 mamul bucket garanti.
pub const WORLD_PLAYER_ID_VALUE: u64 = 0;

/// World Fab'ı kaç tickte bir üretim yapar.
/// 2 → 5: Sanayici 12 fab ile slotların %80'ini kaplıyor, World çok agresif
/// rekabet ediyordu (820 fill vs Sanayici 447). Frekans düşürüldü.
pub const WORLD_FAB_PERIOD: u32 = 5;

/// World Fab her periyotta her (city, mamul) için bu kadar birim üretir.
/// Sanayici NPC fab batch=100/3tick ≈ 33/tick. World fab daha küçük (15)
/// → "baseline güvence" rolü, NPC fab'larını rakip etmez.
pub const WORLD_FAB_QTY_PER_PERIOD: u32 = 15;

/// World Fab SELL emir TTL — 3 tick (kısa, sürekli yenilenir).
pub const WORLD_FAB_SELL_TTL: u32 = 3;

// =============================================================================
// Özel çiftlik (PrivateFarm)
// =============================================================================

/// Özel çiftlik kurulum maliyeti (lira). Fabrika maliyetinden ucuz —
/// ham madde üretimi düşük teknoloji.
pub const PRIVATE_FARM_BUILD_COST_LIRA: i64 = 15_000;

/// Özel çiftlik tick başına üretim (birim ham madde), seviye 1.
///
/// Bu sabit uzun süre **okunmuyordu**: `PrivateFarm::output_per_tick()`
/// içinde 20/35/55 gömülüydü. Kablo bağlanınca değer gerçek bir kaldıraç
/// oldu ve 20'nin fazla olduğu ölçüldü — tarla ham maddeyi pazarı atlayarak
/// ürettiği için Çiftçi'nin müşterisini ve Spekülatör'ün stok değerini
/// siliyor. Debi 20'de Spekülatör'ün üçü de batıyordu.
///
/// 8'de denge duruyor: girdi açlığı %69 → %54, rol makası 2.9× (hedef <3×),
/// bütün kâr rolleri pozitif. Sabitin kendi tarifi de zaten bunu diyordu —
/// "piyasa Çiftçisi ~8-12/tick, özel çiftlik biraz daha az".
pub const PRIVATE_FARM_OUTPUT_PER_TICK: u32 = 8;

/// Sanayici başına max özel çiftlik sayısı.
///
/// 6'ydı. On Sanayici × 6 tarla, pazarı atlayan büyük bir ham madde akışı
/// demek; ölçümde Çiftçi ve Spekülatör'ü çökertiyordu. 2'de dikey
/// bütünleşme hâlâ mümkün ama pazarın yerini almıyor.
pub const PRIVATE_FARM_MAX_PER_OWNER: usize = 2;

// =============================================================================
// Fabrika yükseltme
// =============================================================================

/// Seviye 1→2 yükseltme maliyeti (lira). Yüksek — sadece kârlı fabrika yükseltilmeli.
pub const FACTORY_UPGRADE_LV2_LIRA: i64 = 25_000;
/// Seviye 2→3 yükseltme maliyeti (lira).
pub const FACTORY_UPGRADE_LV3_LIRA: i64 = 50_000;
/// Maksimum fabrika seviyesi.
pub const FACTORY_MAX_LEVEL: u8 = 3;

#[cfg(test)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn factory_cost_table_has_entries() {
        assert_eq!(FACTORY_BUILD_COSTS_LIRA.len(), 2); // sabit maliyet: bedava + 8K
        assert_eq!(FACTORY_BUILD_COSTS_LIRA[0], 0); // starter bedava
        assert!(FACTORY_BUILD_COSTS_LIRA.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn caravan_cost_tables_monotonic_non_decreasing() {
        for w in CARAVAN_COSTS_SANAYICI_LIRA.windows(2) {
            assert!(w[0] <= w[1]);
        }
        for w in CARAVAN_COSTS_TUCCAR_LIRA.windows(2) {
            assert!(w[0] <= w[1]);
        }
    }

    #[test]
    fn tuccar_caravan_capacity_exceeds_sanayici() {
        assert!(CARAVAN_CAPACITY_TUCCAR > CARAVAN_CAPACITY_SANAYICI);
    }

    #[test]
    fn news_tick_costs_are_monotonic() {
        assert_eq!(NEWS_TICK_COST_FREE_LIRA, 0);
        assert!(NEWS_TICK_COST_FREE_LIRA < NEWS_TICK_COST_BRONZE_LIRA);
        assert!(NEWS_TICK_COST_BRONZE_LIRA < NEWS_TICK_COST_SILVER_LIRA);
        assert!(NEWS_TICK_COST_SILVER_LIRA < NEWS_TICK_COST_GOLD_LIRA);
    }

    #[test]
    fn tuccar_news_costs_are_discounted() {
        // Tüccar her tier'da normalden ucuz (rol avantajı korunur).
        assert!(NEWS_TICK_COST_BRONZE_TUCCAR_LIRA < NEWS_TICK_COST_BRONZE_LIRA);
        assert!(NEWS_TICK_COST_SILVER_TUCCAR_LIRA < NEWS_TICK_COST_SILVER_LIRA);
        assert!(NEWS_TICK_COST_GOLD_TUCCAR_LIRA < NEWS_TICK_COST_GOLD_LIRA);
    }

    #[test]
    fn news_leads_are_monotonic() {
        assert_eq!(NEWS_LEAD_FREE, 0);
        // BRONZE şu an 0 (Free ile aynı tier ücreti farklı, lead aynı). Mono
        // invariant ileride Bronze>0 olursa bozulmasın diye kontrol et.
        #[allow(clippy::absurd_extreme_comparisons)]
        {
            assert!(NEWS_LEAD_BRONZE <= NEWS_LEAD_SILVER);
        }
        assert!(NEWS_LEAD_SILVER < NEWS_LEAD_GOLD);
    }

    #[test]
    fn event_lead_covers_max_news_lead() {
        // Altın aboneler `event_tick - NEWS_LEAD_GOLD` görür; underflow olmasın.
        assert!(EVENT_LEAD_TICKS >= NEWS_LEAD_GOLD);
    }

    #[test]
    fn event_probabilities_ascend_with_season() {
        assert!(EVENT_PROB_EARLY_PCT < EVENT_PROB_MID_PCT);
        assert!(EVENT_PROB_MID_PCT < EVENT_PROB_LATE_PCT);
    }

    #[test]
    fn shock_percentages_scale_with_severity() {
        assert!(SHOCK_MINOR_PCT < SHOCK_MAJOR_PCT);
        assert!(SHOCK_MAJOR_PCT < SHOCK_MACRO_PCT);
    }

    #[test]
    fn loan_interest_is_positive_and_reasonable() {
        // Not harici kural: 0 < faiz < %100 (yıkıcı oran olmasın).
        assert!(LOAN_INTEREST_RATE_PERCENT > 0);
        assert!(LOAN_INTEREST_RATE_PERCENT < 100);
    }

    #[test]
    fn factory_score_ratio_is_three_quarters() {
        // v8.24: %50 → %75 (Sanayici yatırım kayıp cezası azaldı).
        assert_eq!(FACTORY_SCORE_NUM * 4, FACTORY_SCORE_DEN * 3);
    }

    #[test]
    fn npc_markup_above_markdown() {
        assert!(NPC_SELL_MARKUP_PCT > NPC_BUY_MARKDOWN_PCT);
    }
}
