//! NPC davranış değişmezleri — sessizce bozulmasınlar diye.
//!
//! Bu oturumda bulunan hataların çoğu "kablo kopuk" tipindeydi: bir sabit
//! okunmuyordu, bir kapı hiç açılmıyordu, bir yedek yol enum sırasına
//! bakıyordu. Hepsi ölçümle bulundu çünkü hiçbiri derlemeyi bozmuyordu.
//!
//! Buradaki testler o sınıfı yakalar: davranışın **gerçekten olduğunu**
//! doğrularlar, tipinin doğru olduğunu değil. Eşikler bilinçli olarak gevşek
//! — amaç ayar değişikliğinde alarm vermek değil, özelliğin ölmesini
//! yakalamak.

use std::collections::BTreeMap;

use moneywar_domain::{NpcKind, ProductKind};
use moneywar_engine::LogEvent;
use moneywar_web::driver::SimDriver;

/// Tek sezonluk koşu — tüm testler aynı kurulumu paylaşır.
struct Season {
    driver: SimDriver,
    farms_built: u32,
    loans_taken: u32,
    caravans_dispatched: u32,
    /// Rol → (aldığı, sattığı) birim.
    trade: BTreeMap<NpcKind, (u64, u64)>,
    /// Ürün → üretilen birim.
    produced: BTreeMap<ProductKind, u64>,
}

fn run_season(ticks: u32) -> Season {
    let mut d = SimDriver::new(moneywar_web::DEFAULT_SEED, ticks, 3, moneywar_web::DIFFICULTY);
    let mut s = Season {
        farms_built: 0,
        loans_taken: 0,
        caravans_dispatched: 0,
        trade: BTreeMap::new(),
        produced: BTreeMap::new(),
        driver: SimDriver::new(moneywar_web::DEFAULT_SEED, ticks, 3, moneywar_web::DIFFICULTY),
    };

    for _ in 0..ticks {
        d.step();
        for entry in &d.last_report.entries {
            match &entry.event {
                LogEvent::PrivateFarmBuilt { .. } => s.farms_built += 1,
                LogEvent::LoanTaken { .. } => s.loans_taken += 1,
                LogEvent::CaravanDispatched { .. } => s.caravans_dispatched += 1,
                LogEvent::ProductionCompleted { product, units, .. } => {
                    *s.produced.entry(*product).or_default() += u64::from(*units);
                }
                LogEvent::OrderMatched {
                    quantity,
                    buyer,
                    seller,
                    ..
                } => {
                    let q = u64::from(*quantity);
                    if let Some(k) = d.state.players.get(buyer).and_then(|p| p.npc_kind) {
                        s.trade.entry(k).or_default().0 += q;
                    }
                    if let Some(k) = d.state.players.get(seller).and_then(|p| p.npc_kind) {
                        s.trade.entry(k).or_default().1 += q;
                    }
                }
                _ => {}
            }
        }
    }
    s.driver = d;
    s
}

// ─────────────────────────────────────────────────────────────────────────────
// Sanayici
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn private_farms_actually_get_built() {
    // Bu özellik bir kez tamamen ölmüştü: kapı 8 fabrika istiyordu ama
    // hiçbir Sanayici 5'i geçemiyordu, dolayısıyla sezon boyunca 0 tarla
    // kuruluyordu ve **tek bir red bile loglanmıyordu** (komut hiç
    // üretilmiyordu). Sessiz ölüm; ancak sayarak yakalanır.
    let s = run_season(350);
    assert!(
        s.farms_built > 0,
        "sezon boyunca hiç özel çiftlik kurulmadı — kurma kapısı yine \
         ulaşılamaz olabilir (bkz. MIN_FACTORIES_FOR_FARM)"
    );
}

#[test]
fn every_finished_good_gets_produced() {
    // Fabrika dağılımı bir dönem `FINISHED_GOODS` enum sırasının kendisiydi
    // (yedek yol `find()` ile ilk boş slotu seçiyordu), listenin sonundaki
    // Ziyafet neredeyse hiç üretilmiyordu. Her mamulün üretilmesi, dağılımın
    // sıralamaya değil ekonomiye bakmasının en basit kanıtı.
    let s = run_season(350);
    for p in ProductKind::FINISHED_GOODS {
        assert!(
            s.produced.get(&p).copied().unwrap_or(0) > 0,
            "{} sezon boyunca hiç üretilmedi",
            p.display_name()
        );
    }
}

#[test]
fn factories_are_not_left_permanently_unstaffed() {
    // Kadro kilidi: atıl fabrikanın kadrosu sıfırlanıyor ve doldurma kuralı
    // atıl fabrikaya işçi vermiyor → fabrika bir daha açılamıyor. Kilit
    // tamamen kapanırsa fabrikaların tamamı kadrosuz kalır; bu test o uç
    // hâli yakalar.
    let s = run_season(350);
    let total = s.driver.state.factories.len();
    let unstaffed = s
        .driver
        .state
        .factories
        .values()
        .filter(|f| f.employees == 0)
        .count();
    assert!(total > 0, "sezon sonunda hiç fabrika yok");
    assert!(
        unstaffed * 2 < total,
        "fabrikaların yarısından çoğu kadrosuz ({unstaffed}/{total}) — \
         kadro kilidi kapanmış olabilir"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Tüccar
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn merchant_buys_what_it_sells() {
    // Tüccar'ın sezona verilen stoğu var. Aracılık ediyor mu, yoksa yalnız
    // o stoğu mu eritiyor? Aldığı, sattığının belirgin bir kısmı olmalı.
    let s = run_season(350);
    let (bought, sold) = s.trade.get(&NpcKind::Tuccar).copied().unwrap_or((0, 0));
    assert!(sold > 0, "Tüccar hiç satmadı");
    assert!(
        bought * 10 >= sold * 6,
        "Tüccar sattığının yalnız {}%'ini almış ({bought}/{sold}) — \
         aracılık değil stok eritme",
        bought * 100 / sold.max(1)
    );
}

#[test]
fn caravans_actually_move() {
    // Kervanlar bir dönem yalnız satın alınıyordu; `DispatchCaravan` hiç
    // çıkmasa da ekonomi "çalışıyor" görünürdü. Taşıma gerçekten olmalı.
    let s = run_season(350);
    assert!(
        s.caravans_dispatched > 0,
        "sezon boyunca hiç kervan yola çıkmadı — şehirler arası taşıma ölü"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Alıcı
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn consumer_only_consumes() {
    // Alıcı tüketici sink'i: alır, tüketir, satmaz. Satmaya başlarsa
    // arz tarafına karışır ve rol ayrımı bozulur.
    let s = run_season(200);
    let (bought, sold) = s.trade.get(&NpcKind::Alici).copied().unwrap_or((0, 0));
    assert!(bought > 0, "Alıcı hiç almadı — talep tarafı ölü");
    assert_eq!(sold, 0, "Alıcı satış yaptı — tüketici sink'i olmaktan çıkmış");
}

#[test]
fn consumer_prefers_finished_goods() {
    // Alıcı ham madde tüketmez; ham alımı yapıyorsa iştah tablosu ya da
    // aday üretimi bozulmuş demektir.
    let mut d = SimDriver::new(moneywar_web::DEFAULT_SEED, 200, 3, moneywar_web::DIFFICULTY);
    let mut raw_bought = 0u64;
    for _ in 0..200 {
        d.step();
        for entry in &d.last_report.entries {
            if let LogEvent::OrderMatched {
                product,
                quantity,
                buyer,
                ..
            } = &entry.event
                && product.is_raw()
                && d.state.players.get(buyer).and_then(|p| p.npc_kind) == Some(NpcKind::Alici)
            {
                raw_bought += u64::from(*quantity);
            }
        }
    }
    assert_eq!(raw_bought, 0, "Alıcı {raw_bought} birim ham madde aldı");
}

// ─────────────────────────────────────────────────────────────────────────────
// Banka
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn bank_actually_lends() {
    // Banka bir dönem tam sıfır kazanıyordu: tek ürünü "batmak üzere olana
    // kredi" idi ve kimse batmıyordu. Sezonda hiç kredi açılmaması o ölü
    // hâlin imzası.
    // Eşik bilinçli olarak 0'dan büyük: kurtarma kredisi ara sıra tek
    // başına ateşleniyor ve `> 0` koşulu yatırım kredisi kapalıyken de
    // geçiyordu (denendi, test ısırmadı). Ölçümde yalnız kurtarma varken
    // sezon başına 0-3 kredi açılıyor, yatırım kredisiyle ~30.
    const MIN_LOANS: u32 = 10;
    let s = run_season(350);
    assert!(
        s.loans_taken >= MIN_LOANS,
        "sezonda yalnız {} kredi açıldı (en az {MIN_LOANS} bekleniyor) — \
         Banka'nın yatırım kredisi ölmüş olabilir",
        s.loans_taken
    );
}
