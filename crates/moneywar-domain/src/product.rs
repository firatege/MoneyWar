//! Ürün kataloğu ve üretim zinciri (docs/finish-plan.md Faz 2).
//!
//! Zincir 4 katmanlıdır. Derinlik entrikanın malzemesidir: çok girdili ürün
//! **boğaz noktası** demektir — Boya pazarını tutan firma, elbise üreten
//! herkesin gırtlağını sıkar. Fabrikan olması yetmez, girdin de olmalı.
//!
//! | Kat | Ürün | Girdiler |
//! |-----|------|----------|
//! | 0 (ham) | Pamuk, Buğday, Zeytin, Boya, Üzüm | — (mahsul) |
//! | 1 | Kumaş, Un, Zeytinyağı, Şarap | tek ham |
//! | 2 | Elbise, Ekmek | iki girdi |
//! | 3 | Ziyafet Sofrası | üç girdi |
//!
//! Üst katman daha kârlı ama daha kırılgan: üç ayrı tedarik zinciri aynı
//! anda ayakta olmalı.
//!
//! **Üretim modeli:** her mamulün bir *ana girdisi* (`raw_input`) ve isteğe
//! bağlı *ek girdileri* (`extra_inputs`) vardır. Batch boyutu ana girdiden
//! belirlenir; ek girdiler batch'in yüzdesi kadar tüketilir. Tek girdili
//! eski ürünlerin (Kumaş/Un/Zeytinyağı) davranışı bu modelde birebir aynıdır.
//!
//! Bozulma (§4):
//! - Un, Ekmek: hızlı bozulur (%100 kayıp)
//! - Zeytinyağı, Ziyafet: kısmi fire
//! - Diğerleri: dayanıklı

use serde::{Deserialize, Serialize};

/// 12 ürün çeşidi — 5 ham, 7 üretilen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ProductKind {
    // Ham maddeler (mahsul — fabrika değil, tarla üretir)
    Pamuk,
    Bugday,
    Zeytin,
    Boya,
    Uzum,
    // Katman 1 — tek ham girdi
    Kumas,
    Un,
    Zeytinyagi,
    Sarap,
    // Katman 2 — iki girdi
    Elbise,
    Ekmek,
    // Katman 3 — üç girdi, sezonun amiral gemisi
    Ziyafet,
}

/// Bir ürünün sınıfı: ham ya da bitmiş.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProductClass {
    Raw,
    Finished,
}

/// Bozulma kuralı. `loss_percent == 100` = ürün tamamen yok olur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Perishability {
    /// Kaç tick depoda beklerse bozulma tetiklenir.
    pub after_ticks: u32,
    /// Kayıp yüzdesi (0-100). 100 = tamamen yok olur.
    pub loss_percent: u32,
}

impl ProductKind {
    /// Tüm ürünler (deterministik sıra: ham → katman 1 → 2 → 3).
    pub const ALL: [Self; 12] = [
        Self::Pamuk,
        Self::Bugday,
        Self::Zeytin,
        Self::Boya,
        Self::Uzum,
        Self::Kumas,
        Self::Un,
        Self::Zeytinyagi,
        Self::Sarap,
        Self::Elbise,
        Self::Ekmek,
        Self::Ziyafet,
    ];

    /// Ham madde listesi — tarlada üretilir, fabrikada değil.
    pub const RAW_MATERIALS: [Self; 5] = [
        Self::Pamuk,
        Self::Bugday,
        Self::Zeytin,
        Self::Boya,
        Self::Uzum,
    ];

    /// Fabrikada üretilen ürünler (tüm katmanlar).
    pub const FINISHED_GOODS: [Self; 7] = [
        Self::Kumas,
        Self::Un,
        Self::Zeytinyagi,
        Self::Sarap,
        Self::Elbise,
        Self::Ekmek,
        Self::Ziyafet,
    ];

    /// Ürünün sınıfı.
    #[must_use]
    pub const fn class(self) -> ProductClass {
        match self {
            Self::Pamuk | Self::Bugday | Self::Zeytin | Self::Boya | Self::Uzum => {
                ProductClass::Raw
            }
            Self::Kumas
            | Self::Un
            | Self::Zeytinyagi
            | Self::Sarap
            | Self::Elbise
            | Self::Ekmek
            | Self::Ziyafet => ProductClass::Finished,
        }
    }

    /// Üretim zincirindeki derinlik: 0 ham, 1-3 işlenmiş katmanlar.
    /// Fiyatlandırma, talep ağırlığı ve UI gruplaması bunu kullanır.
    #[must_use]
    pub const fn tier(self) -> u8 {
        match self {
            Self::Pamuk | Self::Bugday | Self::Zeytin | Self::Boya | Self::Uzum => 0,
            Self::Kumas | Self::Un | Self::Zeytinyagi | Self::Sarap => 1,
            Self::Elbise | Self::Ekmek => 2,
            Self::Ziyafet => 3,
        }
    }

    /// Katmana göre batch ölçeği (yüzde) — üretim piramidinin eğimi.
    ///
    /// Batch boyutu her katmanda aynı olursa piramit ters durur: bir tier-1
    /// fabrika 2 tickte 58 Un üretirken tier-2 Ekmek fabrikası 65 Un tüketir,
    /// yani üst katman alt katmandan **daha hızlı** yer. Ölçüm: Ziyafet
    /// fabrikaları sezonda 4 birim üretebiliyordu, üretim denemelerinin
    /// %83'ü girdisiz kalıyordu ve açlık tier-2/3'te yoğunlaşıyordu
    /// (Ekmek bulamayan Ziyafet, Un bulamayan Ekmek).
    ///
    /// Üst katman daha küçük partiler hâlinde üretilir: bir Un fabrikası
    /// birden çok Ekmek fabrikasını, bir Ekmek fabrikası birden çok Ziyafet
    /// fabrikasını besleyebilsin. Lüks mal zaten unla aynı hacimde üretilmez.
    #[must_use]
    pub const fn batch_scale_pct(self) -> u32 {
        match self.tier() {
            0 | 1 => 100,
            2 => 60,
            _ => 40,
        }
    }

    /// Ana girdinin yanında gereken **ek girdiler**: (ürün, batch'in yüzdesi).
    /// Boş ise tek girdili klasik üretim. Bir tanesi bile eksikse fabrika
    /// girdi açlığına düşer — çok girdili ürünün kırılganlığı buradan gelir.
    #[must_use]
    pub const fn extra_inputs(self) -> &'static [(Self, u32)] {
        match self {
            // Elbise: kumaşı boyamadan elbise olmaz.
            Self::Elbise => &[(Self::Boya, 40)],
            // Ekmek: un + yağ.
            Self::Ekmek => &[(Self::Zeytinyagi, 20)],
            // Ziyafet Sofrası: ekmek + şarap + yağ.
            Self::Ziyafet => &[(Self::Sarap, 50), (Self::Zeytinyagi, 30)],
            _ => &[],
        }
    }

    /// Tam tarif: ana girdi (batch'in %100'ü) + ek girdiler. UI ve NPC
    /// tedarik planlaması bunu okur.
    #[must_use]
    pub fn recipe(self) -> Vec<(Self, u32)> {
        let mut out = Vec::new();
        if let Some(primary) = self.raw_input() {
            out.push((primary, 100));
        }
        out.extend_from_slice(self.extra_inputs());
        out
    }

    #[must_use]
    pub const fn is_raw(self) -> bool {
        matches!(self.class(), ProductClass::Raw)
    }

    #[must_use]
    pub const fn is_finished(self) -> bool {
        matches!(self.class(), ProductClass::Finished)
    }

    /// Bu girdinin beslediği bir üst katman ürünü. Zincirin sonu için `None`.
    /// Çok girdili ürünlerde yalnız **ana girdi** için tanımlıdır.
    #[must_use]
    pub const fn finished_output(self) -> Option<Self> {
        match self {
            Self::Pamuk => Some(Self::Kumas),
            Self::Bugday => Some(Self::Un),
            Self::Zeytin => Some(Self::Zeytinyagi),
            Self::Uzum => Some(Self::Sarap),
            Self::Kumas => Some(Self::Elbise),
            Self::Un => Some(Self::Ekmek),
            Self::Ekmek => Some(Self::Ziyafet),
            _ => None,
        }
    }

    /// Bu ürünün **ana girdisi** — batch boyutunu belirleyen girdi.
    /// Katman 2+ ürünlerde bu bir ham madde değil, alt katman mamulüdür.
    /// Ham maddeler ve girdisiz ürünler için `None`.
    #[must_use]
    pub const fn raw_input(self) -> Option<Self> {
        match self {
            Self::Kumas => Some(Self::Pamuk),
            Self::Un => Some(Self::Bugday),
            Self::Zeytinyagi => Some(Self::Zeytin),
            Self::Sarap => Some(Self::Uzum),
            Self::Elbise => Some(Self::Kumas),
            Self::Ekmek => Some(Self::Un),
            Self::Ziyafet => Some(Self::Ekmek),
            _ => None,
        }
    }

    /// Bozulma kuralı. Dayanıklı ürünler için `None`.
    #[must_use]
    pub const fn perishability(self) -> Option<Perishability> {
        match self {
            Self::Un => Some(Perishability {
                after_ticks: 3,
                loss_percent: 100,
            }),
            Self::Zeytinyagi => Some(Perishability {
                after_ticks: 5,
                loss_percent: 10,
            }),
            // Ekmek unun kaderini paylaşır — bayatlar.
            Self::Ekmek => Some(Perishability {
                after_ticks: 4,
                loss_percent: 100,
            }),
            // Hazır sofra beklemez; şarap ise tam tersine dayanıklıdır.
            Self::Ziyafet => Some(Perishability {
                after_ticks: 3,
                loss_percent: 50,
            }),
            _ => None,
        }
    }

    /// v0.4.1: Ham → mamul dönüşüm verim yüzdesi. Mamul (`is_finished`) için
    /// 100 ham birim → bu sayı kadar mamul. Reel sanayide kayıp var.
    /// - `Kumas` (Pamuk): %80 — dokumacılık fire (kumaş kenar, parça vb)
    /// - `Un` (Buğday): %90 — değirmen az kayıp
    /// - `Zeytinyagi` (Zeytin): %50 — sıkım sonrası posa atılır, yarı verim
    ///
    /// Ham ürünler ve geçerli olmayan üretim için tam verim (identity, no-op).
    #[must_use]
    pub const fn output_ratio_pct(self) -> u32 {
        match self {
            Self::Kumas => 80,
            Self::Un => 90,
            Self::Zeytinyagi => 40,
            Self::Sarap => 60,   // mayalanma firesi
            // Katman 2-3: montaj işi, fire az — değer girdilerin birleşmesinden gelir.
            Self::Elbise => 85,
            Self::Ekmek => 95,
            Self::Ziyafet => 90,
            _ => 100,
        }
    }

    /// Alıcı tüketim hızı (yüzde/periyot). Her tüketim periyodunda Alıcı
    /// stoğunun bu yüzdesi silinir. Gerçek kullanım hızını yansıtır:
    /// - `Un`: %20 — temel gıda, günlük tüketim
    /// - `Kumas`: %12 — mevsimlik, yavaş yıpranır
    /// - `Zeytinyagi`: %7 — lüks, az tüketilir
    ///
    /// Ham ürünler için sıfır — Alıcı ham tüketmez.
    #[must_use]
    pub const fn alici_consume_pct(self) -> u32 {
        match self {
            Self::Un => 20,       // temel gıda, hızlı tüketim
            Self::Kumas => 15,    // eski CONSUME_PCT — stok baskısı Sanayici'yi eziyor
            Self::Zeytinyagi => 8, // lüks ama çok düşük olunca stok birikti
            Self::Sarap => 10,
            // Üst katman: az ama iştahlı talep — kıtlık primi burada doğar.
            Self::Elbise => 18,
            Self::Ekmek => 25,
            Self::Ziyafet => 30,
            _ => 0,
        }
    }

    /// v0.4.1: Mamul üretim süresi (tick). Fab batch başlatıldıktan kaç tick
    /// sonra tamamlanır. Reel sanayide farklı: değirmen hızlı, dokuma yavaş.
    /// - `Un`: 2 (değirmen — mevcut default)
    /// - `Zeytinyagi`: 3 (sıkım orta süre)
    /// - `Kumas`: 4 (dokumacılık yavaş)
    ///
    /// Ham ürünler için sıfır — üretim mekaniği yok, sadece mahsul.
    #[must_use]
    pub const fn production_ticks(self) -> u32 {
        match self {
            Self::Un => 2,
            Self::Zeytinyagi => 3,
            Self::Kumas => 4,
            Self::Sarap => 4,
            // Katman 2-3 montajı hızlıdır; zorluk tedarikte, sürede değil.
            Self::Elbise => 3,
            Self::Ekmek => 2,
            Self::Ziyafet => 2,
            _ => 0,
        }
    }

    /// v0.4.1: Ürün başına baseline fiyat (₺). Verim ve süre farklılaştığı
    /// için baseline'lar da farklılaştı:
    /// - Un: 22 (verim %90 → bol arz → ucuz)
    /// - Kumaş: 35 (verim %80 → orta)
    /// - Zeytinyağı: 65 (verim %50 + 3 tick → kıtlık primi)
    /// - Ham (Pamuk/Buğday/Zeytin): 5 (default ham fiyatı)
    #[must_use]
    pub const fn base_price_lira(self) -> i64 {
        match self {
            Self::Un => 22,
            Self::Kumas => 35,
            Self::Zeytinyagi => 65,
            Self::Sarap => 45,
            // Katman 2-3: girdilerinin toplamı + montaj primi. Üst katman
            // daha kârlı ama üç tedarik zinciri birden ayakta olmalı.
            Self::Elbise => 90,
            Self::Ekmek => 60,
            Self::Ziyafet => 180,
            // Ham ürünler — eski NPC_BASE_PRICE_RAW_LIRA değeri (5)
            Self::Pamuk | Self::Bugday | Self::Zeytin => 5,
            // Boya kimyasal, üzüm bağ işi — hammadde ama daha değerli.
            Self::Boya => 12,
            Self::Uzum => 8,
        }
    }

    /// Ürün kısa adı (UI + log).
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Pamuk => "Pamuk",
            Self::Bugday => "Buğday",
            Self::Zeytin => "Zeytin",
            Self::Kumas => "Kumaş",
            Self::Un => "Un",
            Self::Zeytinyagi => "Zeytinyağı",
            Self::Boya => "Boya",
            Self::Uzum => "Üzüm",
            Self::Sarap => "Şarap",
            Self::Elbise => "Elbise",
            Self::Ekmek => "Ekmek",
            Self::Ziyafet => "Ziyafet Sofrası",
        }
    }
}

impl std::fmt::Display for ProductKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_every_variant_once() {
        assert_eq!(ProductKind::ALL.len(), 12);
        let mut seen: Vec<ProductKind> = ProductKind::ALL.to_vec();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ProductKind::ALL.len(), "ALL tekrar içermemeli");
        assert_eq!(
            ProductKind::RAW_MATERIALS.len() + ProductKind::FINISHED_GOODS.len(),
            ProductKind::ALL.len(),
            "ham + mamul tüm kataloğu kapsamalı"
        );
    }

    #[test]
    fn raw_and_finished_partition_correctly() {
        assert_eq!(ProductKind::RAW_MATERIALS.len(), 5);
        assert_eq!(ProductKind::FINISHED_GOODS.len(), 7);
        for raw in ProductKind::RAW_MATERIALS {
            assert!(raw.is_raw(), "{raw:?} should be raw");
            assert!(!raw.is_finished());
        }
        for finished in ProductKind::FINISHED_GOODS {
            assert!(finished.is_finished(), "{finished:?} should be finished");
            assert!(!finished.is_raw());
        }
    }

    #[test]
    fn production_chains_are_bijective() {
        assert_eq!(
            ProductKind::Pamuk.finished_output(),
            Some(ProductKind::Kumas)
        );
        assert_eq!(ProductKind::Bugday.finished_output(), Some(ProductKind::Un));
        assert_eq!(
            ProductKind::Zeytin.finished_output(),
            Some(ProductKind::Zeytinyagi)
        );

        assert_eq!(ProductKind::Kumas.raw_input(), Some(ProductKind::Pamuk));
        assert_eq!(ProductKind::Un.raw_input(), Some(ProductKind::Bugday));
        assert_eq!(
            ProductKind::Zeytinyagi.raw_input(),
            Some(ProductKind::Zeytin)
        );
    }

    #[test]
    fn raw_has_no_raw_input() {
        assert!(ProductKind::Pamuk.raw_input().is_none());
        assert!(ProductKind::Bugday.raw_input().is_none());
        assert!(ProductKind::Zeytin.raw_input().is_none());
    }

    #[test]
    fn chain_terminates_at_top_tier() {
        // Zincirin ucu: Ziyafet hiçbir şeyin girdisi değil.
        assert!(ProductKind::Ziyafet.finished_output().is_none());
        assert!(ProductKind::Elbise.finished_output().is_none());
    }

    #[test]
    fn recipe_lists_primary_then_extras() {
        // Tek girdili katman 1: sadece ana girdi.
        assert_eq!(ProductKind::Un.recipe(), vec![(ProductKind::Bugday, 100)]);
        // Katman 2: ana girdi + bir ek.
        assert_eq!(
            ProductKind::Elbise.recipe(),
            vec![(ProductKind::Kumas, 100), (ProductKind::Boya, 40)]
        );
        // Katman 3: üç parça.
        assert_eq!(ProductKind::Ziyafet.recipe().len(), 3);
        // Ham maddenin tarifi yok.
        assert!(ProductKind::Pamuk.recipe().is_empty());
    }

    #[test]
    fn every_recipe_input_is_a_real_product_of_lower_tier() {
        for product in ProductKind::FINISHED_GOODS {
            let recipe = product.recipe();
            assert!(!recipe.is_empty(), "{product:?} girdisiz üretilemez");
            for (input, pct) in recipe {
                assert!(pct > 0, "{product:?} girdisi {input:?} sıfır oranlı");
                assert!(
                    input.tier() < product.tier(),
                    "{product:?} (kat {}) kendi katmanından girdi alamaz: {input:?} (kat {})",
                    product.tier(),
                    input.tier()
                );
            }
        }
    }

    #[test]
    fn un_fully_perishes_after_3_ticks() {
        let p = ProductKind::Un.perishability().unwrap();
        assert_eq!(p.after_ticks, 3);
        assert_eq!(p.loss_percent, 100);
    }

    #[test]
    fn zeytinyagi_partially_perishes_after_5_ticks() {
        let p = ProductKind::Zeytinyagi.perishability().unwrap();
        assert_eq!(p.after_ticks, 5);
        assert_eq!(p.loss_percent, 10);
    }

    #[test]
    fn durable_products_have_no_perishability() {
        assert!(ProductKind::Pamuk.perishability().is_none());
        assert!(ProductKind::Bugday.perishability().is_none());
        assert!(ProductKind::Zeytin.perishability().is_none());
        assert!(ProductKind::Kumas.perishability().is_none());
    }

    #[test]
    fn display_name_uses_turkish_characters() {
        assert_eq!(ProductKind::Kumas.to_string(), "Kumaş");
        assert_eq!(ProductKind::Bugday.to_string(), "Buğday");
        assert_eq!(ProductKind::Zeytinyagi.to_string(), "Zeytinyağı");
    }

    #[test]
    fn serde_roundtrip_via_variant_name() {
        let p = ProductKind::Pamuk;
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(json, "\"Pamuk\"");
        let back: ProductKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn ordering_is_stable_for_btreemap_keys() {
        let mut v = vec![
            ProductKind::Un,
            ProductKind::Pamuk,
            ProductKind::Zeytinyagi,
            ProductKind::Bugday,
        ];
        v.sort();
        // Enum definition order dictates Ord: Pamuk < Bugday < Zeytin < Kumas < Un < Zeytinyagi
        assert_eq!(
            v,
            vec![
                ProductKind::Pamuk,
                ProductKind::Bugday,
                ProductKind::Un,
                ProductKind::Zeytinyagi,
            ]
        );
    }
}
