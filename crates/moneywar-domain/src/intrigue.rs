//! Entrika durumu — anlatı dedektörünün tick'ler arası hafızası
//! (docs/finish-plan.md Faz 1).
//!
//! Motor her tick kapanışında pazar verisinden **gözlemlenebilir gerçekleri**
//! çıkarır: kim hangi pazarı domine ediyor (tekel), kim kimin fiyatını
//! kırıyor (undercut → kampanya → fiyat savaşı), kim battı. Bu modül o
//! çıkarımın kalıcı durumunu tutar; `GameState` içinde yaşar ki
//! `advance_tick` saf fonksiyon garantisi bozulmasın (aynı state + komutlar
//! → bit-perfect aynı sonuç).
//!
//! NPC beyinleri de buradan okur: tekelci sömürü fiyatına geçer, kinli firma
//! düşmanına savaş açar. Yani aynı veri hem anlatının hem davranışın kaynağı.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{CityId, PlayerId, ProductKind, Tick};

/// Satış payı bu pencere (tick) üzerinden hesaplanır.
pub const DOMINANCE_WINDOW_TICKS: u32 = 20;
/// Pencerede bu kadar birim eşleşmemişse pay hesabı yapılmaz (gürültü tekeli olmasın).
pub const DOMINANCE_MIN_VOLUME: u64 = 30;
/// Tekel ilanı eşiği (%). Pay bunun üstüne çıkınca aday olunur.
pub const MONOPOLY_FORM_PCT: u64 = 60;
/// Tekel düşme eşiği (%). Histerezis: kurulan tekel ancak bunun altında kırılır.
pub const MONOPOLY_BREAK_PCT: u64 = 50;
/// Tekel ilanı için eşiğin üstünde kesintisiz geçirilmesi gereken tick.
/// Tek tick'lik pay dalgalanması hikâye değildir — saltanat süreklilik ister.
pub const MONOPOLY_CONFIRM_TICKS: u32 = 6;
/// Tekelin kırıldığının ilanı için eşiğin altında kesintisiz geçen tick.
pub const MONOPOLY_BREAK_CONFIRM_TICKS: u32 = 6;
/// Art arda bu kadar tick fiyat kırma = `UndercutCampaign` + kin.
pub const UNDERCUT_CAMPAIGN_TICKS: u32 = 3;
/// Kampanya bu kadar tick sürerse savaş ilan edilmiş sayılır (`PriceWarDeclared`).
pub const PRICE_WAR_DECLARE_TICKS: u32 = 5;
/// Savaş sırasında mağdur bu kadar tick pazara SELL koymazsa çekilmiş sayılır
/// (`PriceWarWon`).
pub const PRICE_WAR_RETREAT_TICKS: u32 = 5;
/// Saldırgan bu kadar tick kırmayı bırakırsa savaş sessizce söner (olay yok).
pub const PRICE_WAR_FIZZLE_TICKS: u32 = 8;
/// Kinin ömrü (tick). Her tick 1 azalır, 0'da unutulur.
pub const GRUDGE_TICKS: u32 = 40;

/// Bir pazarın (şehir, ürün) tek tick'lik satış dökümü: satıcı → birim.
pub type TickSales = BTreeMap<PlayerId, u64>;

/// Aktif fiyat savaşının dedektör durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceWarTrack {
    /// Savaşın ilan edildiği tick (`PriceWarDeclared` anı).
    pub declared_at: Tick,
    /// Mağdurun pazara SELL koymadığı ardışık tick sayısı.
    pub victim_absent_ticks: u32,
    /// Saldırganın kırmayı bıraktığı ardışık tick sayısı.
    pub attacker_idle_ticks: u32,
}

/// Anlatı dedektörünün kalıcı durumu. `GameState.intrigue` olarak yaşar.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntrigueState {
    /// Kayan satış penceresi: pazar → [(tick, satıcı → birim)].
    /// `DOMINANCE_WINDOW_TICKS`'ten eski girişler her tick budanır.
    pub sales_window: BTreeMap<(CityId, ProductKind), Vec<(Tick, TickSales)>>,

    /// Aktif tekeller: pazar → tekelci firma.
    pub monopolist: BTreeMap<(CityId, ProductKind), PlayerId>,

    /// Tekel adayı: pazar → (aday firma, eşik üstünde kesintisiz tick).
    /// `MONOPOLY_CONFIRM_TICKS`'e ulaşınca tekel ilan edilir. Aday değişirse
    /// sayaç sıfırlanır.
    pub monopoly_candidate: BTreeMap<(CityId, ProductKind), (PlayerId, u32)>,

    /// Kırılma adayı: pazar → tekelcinin eşik altında geçirdiği kesintisiz tick.
    pub monopoly_decay: BTreeMap<(CityId, ProductKind), u32>,

    /// Manşete çıkmış tekeller. Bir pazarın **tek** üreticisi olmak tekel
    /// statüsü verir (fiyat primi + haritada taç) ama haber değildir; haber
    /// olan, rakibi olan bir pazarı ele geçirmektir. Kırılma haberi de
    /// yalnız ilan edilmiş tekeller için verilir.
    pub announced_monopolies: BTreeSet<(CityId, ProductKind)>,

    /// Undercut serileri: (saldırgan, mağdur, şehir, ürün) → ardışık tick.
    /// `UNDERCUT_CAMPAIGN_TICKS`'e ulaşınca kampanya damgalanır.
    pub undercut_streak: BTreeMap<(PlayerId, PlayerId, CityId, ProductKind), u32>,

    /// Aktif fiyat savaşları: (saldırgan, mağdur, şehir, ürün) → takip durumu.
    pub price_wars: BTreeMap<(PlayerId, PlayerId, CityId, ProductKind), PriceWarTrack>,

    /// Kinler: (kin tutan, hedef) → kalan tick. Her tick 1 azalır.
    pub grudges: BTreeMap<(PlayerId, PlayerId), u32>,

    /// İflası ilan edilmiş firmalar (sezonda bir kez damgalanır).
    pub bankrupt: BTreeSet<PlayerId>,
}

impl IntrigueState {
    /// Bu firma bu pazarda tekelci mi?
    #[must_use]
    pub fn is_monopolist(&self, player: PlayerId, city: CityId, product: ProductKind) -> bool {
        self.monopolist.get(&(city, product)) == Some(&player)
    }

    /// Bu pazarda `player` DIŞINDA bir tekelci var mı? (Fırsatçı giriş sinyali.)
    #[must_use]
    pub fn rival_monopolist(
        &self,
        player: PlayerId,
        city: CityId,
        product: ProductKind,
    ) -> Option<PlayerId> {
        self.monopolist
            .get(&(city, product))
            .copied()
            .filter(|m| *m != player)
    }

    /// `holder`'ın en taze kini (kalan tick'i en yüksek hedef).
    #[must_use]
    pub fn strongest_grudge_of(&self, holder: PlayerId) -> Option<PlayerId> {
        self.grudges
            .iter()
            .filter(|((h, _), _)| *h == holder)
            .max_by_key(|((_, against), remaining)| (**remaining, *against))
            .map(|((_, against), _)| *against)
    }

    /// Pencere içindeki toplam hacim ve satıcı payları (birim).
    /// Dönen harita satıcı → birim; toplam ayrıca döner.
    #[must_use]
    pub fn window_shares(&self, city: CityId, product: ProductKind) -> (TickSales, u64) {
        let mut merged = TickSales::new();
        let mut total = 0_u64;
        if let Some(entries) = self.sales_window.get(&(city, product)) {
            for (_, sales) in entries {
                for (seller, qty) in sales {
                    *merged.entry(*seller).or_default() += qty;
                    total += qty;
                }
            }
        }
        (merged, total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(v: u64) -> PlayerId {
        PlayerId::new(v)
    }

    #[test]
    fn window_shares_merges_ticks() {
        let mut s = IntrigueState::default();
        let key = (CityId::Istanbul, ProductKind::Un);
        s.sales_window.insert(
            key,
            vec![
                (Tick::new(1), TickSales::from([(pid(1), 10), (pid(2), 5)])),
                (Tick::new(2), TickSales::from([(pid(1), 20)])),
            ],
        );
        let (shares, total) = s.window_shares(key.0, key.1);
        assert_eq!(total, 35);
        assert_eq!(shares[&pid(1)], 30);
        assert_eq!(shares[&pid(2)], 5);
    }

    #[test]
    fn strongest_grudge_prefers_freshest() {
        let mut s = IntrigueState::default();
        s.grudges.insert((pid(1), pid(2)), 10);
        s.grudges.insert((pid(1), pid(3)), 30);
        s.grudges.insert((pid(9), pid(4)), 99); // başkasının kini
        assert_eq!(s.strongest_grudge_of(pid(1)), Some(pid(3)));
        assert_eq!(s.strongest_grudge_of(pid(5)), None);
    }

    #[test]
    fn rival_monopolist_excludes_self() {
        let mut s = IntrigueState::default();
        let key = (CityId::Ankara, ProductKind::Kumas);
        s.monopolist.insert(key, pid(7));
        assert_eq!(s.rival_monopolist(pid(1), key.0, key.1), Some(pid(7)));
        assert_eq!(s.rival_monopolist(pid(7), key.0, key.1), None);
        assert!(s.is_monopolist(pid(7), key.0, key.1));
    }
}
