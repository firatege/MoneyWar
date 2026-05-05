//! Oyuncu kimliği + rol + envanter + nakit.
//!
//! Rol v1'de `Sanayici` + `Tuccar`. v2+ rolleri (Spekulator, Banker, Kartel)
//! sonraya bırakıldı (game-design.md §5).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CityId, DomainError, Money, Personality, PlayerId, ProductKind};

/// Oyuncunun mesleği. Sezon içinde değişmez.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Role {
    /// Fabrika kurabilen tek rol. Ham → bitmiş dönüşümü Sanayici tekelidir.
    Sanayici,
    /// Büyük kapasiteli kervan + haber servisi Gümüş bedava.
    Tuccar,
}

impl Role {
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Sanayici => "Sanayici",
            Self::Tuccar => "Tüccar",
        }
    }

    /// Bu rol fabrika kurabilir mi? (Sanayici tekeli)
    #[must_use]
    pub const fn can_build_factory(self) -> bool {
        matches!(self, Self::Sanayici)
    }

    /// Yeni oyuncuların başlangıç haber tier'ı — herkes Bronze'da başlar.
    /// Free tier istenirse `:news free` komutuyla iptal edilebilir.
    /// Not: bu sadece **başlangıç** tier'ı; gerçek abonelik durumu state'te
    /// tutulur. Tier kilitlemesi `effective_news_tier`'da yapılır (artık
    /// floor yok — herkes eşit, ücret dengelemesi `tick_cost` üstünden).
    #[must_use]
    pub const fn default_news_tier(self) -> crate::NewsTier {
        crate::NewsTier::Bronze
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Oyuncunun (şehir × ürün) başına stok miktarı.
///
/// `BTreeMap` deterministik iterasyon için. Sıfır miktarlı anahtarlar depolanmaz —
/// `remove` sıfıra düşünce otomatik silinir.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    stock: BTreeMap<(CityId, ProductKind), u32>,
}

impl Inventory {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Belirli bir (şehir, ürün) kombinasyonundaki miktar. Yoksa 0.
    #[must_use]
    pub fn get(&self, city: CityId, product: ProductKind) -> u32 {
        self.stock.get(&(city, product)).copied().unwrap_or(0)
    }

    /// Stoğa ekler. Overflow = `Overflow`.
    pub fn add(&mut self, city: CityId, product: ProductKind, qty: u32) -> Result<(), DomainError> {
        if qty == 0 {
            return Ok(());
        }
        let entry = self.stock.entry((city, product)).or_insert(0);
        *entry = entry.checked_add(qty).ok_or_else(|| {
            DomainError::Overflow(format!("inventory add: {city}/{product} {entry} + {qty}"))
        })?;
        Ok(())
    }

    /// Stoktan çıkarır. Yetersiz = `InsufficientStock`.
    pub fn remove(
        &mut self,
        city: CityId,
        product: ProductKind,
        qty: u32,
    ) -> Result<(), DomainError> {
        if qty == 0 {
            return Ok(());
        }
        let key = (city, product);
        let have = self.stock.get(&key).copied().unwrap_or(0);
        if qty > have {
            return Err(DomainError::InsufficientStock {
                city,
                product,
                have,
                want: qty,
            });
        }
        let new_val = have - qty;
        if new_val == 0 {
            self.stock.remove(&key);
        } else {
            self.stock.insert(key, new_val);
        }
        Ok(())
    }

    /// Tüm envanterdeki toplam birim sayısı.
    #[must_use]
    pub fn total_units(&self) -> u64 {
        self.stock.values().map(|&v| u64::from(v)).sum()
    }

    /// (city, product, qty) tuple'ları üstünde deterministik iterasyon.
    pub fn entries(&self) -> impl Iterator<Item = (CityId, ProductKind, u32)> + '_ {
        self.stock.iter().map(|(&(c, p), &q)| (c, p, q))
    }

    /// Envanter boş mu?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stock.is_empty()
    }
}

/// NPC alt-türü. Davranış dispatch'i için kullanılır — eski versiyonda
/// `player.name.starts_with("NPC-Alıcı")` gibi kırılgan string prefix
/// kontrolü vardı; şimdi structural ayrım. İsimler artık serbestçe
/// güzelleştirilebilir ("Selim Bey") çünkü NPC tipi name'den bağımsız.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NpcKind {
    /// Akıllı tüccar — arbitraj + kervan dispatch (sıkı: BUY ucuz şehir, SELL pahalı).
    Tuccar,
    /// Akıllı sanayici — fabrika kurar, sadece kendi `raw_input`'unu alır,
    /// sadece kendi mamulü üretip satar (sıkı role gate).
    Sanayici,
    /// Tüketici (talep sink) — mamul alır, periyodik maaş gelir akışı.
    Alici,
    /// Toptancı (eski Esnaf) — Çiftçi'den ham al, Sanayici'ye/Alıcı'ya sat.
    /// Aracı katman: arz/talep dengesine duyarlı bid-ask.
    Esnaf,
    /// Spekülatör — market maker, hem bid hem ask, spread oyunu.
    Spekulator,
    /// Çiftçi (yeni v4) — hammadde üreticisi. Periyodik mahsul alır,
    /// pazara satar. SELL only — uzman: 1 ürün (Pamuk/Buğday/Zeytin).
    Ciftci,
    /// Banka (yeni v4) — likidite sağlayıcı. Kredi/mevduat akışı.
    /// Faiz dinamik: ekonomi durumuna göre %10-25.
    Banka,
}

impl NpcKind {
    /// İnsan-okunur kısa etiket — leaderboard ve panellerde.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Tuccar => "Tüccar",
            Self::Sanayici => "Sanayici",
            Self::Alici => "Alıcı",
            Self::Esnaf => "Toptancı",
            Self::Spekulator => "Spekülatör",
            Self::Ciftci => "Çiftçi",
            Self::Banka => "Banka",
        }
    }
}

/// Oyuncu (insan veya NPC).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub role: Role,
    pub cash: Money,
    pub inventory: Inventory,
    pub is_npc: bool,
    /// NPC alt-türü. İnsan oyuncuda `None`. Davranış dispatcher'ı bu
    /// field'a bakar; eski kodda string prefix vardı, kaldırıldı.
    /// `serde(default)` ile geriye uyumlu (eski save dosyaları None alır).
    #[serde(default)]
    pub npc_kind: Option<NpcKind>,
    /// NPC strateji arketipi (Aggressive/Hoarder/Arbitrageur vb).
    /// `None` → klasik MarketMaker/SmartTrader davranışı, `Some` → DSS.
    /// Sezon başında seed RNG ile atanır, sezon boyu sabit.
    #[serde(default)]
    pub personality: Option<Personality>,
    /// Sezon başında verilen başlangıç sermayesi. Skor PnL hesabında
    /// referans olarak kullanılır: `pnl_score = current_total - starting_cash`.
    /// Pasif oyuncu (hiçbir şey yapmayan) PnL=0 olur, aktif kâr edenler
    /// pozitif skor alır.
    #[serde(default)]
    pub starting_cash: Money,
}

impl Player {
    /// Yeni oyuncu. Nakit negatif olamaz (starter pack validation).
    pub fn new(
        id: PlayerId,
        name: impl Into<String>,
        role: Role,
        starting_cash: Money,
        is_npc: bool,
    ) -> Result<Self, DomainError> {
        if starting_cash.is_negative() {
            return Err(DomainError::Validation(format!(
                "starting cash cannot be negative: {starting_cash}"
            )));
        }
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::Validation("player name is empty".into()));
        }
        Ok(Self {
            id,
            name,
            role,
            cash: starting_cash,
            inventory: Inventory::new(),
            is_npc,
            npc_kind: None,
            personality: None,
            starting_cash,
        })
    }

    /// Builder-style: NPC strateji arketipini set eder.
    #[must_use]
    pub fn with_personality(mut self, personality: Personality) -> Self {
        self.personality = Some(personality);
        self
    }

    /// Builder-style: NPC alt-türünü set eder. İnsan oyuncuda çağrılmaz.
    /// Seed kodunda kullanılır — `Player::new(...).unwrap().with_kind(NpcKind::Alici)`.
    #[must_use]
    pub fn with_kind(mut self, kind: NpcKind) -> Self {
        self.npc_kind = Some(kind);
        self
    }

    /// `npc_kind` kontrolü. NPC dispatcher ve UI filter'larında kullanılır;
    /// her seferinde `p.npc_kind == Some(X)` yazımını önler.
    #[must_use]
    pub fn has_npc_kind(&self, kind: NpcKind) -> bool {
        self.npc_kind == Some(kind)
    }

    /// Nakit ekler. Overflow = hata.
    pub fn credit(&mut self, amount: Money) -> Result<(), DomainError> {
        self.cash = self.cash.checked_add(amount)?;
        Ok(())
    }

    /// Nakit çıkarır. Yetersiz = `InsufficientFunds`.
    pub fn debit(&mut self, amount: Money) -> Result<(), DomainError> {
        if amount.is_negative() {
            return Err(DomainError::Validation(format!(
                "debit amount cannot be negative: {amount}"
            )));
        }
        if self.cash < amount {
            return Err(DomainError::InsufficientFunds {
                have: self.cash,
                want: amount,
            });
        }
        self.cash = self.cash.checked_sub(amount)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_player(role: Role) -> Player {
        Player::new(
            PlayerId::new(1),
            "Ali",
            role,
            Money::from_lira(1_000).unwrap(),
            false,
        )
        .unwrap()
    }

    #[test]
    fn role_capabilities_match_design() {
        assert!(Role::Sanayici.can_build_factory());
        assert!(!Role::Tuccar.can_build_factory());

        // v3: Herkes Bronze'dan başlar. Ücret indirimi `NewsTier::tick_cost(role)`
        // üzerinden işler — Tüccar hâlâ daha az öder.
        assert_eq!(Role::Tuccar.default_news_tier(), crate::NewsTier::Bronze);
        assert_eq!(Role::Sanayici.default_news_tier(), crate::NewsTier::Bronze);
    }

    #[test]
    fn role_display_names() {
        assert_eq!(Role::Sanayici.to_string(), "Sanayici");
        assert_eq!(Role::Tuccar.to_string(), "Tüccar");
    }

    #[test]
    fn new_inventory_is_empty() {
        let inv = Inventory::new();
        assert!(inv.is_empty());
        assert_eq!(inv.total_units(), 0);
        assert_eq!(inv.get(CityId::Istanbul, ProductKind::Pamuk), 0);
    }

    #[test]
    fn inventory_add_and_get() {
        let mut inv = Inventory::new();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 100).unwrap();
        assert_eq!(inv.get(CityId::Istanbul, ProductKind::Pamuk), 100);
        assert_eq!(inv.get(CityId::Ankara, ProductKind::Pamuk), 0);
    }

    #[test]
    fn inventory_add_accumulates() {
        let mut inv = Inventory::new();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 30).unwrap();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 70).unwrap();
        assert_eq!(inv.get(CityId::Istanbul, ProductKind::Pamuk), 100);
    }

    #[test]
    fn inventory_add_zero_is_noop() {
        let mut inv = Inventory::new();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 0).unwrap();
        assert!(inv.is_empty());
    }

    #[test]
    fn inventory_add_overflow_errors() {
        let mut inv = Inventory::new();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, u32::MAX)
            .unwrap();
        let err = inv
            .add(CityId::Istanbul, ProductKind::Pamuk, 1)
            .expect_err("overflow");
        assert!(matches!(err, DomainError::Overflow(_)));
    }

    #[test]
    fn inventory_remove_zeros_out_and_deletes_key() {
        let mut inv = Inventory::new();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 50).unwrap();
        inv.remove(CityId::Istanbul, ProductKind::Pamuk, 50)
            .unwrap();
        assert!(inv.is_empty());
    }

    #[test]
    fn inventory_remove_partial_keeps_remainder() {
        let mut inv = Inventory::new();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 50).unwrap();
        inv.remove(CityId::Istanbul, ProductKind::Pamuk, 20)
            .unwrap();
        assert_eq!(inv.get(CityId::Istanbul, ProductKind::Pamuk), 30);
    }

    #[test]
    fn inventory_remove_insufficient_errors() {
        let mut inv = Inventory::new();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 10).unwrap();
        let err = inv
            .remove(CityId::Istanbul, ProductKind::Pamuk, 20)
            .expect_err("insufficient");
        match err {
            DomainError::InsufficientStock {
                have, want, city, ..
            } => {
                assert_eq!(have, 10);
                assert_eq!(want, 20);
                assert_eq!(city, CityId::Istanbul);
            }
            _ => panic!("wrong error kind"),
        }
    }

    #[test]
    fn inventory_total_units_sums_all() {
        let mut inv = Inventory::new();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 100).unwrap();
        inv.add(CityId::Ankara, ProductKind::Bugday, 200).unwrap();
        inv.add(CityId::Izmir, ProductKind::Zeytin, 50).unwrap();
        assert_eq!(inv.total_units(), 350);
    }

    #[test]
    fn inventory_entries_iterate_deterministically() {
        let mut inv = Inventory::new();
        inv.add(CityId::Izmir, ProductKind::Zeytin, 10).unwrap();
        inv.add(CityId::Istanbul, ProductKind::Pamuk, 20).unwrap();
        inv.add(CityId::Ankara, ProductKind::Bugday, 30).unwrap();

        let entries: Vec<_> = inv.entries().collect();
        // BTreeMap sıralama: CityId tanımlı sıra + ProductKind tanımlı sıra.
        assert_eq!(entries[0].0, CityId::Istanbul);
        assert_eq!(entries[1].0, CityId::Ankara);
        assert_eq!(entries[2].0, CityId::Izmir);
    }

    #[test]
    fn player_rejects_negative_starting_cash() {
        let err = Player::new(
            PlayerId::new(1),
            "Ali",
            Role::Sanayici,
            Money::from_cents(-100),
            false,
        )
        .expect_err("negative cash");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn player_rejects_empty_name() {
        let err = Player::new(PlayerId::new(1), "   ", Role::Sanayici, Money::ZERO, false)
            .expect_err("empty name");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn player_starts_with_empty_inventory() {
        let p = test_player(Role::Sanayici);
        assert!(p.inventory.is_empty());
        assert_eq!(p.cash, Money::from_lira(1_000).unwrap());
        assert!(!p.is_npc);
    }

    #[test]
    fn player_credit_adds_cash() {
        let mut p = test_player(Role::Tuccar);
        p.credit(Money::from_lira(500).unwrap()).unwrap();
        assert_eq!(p.cash, Money::from_lira(1_500).unwrap());
    }

    #[test]
    fn player_debit_subtracts_cash() {
        let mut p = test_player(Role::Tuccar);
        p.debit(Money::from_lira(300).unwrap()).unwrap();
        assert_eq!(p.cash, Money::from_lira(700).unwrap());
    }

    #[test]
    fn player_debit_insufficient_errors() {
        let mut p = test_player(Role::Tuccar);
        let err = p
            .debit(Money::from_lira(5_000).unwrap())
            .expect_err("insufficient");
        match err {
            DomainError::InsufficientFunds { have, want } => {
                assert_eq!(have, Money::from_lira(1_000).unwrap());
                assert_eq!(want, Money::from_lira(5_000).unwrap());
            }
            _ => panic!("wrong error kind"),
        }
    }

    #[test]
    fn player_debit_rejects_negative() {
        let mut p = test_player(Role::Tuccar);
        let err = p.debit(Money::from_cents(-100)).expect_err("negative");
        assert!(matches!(err, DomainError::Validation(_)));
    }

    #[test]
    fn player_serde_roundtrip() {
        let p = test_player(Role::Sanayici);
        let json = serde_json::to_string(&p).unwrap();
        let back: Player = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
