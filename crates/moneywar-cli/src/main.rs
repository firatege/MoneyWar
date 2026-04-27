//! MoneyWar — terminal izleyici / playtest TUI.
//!
//! Ratatui + crossterm. Tek ekran, 4 panel + alt bar.
//!
//! # Tuşlar
//!
//! - `Space` — bir tick ilerlet (NPC komutları otomatik).
//! - `t`      — auto-sim aç/kapa (her 300ms tick).
//! - `q`      — çık.

// CLI prototipi — pedantic lint'leri gevşek tut.
#![allow(
    clippy::uninlined_format_args,
    clippy::trivially_copy_pass_by_ref,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::missing_errors_doc,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::unused_self,
    clippy::similar_names,
    clippy::doc_markdown,
    clippy::if_not_else,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::too_many_arguments,
    clippy::comparison_chain,
    clippy::unnested_or_patterns,
    clippy::unnecessary_wraps,
    clippy::single_match_else,
    clippy::single_match,
    clippy::enum_glob_use
)]

use std::io::{self, BufWriter, Write};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use moneywar_domain::{
    CityId, Command, GameBalance, GameState, MarketOrder, Money, NewsItem, NewsTier, NpcKind,
    OrderId, OrderSide, Player, PlayerId, ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::{LogEvent, PlayerScore, advance_tick, leaderboard, rng_for, score_player};
use moneywar_npc::{Difficulty, decide_all_npcs};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Row, Table, Wrap};

type Term = Terminal<CrosstermBackend<io::Stdout>>;

/// İnsan oyuncunun ID'si (default test oyuncusu).
const HUMAN_ID: PlayerId = PlayerId::new(1);

/// Son tick olaylarını gösterirken kaç entry tutulsun.
const LOG_WINDOW: usize = 12;

/// Son haberleri gösterirken kaç kayıt tutulsun.
const NEWS_WINDOW: usize = 8;

/// `./moneywar.toml`'u okur. Yoksa veya invalid ise default döner.
/// Dönüş: (balance, yüklendi_mi) — yüklendiyse UI'da gösterilir.
fn load_balance_config() -> (GameBalance, bool) {
    let path = std::path::Path::new("moneywar.toml");
    if !path.exists() {
        return (GameBalance::default_const(), false);
    }
    let Ok(raw) = std::fs::read_to_string(path) else {
        return (GameBalance::default_const(), false);
    };

    #[derive(serde::Deserialize, Default)]
    struct FileRoot {
        #[serde(default)]
        balance: GameBalance,
    }
    match toml::from_str::<FileRoot>(&raw) {
        Ok(root) => match root.balance.validate() {
            Ok(()) => (root.balance, true),
            Err(_) => (GameBalance::default_const(), false),
        },
        Err(_) => (GameBalance::default_const(), false),
    }
}

fn main() -> Result<()> {
    let (balance, loaded) = load_balance_config();
    let mut app = App::new(balance, loaded);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, &mut app);

    // Teardown önce, sonra hata raporla.
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

fn run_app(terminal: &mut Term, app: &mut App) -> Result<()> {
    let mut last_auto_tick = Instant::now();

    loop {
        terminal.draw(|f| render(f, app))?;

        // Auto-sim yalnız Normal mode'da ve overlay yokken.
        if app.auto_sim
            && matches!(app.mode, Mode::Normal)
            && last_auto_tick.elapsed() >= Duration::from_millis(300)
        {
            app.step_one_tick();
            last_auto_tick = Instant::now();
        }

        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if handle_key(app, key.code)? {
                    return Ok(());
                }
            }
        }

        if app.game_over() {
            app.auto_sim = false;
            // Normal mode'dayken otomatik GameOver'a geç — oyun zaten bitmiş.
            if matches!(app.mode, Mode::Normal) {
                app.mode = Mode::GameOver;
            }
        }
    }
}

/// Tuşu mod'a göre işle. Dönüş: `true` → çık.
fn handle_key(app: &mut App, code: KeyCode) -> Result<bool> {
    match app.mode.clone() {
        Mode::Startup => match code {
            // Yalnız `q` kapatır — Esc yanlışlıkla çıkış yapmasın.
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('1') => app.start_game(Role::Sanayici),
            KeyCode::Char('2') => app.start_game(Role::Tuccar),
            KeyCode::Char('p') => {
                app.selected_preset = app.selected_preset.next();
            }
            KeyCode::Char('d') => {
                app.difficulty = app.difficulty.next();
            }
            // İsim input — alfabetik + boşluk + Türkçe karakter, max 20 char.
            // Rakamlar role seçimine ayrılmış, isimde olmaz; özel karakter
            // temiz ekran için filtrelenir.
            KeyCode::Char(c) if (c.is_alphabetic() || c == ' ') => {
                if app.player_name_input.chars().count() < 20 {
                    app.player_name_input.push(c);
                }
            }
            KeyCode::Backspace => {
                app.player_name_input.pop();
            }
            _ => {}
        },
        Mode::Normal => match code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char(' ') => app.step_one_tick(),
            KeyCode::Char(':') => {
                app.mode = Mode::Command {
                    buffer: String::new(),
                };
            }
            KeyCode::Char('?') => app.mode = Mode::Help,
            KeyCode::Char('i') => app.mode = Mode::Info,
            KeyCode::Char('m') => app.mode = Mode::Holdings,
            KeyCode::Char('r') => app.mode = Mode::MarketIntel,
            KeyCode::Char('e') => app.mode = Mode::RecentTrades,
            KeyCode::Char('k') => app.mode = Mode::OpenBook,
            KeyCode::Char('v') => app.mode = Mode::CaravanPanel,
            KeyCode::Char('g') => app.mode = Mode::DebugLog { scroll: 0 },
            // Wizard kısayolları — tek tuşla aksiyon menüsü.
            KeyCode::Char('b') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Buy),
                };
            }
            KeyCode::Char('s') => {
                // s: SAT — `b` (al) ile simetrik, küçük harf.
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Sell),
                };
            }
            KeyCode::Char('f') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Build),
                };
            }
            KeyCode::Char('c') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Caravan),
                };
            }
            KeyCode::Char('d') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Ship),
                };
            }
            KeyCode::Char('l') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Loan),
                };
            }
            KeyCode::Char('R') => {
                // Repay (büyük R) — küçük `r` artık MarketIntel raporuna ayrıldı.
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Repay),
                };
            }
            KeyCode::Char('N') => {
                // Büyük N: HABER abonelik (küçük 'n' news inbox için).
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::News),
                };
            }
            KeyCode::Char('x') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Cancel),
                };
            }
            KeyCode::Char('o') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Offer),
                };
            }
            KeyCode::Char('a') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Accept),
                };
            }
            KeyCode::Char('w') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Withdraw),
                };
            }
            // News inbox / haber penceresi: küçük n (kısayol çakışmasın
            // diye haber ABONELİK büyük N'e taşındı).
            KeyCode::Char('n') => app.mode = Mode::NewsInbox,
            // Auto-sim — `t` (tick/time). `s` satış wizard'ına ayrıldı.
            KeyCode::Char('t') => {
                app.auto_sim = !app.auto_sim;
                app.set_status_info(if app.auto_sim {
                    "Auto-sim AÇIK (t ile kapat)"
                } else {
                    "Auto-sim kapalı"
                });
            }
            _ => {}
        },
        Mode::Command { mut buffer } => match code {
            KeyCode::Esc => app.mode = Mode::Normal,
            KeyCode::Enter => {
                let line = buffer.trim().to_string();
                app.mode = Mode::Normal;
                if line.is_empty() {
                    return Ok(false);
                }
                match parse_command(app, &line) {
                    Ok(cmd) => {
                        let label = describe_command(&cmd);
                        app.pending_human_cmds.push(cmd);
                        app.set_status_ok(format!("→ {label}  (SPACE ile tick ilerlet)"));
                    }
                    Err(msg) => app.set_status_err(format!("Hata: {msg}")),
                }
            }
            KeyCode::Backspace => {
                buffer.pop();
                app.mode = Mode::Command { buffer };
            }
            KeyCode::Char(c) => {
                buffer.push(c);
                app.mode = Mode::Command { buffer };
            }
            _ => app.mode = Mode::Command { buffer },
        },
        Mode::Help
        | Mode::Info
        | Mode::Holdings
        | Mode::NewsInbox
        | Mode::RecentTrades
        | Mode::OpenBook
        | Mode::CaravanPanel
        | Mode::MarketIntel => match code {
            // Overlay'i kapat — Esc burada güvenli (oyundan çıkmaz, panel kapanır).
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                app.mode = Mode::Normal;
            }
            _ => {}
        },
        Mode::DebugLog { scroll } => {
            let max_scroll = app.debug_log.len().saturating_sub(1);
            match code {
                KeyCode::Esc | KeyCode::Enter => app.mode = Mode::Normal,
                KeyCode::Down | KeyCode::Char('j') => {
                    app.mode = Mode::DebugLog {
                        scroll: (scroll + 1).min(max_scroll),
                    };
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.mode = Mode::DebugLog {
                        scroll: scroll.saturating_sub(1),
                    };
                }
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    app.mode = Mode::DebugLog {
                        scroll: (scroll + 20).min(max_scroll),
                    };
                }
                KeyCode::PageUp => {
                    app.mode = Mode::DebugLog {
                        scroll: scroll.saturating_sub(20),
                    };
                }
                KeyCode::Home | KeyCode::Char('0') => {
                    app.mode = Mode::DebugLog { scroll: 0 };
                }
                KeyCode::End => {
                    app.mode = Mode::DebugLog { scroll: max_scroll };
                }
                _ => {}
            }
        }
        Mode::Wizard { mut wizard } => {
            match handle_wizard_key(app, &mut wizard, code) {
                WizardOutcome::Continue => app.mode = Mode::Wizard { wizard },
                WizardOutcome::Cancel => {
                    app.mode = Mode::Normal;
                    app.set_status_info("İptal edildi.");
                }
                WizardOutcome::Submitted(cmd) => {
                    let label = describe_command(&cmd);
                    app.pending_human_cmds.push(cmd);
                    app.mode = Mode::Normal;
                    app.set_status_ok(format!("→ {label}  (SPACE ile tick ilerlet)"));
                }
                WizardOutcome::Error(msg) => {
                    // Wizard açık kalsın, hata gösterilsin.
                    app.mode = Mode::Wizard { wizard };
                    app.set_status_err(format!("Hata: {msg}"));
                }
            }
        }
        Mode::GameOver => match code {
            KeyCode::Char('q') => return Ok(true),
            _ => {}
        },
    }
    Ok(false)
}

// ---------------------------------------------------------------------------
// Uygulama durumu
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Mode {
    /// Oyun başlangıcı — rol seçimi bekleniyor.
    Startup,
    Normal,
    Command {
        buffer: String,
    },
    Help,
    Info,
    /// Varlıklarım overlay — m ile açılır.
    Holdings,
    /// Haber inbox overlay — n ile açılır.
    NewsInbox,
    /// Son eşleşmeler overlay — e ile açılır. Kim kime ne sattı, fiyat, tick.
    RecentTrades,
    /// Açık Pazar (orderbook depth) overlay — k ile açılır. Her (şehir, ürün)
    /// için en iyi bid/ask'lar.
    OpenBook,
    /// Kervan kontrol paneli — v ile açılır. Detaylı kervan listesi + hızlı dispatch.
    CaravanPanel,
    /// Debug log overlay — g ile açılır. Son tick'lerin ham LogEntry akışı
    /// (tüm alanlar Debug formatı ile). Geliştirici penceresi.
    DebugLog {
        /// Scroll offset — 0 = en yeni, büyük değer = daha eski.
        scroll: usize,
    },
    /// Sezon sonu reveal — 90. tick'te otomatik açılır, tüm skorlar görünür.
    GameOver,
    /// Pazar verileri (intel) overlay — `r` ile açılır. Her (şehir × ürün)
    /// için ort.fiyat + tahmini toplam stok aralığı. Bireysel detay gizli.
    MarketIntel,
    /// Tek-tuş aksiyon wizard — b/s/f/c/d/l/r/n/o/a/w/x ile açılır,
    /// adım adım komut kurar, ezbere parametre gerektirmez.
    Wizard {
        wizard: Wizard,
    },
}

// ---------------------------------------------------------------------------
// Wizard sistemi — tek-tuş aksiyon menüleri
// ---------------------------------------------------------------------------

/// Wizard'ın hangi action'ı kurduğunu belirler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    Buy,
    Sell,
    Build,
    Caravan,
    Ship,
    Loan,
    Repay,
    News,
    Cancel,
    Offer,
    Accept,
    Withdraw,
}

impl ActionKind {
    fn label(self) -> &'static str {
        match self {
            Self::Buy => "AL",
            Self::Sell => "SAT",
            Self::Build => "FABRİKA KUR",
            Self::Caravan => "KERVAN AL",
            Self::Ship => "KERVAN GÖNDER",
            Self::Loan => "KREDİ AL",
            Self::Repay => "KREDİ ÖDE",
            Self::News => "HABER ABONELİĞİ",
            Self::Cancel => "EMRİ İPTAL",
            Self::Offer => "KONTRAT ÖNER",
            Self::Accept => "KONTRAT KABUL",
            Self::Withdraw => "KONTRAT GERİ ÇEK",
        }
    }
    /// Bu aksiyonun kurulması için gereken alan zinciri (sırayla doldurulur).
    fn schema(self) -> &'static [FieldKind] {
        use FieldKind::*;
        match self {
            Self::Buy | Self::Sell => &[City, Product, QtyU32, PriceLira, OrderTtl],
            Self::Build => &[City, FinishedProduct],
            Self::Caravan => &[City],
            // `from` kervanın konumundan otomatik alınır — wizard sormaz.
            Self::Ship => &[CaravanId, CityTo, Product, QtyU32],
            Self::Loan => &[AmountLira, DurationTicks],
            Self::Repay => &[LoanId],
            Self::News => &[NewsTier_],
            Self::Cancel => &[OrderId_],
            Self::Offer => &[Product, QtyU32, PriceLira, City, DeliveryTick],
            Self::Accept | Self::Withdraw => &[ContractId_],
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FieldKind {
    City,
    CityTo,
    Product,
    FinishedProduct,
    QtyU32,
    PriceLira,
    AmountLira,
    DurationTicks,
    DeliveryTick,
    /// Emir kitapta kaç tick kalsın (1..=max_order_ttl).
    OrderTtl,
    NewsTier_,
    OrderId_,
    CaravanId,
    LoanId,
    ContractId_,
}

impl FieldKind {
    fn prompt(self) -> &'static str {
        match self {
            Self::City => "Şehir",
            Self::CityTo => "Hedef şehir",
            Self::Product => "Ürün",
            Self::FinishedProduct => "Bitmiş ürün",
            Self::QtyU32 => "Miktar (birim)",
            Self::PriceLira => "Birim fiyat (₺)",
            Self::AmountLira => "Tutar (₺)",
            Self::DurationTicks => "Vade (tick)",
            Self::DeliveryTick => "Teslimat tick'i",
            Self::OrderTtl => "Emir TTL (kaç tick kitapta kalsın)",
            Self::NewsTier_ => "Tier",
            Self::OrderId_ => "Açık emir seç",
            Self::CaravanId => "Kervan seç (Idle)",
            Self::LoanId => "Açık kredi seç",
            Self::ContractId_ => "Kontrat seç",
        }
    }
    /// Bu alan numerik text input mi (Number) yoksa seçim listesi mi (Pick)?
    fn is_text(self) -> bool {
        matches!(
            self,
            Self::QtyU32
                | Self::PriceLira
                | Self::AmountLira
                | Self::DurationTicks
                | Self::DeliveryTick
                | Self::OrderTtl
        )
    }
}

#[derive(Debug, Clone)]
enum FieldValue {
    City(CityId),
    Product(ProductKind),
    Number(u64),
    NewsTier(NewsTier),
    OrderId(OrderId),
    CaravanId(moneywar_domain::CaravanId),
    LoanId(moneywar_domain::LoanId),
    ContractId(moneywar_domain::ContractId),
}

#[derive(Debug, Clone)]
struct Wizard {
    kind: ActionKind,
    fields: Vec<FieldValue>,
    text_buf: String,
    /// Ship wizard'a özgü: ana (Product, Qty) çiftine ek olarak yüklenen
    /// ekstra cargo kalemleri. Multi-product dispatch için.
    extra_cargo: Vec<(ProductKind, u32)>,
    /// Ship wizard'da "daha ürün ekle mi?" confirm adımında olup olmadığını
    /// söyler. `true` iken tek seçim: 1 (ek) veya 2 (bitir).
    confirm_more_cargo: bool,
}

impl Wizard {
    fn new(kind: ActionKind) -> Self {
        Self {
            kind,
            fields: Vec::new(),
            text_buf: String::new(),
            extra_cargo: Vec::new(),
            confirm_more_cargo: false,
        }
    }

    /// Şu an doldurulması gereken alan. `None` → tüm alanlar tamam, confirm.
    fn current(&self) -> Option<FieldKind> {
        self.kind.schema().get(self.fields.len()).copied()
    }

    fn is_done(&self) -> bool {
        self.fields.len() >= self.kind.schema().len() && !self.confirm_more_cargo
    }
}

#[derive(Debug, Clone)]
struct TradeRecord {
    tick: Tick,
    city: CityId,
    product: ProductKind,
    buyer_name: String,
    seller_name: String,
    quantity: u32,
    price: Money,
}

const RECENT_TRADES_WINDOW: usize = 30;
const DEBUG_LOG_WINDOW: usize = 300;
/// Achievement toast'ı bu kadar tick boyunca görünür (engine tick'i —
/// auto-sim ~300ms, manual SPACE'le değişken).
const TOAST_TTL_TICKS: u32 = 3;

/// Oyuncunun bir kez kazandığı milestone. CLI-tarafı, deterministik değil
/// (UI feedback). `App.unlocked` set'inde tutulur, tick advance sonrası
/// `check_achievements` ile güncellenir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Achievement {
    FirstFactory,
    FirstCaravan,
    FirstTrade,
    TenK,
    FiftyK,
    HundredK,
    ThreeFactories,
    ThreeCaravans,
    LeadingNow,
}

impl Achievement {
    fn label(self) -> &'static str {
        match self {
            Self::FirstFactory => "✨  İLK FABRİKAN! ✨",
            Self::FirstCaravan => "🐪  İLK KERVANIN! 🐪",
            Self::FirstTrade => "🤝  İLK İŞLEMİN! 🤝",
            Self::TenK => "💰  +5K kâr — para kazanıyorsun",
            Self::FiftyK => "💎  +20K kâr — iyi sezon",
            Self::HundredK => "👑  +50K kâr — büyük başarı",
            Self::ThreeFactories => "🏭  3 ŞEHRE FABRİKA",
            Self::ThreeCaravans => "🚂  3 KERVANLIK FİLO",
            Self::LeadingNow => "🥇  LEADERBOARD'DA 1.LİK!",
        }
    }
}

/// Şu an ekranda gösterilen toast bildirimi. `expires_at_tick` geldiğinde
/// kaldırılır. Sıraya alma yok — yeni unlock eskisinin üzerine yazar.
#[derive(Debug, Clone)]
struct Toast {
    achievement: Achievement,
    expires_at_tick: Tick,
}

struct App {
    state: GameState,
    last_tick_log: Vec<String>,
    recent_news: Vec<NewsItem>,
    recent_trades: Vec<TradeRecord>,
    /// Ham LogEntry akışı — debug overlay için. Son DEBUG_LOG_WINDOW tutulur.
    debug_log: Vec<moneywar_engine::LogEntry>,
    /// Debug log dosyası — `debug/moneywar-{epoch}.log`. Her tick entry'ler
    /// bu dosyaya append edilir. None → dosya açılamadı (IO hatası, sessiz).
    debug_file: Option<BufWriter<std::fs::File>>,
    prev_prices: std::collections::BTreeMap<(CityId, ProductKind), Money>,
    auto_sim: bool,
    mode: Mode,
    /// Startup ekranında seçilen preset (oyun başlayınca `state.config`'e yazılır).
    selected_preset: PresetChoice,
    /// NPC zorluk — Easy (basit likidite) veya Hard (akıllı, rekabetçi).
    difficulty: Difficulty,
    /// İnsan komutları — tick ilerledikçe NPC komutlarıyla birlikte advance'e iletilir.
    pending_human_cmds: Vec<Command>,
    /// Tek seferlik status satırı (başarı/hata). Bir sonraki tick'te temizlenir.
    status: Option<StatusMsg>,
    /// Human `OrderId` sayacı — her yeni order için monoton artar.
    next_human_order_id: u64,
    /// Runtime knob'lar — `moneywar.toml`'den veya default. `state.config.balance`
    /// ile aynı değer; App burda startup ekranı için tutuyor.
    balance: GameBalance,
    /// TOML dosyası başarıyla yüklendi mi (UI göstergesi için).
    balance_loaded: bool,
    /// Tick başında hesaplanmış leaderboard. Render hot-path'i bunu okur,
    /// her render'da yeniden hesaplamaz. Auto-sim 300ms'de render'da
    /// 11 NPC × scoring iter çalıştırması engellendi.
    cached_leaderboard: Vec<PlayerScore>,
    /// Her `(city, product)` için 8-tikli sparkline string'i. Tick başında
    /// güncelle, render'da olduğu gibi kullan. Auto-sim'de 21 hücre × 4
    /// alloc/render sayısı sıfıra iner.
    cached_sparklines: std::collections::BTreeMap<(CityId, ProductKind), String>,
    /// Oyuncunun kazandığı milestone'lar — her achievement bir kez tetiklenir.
    unlocked_achievements: std::collections::BTreeSet<Achievement>,
    /// Şu an ekranda görünen toast. `current_tick >= expires_at_tick` ise
    /// kaldırılır.
    active_toast: Option<Toast>,
    /// Oyun başında oyuncunun nakdi (cents). Achievement'lar **net kazanç**
    /// üstünden ölçülür — başlangıç sermayesi rolden role değişiyor (50K
    /// Sanayici / 80K Tüccar), mutlak eşik anında unlock olurdu.
    starting_cash_cents: i64,
    /// Startup ekranında girilen oyuncu adı buffer'ı. Boş ise "Sen" default.
    /// Sanayici/Tüccar seçildiğinde role suffix eklenir: "Selim (Sanayici)".
    player_name_input: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PresetChoice {
    Hizli,
    Standart,
    Uzun,
}

impl PresetChoice {
    fn config(self) -> RoomConfig {
        match self {
            Self::Hizli => RoomConfig::hizli(),
            Self::Standart => RoomConfig::standart(),
            Self::Uzun => RoomConfig::uzun(),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Hizli => "Hızlı (90 tick, ~1.5 saat)",
            Self::Standart => "Standart (150 tick, ~3 gün)",
            Self::Uzun => "Uzun (350 tick, ~14 gün)",
        }
    }
    fn next(self) -> Self {
        match self {
            Self::Hizli => Self::Standart,
            Self::Standart => Self::Uzun,
            Self::Uzun => Self::Hizli,
        }
    }
}

#[derive(Debug, Clone)]
struct StatusMsg {
    text: String,
    kind: StatusKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StatusKind {
    Ok,
    Err,
    Info,
}

/// Her oyun için epoch-nanos tabanlı room_id üretir — NPC RNG seed'i bu'dur.
/// Aynı oturumda tekrar çağrılırsa muhtemelen farklı değer verir (nanos).
fn random_room_id() -> RoomId {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1);
    // 0 olursa 1 — RoomId::new(0) geçerli olsa da 1'den başlasın.
    RoomId::new(if nanos == 0 { 1 } else { nanos })
}

/// `./debug/moneywar-{epoch}.log` dosyası açar. Klasör yoksa oluşturur.
/// Başarısız olursa sessiz — debug opsiyoneldir, oyunu kırmasın.
fn open_debug_file(balance: GameBalance) -> Option<BufWriter<std::fs::File>> {
    std::fs::create_dir_all("debug").ok()?;
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let path = format!("debug/moneywar-{epoch}.log");
    let file = std::fs::File::create(&path).ok()?;
    let mut w = BufWriter::new(file);
    // Oturum başlığı — config + composition dump.
    let _ = writeln!(w, "# MoneyWar debug log");
    let _ = writeln!(w, "# session_epoch: {epoch}");
    let _ = writeln!(
        w,
        "# balance: default_ttl={} max_ttl={} cancel_penalty={}% cooldown={}t",
        balance.default_order_ttl,
        balance.max_order_ttl,
        balance.cancel_penalty_pct,
        balance.relist_cooldown_ticks
    );
    let _ = writeln!(
        w,
        "# npcs: sanayici={} tuccar={} alici={} (total={})",
        balance.npcs.sanayici,
        balance.npcs.tuccar,
        balance.npcs.alici,
        balance.npcs.total()
    );
    let _ = writeln!(w, "# format: t{{tick}}  {{actor:<20}}  {{event:?}}");
    let _ = writeln!(w, "# ---");
    let _ = w.flush();
    Some(w)
}

impl App {
    fn new(balance: GameBalance, balance_loaded: bool) -> Self {
        // Boş state; gerçek dünya `start_game(role)` ile kurulur.
        let state = GameState::new(RoomId::new(1), RoomConfig::hizli().with_balance(balance));
        let debug_file = open_debug_file(balance);
        Self {
            state,
            last_tick_log: Vec::new(),
            recent_news: Vec::new(),
            recent_trades: Vec::new(),
            debug_log: Vec::new(),
            debug_file,
            prev_prices: std::collections::BTreeMap::new(),
            auto_sim: false,
            mode: Mode::Startup,
            selected_preset: PresetChoice::Hizli,
            difficulty: Difficulty::Hard,
            pending_human_cmds: Vec::new(),
            status: None,
            next_human_order_id: 1,
            balance,
            balance_loaded,
            cached_leaderboard: Vec::new(),
            cached_sparklines: std::collections::BTreeMap::new(),
            unlocked_achievements: std::collections::BTreeSet::new(),
            active_toast: None,
            starting_cash_cents: 0,
            player_name_input: String::new(),
        }
    }

    /// Tick advance sonrasında milestone'ları tarar. İlk kez ulaşılan
    /// achievement varsa unlock + toast gösterir. Sıraya alma yok; aynı
    /// tick'te birden çok unlock olursa son hesaplanan görünür.
    fn check_achievements(&mut self) {
        let Some(player) = self.state.players.get(&HUMAN_ID) else {
            return;
        };
        // Net kazanç (lira) — başlangıç sermayesi role'e göre değişiyor (50K/80K),
        // mutlak nakit eşikleri instant unlock olurdu. PnL üstünden ölç.
        let pnl_lira = (player.cash.as_cents() - self.starting_cash_cents) / 100;
        let factory_count = self
            .state
            .factories
            .values()
            .filter(|f| f.owner == HUMAN_ID)
            .count();
        let factory_cities: std::collections::BTreeSet<_> = self
            .state
            .factories
            .values()
            .filter(|f| f.owner == HUMAN_ID)
            .map(|f| f.city)
            .collect();
        let caravan_count = self
            .state
            .caravans
            .values()
            .filter(|c| c.owner == HUMAN_ID)
            .count();
        let leading = self
            .cached_leaderboard
            .first()
            .is_some_and(|sc| sc.player_id == HUMAN_ID);

        let candidates: &[(Achievement, bool)] = &[
            (Achievement::FirstFactory, factory_count >= 1),
            (Achievement::ThreeFactories, factory_cities.len() >= 3),
            (Achievement::FirstCaravan, caravan_count >= 1),
            (Achievement::ThreeCaravans, caravan_count >= 3),
            // Net kâr eşikleri (başlangıç sermayesinin üstünde)
            (Achievement::TenK, pnl_lira >= 5_000),
            (Achievement::FiftyK, pnl_lira >= 20_000),
            (Achievement::HundredK, pnl_lira >= 50_000),
            (Achievement::LeadingNow, leading),
        ];

        for (ach, condition) in candidates {
            if *condition && self.unlocked_achievements.insert(*ach) {
                self.active_toast = Some(Toast {
                    achievement: *ach,
                    expires_at_tick: Tick::new(
                        self.state.current_tick.value().saturating_add(TOAST_TTL_TICKS),
                    ),
                });
            }
        }
    }

    /// İnsan oyuncu bu tick'te en az bir match yaşadıysa `FirstTrade` unlock.
    /// Match olayları report'tan harvest edilir.
    fn check_first_trade_in_report(&mut self, report: &moneywar_engine::TickReport) {
        if self.unlocked_achievements.contains(&Achievement::FirstTrade) {
            return;
        }
        let traded = report.entries.iter().any(|e| {
            matches!(
                &e.event,
                LogEvent::OrderMatched { buyer, seller, .. } if *buyer == HUMAN_ID || *seller == HUMAN_ID
            )
        });
        if traded {
            self.unlocked_achievements.insert(Achievement::FirstTrade);
            self.active_toast = Some(Toast {
                achievement: Achievement::FirstTrade,
                expires_at_tick: Tick::new(
                    self.state.current_tick.value().saturating_add(TOAST_TTL_TICKS),
                ),
            });
        }
    }

    /// Tick advance sonrasında çağrılır — render hot-path'inde yeniden
    /// hesaplanan değerleri tek seferde günceller. Leaderboard scoring 12
    /// player × inventory iter, sparkline 21 hücre × min/max normalize.
    fn refresh_caches(&mut self) {
        self.cached_leaderboard = leaderboard(&self.state);
        self.cached_sparklines.clear();
        for city in CityId::ALL {
            for product in ProductKind::ALL {
                if let Some(history) = self.state.price_history.get(&(city, product)) {
                    let prices: Vec<Money> = history
                        .iter()
                        .rev()
                        .take(8)
                        .map(|(_, p)| *p)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    if !prices.is_empty() {
                        self.cached_sparklines
                            .insert((city, product), sparkline(&prices));
                    }
                }
            }
        }
    }

    fn start_game(&mut self, role: Role) {
        let cfg = self.selected_preset.config().with_balance(self.balance);
        let room_id = random_room_id();
        let custom_name = self.player_name_input.trim();
        let custom_name = if custom_name.is_empty() {
            None
        } else {
            Some(custom_name.to_string())
        };
        self.state = seed_world(role, cfg, room_id, custom_name);
        self.mode = Mode::Normal;
        self.starting_cash_cents = self
            .state
            .players
            .get(&HUMAN_ID)
            .map_or(0, |p| p.cash.as_cents());
        self.refresh_caches();
        // Rol'e göre ilk hamle önerisi — terminal açılır açılmaz oyuncu
        // ne yapacağını bilsin.
        let first_move = match role {
            Role::Sanayici => {
                "İlk hamle ipucu: `:build istanbul kumas` (1. fabrika BEDAVA), sonra SPACE."
            }
            Role::Tuccar => {
                "İlk hamle ipucu: `:caravan istanbul` (1. kervan BEDAVA), sonra `:ship`."
            }
        };
        self.set_status_info(first_move);
    }

    fn next_order_id(&mut self) -> OrderId {
        let id = OrderId::new(self.next_human_order_id);
        self.next_human_order_id = self.next_human_order_id.saturating_add(1);
        id
    }

    fn set_status_ok(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMsg {
            text: text.into(),
            kind: StatusKind::Ok,
        });
    }

    fn set_status_err(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMsg {
            text: text.into(),
            kind: StatusKind::Err,
        });
    }

    fn set_status_info(&mut self, text: impl Into<String>) {
        self.status = Some(StatusMsg {
            text: text.into(),
            kind: StatusKind::Info,
        });
    }

    fn step_one_tick(&mut self) {
        if self.game_over() {
            return;
        }
        let next_tick = self.state.current_tick.next();
        let mut rng = rng_for(self.state.room_id, next_tick);
        let npc_cmds = decide_all_npcs(&self.state, &mut rng, next_tick, self.difficulty);

        // İnsan komutları önce (sıra fark etmez ama insan kararını önce
        // göstermek log'da daha okunur).
        let human_cmds: Vec<Command> = self.pending_human_cmds.drain(..).collect();
        // Tick sonrası feedback için sadece insanın yolladığı emirleri sakla.
        let human_orders: Vec<MarketOrder> = human_cmds
            .iter()
            .filter_map(|c| match c {
                Command::SubmitOrder(o) if o.player == HUMAN_ID => Some(o.clone()),
                _ => None,
            })
            .collect();

        let mut cmds = human_cmds;
        cmds.extend(npc_cmds);

        let Ok((new_state, report)) = advance_tick(&self.state, &cmds) else {
            self.last_tick_log
                .push("[ENGINE HATASI] advance_tick başarısız".into());
            return;
        };

        self.prev_prices = new_state
            .price_history
            .iter()
            .filter_map(|(k, v)| v.last().map(|(_, p)| (*k, *p)))
            .collect();

        self.state = new_state;
        self.last_tick_log = summarize_report(&report, &self.state);
        self.harvest_trades(&report);
        self.harvest_debug_log(&report);
        self.harvest_news();
        // Render hot-path'inde yeniden hesaplanan değerleri burada cache'le.
        self.refresh_caches();
        // Toast TTL: süresi dolduysa kaldır.
        if let Some(t) = &self.active_toast {
            if !self.state.current_tick.is_before(t.expires_at_tick) {
                self.active_toast = None;
            }
        }
        // Achievement kontrolleri — state-bazlı (cash/fabrika) ve report-bazlı.
        self.check_first_trade_in_report(&report);
        self.check_achievements();
        // Eski status mesajı varsa temizle (yeni tick'in kendi mesajı olsun).
        self.status = None;
        // İnsanın bu tick'teki emirleri için "ne oldu" özeti — kritik UX
        // ipucu, kullanıcı "neden hiçbir şey olmadı" demesin.
        self.report_order_outcomes(&report, &human_orders);
    }

    /// İnsanın bu tick'te yolladığı her market emri için sonuç özeti.
    /// MarketCleared ve OrderMatched event'lerini tarar, kullanıcıya somut
    /// geri bildirim üretir.
    fn report_order_outcomes(
        &mut self,
        report: &moneywar_engine::TickReport,
        human_orders: &[MarketOrder],
    ) {
        if human_orders.is_empty() {
            return;
        }
        let mut messages: Vec<String> = Vec::new();
        for order in human_orders {
            // Bu emir için eşleşmeleri topla (OrderMatched event'lerinden).
            let matched_qty: u32 = report
                .entries
                .iter()
                .filter_map(|e| match &e.event {
                    LogEvent::OrderMatched {
                        buy_order_id,
                        sell_order_id,
                        quantity,
                        ..
                    } if *buy_order_id == order.id || *sell_order_id == order.id => Some(*quantity),
                    _ => None,
                })
                .sum();
            // Aynı bucket'ın MarketCleared özetinden clearing fiyatı + karşı taraf var mıydı
            let cleared = report.entries.iter().find_map(|e| match &e.event {
                LogEvent::MarketCleared {
                    city,
                    product,
                    clearing_price,
                    submitted_buy_qty,
                    submitted_sell_qty,
                    ..
                } if *city == order.city && *product == order.product => {
                    Some((*clearing_price, *submitted_buy_qty, *submitted_sell_qty))
                }
                _ => None,
            });
            let side_name = if matches!(order.side, OrderSide::Buy) {
                "AL"
            } else {
                "SAT"
            };
            let header = format!(
                "{side_name} {} {} @ {} ({})",
                order.quantity,
                order.product,
                order.unit_price,
                city_short(order.city),
            );
            if matched_qty > 0 {
                if let Some((Some(clearing_price), _, _)) = cleared {
                    let leftover = order.quantity.saturating_sub(matched_qty);
                    if leftover == 0 {
                        messages.push(format!(
                            "✓ {header} → {matched_qty} eşleşti @ {clearing_price}"
                        ));
                    } else {
                        messages.push(format!(
                            "✓ {header} → {matched_qty} eşleşti @ {clearing_price}, kalan {leftover} çöpe"
                        ));
                    }
                } else {
                    messages.push(format!("✓ {header} → {matched_qty} eşleşti"));
                }
            } else {
                let reason = match cleared {
                    Some((_, _, sell_qty))
                        if matches!(order.side, OrderSide::Buy) && sell_qty == 0 =>
                    {
                        format!(
                            "kimse {} pazarında {} satmıyor (NPC'ler diğer ürün/şehirde)",
                            city_short(order.city),
                            order.product
                        )
                    }
                    Some((_, buy_qty, _))
                        if matches!(order.side, OrderSide::Sell) && buy_qty == 0 =>
                    {
                        format!(
                            "kimse {} pazarında {} almıyor",
                            city_short(order.city),
                            order.product
                        )
                    }
                    Some((Some(clearing_price), _, _)) => {
                        if matches!(order.side, OrderSide::Buy) {
                            format!(
                                "fiyatın {} düşük — clearing {} oldu, daha yüksek teklif lazım",
                                order.unit_price, clearing_price
                            )
                        } else {
                            format!(
                                "fiyatın {} yüksek — clearing {} oldu, daha düşük teklif lazım",
                                order.unit_price, clearing_price
                            )
                        }
                    }
                    Some((None, _, _)) => "spread oldu, kimse kesişmedi".into(),
                    None => "bu pazarda hiç hareket yok".into(),
                };
                messages.push(format!("✗ {header} → eşleşme yok: {reason}"));
            }
        }
        // İlk mesajı status bar'a (kompakt), tümünü Son Tick log'unun başına ekle.
        if let Some(first) = messages.first() {
            // OK ya da err'a göre renk ayır.
            if first.starts_with('✓') {
                self.set_status_ok(first.clone());
            } else {
                self.set_status_err(first.clone());
            }
        }
        // Tüm sonuçları Son Tick log'unun en üstüne ekle.
        let mut prepend = vec!["── 🧾 Senin emirlerin ──".to_string()];
        prepend.extend(messages);
        prepend.push("── ── ──".to_string());
        prepend.append(&mut self.last_tick_log);
        self.last_tick_log = prepend;
    }

    fn harvest_trades(&mut self, report: &moneywar_engine::TickReport) {
        let player_name = |pid: PlayerId| -> String {
            self.state
                .players
                .get(&pid)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| format!("#{}", pid.value()))
        };
        for entry in &report.entries {
            if let LogEvent::OrderMatched {
                city,
                product,
                buyer,
                seller,
                quantity,
                price,
                ..
            } = &entry.event
            {
                self.recent_trades.push(TradeRecord {
                    tick: entry.tick,
                    city: *city,
                    product: *product,
                    buyer_name: player_name(*buyer),
                    seller_name: player_name(*seller),
                    quantity: *quantity,
                    price: *price,
                });
            }
        }
        // Pencere boyutunu aştık mı — baştaki eskileri at.
        if self.recent_trades.len() > RECENT_TRADES_WINDOW {
            let drop_count = self.recent_trades.len() - RECENT_TRADES_WINDOW;
            self.recent_trades.drain(..drop_count);
        }
    }

    fn harvest_debug_log(&mut self, report: &moneywar_engine::TickReport) {
        for entry in &report.entries {
            self.debug_log.push(entry.clone());
        }
        if self.debug_log.len() > DEBUG_LOG_WINDOW {
            let drop_count = self.debug_log.len() - DEBUG_LOG_WINDOW;
            self.debug_log.drain(..drop_count);
        }
        // Dosyaya da yaz — IO hatası oyunu kırmasın.
        if let Some(f) = &mut self.debug_file {
            for entry in &report.entries {
                let actor_str = entry
                    .actor
                    .map(|a| {
                        self.state
                            .players
                            .get(&a)
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| format!("#{}", a.value()))
                    })
                    .unwrap_or_else(|| "system".into());
                let _ = writeln!(
                    f,
                    "t{:>3}  {:<22}  {:?}",
                    entry.tick.value(),
                    actor_str,
                    entry.event
                );
            }
            let _ = f.flush();
        }
    }

    fn harvest_news(&mut self) {
        if let Some(inbox) = self.state.news_inbox.get(&HUMAN_ID) {
            // Yalnız disclosed_tick <= current_tick olanları göster; son N'i tut.
            let disclosed: Vec<NewsItem> = inbox
                .iter()
                .filter(|n| !self.state.current_tick.is_before(n.disclosed_tick))
                .cloned()
                .collect();
            let start = disclosed.len().saturating_sub(NEWS_WINDOW);
            self.recent_news = disclosed[start..].to_vec();
        }
    }

    fn game_over(&self) -> bool {
        self.state.current_tick.value() >= self.state.config.season_ticks
    }
}

// ---------------------------------------------------------------------------
// Dünya kurulumu
// ---------------------------------------------------------------------------

/// Türkçe NPC ön ad havuzu — seed RNG'den deterministik dağıtılır.
/// Anadolu/Osmanlı tüccar atmosferi: leaderboard'da "NPC-3" yerine "Selim Bey"
/// görünür → rakip hissi.
const NPC_FIRST_NAMES: &[&str] = &[
    "Selim", "Hasan", "İbrahim", "Mehmet", "Ahmet", "Mustafa", "Ali", "Ömer", "Hüseyin", "Yusuf",
    "Kemal", "Cemal", "Rıza", "Sadık", "Bekir", "Halil", "Zeynep", "Ayşe", "Fatma", "Hatice",
    "Emine", "Hanife", "Saliha", "Naime", "Şerife", "Rukiye", "Sakine", "Hayriye", "Bedriye",
    "Nevzat", "Şükrü", "Necati",
];

/// NPC unvanı — alt-türe göre. "Selim Bey" (Tüccar), "Hasan Usta" (Sanayici),
/// "Zeynep Hanım" (Alıcı), "Ali Esnaf" (Esnaf).
fn npc_title(kind: NpcKind, name: &str) -> &'static str {
    // Kadın isimleri için "Hanım", erkekler için role'e göre.
    let is_female = matches!(
        name,
        "Zeynep"
            | "Ayşe"
            | "Fatma"
            | "Hatice"
            | "Emine"
            | "Hanife"
            | "Saliha"
            | "Naime"
            | "Şerife"
            | "Rukiye"
            | "Sakine"
            | "Hayriye"
            | "Bedriye"
    );
    if is_female {
        return "Hanım";
    }
    match kind {
        NpcKind::Sanayici => "Usta",
        NpcKind::Esnaf => "Esnaf",
        NpcKind::Alici | NpcKind::Tuccar => "Bey",
        NpcKind::Spekulator => "Efendi",
    }
}

/// Deterministik isim üret — seed RNG'den çek, kullanılmış set'e ekle.
/// Çakışmayı önlemek için aynı isim alındıysa üstüne suffix.
fn pick_npc_name(
    rng: &mut ChaCha8Rng,
    kind: NpcKind,
    used: &mut std::collections::BTreeSet<String>,
) -> String {
    for _ in 0..32 {
        let first = NPC_FIRST_NAMES[rng.random_range(0..NPC_FIRST_NAMES.len())];
        let title = npc_title(kind, first);
        let candidate = format!("{first} {title}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    // Havuz tükendiyse numerik fallback (16+ NPC senaryosu).
    let first = NPC_FIRST_NAMES[rng.random_range(0..NPC_FIRST_NAMES.len())];
    let title = npc_title(kind, first);
    let mut n = 2u32;
    loop {
        let candidate = format!("{first} {title} ({n})");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

fn seed_world(
    human_role: Role,
    config: RoomConfig,
    room_id: RoomId,
    custom_player_name: Option<String>,
) -> GameState {
    let composition = config.balance.npcs;
    let mut s = GameState::new(room_id, config);

    // Seed'den RNG — tüm baseline + inventory dağılımı bundan türer.
    // Aynı room_id → aynı dünya (reproducibility için debug log'da tutulur).
    let mut rng = ChaCha8Rng::seed_from_u64(room_id.value());

    // ŞEHİR UZMANLAŞMASI — her oyun farklı.
    // 3 ham maddeyi 3 şehre rastgele dağıt (Fisher-Yates shuffle).
    // Bu sezon İstanbul Buğday üretebilir, sonraki oyunda Zeytin → "ezbere
    // strateji" sorunu çözülür, oyuncu her sezon haritayı keşfeder.
    {
        let mut raws = ProductKind::RAW_MATERIALS;
        for i in (1..raws.len()).rev() {
            let j = rng.random_range(0..=i);
            raws.swap(i, j);
        }
        for (city, raw) in CityId::ALL.iter().zip(raws.iter()) {
            s.city_specialty.insert(*city, *raw);
        }
    }

    // İsim çakışmasını önlemek için kullanılan isimleri takip et.
    let mut used_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    // price_baseline: her (şehir × ürün) için 0.80-1.20 çarpan.
    // Bazı şehirler pahalı, bazıları ucuz → doğal arbitraj fırsatı.
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let base_lira = if product.is_raw() {
                moneywar_domain::balance::NPC_BASE_PRICE_RAW_LIRA
            } else {
                moneywar_domain::balance::NPC_BASE_PRICE_FINISHED_LIRA
            };
            let multiplier: u32 = rng.random_range(80..=120);
            let price_cents = base_lira.saturating_mul(100) * i64::from(multiplier) / 100;
            s.price_baseline
                .insert((city, product), Money::from_cents(price_cents));
        }
    }

    // İnsan oyuncu — rol'e göre özelleştirilmiş başlangıç paketi.
    // Sıkı bütçe: 1. fabrika/kervan zaten bedava + Sanayici 100 birim ham
    // starter alıyor. Bol nakit "para baskısı yok, riske girmem" hissi
    // veriyordu; düşürüldü.
    let starting_cash = match human_role {
        Role::Sanayici => 25_000_i64,
        Role::Tuccar => 40_000_i64,
    };
    // Saf isim — leaderboard'da `[Sanayici]` etiketi rolü zaten gösteriyor,
    // isimde tekrar ekleme yapma.
    let human_name = custom_player_name.unwrap_or_else(|| "Sen".to_string());
    let mut human = Player::new(
        HUMAN_ID,
        human_name,
        human_role,
        Money::from_lira(starting_cash).unwrap(),
        false,
    )
    .unwrap();
    if matches!(human_role, Role::Sanayici) {
        // Rastgele şehir + o şehrin **bu sezonki** ucuz ham maddesi × 70-130
        // birim. city_specialty her oyun farklı (yukarıda shuffle).
        let city_idx = rng.random_range(0usize..CityId::ALL.len());
        let starter_city = CityId::ALL[city_idx];
        let starter_raw = s.cheap_raw_for(starter_city);
        let starter_qty: u32 = rng.random_range(70..=130);
        human
            .inventory
            .add(starter_city, starter_raw, starter_qty)
            .unwrap();
    }
    s.players.insert(human.id, human);

    // NPC ID ofseti: 100, 101, 102, ... Her tip için ardışık.
    let mut next_id: u64 = 100;

    // NPC-Tüccar(lar) — arbitraj + likidite. Toplam 500 birim stok
    // weighted random olarak 21 bucket'a dağıtılır (bazı NPC belirli bir
    // ürün/şehirde "specialist" olur). Dumping olmasın diye sabit budget.
    for _ in 0..composition.tuccar {
        let name = pick_npc_name(&mut rng, NpcKind::Tuccar, &mut used_names);
        let mut npc = Player::new(
            PlayerId::new(next_id),
            name,
            Role::Tuccar,
            Money::from_lira(15_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Tuccar);
        distribute_inventory(&mut npc, &mut rng, 500);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Sanayici(ler) — fabrika kurar, ham → bitmiş üretir.
    // Başlangıç stoğu: 1 şehirde karışık ham + hafif finished. Böylece
    // t1'de fabrika kurdugunda raw stoğu var, hemen üretime başlayabilir.
    for _ in 0..composition.sanayici {
        let name = pick_npc_name(&mut rng, NpcKind::Sanayici, &mut used_names);
        let mut npc = Player::new(
            PlayerId::new(next_id),
            name,
            Role::Sanayici,
            Money::from_lira(30_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Sanayici);
        let city_idx = rng.random_range(0usize..CityId::ALL.len());
        let starter_city = CityId::ALL[city_idx];
        // Raw starter: 30-50 birim, ürün şehrin **bu sezonki** ucuz hamı.
        let starter_raw = s.cheap_raw_for(starter_city);
        let raw_qty: u32 = rng.random_range(30..=50);
        let _ = npc.inventory.add(starter_city, starter_raw, raw_qty);
        // Finished starter: küçük — piyasaya erken birkaç emir versin.
        let finished_idx = rng.random_range(0usize..ProductKind::FINISHED_GOODS.len());
        let fin_qty: u32 = rng.random_range(10..=20);
        let _ = npc.inventory.add(
            starter_city,
            ProductKind::FINISHED_GOODS[finished_idx],
            fin_qty,
        );
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Alıcı(lar) — saf alıcı, **200k bol nakit**, stok yok.
    // 100k yetmiyordu — sezon boyu demand pressure kaybolup NPC'ler zarara
    // gidiyordu. 200k ile sezon sonuna kadar alım gücü kalır.
    for _ in 0..composition.alici {
        let name = pick_npc_name(&mut rng, NpcKind::Alici, &mut used_names);
        let npc = Player::new(
            PlayerId::new(next_id),
            name,
            Role::Tuccar,
            Money::from_lira(200_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Alici);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Esnaf(lar) — saf satıcı (dükkan). Devasa stok (~3000 birim
    // dengeli dağıtım), her tick 4 sell emri verir. Kervan/fabrika yok.
    // Cash önemli değil — sadece satıyorlar.
    for _ in 0..composition.esnaf {
        let name = pick_npc_name(&mut rng, NpcKind::Esnaf, &mut used_names);
        let mut npc = Player::new(
            PlayerId::new(next_id),
            name,
            Role::Tuccar,
            Money::from_lira(10_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Esnaf);
        distribute_inventory(&mut npc, &mut rng, 3_000);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    // NPC-Spekülatör(ler) — market maker. Hem alış hem satış emri verir,
    // spread'i daraltır → mallar bekleyici kalmasın diye. Orta sermaye +
    // dengeli başlangıç stoğu (~800 birim weighted) — iki yöne de likidite.
    for _ in 0..composition.spekulator {
        let name = pick_npc_name(&mut rng, NpcKind::Spekulator, &mut used_names);
        let mut npc = Player::new(
            PlayerId::new(next_id),
            name,
            Role::Tuccar,
            Money::from_lira(40_000).unwrap(),
            true,
        )
        .unwrap()
        .with_kind(NpcKind::Spekulator);
        distribute_inventory(&mut npc, &mut rng, 800);
        s.players.insert(npc.id, npc);
        next_id += 1;
    }

    s
}

/// Bir NPC'ye `total_budget` kadar birim stoğu, (şehir × ürün) bucket'larına
/// weighted random olarak dağıtır. Her bucket için 0-10 arası ağırlık çekilir,
/// ağırlıklar toplamına göre orantılı miktar verilir. Determinism: caller
/// aynı RNG state'ini geçerse aynı sonuç.
fn distribute_inventory(player: &mut Player, rng: &mut ChaCha8Rng, total_budget: u32) {
    let buckets: Vec<(CityId, ProductKind)> = CityId::ALL
        .iter()
        .flat_map(|c| ProductKind::ALL.iter().map(move |p| (*c, *p)))
        .collect();

    // Her bucket için 0-10 arası ağırlık. Bazı bucket'lar ağırlıklı (specialist).
    let weights: Vec<u32> = (0..buckets.len())
        .map(|_| rng.random_range(0u32..=10))
        .collect();
    let total_weight: u32 = weights.iter().sum();
    if total_weight == 0 {
        return;
    }

    for ((city, product), w) in buckets.iter().zip(weights.iter()) {
        let share =
            u32::try_from(u64::from(total_budget) * u64::from(*w) / u64::from(total_weight))
                .unwrap_or(0);
        if share > 0 {
            let _ = player.inventory.add(*city, *product, share);
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

fn render(f: &mut ratatui::Frame<'_>, app: &App) {
    let area = f.area();

    // Startup ekranı — tüm alanı kaplar, oyun paneli yok.
    if matches!(app.mode, Mode::Startup) {
        render_startup(f, area, app);
        return;
    }

    // Aktif şok varsa header altında bir satır ayır; yoksa görünmez (0 satır).
    let shock_height: u16 = u16::from(!app.state.active_shocks.is_empty());

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),            // header
            Constraint::Length(shock_height), // aktif olaylar şeridi
            Constraint::Min(10),              // middle (panels)
            Constraint::Length(1),            // kervan status çubuğu
            Constraint::Length(3),            // leaderboard
            Constraint::Length(1),            // footer
        ])
        .split(area);

    render_header(f, chunks[0], app);
    if shock_height > 0 {
        render_active_shocks(f, chunks[1], app);
    }
    render_middle(f, chunks[2], app);
    render_caravan_strip(f, chunks[3], app);
    // Command mode'dayken leaderboard yerine komut hint'i göster —
    // oyuncu hangi parametreleri girmesi gerektiğini bilsin.
    if matches!(app.mode, Mode::Command { .. }) {
        render_command_hint(f, chunks[4], app);
    } else {
        render_leaderboard(f, chunks[4], app);
    }
    render_footer(f, chunks[5], app);

    // Overlay'ler — ortada popup, arkaplanı temizle.
    match app.mode {
        Mode::Help => render_help_overlay(f, area, app),
        Mode::Info => render_info_overlay(f, area, app),
        Mode::Holdings => render_holdings_overlay(f, area, app),
        Mode::NewsInbox => render_news_inbox_overlay(f, area, app),
        Mode::RecentTrades => render_recent_trades_overlay(f, area, app),
        Mode::OpenBook => render_open_book_overlay(f, area, app),
        Mode::CaravanPanel => render_caravan_panel_overlay(f, area, app),
        Mode::MarketIntel => render_market_intel_overlay(f, area, app),
        Mode::DebugLog { scroll } => render_debug_log_overlay(f, area, app, scroll),
        Mode::GameOver => render_game_over_overlay(f, area, app),
        Mode::Wizard { ref wizard } => render_wizard_overlay(f, area, app, wizard),
        _ => {}
    }

    // Toast en üstte — diğer overlay'lerin de üzerinde flash gösterilir.
    if let Some(toast) = &app.active_toast {
        render_toast(f, area, toast);
    }
}

/// Pazar intel overlay — `r` ile açılır. Her (şehir × ürün) için:
/// - Rolling avg fiyat (son 5 tick clearing'i)
/// - Toplam stok **aralığı** (fog: exact yerine ±bant)
///
/// Bireysel detay (kim ne kadar tutuyor) gizli — sadece piyasanın toplam
/// büyüklüğü görünür. Oyuncu makro-strateji kurabilir ama rakip mikrosunu
/// göremez.
fn render_market_intel_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(80, 80, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 📊  Pazar Verileri  —  son 5 tick (fog) ")
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "  Şehirde toplam ne kadar mal var? Ortalama fiyat nedir?",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(Span::styled(
        "  Bireysel stok gizli — rakamlar tahmini aralık.",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "  {:<10} {:<11} {:>10}      {}",
            "Şehir", "Ürün", "Ort. fiyat", "Tahmini stok"
        ),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));

    for city in CityId::ALL {
        for product in ProductKind::ALL {
            // Toplam stok — tüm oyuncuların inventory'sinin (city, product) hücresi.
            let total: u64 = app
                .state
                .players
                .values()
                .map(|p| u64::from(p.inventory.get(city, product)))
                .sum();
            // Fog: bant = max(50, total/8). 0 ise "yok".
            let stock_label = if total == 0 {
                "—".to_string()
            } else {
                let band = (total / 8).max(50);
                let low = total.saturating_sub(band);
                let high = total.saturating_add(band);
                format!("~{low}–{high}")
            };
            // Ortalama fiyat — son 5 tick rolling avg, yoksa baseline (effective).
            let avg = app
                .state
                .rolling_avg_price(city, product, 5)
                .or_else(|| app.state.effective_baseline(city, product));
            let price_label = match avg {
                Some(m) => format!("{m}"),
                None => "—".to_string(),
            };
            let highlight = total > 0;
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{:<10} ", city_short(city)),
                    Style::default().fg(Color::Blue),
                ),
                Span::styled(
                    format!("{:<11} ", product),
                    Style::default().fg(product_color(product)),
                ),
                Span::styled(
                    format!("{:>10}      ", price_label),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    stock_label,
                    Style::default().fg(if highlight {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "  Esc / Enter / Space ile kapat",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

/// Achievement toast — ekranın üst-orta noktasında 1 satırlık parlak şerit.
/// Diğer overlay'lerin önünde, oyuncu görmezden gelemeyecek.
fn render_toast(f: &mut ratatui::Frame<'_>, area: Rect, toast: &Toast) {
    let label = toast.achievement.label();
    // Yatay merkezde ~50 char, üstten 4 satır aşağıda.
    let width = (label.chars().count() as u16 + 6).min(area.width.saturating_sub(4));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + 4;
    let rect = Rect::new(x, y, width, 3);
    f.render_widget(Clear, rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD | Modifier::RAPID_BLINK),
        );
    let line = Line::from(Span::styled(
        format!("  {label}  "),
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ));
    let para = Paragraph::new(line).block(block);
    f.render_widget(para, rect);
}

fn render_startup(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(80, 90, area);

    let title_block = Block::default()
        .borders(Borders::ALL)
        .title(" 💰  MoneyWar  —  Yeni Sezon  💰 ")
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let inner = title_block.inner(popup);
    f.render_widget(title_block, popup);

    let lines = vec![
        Line::from(Span::styled(
            "  ╔══════════════════════════════════════════════════════════╗",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "  ║  tick-tabanlı ekonomi simülasyonu  —  2-5 oyuncu ölçeği  ║",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(Span::styled(
            "  ╚══════════════════════════════════════════════════════════╝",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Rolünü seç:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "🏭  1) Sanayici",
                Style::default()
                    .fg(Color::Rgb(210, 140, 80))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from("      • Fabrika kurar — ham maddeyi bitmiş ürüne çevirir (tekel)"),
        Line::from("      • Başlangıç: 50.000₺ + İstanbul'da 100 pamuk"),
        Line::from("      • Kervan kapasite: 20 (küçük, yakın mesafe)"),
        Line::from("      • Oynayış: pasif gelir, üretim + yavaş büyüme"),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "🚚  2) Tüccar",
                Style::default()
                    .fg(Color::Rgb(120, 180, 240))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from("      • Arbitraj — ucuz şehirden al, pahalı şehirde sat"),
        Line::from("      • Başlangıç: 80.000₺ (kervan + sermaye)"),
        Line::from("      • Kervan kapasite: 50 (büyük, uzak mesafe)"),
        Line::from("      • Haber Gümüş bedava — bilgi avantajı"),
        Line::from("      • Oynayış: aktif, fırsatçı, hızlı karar"),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ⚙️   Preset: ", Style::default().fg(Color::White)),
            Span::styled(
                app.selected_preset.label(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   ("),
            Span::styled(
                "p",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ile değiştir)"),
        ]),
        Line::from(vec![
            Span::styled("  🤖  NPC zorluğu: ", Style::default().fg(Color::White)),
            Span::styled(
                app.difficulty.label(),
                Style::default()
                    .fg(if matches!(app.difficulty, Difficulty::Hard) {
                        Color::Red
                    } else {
                        Color::Green
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   ("),
            Span::styled(
                "d",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ile değiştir)"),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  📄  Config: ", Style::default().fg(Color::White)),
            Span::styled(
                if app.balance_loaded {
                    "moneywar.toml yüklendi"
                } else {
                    "moneywar.toml yok — default balance"
                },
                Style::default().fg(if app.balance_loaded {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::raw("   "),
            Span::styled(
                format!(
                    "TTL default={} max={}  cooldown={}t  ceza=%{}",
                    app.balance.default_order_ttl,
                    app.balance.max_order_ttl,
                    app.balance.relist_cooldown_ticks,
                    app.balance.cancel_penalty_pct,
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  📝  Adın: ", Style::default().fg(Color::White)),
            Span::styled(
                if app.player_name_input.is_empty() {
                    "(harf yaz, Backspace ile sil)".to_string()
                } else {
                    app.player_name_input.clone()
                },
                Style::default()
                    .fg(if app.player_name_input.is_empty() {
                        Color::DarkGray
                    } else {
                        Color::Cyan
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "_",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::RAPID_BLINK),
            ),
        ]),
        Line::from(Span::styled(
            "      (boş bırakırsan \"Sen\" olarak başlar)",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Skor = Nakit + Stok × ort.fiyat + Fabrika × 0.5 + Aktif escrow",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Hedef: sezon sonunda leaderboard'da #1",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                " 1 ",
                Style::default()
                    .bg(Color::Rgb(210, 140, 80))
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Sanayici    "),
            Span::styled(
                " 2 ",
                Style::default()
                    .bg(Color::Rgb(120, 180, 240))
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Tüccar    "),
            Span::styled(
                " p ",
                Style::default()
                    .bg(Color::Magenta)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" preset    "),
            Span::styled(" q ", Style::default().bg(Color::DarkGray).fg(Color::White)),
            Span::raw(" çık"),
        ]),
    ];

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}

fn render_help_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(75, 85, area);
    f.render_widget(Clear, popup);

    // İnsan oyuncunun rolü — komut listesini ona göre filtrele.
    let role = app
        .state
        .players
        .get(&HUMAN_ID)
        .map(|p| p.role)
        .unwrap_or(Role::Tuccar);
    let is_sanayici = matches!(role, Role::Sanayici);
    let is_tuccar = matches!(role, Role::Tuccar);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "📖  Tuşlar  ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        Span::styled(
            format!("(rol: {})", role),
            Style::default().fg(role_color(role)),
        ),
    ]));
    lines.push(Line::from(""));
    lines.extend([
        help_kv("SPACE", "Bir tick ilerlet (bekleyen komutlar + NPC'ler)"),
        help_kv("t", "Auto-sim aç/kapa (300ms tick)"),
        help_kv(
            "m",
            "Varlıklarım (emir / fabrika / kervan / kontrat / kredi)",
        ),
        help_kv("n", "Haber kutusu"),
        help_kv("e", "🔄 Son eşleşmeler (kim kime ne sattı)"),
        help_kv("k", "📖 Açık Pazar (orderbook depth)"),
        help_kv("r", "📊 Pazar raporu (ort. fiyat + tahmini stok aralığı)"),
        help_kv("v", "🚚 Kervan kontrol paneli (detaylı + cargo)"),
        help_kv("g", "🛠 Debug log (ham event akışı, ↑↓ scroll)"),
        help_kv("?", "Bu yardım"),
        help_kv("i", "Oyun kuralları"),
        help_kv("q", "Çıkış"),
        help_kv("Esc", "Overlay'i kapat (oyundan çıkmaz)"),
    ]);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("🎯  Tek-tuş aksiyon menüleri — {}", role),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    lines.push(Line::from(Span::styled(
        "    (sayı tuşları ile seçim, rakam tuşları ile sayı, Enter onay, Backspace geri)",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    let mut shortcuts = vec![
        help_kv("b", "🛒 AL — şehir × ürün × miktar × fiyat × TTL"),
        help_kv("s", "💸 SAT — şehir × ürün × miktar × fiyat × TTL"),
    ];
    if matches!(role, Role::Sanayici) {
        shortcuts.push(help_kv("f", "🏭 Fabrika kur — Sanayici tekeli"));
    }
    shortcuts.extend([
        help_kv("c", "🚚 Kervan al"),
        help_kv("d", "📦 Kervanı yola çıkar (dispatch)"),
        help_kv("x", "❌ Açık emri iptal"),
        help_kv("o", "🤝 Kontrat öner"),
        help_kv("a", "✅ Kontrat kabul"),
        help_kv("w", "↩  Kontrat geri çek (withdraw)"),
        help_kv("l", "💰 Kredi al"),
        help_kv("R", "💳 Kredi öde (büyük R — küçük 'r' Pazar raporu)"),
        help_kv("N", "📰 Haber abonelik (büyük N — küçük 'n' inbox)"),
    ]);
    lines.extend(shortcuts);

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        ":  İleri kullanıcı — komut metnini doğrudan yaz (vim tarzı)",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    lines.push(Line::from(""));

    // Her rolün kullanabileceği ortak ticaret komutları
    lines.extend([
        help_cmd(
            ":buy <şehir> <ürün> <miktar> <fiyat>",
            "Hal Pazarı alım — örn: buy istanbul pamuk 20 7",
        ),
        help_cmd(":sell <şehir> <ürün> <miktar> <fiyat>", "Hal Pazarı satım"),
        help_cmd(":cancel <order_id>", "Açık emri geri çek (tick kapanmadan)"),
    ]);

    // Sanayici-özel
    if is_sanayici {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  🏭 Sanayici tekeli",
            Style::default().fg(Color::Rgb(210, 140, 80)),
        )));
        lines.push(help_cmd(
            ":build <şehir> <bitmiş_ürün>",
            "Fabrika kur — kumas/un/zeytinyagi üretir",
        ));
    }

    // Kervan — iki rol de ama kapasite farklı
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        if is_tuccar {
            "  🚚 Tüccar kervanı (kapasite 50, uzak mesafe avantajı)"
        } else {
            "  🚚 Sanayici kervanı (kapasite 20, yakın mesafe)"
        },
        Style::default().fg(role_color(role)),
    )));
    lines.extend([
        help_cmd(":caravan <şehir>", "Kervan satın al"),
        help_cmd(
            ":ship <caravan_id> <from> <to> <ürün> <qty>",
            "Kervanı yola çıkar (varış: mesafe tick sonra)",
        ),
    ]);

    // Kontrat — iki rol de yapabilir
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  🤝 Anlaşma Masası (kontrat)",
        Style::default().fg(Color::Magenta),
    )));
    lines.extend([
        help_cmd(
            ":offer <ürün> <qty> <fiyat> <şehir> <delivery>",
            "Kontrat önerisi (public, deposit=%10)",
        ),
        help_cmd(":accept <contract_id>", "Kontrat önerisini kabul et"),
        help_cmd(":withdraw <contract_id>", "Kendi önerini geri çek"),
    ]);

    // Kredi — iki rol de
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  💰 NPC banka",
        Style::default().fg(Color::Green),
    )));
    lines.extend([
        help_cmd(":loan <miktar> <vade_tick>", "Kredi al (%15 sabit faiz)"),
        help_cmd(":repay <loan_id>", "Krediyi manuel öde"),
    ]);

    // Haber — Tüccar için ek bilgi
    lines.push(Line::from(""));
    let news_hint = if is_tuccar {
        "Haber Gümüş bedava (Tüccar avantajı), Altın ücretli"
    } else {
        "Gümüş 500₺, Altın 2000₺"
    };
    lines.push(Line::from(Span::styled(
        format!("  📰 Haber — {news_hint}"),
        Style::default().fg(Color::Cyan),
    )));
    lines.push(help_cmd(
        ":news <bronze|silver|gold>",
        "Haber aboneliği değiştir",
    ));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Şehirler: istanbul(ist), ankara(ank), izmir(izm)",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        "  Ürünler: pamuk, bugday, zeytin (ham) | kumas, un, zeytinyagi (bitmiş)",
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Esc / Enter / Space → kapat",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    )));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Yardım ")
        .border_style(Style::default().fg(Color::Cyan));
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(para, popup);
}

fn render_info_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(75, 85, area);
    f.render_widget(Clear, popup);

    let role = app
        .state
        .players
        .get(&HUMAN_ID)
        .map(|p| p.role)
        .unwrap_or(Role::Tuccar);
    let (role_line, role_color_val) = match role {
        Role::Sanayici => (
            "Rolün: Sanayici — fabrika kur, üret, sat.",
            Color::Rgb(210, 140, 80),
        ),
        Role::Tuccar => (
            "Rolün: Tüccar — ucuz al, pahalı yerde sat.",
            Color::Rgb(120, 180, 240),
        ),
    };

    let lines = vec![
        Line::from(Span::styled(
            "🎮  MoneyWar — Nasıl Oynanır",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(""),
        Line::from(Span::styled(
            role_line,
            Style::default()
                .fg(role_color_val)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(format!(
            "Hedef: sezon sonunda ({} tick) leaderboard'da #1.",
            app.state.config.season_ticks
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Skor formülü",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  Skor = Nakit + Σ(stok × son5 ort.fiyat) + Σ(fabrika×0.5) + Σ(aktif escrow)"),
        Line::from(""),
        Line::from(Span::styled(
            "Tick akışı",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  1) Tek tuş aksiyon (b/S/f/c/d/...) → wizard'da seç → komut queue'ya"),
        Line::from("  2) SPACE bas → komutlar uygulanır, NPC'ler hamle yapar, piyasa temizlenir"),
        Line::from("  3) Üretim / kervan varış / kontrat / kredi otomatik işlenir"),
        Line::from(""),
        Line::from(Span::styled(
            "Lojistik akışı (önemli!)",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  • Hal Pazarı = AYNI ŞEHİR içi alım/satım. Eşleşince mal anında envantere."),
        Line::from("  • Şehirler arası = KERVAN gerek. Akış:"),
        Line::from(Span::styled(
            "      İstanbul[al] → kervan dispatch → Ankara[varış 3 tick sonra] → sat",
            Style::default().fg(Color::Cyan),
        )),
        Line::from("  • İstanbul'da pamuk istiyorsan → istanbul pazarında al."),
        Line::from("  • Ankara'da satmak istiyorsan → kervan ile götür, varınca sat."),
        Line::from(""),
        Line::from(Span::styled(
            "Emir mantığı",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "  • LIMIT order: AL emrinde max ödemek istediğin fiyat. Daha düşüğüne deniyor.",
        ),
        Line::from(
            "  • Karşı taraf YOKSA emir tick sonunda çöpe atılır. \"Son Tick\" panelinde sonuç.",
        ),
        Line::from("  • Yüksek fiyat ver → daha çok eşleşme şansı. Çok düşük → boşa düşer."),
        Line::from(""),
        Line::from(Span::styled(
            "Haber katmanları",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(
            "  🥉 Bronz bedava (olay tick'inde)   🥈 Gümüş 1 tick önce   🥇 Altın 2 tick önce",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Tuşlar",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  :  komut    m  varlık    n  haber    SPACE  tick    ?  yardım"),
        Line::from(""),
        Line::from(Span::styled(
            "Esc / Enter / Space → kapat",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Nasıl Oynanır ")
        .border_style(Style::default().fg(Color::Magenta));
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(para, popup);
}

/// Wizard başlığında görünecek cash + maliyet satırları. Aksiyona göre
/// ek bilgi: Build'de sıradaki fabrika maliyeti, Caravan'da kervan maliyeti.
fn wizard_money_header(app: &App, wizard: &Wizard) -> Vec<Line<'static>> {
    let Some(player) = app.state.players.get(&HUMAN_ID) else {
        return Vec::new();
    };
    let cash = player.cash;

    let mut out: Vec<Line<'static>> = Vec::new();
    let cash_span = Span::styled(
        format!("  💰 Nakit: {cash}"),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    match wizard.kind {
        ActionKind::Build => {
            let built = app
                .state
                .factories
                .values()
                .filter(|f| f.owner == HUMAN_ID)
                .count();
            // Queue'daki BuildFactory komutları da sayılır — aynı tick'te
            // birden çok fabrika kurarsan 2.'si "bedava" yazmasın.
            let pending = app
                .pending_human_cmds
                .iter()
                .filter(|c| matches!(c, Command::BuildFactory { owner, .. } if *owner == HUMAN_ID))
                .count();
            let existing = built + pending;
            let cost =
                moneywar_domain::Factory::build_cost(u32::try_from(existing).unwrap_or(u32::MAX));
            let affordable = cash >= cost;
            out.push(Line::from(vec![
                cash_span,
                Span::raw("   "),
                Span::styled(
                    format!("🏭 Yeni fabrika (#{}) maliyeti: {cost}", existing + 1),
                    Style::default().fg(if affordable { Color::Green } else { Color::Red }),
                ),
                Span::raw("  "),
                Span::styled(
                    if affordable {
                        "✓ yeterli".to_string()
                    } else {
                        format!(
                            "✗ eksik {}",
                            Money::from_cents(cost.as_cents().saturating_sub(cash.as_cents()))
                        )
                    },
                    Style::default().fg(if affordable { Color::Green } else { Color::Red }),
                ),
            ]));
            if pending > 0 {
                out.push(Line::from(Span::styled(
                    format!(
                        "  ℹ Queue'da {pending} fabrika emri var — bu SPACE'te işlenince maliyet sıralı uygulanır"
                    ),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        ActionKind::Caravan => {
            let role = player.role;
            let built = app
                .state
                .caravans
                .values()
                .filter(|c| c.owner == HUMAN_ID)
                .count();
            let pending = app
                .pending_human_cmds
                .iter()
                .filter(|c| matches!(c, Command::BuyCaravan { owner, .. } if *owner == HUMAN_ID))
                .count();
            let existing = built + pending;
            let cost = moneywar_domain::Caravan::buy_cost(
                role,
                u32::try_from(existing).unwrap_or(u32::MAX),
            );
            let affordable = cash >= cost;
            out.push(Line::from(vec![
                cash_span,
                Span::raw("   "),
                Span::styled(
                    format!("🚚 Yeni kervan (#{}) maliyeti: {cost}", existing + 1),
                    Style::default().fg(if affordable { Color::Green } else { Color::Red }),
                ),
            ]));
            if pending > 0 {
                out.push(Line::from(Span::styled(
                    format!("  ℹ Queue'da {pending} kervan emri var"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        _ => {
            // Diğer aksiyonlar: sadece cash göster.
            out.push(Line::from(vec![cash_span]));
        }
    }
    out
}

/// Wizard açıkken gösterilen pre-flight uyarıları — "kervanın yok", "bu
/// şehirde stok 0" gibi tuş basmadan önce bilmek istediğin durum bilgisi.
fn wizard_preflight_hints(app: &App, wizard: &Wizard) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();

    // Dispatch wizard: idle kervan yoksa erken uyar; seçildiyse konum + ETA.
    if matches!(wizard.kind, ActionKind::Ship) {
        let idle_count = app
            .state
            .caravans
            .values()
            .filter(|c| c.owner == HUMAN_ID && c.is_idle())
            .count();
        if idle_count == 0 {
            let any_owned = app.state.caravans.values().any(|c| c.owner == HUMAN_ID);
            let msg = if any_owned {
                "  ⚠ Idle kervanın yok — hepsi yolda. Varmalarını bekle ya da yeni al ('c')"
                    .to_string()
            } else {
                "  ⚠ Hiç kervanın yok — önce 'c' ile kervan al".to_string()
            };
            out.push(Line::from(Span::styled(
                msg,
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )));
        }
        // Kervan seçildiyse konumunu ve ETA bilgisini göster.
        if let Some(FieldValue::CaravanId(cid)) = wizard.fields.first() {
            if let Some(caravan) = app.state.caravans.get(cid) {
                let loc = caravan.state.current_city().map(city_short).unwrap_or("?");
                let cap = caravan.capacity;
                out.push(Line::from(Span::styled(
                    format!(
                        "  📍 Kervan #{} @ {} (kapasite {} birim) — 'from' otomatik",
                        cid.value(),
                        loc,
                        cap
                    ),
                    Style::default().fg(Color::Cyan),
                )));
                // Hedef seçildiyse mesafe + bozulma uyarısı.
                if let Some(FieldValue::City(to_city)) = wizard.fields.get(1) {
                    if let Some(from_city) = caravan.state.current_city() {
                        let distance = from_city.distance_to(*to_city);
                        out.push(Line::from(Span::styled(
                            format!(
                                "  🛣  {} → {} = {} tick yol (varış t{})",
                                city_short(from_city),
                                city_short(*to_city),
                                distance,
                                app.state.current_tick.value().saturating_add(distance)
                            ),
                            Style::default().fg(Color::Yellow),
                        )));
                        // Ürün seçildiyse bozulma kontrolü.
                        if let Some(FieldValue::Product(p)) = wizard.fields.get(2) {
                            if let Some(perish) = p.perishability() {
                                let danger = distance >= perish.after_ticks;
                                let (color, icon) = if perish.loss_percent == 100 {
                                    (Color::Red, "☠")
                                } else {
                                    (Color::Yellow, "⚠")
                                };
                                if danger {
                                    out.push(Line::from(Span::styled(
                                        format!(
                                            "  {icon} {} bozulur: {} tick sonra %{} kayıp. Rota {} tick — TEHLİKELİ!",
                                            p, perish.after_ticks, perish.loss_percent, distance
                                        ),
                                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                                    )));
                                } else {
                                    out.push(Line::from(Span::styled(
                                        format!(
                                            "  ℹ {} bozulur: {} tick sonra %{} kayıp. Rota {} tick — güvenli.",
                                            p, perish.after_ticks, perish.loss_percent, distance
                                        ),
                                        Style::default().fg(Color::DarkGray),
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Sell wizard: şehir + ürün seçildiyse stok durumunu göster.
    if matches!(wizard.kind, ActionKind::Sell) && wizard.fields.len() >= 2 {
        let city = match wizard.fields.first() {
            Some(FieldValue::City(c)) => *c,
            _ => return out,
        };
        let product = match wizard.fields.get(1) {
            Some(FieldValue::Product(p)) => *p,
            _ => return out,
        };
        let stock = app
            .state
            .players
            .get(&HUMAN_ID)
            .map(|p| p.inventory.get(city, product))
            .unwrap_or(0);
        let (msg, color) = if stock == 0 {
            (
                format!(
                    "  ⚠ {} {}'de stoğun YOK — satış eşleşirse reject olur. Kervanla taşıman lazım.",
                    product,
                    city_short(city)
                ),
                Color::Red,
            )
        } else {
            (
                format!(
                    "  📦 {} {}'de stok: {} birim",
                    product,
                    city_short(city),
                    stock
                ),
                Color::Green,
            )
        };
        out.push(Line::from(Span::styled(
            msg,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )));
    }

    // Buy wizard: şehir + ürün seçildiyse mevcut stok + son clearing fiyatı.
    if matches!(wizard.kind, ActionKind::Buy) && wizard.fields.len() >= 2 {
        let city = match wizard.fields.first() {
            Some(FieldValue::City(c)) => *c,
            _ => return out,
        };
        let product = match wizard.fields.get(1) {
            Some(FieldValue::Product(p)) => *p,
            _ => return out,
        };
        let my_stock = app
            .state
            .players
            .get(&HUMAN_ID)
            .map(|p| p.inventory.get(city, product))
            .unwrap_or(0);
        let last_price = app
            .state
            .price_history
            .get(&(city, product))
            .and_then(|v| v.last())
            .map(|(_, p)| *p);
        let price_str = match last_price {
            Some(p) => format!("son fiyat {p}"),
            None => {
                let baseline = app
                    .state
                    .price_baseline
                    .get(&(city, product))
                    .copied()
                    .unwrap_or(Money::ZERO);
                format!("baseline {baseline} (henüz trade yok)")
            }
        };
        out.push(Line::from(Span::styled(
            format!(
                "  📦 {} {}'de zaten {} birim   💹 {}",
                product,
                city_short(city),
                my_stock,
                price_str
            ),
            Style::default().fg(Color::Cyan),
        )));
    }

    out
}

fn render_wizard_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App, wizard: &Wizard) {
    let popup = centered_rect(70, 70, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" 🎯  {}  ", wizard.kind.label()))
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::new();

    // Cash + ilgili maliyet bilgisi — wizard'ın üstünde hep görünür.
    lines.extend(wizard_money_header(app, wizard));
    // Pre-flight uyarıları — "kervan yok", "stok 0" gibi durum bildirimi.
    lines.extend(wizard_preflight_hints(app, wizard));
    // Multi-cargo durumu (Ship wizard): şu ana dek yüklenen ek kalemler.
    if matches!(wizard.kind, ActionKind::Ship) && !wizard.extra_cargo.is_empty() {
        let total: u32 = wizard.extra_cargo.iter().map(|(_, q)| *q).sum();
        let items: Vec<String> = wizard
            .extra_cargo
            .iter()
            .map(|(p, q)| format!("{q} {p}"))
            .collect();
        // Kapasite hesaplaması — seçili kervana göre.
        let cap = if let Some(FieldValue::CaravanId(cid)) = wizard.fields.first() {
            app.state.caravans.get(cid).map(|c| c.capacity)
        } else {
            None
        };
        let cap_str = cap
            .map(|c| format!("  (kapasite {total}/{c} dolu)"))
            .unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!("  📦 Yüklendi: {}{}", items.join(" + "), cap_str),
            Style::default().fg(Color::Cyan),
        )));
    }
    lines.push(Line::from(""));

    // Yol haritası: tamamlanan adımlar + şu anki adım göstergesi.
    if !wizard.fields.is_empty() {
        let mut chips: Vec<Span> = vec![Span::raw("  ")];
        for (i, value) in wizard.fields.iter().enumerate() {
            chips.push(Span::styled(
                format!(" {} ", field_value_label(value)),
                Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            if i < wizard.fields.len() - 1 {
                chips.push(Span::raw(" → "));
            }
        }
        lines.push(Line::from(chips));
        lines.push(Line::from(""));
    }

    if let Some(field) = wizard.current() {
        // Aktif adım: prompt + seçenekler.
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", field.prompt()),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "(adım ".to_string()
                    + &(wizard.fields.len() + 1).to_string()
                    + "/"
                    + &wizard.kind.schema().len().to_string()
                    + ")",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        lines.push(Line::from(""));
        match field {
            FieldKind::City | FieldKind::CityTo => {
                for (i, c) in CityId::ALL.iter().enumerate() {
                    lines.push(option_line(i + 1, city_short(*c), Color::Blue));
                }
            }
            FieldKind::Product => {
                for (i, p) in ProductKind::ALL.iter().enumerate() {
                    lines.push(option_line(i + 1, &format!("{p}"), product_color(*p)));
                }
            }
            FieldKind::FinishedProduct => {
                for (i, p) in ProductKind::FINISHED_GOODS.iter().enumerate() {
                    lines.push(option_line(i + 1, &format!("{p}"), product_color(*p)));
                }
            }
            FieldKind::NewsTier_ => {
                lines.push(option_line(1, "Bronz (bedava)", Color::Rgb(205, 127, 50)));
                lines.push(option_line(
                    2,
                    "Gümüş (500₺, Tüccar bedava)",
                    Color::Rgb(192, 192, 192),
                ));
                lines.push(option_line(
                    3,
                    "Altın (2000₺, 2 tick önce)",
                    Color::Rgb(255, 215, 0),
                ));
            }
            FieldKind::OrderId_ => {
                let mine: Vec<&MarketOrder> = app
                    .state
                    .order_book
                    .values()
                    .flatten()
                    .filter(|o| o.player == HUMAN_ID)
                    .collect();
                if mine.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  (açık emrin yok)",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for (i, o) in mine.iter().enumerate() {
                        let label = format!(
                            "#{} {} {} {} @ {} ({})",
                            o.id.value(),
                            if matches!(o.side, OrderSide::Buy) {
                                "BUY"
                            } else {
                                "SELL"
                            },
                            o.quantity,
                            o.product,
                            o.unit_price,
                            city_short(o.city)
                        );
                        lines.push(option_line(i + 1, &label, Color::White));
                    }
                }
            }
            FieldKind::CaravanId => {
                let mine: Vec<&moneywar_domain::Caravan> = app
                    .state
                    .caravans
                    .values()
                    .filter(|c| c.owner == HUMAN_ID && c.is_idle())
                    .collect();
                if mine.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  (Idle kervanın yok — önce :caravan ile al ya da kervan dönsün)",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for (i, c) in mine.iter().enumerate() {
                        let loc = c.state.current_city().map_or("?", city_short);
                        let label = format!("#{} kap:{}  @ {}", c.id.value(), c.capacity, loc);
                        lines.push(option_line(i + 1, &label, Color::White));
                    }
                }
            }
            FieldKind::LoanId => {
                let mine: Vec<&moneywar_domain::Loan> = app
                    .state
                    .loans
                    .values()
                    .filter(|l| l.borrower == HUMAN_ID && !l.repaid)
                    .collect();
                if mine.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  (açık kredin yok)",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for (i, l) in mine.iter().enumerate() {
                        let total = l.total_due().unwrap_or(Money::ZERO);
                        let label = format!(
                            "#{} principal:{}  borç:{}  vade:tick {}",
                            l.id.value(),
                            l.principal,
                            total,
                            l.due_tick.value()
                        );
                        lines.push(option_line(i + 1, &label, Color::White));
                    }
                }
            }
            FieldKind::ContractId_ => {
                let mine: Vec<&moneywar_domain::Contract> = app.state.contracts.values().collect();
                if mine.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  (kontrat yok)",
                        Style::default().fg(Color::DarkGray),
                    )));
                } else {
                    for (i, c) in mine.iter().enumerate() {
                        let label = format!(
                            "#{} {:?} {} {} @ {}  delivery tick {}",
                            c.id.value(),
                            c.state,
                            c.quantity,
                            c.product,
                            c.unit_price,
                            c.delivery_tick.value()
                        );
                        lines.push(option_line(i + 1, &label, Color::White));
                    }
                }
            }
            FieldKind::QtyU32
            | FieldKind::PriceLira
            | FieldKind::AmountLira
            | FieldKind::DurationTicks
            | FieldKind::DeliveryTick
            | FieldKind::OrderTtl => {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{}_", wizard.text_buf),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                let hint_str: String;
                let hint: &str = match field {
                    FieldKind::QtyU32 if matches!(wizard.kind, ActionKind::Ship) => {
                        "rakamlar → miktar, M → max yükle (stok × kapasite min), Enter onay"
                    }
                    FieldKind::QtyU32 => "rakam tuşları → birim sayısı, Enter onay",
                    FieldKind::PriceLira => {
                        "rakamlar + '.' veya ',' ile ondalık (örn 15.75), Enter onay"
                    }
                    FieldKind::AmountLira => {
                        "rakamlar + '.' veya ',' ile ondalık (örn 10000.50), Enter onay"
                    }
                    FieldKind::DurationTicks => "rakam tuşları → kaç tick, Enter onay",
                    FieldKind::DeliveryTick => "rakam tuşları → teslimat tick'i, Enter onay",
                    FieldKind::OrderTtl => {
                        let b = app.state.config.balance;
                        hint_str = format!(
                            "kitapta kaç tick kalsın (1..{})  boş → default {}",
                            b.max_order_ttl, b.default_order_ttl
                        );
                        &hint_str
                    }
                    _ => "",
                };
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  💡 {hint}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
    } else if wizard.confirm_more_cargo {
        // Ship: son (Product, Qty) yüklendi. Kullanıcı başka ürün eklemek
        // isterse `1`, bitirmek isterse `2`/Enter.
        let (last_prod, last_qty) = match (wizard.fields.get(2), wizard.fields.get(3)) {
            (Some(FieldValue::Product(p)), Some(FieldValue::Number(q))) => (*p, *q),
            _ => (ProductKind::Pamuk, 0),
        };
        lines.push(Line::from(Span::styled(
            format!("  ✓ {} × {} yüklendi.", last_prod, last_qty),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                "  [1] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("başka ürün ekle      "),
            Span::styled(
                "[2] / Enter",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" dispatch'i başlat"),
        ]));
    } else {
        // Tüm alanlar dolu → onay ekranı.
        lines.push(Line::from(Span::styled(
            "  ✓ Tüm alanlar tamam.",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Enter → komutu queue'ya ekle (sonra SPACE ile tick ilerlet)",
            Style::default().fg(Color::Yellow),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ← Backspace: bir adım geri    Esc: iptal",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn option_line(num: usize, label: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!(" {num} "),
            Style::default()
                .bg(Color::Yellow)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            label.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

fn field_value_label(value: &FieldValue) -> String {
    match value {
        FieldValue::City(c) => city_short(*c).to_string(),
        FieldValue::Product(p) => format!("{p}"),
        FieldValue::Number(n) => n.to_string(),
        FieldValue::NewsTier(t) => format!("{t}"),
        FieldValue::OrderId(id) => format!("ord#{}", id.value()),
        FieldValue::CaravanId(id) => format!("crv#{}", id.value()),
        FieldValue::LoanId(id) => format!("loan#{}", id.value()),
        FieldValue::ContractId(id) => format!("ctr#{}", id.value()),
    }
}

fn render_holdings_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(85, 90, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 💼  Varlıklarım ")
        .border_style(Style::default().fg(Color::Green));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::new();

    // --- Açık emirler ---
    lines.push(Line::from(Span::styled(
        "📋  Açık Emirler (book'ta, tick sonunda eşleşecek)",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    let mut open_orders: Vec<&MarketOrder> = Vec::new();
    for orders in app.state.order_book.values() {
        for o in orders {
            if o.player == HUMAN_ID {
                open_orders.push(o);
            }
        }
    }
    if open_orders.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (yok — `:buy` veya `:sell` ile emir gir)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for o in open_orders {
            let side_color = if matches!(o.side, OrderSide::Buy) {
                Color::Green
            } else {
                Color::Red
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("#{:<3}", o.id.value()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!(
                        "{:<4}",
                        if matches!(o.side, OrderSide::Buy) {
                            "BUY"
                        } else {
                            "SELL"
                        }
                    ),
                    Style::default().fg(side_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:>4} ", o.quantity),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:<10}", o.product),
                    Style::default().fg(product_color(o.product)),
                ),
                Span::raw("@ "),
                Span::styled(
                    format!("{}", o.unit_price),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("({})", city_short(o.city)),
                    Style::default().fg(Color::Blue),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));

    // --- Fabrikalar ---
    lines.push(Line::from(Span::styled(
        "🏭  Fabrikalar",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    let mut my_factories: Vec<&moneywar_domain::Factory> = app
        .state
        .factories
        .values()
        .filter(|f| f.owner == HUMAN_ID)
        .collect();
    my_factories.sort_by_key(|f| f.id);
    if my_factories.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (yok — `:build <şehir> <bitmiş_ürün>` ile kur, ilki bedava)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for factory in my_factories {
            let is_idle = factory.is_atil(
                app.state.current_tick,
                moneywar_engine::IDLE_FACTORY_THRESHOLD,
            );
            let last = factory
                .last_production_tick
                .map_or_else(|| "hiç".into(), |t| format!("tick {}", t.value()));
            let pending = factory.pending_units();
            let status_color = if is_idle { Color::Red } else { Color::Green };
            let status_label = if is_idle { "ATIL" } else { "ÜRETİYOR" };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("#{:<3}", factory.id.value()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{:<10}", factory.product),
                    Style::default().fg(product_color(factory.product)),
                ),
                Span::styled(
                    format!("{:<10}", city_short(factory.city)),
                    Style::default().fg(Color::Blue),
                ),
                Span::raw("son üretim: "),
                Span::styled(format!("{:<12}", last), Style::default().fg(Color::Gray)),
                Span::raw("batches: "),
                Span::styled(format!("{pending:<3}"), Style::default().fg(Color::Magenta)),
                Span::raw("  "),
                Span::styled(
                    format!(" {status_label} "),
                    Style::default()
                        .bg(status_color)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));

    // --- Kervanlar ---
    lines.push(Line::from(Span::styled(
        "🚚  Kervanlar",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    let mut my_caravans: Vec<&moneywar_domain::Caravan> = app
        .state
        .caravans
        .values()
        .filter(|c| c.owner == HUMAN_ID)
        .collect();
    my_caravans.sort_by_key(|c| c.id);
    if my_caravans.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (yok — `:caravan <şehir>` ile al)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for caravan in my_caravans {
            let state_desc = match &caravan.state {
                moneywar_domain::CaravanState::Idle { location } => {
                    format!("IDLE  @ {}", city_short(*location))
                }
                moneywar_domain::CaravanState::EnRoute {
                    from,
                    to,
                    arrival_tick,
                    cargo,
                } => {
                    format!(
                        "EN ROUTE  {}→{}  varış: tick {}  yük: {}",
                        city_short(*from),
                        city_short(*to),
                        arrival_tick.value(),
                        cargo.total_units()
                    )
                }
            };
            let color = if caravan.is_idle() {
                Color::Gray
            } else {
                Color::Cyan
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("#{:<3}", caravan.id.value()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("kap {:>3}  ", caravan.capacity),
                    Style::default().fg(Color::Magenta),
                ),
                Span::styled(state_desc, Style::default().fg(color)),
            ]));
        }
    }

    lines.push(Line::from(""));

    // --- Aktif kontratlar ---
    lines.push(Line::from(Span::styled(
        "🤝  Aktif Kontratlar",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    let my_contracts: Vec<&moneywar_domain::Contract> = app
        .state
        .contracts
        .values()
        .filter(|c| c.seller == HUMAN_ID || c.accepted_by == Some(HUMAN_ID))
        .collect();
    if my_contracts.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (yok)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for c in my_contracts {
            let role = if c.seller == HUMAN_ID {
                "SATICI"
            } else {
                "ALICI"
            };
            let state_str = match c.state {
                moneywar_domain::ContractState::Proposed => "ÖNERİLDİ".to_string(),
                moneywar_domain::ContractState::Active => "AKTİF".to_string(),
                moneywar_domain::ContractState::Fulfilled => "TESLİM".to_string(),
                moneywar_domain::ContractState::Breached { .. } => "BREACH".to_string(),
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("#{:<3}", c.id.value()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    format!("{:<7}", role),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:>4} ", c.quantity),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:<10}", c.product),
                    Style::default().fg(product_color(c.product)),
                ),
                Span::raw("@ "),
                Span::styled(
                    format!("{}", c.unit_price),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("({})", city_short(c.delivery_city)),
                    Style::default().fg(Color::Blue),
                ),
                Span::raw(" teslim: "),
                Span::styled(
                    format!("tick {}", c.delivery_tick.value()),
                    Style::default().fg(Color::Gray),
                ),
                Span::raw(" "),
                Span::styled(
                    format!(" {state_str} "),
                    Style::default()
                        .bg(Color::Cyan)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));

    // --- Aktif krediler ---
    lines.push(Line::from(Span::styled(
        "💰  Aktif Krediler",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    let my_loans: Vec<&moneywar_domain::Loan> = app
        .state
        .loans
        .values()
        .filter(|l| l.borrower == HUMAN_ID && !l.repaid)
        .collect();
    if my_loans.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (yok — `:loan <miktar> <vade>` ile al, %15 faiz)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for l in my_loans {
            let due = l.total_due().unwrap_or(Money::ZERO);
            let ticks_left = l
                .due_tick
                .value()
                .saturating_sub(app.state.current_tick.value());
            let urgency = if ticks_left <= 2 {
                Color::Red
            } else {
                Color::Yellow
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("#{:<3}", l.id.value()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw("principal: "),
                Span::styled(
                    format!("{}", l.principal),
                    Style::default().fg(Color::Green),
                ),
                Span::raw("  borç: "),
                Span::styled(
                    format!("{}", due),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  vade: "),
                Span::styled(
                    format!("tick {}", l.due_tick.value()),
                    Style::default().fg(Color::Gray),
                ),
                Span::raw("  kalan: "),
                Span::styled(
                    format!("{ticks_left}t"),
                    Style::default().fg(urgency).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Herhangi bir tuşa bas → kapat",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_news_inbox_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(80, 85, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 📰  Haber Kutusu ")
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let current = app.state.current_tick;
    let inbox = app.state.news_inbox.get(&HUMAN_ID);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "  Açıklanmış haberler (disclosed_tick ≤ now):",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));

    let disclosed: Vec<&NewsItem> = inbox
        .map(|v| {
            v.iter()
                .filter(|n| !current.is_before(n.disclosed_tick))
                .collect()
        })
        .unwrap_or_default();
    if disclosed.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (yok — henüz açıklanmış haber yok)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for n in disclosed.iter().rev().take(20) {
            let icon = match n.tier {
                NewsTier::Bronze => "🥉",
                NewsTier::Silver => "🥈",
                NewsTier::Gold => "🥇",
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{icon} "),
                    Style::default()
                        .fg(tier_color(n.tier))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("tick {:>3}: ", n.event_tick.value()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format_event(&n.event), Style::default().fg(Color::White)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Bekleyen haberler (gelecekte açılacak — sadece Gold'un önden gördükleri):",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));

    let pending: Vec<&NewsItem> = inbox
        .map(|v| {
            v.iter()
                .filter(|n| current.is_before(n.disclosed_tick))
                .collect()
        })
        .unwrap_or_default();
    if pending.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (yok — üst tier aboneliği alırsan ön-görüler burada)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for n in pending.iter().take(10) {
            let icon = match n.tier {
                NewsTier::Bronze => "🥉",
                NewsTier::Silver => "🥈",
                NewsTier::Gold => "🥇",
            };
            let ticks_ahead = n.disclosed_tick.value().saturating_sub(current.value());
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{icon} "),
                    Style::default()
                        .fg(tier_color(n.tier))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("+{ticks_ahead}t: "),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(format_event(&n.event), Style::default().fg(Color::Gray)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Herhangi bir tuşa bas → kapat",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::ITALIC),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_recent_trades_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(80, 80, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 🔄  Son Eşleşmeler  —  kim kime ne sattı ")
        .border_style(Style::default().fg(Color::Green));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "  En yeni eşleşmeler üstte. Kendi işlemlerin sarı.",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    if app.recent_trades.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (henüz eşleşme yok — SPACE ile tick ilerlet)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Başlık satırı.
        lines.push(Line::from(Span::styled(
            "  Tick  Satıcı               →  Alıcı                 Miktar  Ürün         Şehir     Fiyat",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  ────  ──────────────────     ──────────────────      ──────  ───────────  ────────  ─────",
            Style::default().fg(Color::DarkGray),
        )));

        for t in app.recent_trades.iter().rev() {
            let is_mine = t.buyer_name.starts_with("Sen") || t.seller_name.starts_with("Sen");
            let color = if is_mine { Color::Yellow } else { Color::White };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {:>4}  {:<20}  →  {:<20}  {:>6}  {:<11}  {:<8}  {}",
                    t.tick.value(),
                    truncate_str(&t.seller_name, 20),
                    truncate_str(&t.buyer_name, 20),
                    t.quantity,
                    format!("{}", t.product),
                    city_short(t.city),
                    t.price,
                ),
                Style::default().fg(color).add_modifier(if is_mine {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Esc / Enter / Space → kapat",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_open_book_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(85, 85, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 📖  Açık Pazar  —  orderbook depth ")
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "  Her (şehir × ürün) için en iyi 3 bid (alıcı) + 3 ask (satıcı). Kendi emirlerin sarı.",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    // Sadece emir bulunan (şehir, ürün)'ü göster — determinism için BTreeMap sırası.
    let mut any = false;
    for ((city, product), orders) in &app.state.order_book {
        let (mut buys, mut sells): (Vec<_>, Vec<_>) = orders
            .iter()
            .partition(|o| matches!(o.side, OrderSide::Buy));
        // Best bid: yüksek fiyat önce. Best ask: düşük fiyat önce.
        buys.sort_by(|a, b| b.unit_price.cmp(&a.unit_price));
        sells.sort_by(|a, b| a.unit_price.cmp(&b.unit_price));

        if buys.is_empty() && sells.is_empty() {
            continue;
        }
        any = true;

        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{} {}", city_short(*city), product),
                Style::default()
                    .fg(product_color(*product))
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        // Başlık.
        lines.push(Line::from(Span::styled(
            "     BID (alıcı)                          │     ASK (satıcı)",
            Style::default().fg(Color::DarkGray),
        )));

        // İlk 3'er satır yan yana.
        for i in 0..3 {
            let bid = buys.get(i);
            let ask = sells.get(i);
            let bid_text = bid.map(|o| {
                let is_mine = o.player == HUMAN_ID;
                let owner = app
                    .state
                    .players
                    .get(&o.player)
                    .map(|p| truncate_str(&p.name, 14))
                    .unwrap_or_default();
                (
                    format!(
                        "{:<14}  {:>6} adet @ {:>8}  (TTL {})",
                        owner, o.quantity, o.unit_price, o.remaining_ticks
                    ),
                    is_mine,
                )
            });
            let ask_text = ask.map(|o| {
                let is_mine = o.player == HUMAN_ID;
                let owner = app
                    .state
                    .players
                    .get(&o.player)
                    .map(|p| truncate_str(&p.name, 14))
                    .unwrap_or_default();
                (
                    format!(
                        "{:<14}  {:>6} adet @ {:>8}  (TTL {})",
                        owner, o.quantity, o.unit_price, o.remaining_ticks
                    ),
                    is_mine,
                )
            });

            let mut spans = vec![Span::raw("     ")];
            match bid_text {
                Some((txt, mine)) => spans.push(Span::styled(
                    txt,
                    Style::default().fg(if mine { Color::Yellow } else { Color::Green }),
                )),
                None => spans.push(Span::styled(
                    format!("{:<44}", "—"),
                    Style::default().fg(Color::DarkGray),
                )),
            }
            spans.push(Span::raw("  │  "));
            match ask_text {
                Some((txt, mine)) => spans.push(Span::styled(
                    txt,
                    Style::default().fg(if mine { Color::Yellow } else { Color::Red }),
                )),
                None => spans.push(Span::styled(
                    "—".to_string(),
                    Style::default().fg(Color::DarkGray),
                )),
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
    }

    if !any {
        lines.push(Line::from(Span::styled(
            "  (pazar boş — bir emir ver ya da SPACE ile tick ilerlet)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Esc / Enter / Space → kapat",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_caravan_panel_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(85, 80, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 🚚  Kervanlarım  —  detaylı yönetim ")
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let mine: Vec<&moneywar_domain::Caravan> = app
        .state
        .caravans
        .values()
        .filter(|c| c.owner == HUMAN_ID)
        .collect();

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "  Tüm kervanlarının anlık durumu. Hızlı dispatch için 'd' (Ship wizard).",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    if mine.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (Kervanın yok — 'c' ile satın al)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // Başlık satırı.
        lines.push(Line::from(Span::styled(
            "  ID    Kapasite   Durum                              Cargo",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  ────  ────────   ─────────────────────────────────  ─────────────────",
            Style::default().fg(Color::DarkGray),
        )));

        let current = app.state.current_tick;
        for caravan in &mine {
            let (status, status_color): (String, Color) = match &caravan.state {
                moneywar_domain::CaravanState::Idle { location } => {
                    (format!("🟢 Idle @ {}", city_short(*location)), Color::Green)
                }
                moneywar_domain::CaravanState::EnRoute {
                    from,
                    to,
                    arrival_tick,
                    ..
                } => {
                    let eta = arrival_tick.value().saturating_sub(current.value());
                    (
                        format!(
                            "🟡 {} → {} (t{}, {} tick kaldı)",
                            city_short(*from),
                            city_short(*to),
                            arrival_tick.value(),
                            eta
                        ),
                        Color::Yellow,
                    )
                }
            };

            // Cargo özet: EnRoute'taysa cargo items, Idle'daysa boş.
            let cargo_str: String = match &caravan.state {
                moneywar_domain::CaravanState::EnRoute { cargo, .. } => {
                    let parts: Vec<String> =
                        cargo.entries().map(|(p, q)| format!("{q} {p}")).collect();
                    if parts.is_empty() {
                        "—".into()
                    } else {
                        parts.join(" + ")
                    }
                }
                moneywar_domain::CaravanState::Idle { .. } => "(boş)".into(),
            };

            lines.push(Line::from(vec![
                Span::styled(
                    format!("  #{:<4}  ", caravan.id.value()),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:>3} brm    ", caravan.capacity),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    format!("{status:<35}"),
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(cargo_str, Style::default().fg(Color::Cyan)),
            ]));
        }

        let idle_count = mine.iter().filter(|c| c.is_idle()).count();
        let moving_count = mine.len() - idle_count;
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "  Toplam: {}  |  Idle: {idle_count}  |  Yolda: {moving_count}",
                mine.len()
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Esc/Enter/Space → kapat  |  d → yeni dispatch  |  c → yeni kervan al",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_debug_log_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App, scroll: usize) {
    let popup = centered_rect(90, 90, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 🛠  Debug Log  —  ham LogEntry akışı ")
        .border_style(Style::default().fg(Color::Magenta));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let total = app.debug_log.len();
    let visible_rows = (inner.height as usize).saturating_sub(3); // header + footer payı
    let visible_rows = visible_rows.max(1);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            "  ↑/k yukarı  ↓/j aşağı  PgUp/PgDn  Home/End  Esc kapat   ",
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            format!("toplam {total} entry, scroll {scroll}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    if total == 0 {
        lines.push(Line::from(Span::styled(
            "  (henüz log yok — SPACE ile tick ilerlet)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        // En yeni baştan: iter().rev() ile geri-sıralı görüntü, scroll offset'iyle kaydır.
        let iter = app.debug_log.iter().rev().skip(scroll).take(visible_rows);
        for entry in iter {
            // Tick + actor renkli başlık, sonra Debug format'lı event tek satırda.
            let actor_str = entry
                .actor
                .map(|a| {
                    app.state
                        .players
                        .get(&a)
                        .map(|p| truncate_str(&p.name, 20))
                        .unwrap_or_else(|| format!("#{}", a.value()))
                })
                .unwrap_or_else(|| "system".into());
            let event_color = debug_event_color(&entry.event);
            let event_debug = format!("{:?}", entry.event);
            // Event debug çok uzun olabilir — inner.width'e göre kısalt.
            let max_width = (inner.width as usize).saturating_sub(32).max(40);
            let event_str = if event_debug.chars().count() > max_width {
                let mut out: String = event_debug.chars().take(max_width - 1).collect();
                out.push('…');
                out
            } else {
                event_debug
            };

            lines.push(Line::from(vec![
                Span::styled(
                    format!("  t{:>3}  ", entry.tick.value()),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("{:<20}  ", actor_str),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(event_str, Style::default().fg(event_color)),
            ]));
        }
    }

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn debug_event_color(event: &moneywar_engine::LogEvent) -> Color {
    use moneywar_engine::LogEvent;
    match event {
        LogEvent::OrderMatched { .. } => Color::Green,
        LogEvent::OrderExpired { .. } => Color::DarkGray,
        LogEvent::FillRejected { .. } | LogEvent::CommandRejected { .. } => Color::Red,
        LogEvent::CommandAccepted { .. } => Color::White,
        LogEvent::ProductionCompleted { .. } | LogEvent::ProductionStarted { .. } => {
            Color::LightYellow
        }
        LogEvent::CaravanDispatched { .. } | LogEvent::CaravanArrived { .. } => Color::Blue,
        LogEvent::EventScheduled { .. } => Color::Magenta,
        LogEvent::MarketCleared { .. } => Color::Gray,
        LogEvent::LoanTaken { .. }
        | LogEvent::LoanRepaid { .. }
        | LogEvent::LoanDefaulted { .. } => Color::LightRed,
        _ => Color::White,
    }
}

/// "15.75" veya "15,75" → 1575 (cents). Ondalık yoksa lira × 100.
/// Ondalık basamak 2'den fazlaysa truncate edilir (kuruş altı yok).
/// Geçersiz input (harf, birden çok nokta, tamamen negatif) → None.
fn parse_decimal_as_cents(raw: &str) -> Option<u64> {
    let s = raw.trim().replace(',', ".");
    if s.is_empty() {
        return None;
    }
    let (whole_str, frac_str) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s.as_str(), ""),
    };
    // Whole part: digits only.
    if whole_str.is_empty() || !whole_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: u64 = whole_str.parse().ok()?;
    // Frac part: en fazla 2 digit kullan, eksikse sağdan sıfır doldur.
    let frac2: String = frac_str.chars().take(2).collect();
    if !frac2.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let padded: String = frac2
        .chars()
        .chain(std::iter::repeat_n('0', 2 - frac2.len()))
        .collect();
    let frac: u64 = if padded.is_empty() {
        0
    } else {
        padded.parse().ok()?
    };
    whole.checked_mul(100)?.checked_add(frac)
}

/// Stringi max uzunlukta kes, uzunsa `…` ile bitir.
fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn render_game_over_overlay(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let popup = centered_rect(85, 90, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" ★  SEZON SONU  —  REVEAL  ★ ")
        .border_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    // Sezon sonu reveal — leaderboard ile aynı filter (sadece gerçek rakipler:
    // insan + Sanayici + Tüccar). Likidite NPC'leri Alıcı/Esnaf/Spekülatör
    // skor tablosuna katılmaz.
    let board: Vec<PlayerScore> = leaderboard(&app.state)
        .into_iter()
        .filter(|sc| {
            app.state.players.get(&sc.player_id).is_none_or(|p| {
                !p.has_npc_kind(NpcKind::Alici)
                    && !p.has_npc_kind(NpcKind::Esnaf)
                    && !p.has_npc_kind(NpcKind::Spekulator)
            })
        })
        .collect();
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "         🏁  90 tick bitti — rakamlar açılıyor  🏁",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    // Kazanan dev başlık
    if let Some(winner) = board.first() {
        let name = app
            .state
            .players
            .get(&winner.player_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(
                format!("🥇  Şampiyon: {}", name),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                format!("{}", winner.total),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));
    }

    // Detay tablo başlığı
    lines.push(Line::from(Span::styled(
        "  Sıra  Oyuncu              Nakit      Stok       Fabrika    Escrow     TOPLAM",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    )));
    for (idx, sc) in board.iter().enumerate() {
        let name = app
            .state
            .players
            .get(&sc.player_id)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        let medal = match idx {
            0 => "🥇",
            1 => "🥈",
            2 => "🥉",
            _ => "  ",
        };
        let is_human = sc.player_id == HUMAN_ID;
        let row_style = if is_human {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else if idx == 0 {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!(
                "  {medal} {:<2}   {:<18}  {:>9}  {:>9}  {:>9}  {:>9}  {:>9}",
                idx + 1,
                name,
                format!("{}", sc.cash),
                format!("{}", sc.stock_value),
                format!("{}", sc.factory_value),
                format!("{}", sc.escrow_value),
                format!("{}", sc.total),
            ),
            row_style,
        )));
    }

    lines.push(Line::from(""));

    // Kendi yerini özellikle vurgula
    if let Some(my) = board.iter().position(|s| s.player_id == HUMAN_ID) {
        let my_sc = &board[my];
        let rank = my + 1;
        let comment = match rank {
            1 => "🎉  Şampiyonsun! Stratejin işe yaradı.",
            2 => "💪  İkincilik fena değil — sonraki sezona güçlü başlarsın.",
            3 => "🥉  Podyumda yer aldın.",
            r if r <= 5 => "👍  Top 5 — iyi bir sezon.",
            _ => "📚  Kaybettin bu sezon — kalem dökümüne bak, nereyi kaçırdın?",
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  Senin yerin: #{rank}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                format!("{}", my_sc.total),
                Style::default().fg(Color::Yellow),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {comment}"),
            Style::default().fg(Color::White),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  q ile çık",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn help_kv(key: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{key:<12}"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(desc.to_string(), Style::default().fg(Color::White)),
    ])
}

fn help_cmd(cmd: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{cmd:<52}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(desc.to_string(), Style::default().fg(Color::Gray)),
    ])
}

fn render_header(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let season = app.state.season_progress();
    // Sezon arc faz göstergesi: erken=Bahar, orta=Yaz, geç=Hasat. Olay
    // sıklığı ve atmosfer bu fazlara göre değişir; oyuncu görsel olarak
    // nerede olduğunu görsün → "tickleri amaçsız geçmiyor" hissi.
    let (phase_icon, phase_label, phase_color) = if season.is_late() {
        ("🍂", "Hasat", Color::Rgb(220, 120, 60))
    } else if season.is_mid() {
        ("🌞", "Yaz", Color::Yellow)
    } else {
        ("🌱", "Bahar", Color::Green)
    };
    let title = Line::from(vec![
        Span::styled(
            "  MoneyWar  ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(
                "Tick {}/{}",
                app.state.current_tick.value(),
                app.state.config.season_ticks
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{phase_icon} {phase_label}"),
            Style::default()
                .fg(phase_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(format!("({season})"), Style::default().fg(Color::DarkGray)),
        Span::raw("  "),
        Span::styled(
            format!("{}", app.state.config.preset),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw("  "),
        Span::styled(
            format!("seed {}", app.state.room_id.value() % 1_000_000),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("  "),
        if app.auto_sim {
            Span::styled(
                "● AUTO-SIM",
                Style::default()
                    .fg(Color::LightRed)
                    .add_modifier(Modifier::RAPID_BLINK | Modifier::BOLD),
            )
        } else {
            Span::styled("○ manual", Style::default().fg(Color::DarkGray))
        },
        Span::raw("  "),
        if app.game_over() {
            Span::styled(
                "★ SEZON SONU ★",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        },
    ]);

    let block = Block::default().borders(Borders::BOTTOM);
    let para = Paragraph::new(title).block(block);
    f.render_widget(para, area);
}

/// Header altında 1 satırlık aktif olay şeridi. Her aktif şok bir chip:
/// `🌵 Ankara/Buğday +18%`, `🌧️ İzmir/Zeytin -8%`. Renkler etki yönüne göre.
/// Şok yoksa fonksiyon çağrılmaz (layout'ta 0 satır ayrılır).
fn render_active_shocks(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    use moneywar_domain::GameEvent;
    let mut spans: Vec<Span> = vec![Span::styled(
        " 📰 ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(220, 200, 80))
            .add_modifier(Modifier::BOLD),
    )];
    spans.push(Span::raw(" "));

    // BTreeMap zaten (CityId, ProductKind) anahtar sırasına göre stabil iter.
    let total = app.state.active_shocks.len();
    for (idx, ((city, product), shock)) in app.state.active_shocks.iter().enumerate().take(6) {
        let icon = match shock.source {
            GameEvent::Drought { .. } => "🌵",
            GameEvent::Strike { .. } => "✊",
            GameEvent::BumperHarvest { .. } => "🌾",
            GameEvent::RoadClosure { .. } => "🚧",
            GameEvent::NewMarket { .. } => "🎉",
        };
        let pct = shock.multiplier_pct;
        let (sign, color) = if pct > 0 {
            ("+", Color::LightRed)
        } else if pct < 0 {
            ("", Color::LightGreen) // negatif yüzde zaten "-" içeriyor
        } else {
            ("", Color::Gray)
        };
        let remaining = app
            .state
            .current_tick
            .ticks_until(shock.expires_at)
            .unwrap_or(0);
        spans.push(Span::styled(
            format!(
                "{icon} {}/{} {sign}{pct}% ({remaining}t)",
                city_short(*city),
                product
            ),
            Style::default().fg(color),
        ));
        if idx + 1 < total.min(6) {
            spans.push(Span::raw("  "));
        }
    }
    if total > 6 {
        spans.push(Span::styled(
            format!("  +{} daha", total - 6),
            Style::default().fg(Color::DarkGray),
        ));
    }
    let line = Line::from(spans);
    let para = Paragraph::new(line);
    f.render_widget(para, area);
}

fn render_middle(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // Sol sütun: player paneli (üst) + role-adaptive panel (alt):
    //   Sanayici → Fabrikalarım durum tablosu
    //   Tüccar   → Arbitraj fırsatları
    // Haber paneli ana sayfadan çıkarıldı — `n` ile overlay'den açılır.
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(cols[0]);
    render_player_panel(f, left[0], app);
    render_role_adaptive_panel(f, left[1], app);

    // Sağ sütun: market (üst) + tick log (alt)
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(cols[1]);
    render_market_panel(f, right[0], app);
    render_log_panel(f, right[1], app);
}

fn render_player_panel(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let Some(player) = app.state.players.get(&HUMAN_ID) else {
        return;
    };
    let score = score_player(&app.state, HUMAN_ID);
    let factory_count = app
        .state
        .factories
        .values()
        .filter(|fac| fac.owner == HUMAN_ID)
        .count();
    let caravan_count = app
        .state
        .caravans
        .values()
        .filter(|c| c.owner == HUMAN_ID)
        .count();

    let mut lines = Vec::new();

    // Satır 1: isim + rol
    lines.push(Line::from(vec![
        Span::styled(
            player.name.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{}", player.role),
            Style::default().fg(role_color(player.role)),
        ),
    ]));

    // Satır 2: nakit + skor tek satırda (büyük vurgu)
    lines.push(Line::from(vec![
        Span::styled("💰 ", Style::default().fg(Color::Green)),
        Span::styled(
            format!("{}", player.cash),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("★ ", Style::default().fg(Color::Yellow)),
        Span::styled(
            format!("{}", score.total),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Satır 3: skor döküm kompakt (minik)
    lines.push(Line::from(vec![Span::styled(
        format!(
            "  cash {}  stok {}  fab {}  esc {}",
            compact_money(score.cash),
            compact_money(score.stock_value),
            compact_money(score.factory_value),
            compact_money(score.escrow_value),
        ),
        Style::default().fg(Color::DarkGray),
    )]));

    // Satır 4: varlık sayıları tek satırda
    lines.push(Line::from(vec![
        Span::styled("🏭 ", Style::default().fg(Color::Rgb(210, 140, 80))),
        Span::styled(
            format!("{factory_count}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled("🚚 ", Style::default().fg(Color::Rgb(120, 180, 240))),
        Span::styled(
            format!("{caravan_count}"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            format!("stok toplam {}", player.inventory.total_units()),
            Style::default().fg(Color::Gray),
        ),
    ]));

    // Pending komutlar — SPACE'e basılınca işlenecek (henüz motor görmedi).
    let pending: Vec<&Command> = app.pending_human_cmds.iter().collect();
    if !pending.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("⏳ Bekleyen ({}) — SPACE'le yolla", pending.len()),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        )));
        for cmd in pending.iter().take(3) {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    describe_command_short(cmd),
                    Style::default().fg(Color::White),
                ),
            ]));
        }
        if pending.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("  … +{} daha", pending.len() - 3),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    // Açık emirler — book'ta, tick sonu clearing'e gidecek.
    let mut my_open: Vec<&MarketOrder> = app
        .state
        .order_book
        .values()
        .flatten()
        .filter(|o| o.player == HUMAN_ID)
        .collect();
    my_open.sort_by_key(|o| o.id);
    if !my_open.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("📋 Açık emirler ({}) — bu tick clearing'e", my_open.len()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for o in my_open.iter().take(3) {
            let side_color = if matches!(o.side, OrderSide::Buy) {
                Color::Green
            } else {
                Color::Red
            };
            let side_label = if matches!(o.side, OrderSide::Buy) {
                "BUY"
            } else {
                "SELL"
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("#{}", o.id.value()),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<4}", side_label),
                    Style::default().fg(side_color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{:>3} ", o.quantity),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{:<8}", o.product),
                    Style::default().fg(product_color(o.product)),
                ),
                Span::raw("@"),
                Span::styled(
                    format!("{}", o.unit_price),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!(" {}", city_short(o.city)),
                    Style::default().fg(Color::Blue),
                ),
            ]));
        }
        if my_open.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("  … +{} daha (m ile tam)", my_open.len() - 3),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    // Stok — en sonda, daha az yer alsın
    let mut entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, _, q)| *q > 0)
        .collect();
    entries.sort_by_key(|(_, _, q)| std::cmp::Reverse(*q));
    lines.push(Line::from(""));
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "📦 Stok yok",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("📦 En çok stok ({}):", entries.len()),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
        for (city, product, qty) in entries.iter().take(2) {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{:>4}", qty),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<10}", product),
                    Style::default().fg(product_color(*product)),
                ),
                Span::styled(
                    format!("@ {}", city_short(*city)),
                    Style::default().fg(Color::Blue),
                ),
            ]));
        }
        if entries.len() > 2 {
            lines.push(Line::from(Span::styled(
                format!("  … +{} (m ile tam)", entries.len() - 2),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Oyuncu ")
        .border_style(Style::default().fg(Color::Cyan));
    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

/// Para kısa formatı — skor dökümü gibi dar alanlar için.
/// 12 345 678 cent → "123k", 1_234_567_890 → "12.3M".
fn compact_money(money: Money) -> String {
    let cents = money.as_cents();
    let abs = cents.unsigned_abs();
    let sign = if cents < 0 { "-" } else { "" };
    let lira = abs / 100;
    if lira >= 1_000_000 {
        format!("{sign}{:.1}M", lira as f64 / 1_000_000.0)
    } else if lira >= 10_000 {
        format!("{sign}{}k", lira / 1_000)
    } else if lira >= 1_000 {
        format!("{sign}{:.1}k", lira as f64 / 1_000.0)
    } else {
        format!("{sign}{lira}")
    }
}

fn render_market_panel(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    // Matrix layout: ürün satır, şehir sütun — çok daha kompakt.
    // Her hücre 2 satır: 1) fiyat + delta oku, 2) 6-tik sparkline.
    let mut header = vec![ratatui::text::Text::from(Span::styled(
        "Ürün",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    ))];
    for city in CityId::ALL {
        header.push(ratatui::text::Text::from(Span::styled(
            city_short(city),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
    }
    let header_row = Row::new(header);

    let mut rows: Vec<Row> = Vec::new();
    let mut any_price = false;
    for product in ProductKind::ALL {
        let mut cells: Vec<ratatui::text::Text> = vec![ratatui::text::Text::from(Span::styled(
            format!("{product}"),
            Style::default()
                .fg(product_color(product))
                .add_modifier(Modifier::BOLD),
        ))];
        for city in CityId::ALL {
            let key = (city, product);
            let history = app.state.price_history.get(&key);
            let last = history.and_then(|v| v.last()).map(|(_, p)| *p);
            let Some(price) = last else {
                cells.push(ratatui::text::Text::from(Span::styled(
                    "  —",
                    Style::default().fg(Color::DarkGray),
                )));
                continue;
            };
            any_price = true;
            let prev = app.prev_prices.get(&key).copied();
            let delta = prev.map_or(0, |p| price.as_cents() - p.as_cents());
            let (arrow, color) = if delta > 0 {
                ("↑", Color::Green)
            } else if delta < 0 {
                ("↓", Color::Red)
            } else {
                ("·", Color::DarkGray)
            };
            let price_line = Line::from(vec![
                Span::styled(
                    format!("{}", price),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {arrow}"), Style::default().fg(color)),
            ]);
            // Sparkline tick başında cache'lendi — render'da sadece okuma.
            let spark_str = app
                .cached_sparklines
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let spark_line = Line::from(Span::styled(
                spark_str,
                Style::default().fg(Color::Rgb(150, 180, 220)),
            ));
            let mut text = ratatui::text::Text::from(price_line);
            text.lines.push(spark_line);
            cells.push(text);
        }
        rows.push(Row::new(cells).height(2));
    }

    if !any_price {
        rows = vec![Row::new(vec![ratatui::text::Text::from(Span::styled(
            "(henüz clearing olmadı — SPACE ile tick)",
            Style::default().fg(Color::DarkGray),
        ))])];
    }

    let widths = [
        Constraint::Length(12),
        Constraint::Min(10),
        Constraint::Min(10),
        Constraint::Min(10),
    ];
    // Sezon haritası — bu oyunda hangi şehir hangi hamı ucuza üretir.
    // Specialty her oyun farklı (seed shuffled), oyuncu burada hızlıca görür.
    let specialty_summary: String = CityId::ALL
        .iter()
        .map(|c| format!("{}→{}", city_short(*c), app.state.cheap_raw_for(*c)))
        .collect::<Vec<_>>()
        .join("  ·  ");
    let title = format!(" Pazar  ·  Sezon haritası: {specialty_summary} ");
    let table = Table::new(rows, widths).header(header_row).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(table, area);
}

/// 8-seviyeli Unicode block elements ile fiyat tarihçesi sparkline'ı.
/// Min-max normalize + en yakın seviye. Boş slice → boş string.
/// Tek değer → tek tepe (▄). Aralık 0 ise tüm değerler aynı → flat orta seviye.
fn sparkline(prices: &[Money]) -> String {
    const CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if prices.is_empty() {
        return String::new();
    }
    let cents: Vec<i64> = prices.iter().map(|p| p.as_cents()).collect();
    let min = *cents.iter().min().unwrap_or(&0);
    let max = *cents.iter().max().unwrap_or(&0);
    let range = (max - min).max(1);
    cents
        .iter()
        .map(|c| {
            // 0..range → 0..7 yuvarlanmış index
            let normalized = ((c - min) * 7 + range / 2) / range;
            let idx = normalized.clamp(0, 7) as usize;
            CHARS[idx]
        })
        .collect()
}

/// Role-adaptive panel — Sanayici için fabrika durumu, Tüccar için arbitraj.
/// Ana ekranda sürekli görünür, tick başında güncellenir.
fn render_role_adaptive_panel(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let role = app
        .state
        .players
        .get(&HUMAN_ID)
        .map(|p| p.role)
        .unwrap_or(Role::Tuccar);
    match role {
        Role::Sanayici => render_my_factories_panel(f, area, app),
        Role::Tuccar => render_arbitrage_panel(f, area, app),
    }
}

/// Sanayici panel: her fabrikan için durum + hammadde kıtlığı uyarısı.
fn render_my_factories_panel(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 🏭  Fabrikalarım ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(player) = app.state.players.get(&HUMAN_ID) else {
        return;
    };
    let my_factories: Vec<&moneywar_domain::Factory> = app
        .state
        .factories
        .values()
        .filter(|fac| fac.owner == HUMAN_ID)
        .collect();

    let mut lines: Vec<Line> = Vec::new();
    if my_factories.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (fabrika yok — 'f' ile kur)",
            Style::default().fg(Color::DarkGray),
        )));
        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        f.render_widget(para, inner);
        return;
    }

    let current = app.state.current_tick;
    for factory in my_factories {
        let raw = factory.raw_input();
        let have_raw = player.inventory.get(factory.city, raw);
        let active_batch = factory.batches.first();

        let status: (String, Color) = if let Some(batch) = active_batch {
            let eta = batch
                .completion_tick
                .value()
                .saturating_sub(current.value());
            (
                format!(
                    "🟢 Üretiyor ({} birim, t{} biter, {} tick)",
                    batch.units,
                    batch.completion_tick.value(),
                    eta
                ),
                Color::Green,
            )
        } else if have_raw < moneywar_domain::Factory::BATCH_SIZE {
            (
                format!(
                    "🔴 Ham madde yok ({}: var {}, gerek {})",
                    raw,
                    have_raw,
                    moneywar_domain::Factory::BATCH_SIZE
                ),
                Color::Red,
            )
        } else {
            ("🟡 Hazır, tick'te başlar".to_string(), Color::Yellow)
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "  #{} {:<8} {:<11} ",
                    factory.id.value(),
                    city_short(factory.city),
                    factory.product
                ),
                Style::default().fg(product_color(factory.product)),
            ),
            Span::styled(
                status.0,
                Style::default().fg(status.1).add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Son üretim / atıl durumu özeti.
    let total = app
        .state
        .factories
        .values()
        .filter(|fac| fac.owner == HUMAN_ID)
        .count();
    let active = app
        .state
        .factories
        .values()
        .filter(|fac| fac.owner == HUMAN_ID && !fac.batches.is_empty())
        .count();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(
            "  Toplam: {total}  |  Aktif üretim: {active}  |  Atıl: {}",
            total - active
        ),
        Style::default().fg(Color::DarkGray),
    )));

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

/// Tüccar panel: her ürün için en kârlı (from → to) rotası. Son clearing
/// yoksa `price_baseline` kullanılır.
fn render_arbitrage_panel(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" 💹  Arbitraj Fırsatları ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        "  Her ürün için en ucuz → en pahalı şehir (fiyat: son clearing ya da baseline)",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    // Her ürün için min/max şehir fiyatı topla.
    let mut opportunities: Vec<(ProductKind, CityId, Money, CityId, Money, i64)> = Vec::new();
    for product in ProductKind::ALL {
        let mut per_city: Vec<(CityId, Money)> = Vec::new();
        for city in CityId::ALL {
            let price = app
                .state
                .price_history
                .get(&(city, product))
                .and_then(|v| v.last())
                .map(|(_, p)| *p)
                .or_else(|| app.state.price_baseline.get(&(city, product)).copied());
            if let Some(p) = price {
                per_city.push((city, p));
            }
        }
        if per_city.len() < 2 {
            continue;
        }
        per_city.sort_by_key(|(_, p)| p.as_cents());
        let (low_city, low) = per_city[0];
        let (high_city, high) = *per_city.last().unwrap();
        let spread = high.as_cents() - low.as_cents();
        if spread > 0 {
            opportunities.push((product, low_city, low, high_city, high, spread));
        }
    }

    // En yüksek spread üstte.
    opportunities.sort_by_key(|t| std::cmp::Reverse(t.5));

    if opportunities.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (henüz fiyat verisi yok — SPACE ile tick ilerlet)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (product, from, low, to, high, spread) in opportunities.into_iter().take(6) {
            let spread_money = Money::from_cents(spread);
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<11}  ", product),
                    Style::default()
                        .fg(product_color(product))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{} {} → {} {}", city_short(from), low, city_short(to), high),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("   +{spread_money}/birim"),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        }
    }

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    f.render_widget(para, inner);
}

fn render_log_panel(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let items: Vec<ListItem> = if app.last_tick_log.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (henüz tick yok — Space'e bas)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        let start = app.last_tick_log.len().saturating_sub(LOG_WINDOW);
        app.last_tick_log[start..]
            .iter()
            .map(|s| {
                let color = if s.contains("eşleşti") || s.contains("fulfilled") {
                    Color::Green
                } else if s.contains("REJECT") || s.contains("breach") || s.contains("default") {
                    Color::Red
                } else if s.contains("olay") || s.contains("kuraklık") || s.contains("grev") {
                    Color::Yellow
                } else {
                    Color::Gray
                };
                ListItem::new(Line::from(Span::styled(
                    s.clone(),
                    Style::default().fg(color),
                )))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Son Tick ({}) ", app.state.current_tick.value()))
            .border_style(Style::default().fg(Color::Green)),
    );
    f.render_widget(list, area);
}

/// Command mode sırasında leaderboard satırı yerine açılan akıllı hint.
/// Tampondaki ilk kelimeye göre kullanım örneği gösterir.
fn render_command_hint(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let Mode::Command { buffer } = &app.mode else {
        return;
    };
    let first = buffer
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase();
    let (label, example) = hint_for(&first);
    let line = Line::from(vec![
        Span::styled(
            " ℹ  ",
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            label,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(example, Style::default().fg(Color::Gray)),
    ]);
    let block = Block::default().borders(Borders::TOP | Borders::BOTTOM);
    f.render_widget(Paragraph::new(line).block(block), area);
}

fn hint_for(cmd: &str) -> (&'static str, &'static str) {
    match cmd {
        "" => (
            "komut yaz (Enter ile gönder)",
            "örn:  buy istanbul pamuk 20 7   |   help için '?'",
        ),
        "buy" | "b" => (
            "buy <şehir> <ürün> <miktar> <fiyat>",
            "örn:  buy istanbul pamuk 20 7",
        ),
        "sell" | "s" => (
            "sell <şehir> <ürün> <miktar> <fiyat>",
            "örn:  sell istanbul kumas 10 18",
        ),
        "cancel" => (
            "cancel <order_id>",
            "açık emri geri çek (m ile order_id'leri gör)",
        ),
        "build" => (
            "build <şehir> <bitmiş_ürün>",
            "örn:  build istanbul kumas (Sanayici tekeli)",
        ),
        "caravan" | "kervan" => (
            "caravan <başlangıç_şehri>",
            "örn:  caravan istanbul (kervan satın al)",
        ),
        "ship" | "dispatch" => (
            "ship <caravan_id> <from> <to> <ürün> <miktar>",
            "örn:  ship 1 istanbul ankara pamuk 20",
        ),
        "loan" | "kredi" => (
            "loan <miktar_lira> <vade_tick>",
            "örn:  loan 10000 30 (%15 sabit faiz)",
        ),
        "repay" | "ode" => ("repay <loan_id>", "açık krediyi öde (m ile loan_id'yi gör)"),
        "news" | "haber" => (
            "news <bronze|silver|gold>",
            "abonelik değiştir (Tüccar Silver bedava)",
        ),
        "offer" | "propose" => (
            "offer <ürün> <qty> <fiyat> <şehir> <delivery_tick>",
            "örn:  offer kumas 10 18 istanbul 15 (public kontrat)",
        ),
        "accept" => (
            "accept <contract_id>",
            "kontrat önerisini kabul et (m ile id'yi gör)",
        ),
        "withdraw" => (
            "withdraw <contract_id>",
            "kendi önerini geri çek (yalnız Proposed)",
        ),
        _ => (
            "bilinmeyen komut",
            "? ile yardım  |  örn: buy istanbul pamuk 20 7",
        ),
    }
}

/// Ana ekranda her zaman görünen tek satırlık kervan durumu.
/// "🚚 #1 Izmir→Istanbul (t9 varış) | #2 Idle @ Ankara" gibi.
fn render_caravan_strip(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let mine: Vec<&moneywar_domain::Caravan> = app
        .state
        .caravans
        .values()
        .filter(|c| c.owner == HUMAN_ID)
        .collect();

    let mut spans: Vec<Span> = vec![Span::styled(
        " 🚚 Kervanlar: ",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )];

    if mine.is_empty() {
        spans.push(Span::styled(
            "— (kervan yok, 'c' ile al)",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        for (i, caravan) in mine.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
            }
            let id = caravan.id.value();
            match &caravan.state {
                moneywar_domain::CaravanState::Idle { location } => {
                    spans.push(Span::styled(
                        format!("#{id} Idle @ {}", city_short(*location)),
                        Style::default().fg(Color::Green),
                    ));
                }
                moneywar_domain::CaravanState::EnRoute {
                    from,
                    to,
                    arrival_tick,
                    ..
                } => {
                    let eta = arrival_tick
                        .value()
                        .saturating_sub(app.state.current_tick.value());
                    spans.push(Span::styled(
                        format!(
                            "#{id} {}→{} (t{} varış, {} tick)",
                            city_short(*from),
                            city_short(*to),
                            arrival_tick.value(),
                            eta
                        ),
                        Style::default().fg(Color::Yellow),
                    ));
                }
            }
        }
    }

    let para = Paragraph::new(Line::from(spans));
    f.render_widget(para, area);
}

fn render_leaderboard(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    // Tick başında hesaplanmış cache'ten oku — render hot-path'inde
    // 12 player × scoring iter çalıştırma yok.
    // Sadece **gerçek rakipler** (insan + Sanayici + Tüccar) görünsün; likidite
    // sağlayan tipler (Alıcı/Esnaf/Spekülatör) saklanır.
    let filtered: Vec<&PlayerScore> = app
        .cached_leaderboard
        .iter()
        .filter(|sc| {
            app.state.players.get(&sc.player_id).is_none_or(|p| {
                !p.has_npc_kind(NpcKind::Alici)
                    && !p.has_npc_kind(NpcKind::Esnaf)
                    && !p.has_npc_kind(NpcKind::Spekulator)
            })
        })
        .collect();

    let mut spans: Vec<Span> = vec![Span::styled(
        " ★ Leaderboard ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    spans.push(Span::raw("  "));
    for (idx, sc) in filtered.iter().take(10).enumerate() {
        spans.push(rank_span(idx, sc, &app.state));
        spans.push(Span::raw("  "));
    }
    let line = Line::from(spans);
    let block = Block::default().borders(Borders::TOP | Borders::BOTTOM);
    let para = Paragraph::new(line).block(block);
    f.render_widget(para, area);
}

fn rank_span<'a>(idx: usize, sc: &PlayerScore, state: &GameState) -> Span<'a> {
    let player = state.players.get(&sc.player_id);
    let raw_name = player
        .map(|p| p.name.clone())
        .unwrap_or_else(|| format!("{}", sc.player_id));
    let medal = match idx {
        0 => "🥇",
        1 => "🥈",
        2 => "🥉",
        _ => "  ",
    };
    let is_human = sc.player_id == HUMAN_ID;
    let role_label = player
        .map(|p| match p.npc_kind {
            Some(kind) => kind.label(),
            None => p.role.display_name(),
        })
        .unwrap_or("?");
    let style = if is_human {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        let color = match role_label {
            "Sanayici" => Color::Rgb(210, 140, 80),
            "Tüccar" => Color::Rgb(120, 180, 240),
            _ => Color::White,
        };
        Style::default().fg(color)
    };
    // Skor fog'u: kendi skorun her zaman açık, rakibin skoru 5K'ya yuvarlanır
    // ve `~` prefix'i ile fog hissi verir. Sıra + seviye görünür, mikro
    // detay gizli. Game over reveal'da exact değerler açılır.
    let score_label = if is_human {
        compact_money(sc.total)
    } else {
        let cents = sc.total.as_cents();
        let lira = cents / 100;
        let rounded_lira = (lira / 5_000) * 5_000;
        let rounded = Money::from_lira(rounded_lira).unwrap_or(Money::ZERO);
        format!("~{}", compact_money(rounded))
    };
    Span::styled(
        format!("{medal} {raw_name} [{role_label}] {score_label}"),
        style,
    )
}

fn render_footer(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    match &app.mode {
        Mode::Command { buffer } => {
            let line = Line::from(vec![
                Span::styled(
                    " KOMUT ",
                    Style::default()
                        .bg(Color::Yellow)
                        .fg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" :"),
                Span::styled(
                    buffer.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "_",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::RAPID_BLINK),
                ),
                Span::raw("   "),
                hotkey("Enter"),
                Span::raw(" gönder  "),
                hotkey("Esc"),
                Span::raw(" iptal"),
            ]);
            f.render_widget(Paragraph::new(line), area);
        }
        _ => {
            let mut spans: Vec<Span> = Vec::new();
            if let Some(msg) = &app.status {
                let (fg, bg) = match msg.kind {
                    StatusKind::Ok => (Color::Black, Color::Green),
                    StatusKind::Err => (Color::White, Color::Red),
                    StatusKind::Info => (Color::Black, Color::Cyan),
                };
                spans.push(Span::styled(
                    format!(" {} ", msg.text),
                    Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw("  "));
            }
            let pending = app.pending_human_cmds.len();
            if pending > 0 {
                spans.push(Span::styled(
                    format!(" ⏳ {pending} komut bekliyor "),
                    Style::default()
                        .bg(Color::Magenta)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw("  "));
            }
            // Sadeleştirilmiş footer — sadece kritik tuşlar. Tüm liste için `?`.
            spans.extend_from_slice(&[
                hotkey("SPACE"),
                Span::styled(" tick", Style::default().fg(Color::Gray)),
                Span::styled("   ·   ", Style::default().fg(Color::DarkGray)),
                hotkey("b/s"),
                Span::styled(" al-sat", Style::default().fg(Color::Gray)),
                Span::styled("   ·   ", Style::default().fg(Color::DarkGray)),
                hotkey("?"),
                Span::styled(" tüm tuşlar", Style::default().fg(Color::Gray)),
                Span::styled("   ·   ", Style::default().fg(Color::DarkGray)),
                hotkey("q"),
                Span::styled(" çık", Style::default().fg(Color::Gray)),
            ]);
            f.render_widget(Paragraph::new(Line::from(spans)), area);
        }
    }
}

/// Tuş rozeti — minimal: `[SPACE]` formatı. Eski versiyonda her tuş için
/// dolu bg-render vardı; çok yer kapsıyordu, bracket'a indirildi.
fn hotkey(label: &str) -> Span<'static> {
    Span::styled(
        format!("[{label}]"),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
}

// ---------------------------------------------------------------------------
// Wizard input handler + command builder
// ---------------------------------------------------------------------------

enum WizardOutcome {
    Continue,
    Cancel,
    Submitted(Command),
    Error(String),
}

fn handle_wizard_key(app: &mut App, wizard: &mut Wizard, code: KeyCode) -> WizardOutcome {
    if matches!(code, KeyCode::Esc) {
        return WizardOutcome::Cancel;
    }
    // Backspace: text field'da char sil; aksi halde son seçimi geri al.
    if matches!(code, KeyCode::Backspace) {
        if !wizard.text_buf.is_empty() {
            wizard.text_buf.pop();
        } else if !wizard.fields.is_empty() {
            wizard.fields.pop();
            wizard.text_buf.clear();
        }
        return WizardOutcome::Continue;
    }
    // Ship wizard'da "daha ürün ekle?" confirm adımı — multi-product cargo.
    if wizard.confirm_more_cargo {
        match code {
            KeyCode::Char('1') => {
                // Ek ürün: ana Product+Qty'yi extra_cargo'ya taşı, fields'ı Product'a geri sar.
                if let (Some(FieldValue::Number(qty)), Some(FieldValue::Product(prod))) =
                    (wizard.fields.get(3).cloned(), wizard.fields.get(2).cloned())
                {
                    if let Ok(q32) = u32::try_from(qty) {
                        wizard.extra_cargo.push((prod, q32));
                    }
                }
                wizard.fields.truncate(2); // CaravanId + CityTo kalsın
                wizard.confirm_more_cargo = false;
                return WizardOutcome::Continue;
            }
            KeyCode::Char('2') | KeyCode::Enter => {
                wizard.confirm_more_cargo = false;
                return match build_command_from_wizard(app, wizard) {
                    Ok(cmd) => WizardOutcome::Submitted(cmd),
                    Err(e) => WizardOutcome::Error(e),
                };
            }
            _ => return WizardOutcome::Continue,
        }
    }
    // Tüm alanlar tamam → Enter ile gönder.
    if wizard.is_done() {
        if matches!(code, KeyCode::Enter) {
            return match build_command_from_wizard(app, wizard) {
                Ok(cmd) => WizardOutcome::Submitted(cmd),
                Err(e) => WizardOutcome::Error(e),
            };
        }
        return WizardOutcome::Continue;
    }
    let Some(field) = wizard.current() else {
        return WizardOutcome::Continue;
    };
    if field.is_text() {
        let is_money = matches!(field, FieldKind::PriceLira | FieldKind::AmountLira);
        // Ship wizard Qty adımında `M` ile max yükle — stok × kalan kapasite min.
        if matches!(wizard.kind, ActionKind::Ship)
            && matches!(field, FieldKind::QtyU32)
            && matches!(code, KeyCode::Char('m') | KeyCode::Char('M'))
        {
            let cid = match wizard.fields.first() {
                Some(FieldValue::CaravanId(id)) => *id,
                _ => return WizardOutcome::Continue,
            };
            let product = match wizard.fields.get(2) {
                Some(FieldValue::Product(p)) => *p,
                _ => return WizardOutcome::Continue,
            };
            let (from_city, capacity) = match app.state.caravans.get(&cid) {
                Some(c) => match c.state.current_city() {
                    Some(city) => (city, c.capacity),
                    None => return WizardOutcome::Continue,
                },
                None => return WizardOutcome::Continue,
            };
            let stock = app
                .state
                .players
                .get(&HUMAN_ID)
                .map(|p| p.inventory.get(from_city, product))
                .unwrap_or(0);
            let used: u32 = wizard.extra_cargo.iter().map(|(_, q)| *q).sum();
            let remaining_cap = capacity.saturating_sub(used);
            let max_load = stock.min(remaining_cap);
            if max_load == 0 {
                return WizardOutcome::Error(format!(
                    "max yüklenecek miktar 0 ({product} stok: {stock}, kapasite: {remaining_cap})"
                ));
            }
            wizard.text_buf = max_load.to_string();
            return WizardOutcome::Continue;
        }
        match code {
            KeyCode::Char(c) if c.is_ascii_digit() => wizard.text_buf.push(c),
            // Para alanlarında ondalık ayıracı: `.` veya `,`. Sadece bir kez,
            // ve başta değil.
            KeyCode::Char('.') | KeyCode::Char(',')
                if is_money && !wizard.text_buf.is_empty() && !wizard.text_buf.contains('.') =>
            {
                wizard.text_buf.push('.');
            }
            KeyCode::Enter => {
                // OrderTtl boş bırakılabilir → default kullanılır.
                if matches!(field, FieldKind::OrderTtl) && wizard.text_buf.is_empty() {
                    let default_ttl = app.state.config.balance.default_order_ttl;
                    wizard
                        .fields
                        .push(FieldValue::Number(u64::from(default_ttl)));
                    wizard.text_buf.clear();
                    return WizardOutcome::Continue;
                }
                // Para alanları için ondalık → cents. Diğer alanlar için
                // basit pozitif tam sayı.
                let parsed: Option<u64> = if is_money {
                    parse_decimal_as_cents(&wizard.text_buf)
                } else {
                    wizard.text_buf.parse::<u64>().ok()
                };
                match parsed {
                    Some(n) if n > 0 => {
                        // OrderTtl üst sınırını burada kontrol et.
                        if matches!(field, FieldKind::OrderTtl) {
                            let max = u64::from(app.state.config.balance.max_order_ttl);
                            if n > max {
                                return WizardOutcome::Error(format!(
                                    "TTL en fazla {max} tick olabilir"
                                ));
                            }
                        }
                        wizard.fields.push(FieldValue::Number(n));
                        wizard.text_buf.clear();
                        // Ship wizard'da Qty alanı bittiğinde: "daha ürün ekle mi?"
                        // confirm adımını aktifleştir. Kapasite dolmadıysa loop mümkün.
                        if matches!(wizard.kind, ActionKind::Ship)
                            && matches!(field, FieldKind::QtyU32)
                        {
                            wizard.confirm_more_cargo = true;
                        }
                    }
                    _ => {
                        let msg = if is_money {
                            "geçerli fiyat gir (örn: 15 veya 15.75)"
                        } else {
                            "geçerli bir pozitif sayı gir"
                        };
                        return WizardOutcome::Error(msg.into());
                    }
                }
            }
            _ => {}
        }
        return WizardOutcome::Continue;
    }
    // Seçim alanı — sayı tuşu (1-9) ile seç.
    let KeyCode::Char(c) = code else {
        return WizardOutcome::Continue;
    };
    let Some(idx) = c.to_digit(10) else {
        return WizardOutcome::Continue;
    };
    if idx == 0 {
        return WizardOutcome::Continue;
    }
    let idx = (idx - 1) as usize;
    let val = match field {
        FieldKind::City | FieldKind::CityTo => CityId::ALL.get(idx).map(|c| FieldValue::City(*c)),
        FieldKind::Product => ProductKind::ALL.get(idx).map(|p| FieldValue::Product(*p)),
        FieldKind::FinishedProduct => ProductKind::FINISHED_GOODS
            .get(idx)
            .map(|p| FieldValue::Product(*p)),
        FieldKind::NewsTier_ => {
            let tiers = [NewsTier::Bronze, NewsTier::Silver, NewsTier::Gold];
            tiers.get(idx).map(|t| FieldValue::NewsTier(*t))
        }
        FieldKind::OrderId_ => {
            let mine: Vec<OrderId> = app
                .state
                .order_book
                .values()
                .flatten()
                .filter(|o| o.player == HUMAN_ID)
                .map(|o| o.id)
                .collect();
            mine.get(idx).map(|id| FieldValue::OrderId(*id))
        }
        FieldKind::CaravanId => {
            let mine: Vec<moneywar_domain::CaravanId> = app
                .state
                .caravans
                .values()
                .filter(|c| c.owner == HUMAN_ID && c.is_idle())
                .map(|c| c.id)
                .collect();
            mine.get(idx).map(|id| FieldValue::CaravanId(*id))
        }
        FieldKind::LoanId => {
            let mine: Vec<moneywar_domain::LoanId> = app
                .state
                .loans
                .values()
                .filter(|l| l.borrower == HUMAN_ID && !l.repaid)
                .map(|l| l.id)
                .collect();
            mine.get(idx).map(|id| FieldValue::LoanId(*id))
        }
        FieldKind::ContractId_ => {
            let mine: Vec<moneywar_domain::ContractId> =
                app.state.contracts.keys().copied().collect();
            mine.get(idx).map(|id| FieldValue::ContractId(*id))
        }
        // Text fields handled above
        _ => None,
    };
    if let Some(v) = val {
        wizard.fields.push(v);
    }
    WizardOutcome::Continue
}

fn build_command_from_wizard(app: &mut App, wizard: &Wizard) -> Result<Command, String> {
    let f = &wizard.fields;
    let tick = app.state.current_tick.next();
    let pick_city = |idx: usize| -> Result<CityId, String> {
        match f.get(idx) {
            Some(FieldValue::City(c)) => Ok(*c),
            _ => Err("şehir eksik".into()),
        }
    };
    let pick_product = |idx: usize| -> Result<ProductKind, String> {
        match f.get(idx) {
            Some(FieldValue::Product(p)) => Ok(*p),
            _ => Err("ürün eksik".into()),
        }
    };
    let pick_number = |idx: usize| -> Result<u64, String> {
        match f.get(idx) {
            Some(FieldValue::Number(n)) => Ok(*n),
            _ => Err("sayı eksik".into()),
        }
    };
    match wizard.kind {
        ActionKind::Buy | ActionKind::Sell => {
            let city = pick_city(0)?;
            let product = pick_product(1)?;
            let qty = u32::try_from(pick_number(2)?).map_err(|_| "miktar çok büyük")?;
            // PriceLira alanı zaten cents olarak saklanıyor (wizard'da ondalık → cents).
            let price_cents = i64::try_from(pick_number(3)?).map_err(|_| "fiyat çok büyük")?;
            let price = Money::from_cents(price_cents);
            let ttl = u32::try_from(pick_number(4)?).map_err(|_| "TTL çok büyük")?;
            let side = if matches!(wizard.kind, ActionKind::Buy) {
                OrderSide::Buy
            } else {
                OrderSide::Sell
            };
            let order = MarketOrder::new_with_ttl(
                app.next_order_id(),
                HUMAN_ID,
                city,
                product,
                side,
                qty,
                price,
                tick,
                ttl,
            )
            .map_err(|e| format!("{e}"))?;
            Ok(Command::SubmitOrder(order))
        }
        ActionKind::Build => {
            let city = pick_city(0)?;
            let product = pick_product(1)?;
            Ok(Command::BuildFactory {
                owner: HUMAN_ID,
                city,
                product,
            })
        }
        ActionKind::Caravan => Ok(Command::BuyCaravan {
            owner: HUMAN_ID,
            starting_city: pick_city(0)?,
        }),
        ActionKind::Ship => {
            let cid = match f.first() {
                Some(FieldValue::CaravanId(id)) => *id,
                _ => return Err("kervan eksik".into()),
            };
            // `from` kervan konumundan otomatik alınır — wizard'da sorulmuyor.
            let from = app
                .state
                .caravans
                .get(&cid)
                .and_then(|c| c.state.current_city())
                .ok_or_else(|| "kervan idle değil, konumu okunamıyor".to_string())?;
            let to = pick_city(1)?;
            let mut cargo = moneywar_domain::CargoSpec::new();
            // Önce extra_cargo'daki önceki kalemleri ekle.
            for (prod, qty) in &wizard.extra_cargo {
                cargo.add(*prod, *qty).map_err(|e| format!("{e}"))?;
            }
            // Sonra ana (Product, Qty) — fields[2,3] varsa.
            if let (Some(FieldValue::Product(product)), Some(FieldValue::Number(qty))) =
                (f.get(2), f.get(3))
            {
                let qty32 = u32::try_from(*qty).map_err(|_| "miktar çok büyük")?;
                cargo.add(*product, qty32).map_err(|e| format!("{e}"))?;
            }
            if cargo.is_empty() {
                return Err("en az 1 ürün yüklemen lazım".into());
            }
            Ok(Command::DispatchCaravan {
                caravan_id: cid,
                from,
                to,
                cargo,
            })
        }
        ActionKind::Loan => {
            // AmountLira alanı cents olarak saklanıyor (ondalık destekli).
            let amount_cents = i64::try_from(pick_number(0)?).map_err(|_| "tutar çok büyük")?;
            let duration = u32::try_from(pick_number(1)?).map_err(|_| "vade çok büyük")?;
            let amount = Money::from_cents(amount_cents);
            Ok(Command::TakeLoan {
                player: HUMAN_ID,
                amount,
                duration_ticks: duration,
            })
        }
        ActionKind::Repay => match f.first() {
            Some(FieldValue::LoanId(id)) => Ok(Command::RepayLoan {
                player: HUMAN_ID,
                loan_id: *id,
            }),
            _ => Err("kredi seç".into()),
        },
        ActionKind::News => match f.first() {
            Some(FieldValue::NewsTier(t)) => Ok(Command::SubscribeNews {
                player: HUMAN_ID,
                tier: *t,
            }),
            _ => Err("tier seç".into()),
        },
        ActionKind::Cancel => match f.first() {
            Some(FieldValue::OrderId(id)) => Ok(Command::CancelOrder {
                order_id: *id,
                requester: HUMAN_ID,
            }),
            _ => Err("emir seç".into()),
        },
        ActionKind::Offer => {
            let product = pick_product(0)?;
            let qty = u32::try_from(pick_number(1)?).map_err(|_| "miktar")?;
            // PriceLira alanı cents olarak saklanıyor (ondalık destekli).
            let price_cents = i64::try_from(pick_number(2)?).map_err(|_| "fiyat")?;
            let unit_price = Money::from_cents(price_cents);
            let city = pick_city(3)?;
            let delivery_n = u32::try_from(pick_number(4)?).map_err(|_| "delivery_tick")?;
            let delivery_tick = Tick::new(delivery_n);
            if !tick.is_before(delivery_tick) {
                return Err(format!(
                    "delivery_tick ({}) şu andan (tick {}) sonra olmalı",
                    delivery_tick.value(),
                    tick.value()
                ));
            }
            let total = unit_price
                .checked_mul_scalar(i64::from(qty))
                .map_err(|e| format!("{e}"))?;
            let deposit = Money::from_cents(total.as_cents() / 10);
            Ok(Command::ProposeContract(
                moneywar_domain::ContractProposal {
                    seller: HUMAN_ID,
                    listing: moneywar_domain::ListingKind::Public,
                    product,
                    quantity: qty,
                    unit_price,
                    delivery_city: city,
                    delivery_tick,
                    seller_deposit: deposit,
                    buyer_deposit: deposit,
                },
            ))
        }
        ActionKind::Accept => match f.first() {
            Some(FieldValue::ContractId(id)) => Ok(Command::AcceptContract {
                contract_id: *id,
                acceptor: HUMAN_ID,
            }),
            _ => Err("kontrat seç".into()),
        },
        ActionKind::Withdraw => match f.first() {
            Some(FieldValue::ContractId(id)) => Ok(Command::CancelContractProposal {
                contract_id: *id,
                requester: HUMAN_ID,
            }),
            _ => Err("kontrat seç".into()),
        },
    }
}

// ---------------------------------------------------------------------------
// Komut parser
// ---------------------------------------------------------------------------

fn parse_command(app: &mut App, line: &str) -> Result<Command, String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return Err("boş komut".into());
    }
    let head = parts[0].to_lowercase();
    let args = &parts[1..];
    let tick = app.state.current_tick.next();
    match head.as_str() {
        "buy" | "b" => parse_order_cmd(app, OrderSide::Buy, args, tick),
        "sell" | "s" => parse_order_cmd(app, OrderSide::Sell, args, tick),
        "cancel" => parse_cancel_cmd(args),
        "build" => parse_build_cmd(args),
        "caravan" | "kervan" => parse_caravan_cmd(args),
        "ship" | "dispatch" => parse_ship_cmd(args),
        "loan" | "kredi" => parse_loan_cmd(args),
        "repay" | "ode" => parse_repay_cmd(args),
        "news" | "haber" => parse_news_cmd(args),
        "offer" | "propose" => parse_offer_cmd(args, tick),
        "accept" => parse_accept_cmd(args),
        "withdraw" => parse_withdraw_cmd(args),
        _ => Err(format!("bilinmeyen komut '{head}' — `?` yardım için")),
    }
}

fn parse_offer_cmd(args: &[&str], proposed_tick: Tick) -> Result<Command, String> {
    if !(5..=6).contains(&args.len()) {
        return Err(
            "kullanım: offer <ürün> <miktar> <fiyat> <şehir> <delivery_tick> [deposit_lira]".into(),
        );
    }
    let product = parse_product(args[0])?;
    let quantity: u32 = args[1]
        .parse()
        .map_err(|_| format!("geçersiz miktar: {}", args[1]))?;
    let price_cents: i64 = parse_decimal_as_cents(args[2])
        .and_then(|c| i64::try_from(c).ok())
        .ok_or_else(|| format!("geçersiz fiyat: {} (örn: 15 veya 15.75)", args[2]))?;
    let unit_price = Money::from_cents(price_cents);
    let city = parse_city(args[3])?;
    let delivery: u32 = args[4]
        .parse()
        .map_err(|_| format!("geçersiz delivery_tick: {}", args[4]))?;
    let delivery_tick = Tick::new(delivery);
    if !proposed_tick.is_before(delivery_tick) {
        return Err(format!(
            "delivery_tick ({}) şu andan (tick {}) sonra olmalı",
            delivery_tick.value(),
            proposed_tick.value()
        ));
    }
    let deposit = if args.len() == 6 {
        let d_cents: i64 = parse_decimal_as_cents(args[5])
            .and_then(|c| i64::try_from(c).ok())
            .ok_or_else(|| format!("geçersiz deposit: {}", args[5]))?;
        Money::from_cents(d_cents)
    } else {
        let total = unit_price
            .checked_mul_scalar(i64::from(quantity))
            .map_err(|e| format!("{e}"))?;
        Money::from_cents(total.as_cents() / 10)
    };
    Ok(Command::ProposeContract(
        moneywar_domain::ContractProposal {
            seller: HUMAN_ID,
            listing: moneywar_domain::ListingKind::Public,
            product,
            quantity,
            unit_price,
            delivery_city: city,
            delivery_tick,
            seller_deposit: deposit,
            buyer_deposit: deposit,
        },
    ))
}

fn parse_accept_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 1 {
        return Err("kullanım: accept <contract_id>".into());
    }
    let id: u64 = args[0]
        .parse()
        .map_err(|_| format!("geçersiz contract_id: {}", args[0]))?;
    Ok(Command::AcceptContract {
        contract_id: moneywar_domain::ContractId::new(id),
        acceptor: HUMAN_ID,
    })
}

fn parse_withdraw_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 1 {
        return Err("kullanım: withdraw <contract_id>".into());
    }
    let id: u64 = args[0]
        .parse()
        .map_err(|_| format!("geçersiz contract_id: {}", args[0]))?;
    Ok(Command::CancelContractProposal {
        contract_id: moneywar_domain::ContractId::new(id),
        requester: HUMAN_ID,
    })
}

fn parse_order_cmd(
    app: &mut App,
    side: OrderSide,
    args: &[&str],
    tick: Tick,
) -> Result<Command, String> {
    if !(4..=5).contains(&args.len()) {
        return Err("kullanım: buy/sell <şehir> <ürün> <miktar> <fiyat> [ttl]".into());
    }
    let city = parse_city(args[0])?;
    let product = parse_product(args[1])?;
    let qty: u32 = args[2]
        .parse()
        .map_err(|_| format!("geçersiz miktar: {}", args[2]))?;
    // Fiyat ondalık destekli: "15" veya "15.75" veya "15,75" → cents.
    let price_cents: i64 = parse_decimal_as_cents(args[3])
        .and_then(|c| i64::try_from(c).ok())
        .ok_or_else(|| format!("geçersiz fiyat: {} (örn: 15 veya 15.75)", args[3]))?;
    let price = Money::from_cents(price_cents);
    let balance = app.state.config.balance;
    let ttl: u32 = if let Some(arg) = args.get(4) {
        let n: u32 = arg.parse().map_err(|_| format!("geçersiz ttl: {arg}"))?;
        if n == 0 || n > balance.max_order_ttl {
            return Err(format!(
                "ttl 1..{} aralığında olmalı",
                balance.max_order_ttl
            ));
        }
        n
    } else {
        balance.default_order_ttl
    };
    let order = MarketOrder::new_with_ttl(
        app.next_order_id(),
        HUMAN_ID,
        city,
        product,
        side,
        qty,
        price,
        tick,
        ttl,
    )
    .map_err(|e| format!("emir oluşturulamadı: {e}"))?;
    Ok(Command::SubmitOrder(order))
}

fn parse_cancel_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 1 {
        return Err("kullanım: cancel <order_id>".into());
    }
    let id: u64 = args[0]
        .parse()
        .map_err(|_| format!("geçersiz order_id: {}", args[0]))?;
    Ok(Command::CancelOrder {
        order_id: OrderId::new(id),
        requester: HUMAN_ID,
    })
}

fn parse_build_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 2 {
        return Err("kullanım: build <şehir> <bitmiş_ürün>".into());
    }
    let city = parse_city(args[0])?;
    let product = parse_product(args[1])?;
    if !product.is_finished() {
        return Err(format!("fabrika bitmiş ürün üretmeli; {product} ham madde"));
    }
    Ok(Command::BuildFactory {
        owner: HUMAN_ID,
        city,
        product,
    })
}

fn parse_caravan_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 1 {
        return Err("kullanım: caravan <başlangıç_şehri>".into());
    }
    let city = parse_city(args[0])?;
    Ok(Command::BuyCaravan {
        owner: HUMAN_ID,
        starting_city: city,
    })
}

fn parse_ship_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 5 {
        return Err("kullanım: ship <caravan_id> <nereden> <nereye> <ürün> <miktar>".into());
    }
    let caravan_id: u64 = args[0]
        .parse()
        .map_err(|_| format!("geçersiz caravan_id: {}", args[0]))?;
    let from = parse_city(args[1])?;
    let to = parse_city(args[2])?;
    let product = parse_product(args[3])?;
    let qty: u32 = args[4]
        .parse()
        .map_err(|_| format!("geçersiz miktar: {}", args[4]))?;
    let mut cargo = moneywar_domain::CargoSpec::new();
    cargo.add(product, qty).map_err(|e| format!("cargo: {e}"))?;
    Ok(Command::DispatchCaravan {
        caravan_id: moneywar_domain::CaravanId::new(caravan_id),
        from,
        to,
        cargo,
    })
}

fn parse_loan_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 2 {
        return Err("kullanım: loan <miktar_lira> <vade_tick>".into());
    }
    let amount_cents: i64 = parse_decimal_as_cents(args[0])
        .and_then(|c| i64::try_from(c).ok())
        .ok_or_else(|| format!("geçersiz miktar: {} (örn: 1000 veya 1000.50)", args[0]))?;
    let amount = Money::from_cents(amount_cents);
    let duration: u32 = args[1]
        .parse()
        .map_err(|_| format!("geçersiz vade: {}", args[1]))?;
    Ok(Command::TakeLoan {
        player: HUMAN_ID,
        amount,
        duration_ticks: duration,
    })
}

fn parse_repay_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 1 {
        return Err("kullanım: repay <loan_id>".into());
    }
    let id: u64 = args[0]
        .parse()
        .map_err(|_| format!("geçersiz loan_id: {}", args[0]))?;
    Ok(Command::RepayLoan {
        player: HUMAN_ID,
        loan_id: moneywar_domain::LoanId::new(id),
    })
}

fn parse_news_cmd(args: &[&str]) -> Result<Command, String> {
    if args.len() != 1 {
        return Err("kullanım: news <bronze|silver|gold>".into());
    }
    let tier = match args[0].to_lowercase().as_str() {
        "bronze" | "bronz" => NewsTier::Bronze,
        "silver" | "gumus" | "gümüş" => NewsTier::Silver,
        "gold" | "altın" | "altin" => NewsTier::Gold,
        _ => return Err(format!("geçersiz tier: {}", args[0])),
    };
    Ok(Command::SubscribeNews {
        player: HUMAN_ID,
        tier,
    })
}

fn parse_city(s: &str) -> Result<CityId, String> {
    match s.to_lowercase().as_str() {
        "istanbul" | "ist" | "ıstanbul" | "i̇stanbul" => Ok(CityId::Istanbul),
        "ankara" | "ank" => Ok(CityId::Ankara),
        "izmir" | "izm" | "ızmir" | "i̇zmir" => Ok(CityId::Izmir),
        _ => Err(format!("bilinmeyen şehir: {s}")),
    }
}

fn parse_product(s: &str) -> Result<ProductKind, String> {
    match s.to_lowercase().as_str() {
        "pamuk" => Ok(ProductKind::Pamuk),
        "bugday" | "buğday" => Ok(ProductKind::Bugday),
        "zeytin" => Ok(ProductKind::Zeytin),
        "kumas" | "kumaş" => Ok(ProductKind::Kumas),
        "un" => Ok(ProductKind::Un),
        "zeytinyagi" | "zeytinyağı" | "yag" | "yağ" => Ok(ProductKind::Zeytinyagi),
        _ => Err(format!("bilinmeyen ürün: {s}")),
    }
}

/// Pending komut için kompakt tek-satır gösterim.
fn describe_command_short(cmd: &Command) -> String {
    match cmd {
        Command::SubmitOrder(o) => format!(
            "{} {} {} @{} {}",
            if matches!(o.side, OrderSide::Buy) {
                "AL"
            } else {
                "SAT"
            },
            o.quantity,
            o.product,
            o.unit_price,
            city_short(o.city),
        ),
        Command::CancelOrder { order_id, .. } => format!("emir iptal #{}", order_id.value()),
        Command::BuildFactory { city, product, .. } => {
            format!("fab kur {} {}", city_short(*city), product)
        }
        Command::BuyCaravan { starting_city, .. } => {
            format!("kervan al @{}", city_short(*starting_city))
        }
        Command::DispatchCaravan {
            caravan_id,
            from,
            to,
            ..
        } => format!(
            "kervan #{} {}→{}",
            caravan_id.value(),
            city_short(*from),
            city_short(*to)
        ),
        Command::TakeLoan {
            amount,
            duration_ticks,
            ..
        } => {
            format!("kredi {} / {}t", amount, duration_ticks)
        }
        Command::RepayLoan { loan_id, .. } => format!("kredi öde #{}", loan_id.value()),
        Command::SubscribeNews { tier, .. } => format!("haber {}", tier),
        Command::ProposeContract(p) => format!(
            "kontrat öner {} {} @{}",
            p.quantity, p.product, p.unit_price
        ),
        Command::AcceptContract { contract_id, .. } => {
            format!("kontrat kabul #{}", contract_id.value())
        }
        Command::CancelContractProposal { contract_id, .. } => {
            format!("kontrat geri çek #{}", contract_id.value())
        }
    }
}

fn describe_command(cmd: &Command) -> String {
    match cmd {
        Command::SubmitOrder(o) => format!(
            "{:?} {} {} @{} ({})",
            o.side,
            o.quantity,
            o.product,
            o.unit_price,
            city_short(o.city)
        ),
        Command::CancelOrder { order_id, .. } => format!("cancel order {order_id}"),
        Command::BuildFactory { city, product, .. } => {
            format!("fabrika kur: {} / {}", city_short(*city), product)
        }
        Command::BuyCaravan { starting_city, .. } => {
            format!("kervan al: {}", city_short(*starting_city))
        }
        Command::DispatchCaravan {
            caravan_id,
            from,
            to,
            cargo,
        } => {
            format!(
                "kervan {caravan_id} {}→{} ({} birim)",
                city_short(*from),
                city_short(*to),
                cargo.total_units()
            )
        }
        Command::TakeLoan {
            amount,
            duration_ticks,
            ..
        } => format!("kredi al: {amount} vade {duration_ticks}t"),
        Command::RepayLoan { loan_id, .. } => format!("kredi öde: {loan_id}"),
        Command::SubscribeNews { tier, .. } => format!("haber: {tier}"),
        Command::ProposeContract(_) => "kontrat önerisi".into(),
        Command::AcceptContract { contract_id, .. } => format!("kontrat kabul: {contract_id}"),
        Command::CancelContractProposal { contract_id, .. } => {
            format!("kontrat iptal: {contract_id}")
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn role_color(role: Role) -> Color {
    match role {
        Role::Sanayici => Color::Rgb(210, 140, 80),
        Role::Tuccar => Color::Rgb(120, 180, 240),
    }
}

fn tier_color(tier: NewsTier) -> Color {
    match tier {
        NewsTier::Bronze => Color::Rgb(205, 127, 50),
        NewsTier::Silver => Color::Rgb(192, 192, 192),
        NewsTier::Gold => Color::Rgb(255, 215, 0),
    }
}

fn product_color(p: ProductKind) -> Color {
    match p {
        ProductKind::Pamuk => Color::Rgb(240, 240, 220),
        ProductKind::Bugday => Color::Rgb(220, 190, 100),
        ProductKind::Zeytin => Color::Rgb(120, 130, 70),
        ProductKind::Kumas => Color::Rgb(200, 160, 220),
        ProductKind::Un => Color::Rgb(240, 220, 180),
        ProductKind::Zeytinyagi => Color::Rgb(180, 200, 100),
    }
}

fn city_short(city: CityId) -> &'static str {
    match city {
        CityId::Istanbul => "İstanbul",
        CityId::Ankara => "Ankara",
        CityId::Izmir => "İzmir",
    }
}

fn format_event(event: &moneywar_domain::GameEvent) -> String {
    use moneywar_domain::GameEvent;
    match event {
        GameEvent::Drought {
            city,
            product,
            severity,
        } => format!(
            "kuraklık {} / {} ({:?})",
            city_short(*city),
            product,
            severity
        ),
        GameEvent::Strike {
            city,
            product,
            severity,
        } => format!("grev {} / {} ({:?})", city_short(*city), product, severity),
        GameEvent::BumperHarvest {
            city,
            product,
            severity,
        } => format!(
            "bereket {} / {} ({:?})",
            city_short(*city),
            product,
            severity
        ),
        GameEvent::RoadClosure {
            from,
            to,
            extra_ticks,
            ..
        } => format!(
            "yol kapalı {}↔{} (+{}t)",
            city_short(*from),
            city_short(*to),
            extra_ticks
        ),
        GameEvent::NewMarket {
            city,
            product,
            extra_demand,
        } => {
            // Talep şoku — "düğün/festival/karşı kıyıdan tüccar" tarzı tematik
            // mesaj, miktar büyüklüğüne göre.
            let theme = if *extra_demand >= 100 {
                "büyük şenlik"
            } else if *extra_demand >= 50 {
                "düğün"
            } else {
                "yeni alıcılar"
            };
            format!(
                "{theme} {} / {} talebi patladı (+{})",
                city_short(*city),
                product,
                extra_demand
            )
        }
    }
}

fn summarize_report(report: &moneywar_engine::TickReport, state: &GameState) -> Vec<String> {
    let tick = report.tick.value();

    let player_name = |pid: PlayerId| -> String {
        state
            .players
            .get(&pid)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| format!("#{}", pid.value()))
    };

    // İki akım: önce counters + human-özel satırları topla, sonra tek temiz çıktı.
    let mut human_lines: Vec<String> = Vec::new();
    let mut events_lines: Vec<String> = Vec::new();
    let mut total_matches: u32 = 0;
    let mut human_matches: u32 = 0;
    let mut human_production: u32 = 0;
    let mut npc_production: u32 = 0;
    let mut human_arrivals: u32 = 0;
    let mut npc_arrivals: u32 = 0;
    let mut rejects: u32 = 0;
    let mut contracts_events: u32 = 0;

    for entry in &report.entries {
        match &entry.event {
            LogEvent::OrderMatched {
                quantity,
                price,
                city,
                product,
                buyer,
                seller,
                ..
            } => {
                total_matches += 1;
                let buyer_human = *buyer == HUMAN_ID;
                let seller_human = *seller == HUMAN_ID;
                if buyer_human || seller_human {
                    human_matches += 1;
                    let verb = if seller_human { "sattın" } else { "aldın" };
                    let other = if seller_human {
                        player_name(*buyer)
                    } else {
                        player_name(*seller)
                    };
                    human_lines.push(format!(
                        "  🔄 {verb}: {} {} @ {} {} (→ {})",
                        quantity,
                        product,
                        city_short(*city),
                        price,
                        other
                    ));
                }
            }
            LogEvent::CommandRejected { reason, .. } => {
                if entry.actor == Some(HUMAN_ID) {
                    rejects += 1;
                    human_lines.push(format!("  ✗ REJECT: {reason}"));
                }
            }
            LogEvent::EventScheduled {
                game_event,
                event_tick,
                ..
            } => {
                events_lines.push(format!(
                    "  🎲 tick {}'te: {}",
                    event_tick.value(),
                    format_event(game_event)
                ));
            }
            LogEvent::ProductionCompleted { units, product, .. } => {
                if entry.actor == Some(HUMAN_ID) {
                    human_production += *units;
                    human_lines.push(format!("  🏭 +{} {} üretim tamam", units, product));
                } else {
                    npc_production += *units;
                }
            }
            LogEvent::CaravanArrived {
                city, cargo_total, ..
            } => {
                if entry.actor == Some(HUMAN_ID) {
                    human_arrivals += 1;
                    human_lines.push(format!(
                        "  🚚 kervan {} vardı ({} birim)",
                        city_short(*city),
                        cargo_total
                    ));
                } else {
                    npc_arrivals += 1;
                }
            }
            LogEvent::ContractSettled { final_state, .. } => match final_state {
                moneywar_domain::ContractState::Fulfilled => {
                    contracts_events += 1;
                    human_lines.push("  ✓ kontrat teslim edildi".into());
                }
                moneywar_domain::ContractState::Breached { .. } => {
                    contracts_events += 1;
                    human_lines.push("  ⚠ kontrat caydı (breach)".into());
                }
                _ => {}
            },
            LogEvent::LoanRepaid { amount_paid, .. } => {
                if entry.actor == Some(HUMAN_ID) {
                    human_lines.push(format!("  💰 kredi ödendi: {amount_paid}"));
                }
            }
            LogEvent::LoanDefaulted { seized, .. } => {
                if entry.actor == Some(HUMAN_ID) {
                    human_lines.push(format!("  ⚠ kredi DEFAULT (çekildi: {seized})"));
                }
            }
            // NPC chatter — rakipler "yaşıyor" hissi versin. Sadece dramatic
            // eylemler: fabrika kuruldu, kervan satın alındı, kervan dispatch.
            // Sıradan SubmitOrder spam'i atlanıyor (zaten match satırı var).
            LogEvent::FactoryBuilt { owner, city, .. } if *owner != HUMAN_ID => {
                events_lines.push(format!(
                    "  🏭 {} {}'da fabrika kurdu",
                    player_name(*owner),
                    city_short(*city)
                ));
            }
            LogEvent::CaravanBought {
                owner,
                starting_city,
                ..
            } if *owner != HUMAN_ID => {
                events_lines.push(format!(
                    "  🐪 {} {}'da kervan aldı",
                    player_name(*owner),
                    city_short(*starting_city)
                ));
            }
            LogEvent::CaravanDispatched {
                from,
                to,
                cargo_total,
                ..
            } => {
                if let Some(actor) = entry.actor {
                    if actor != HUMAN_ID {
                        events_lines.push(format!(
                            "  🚂 {} {} → {} ({} birim)",
                            player_name(actor),
                            city_short(*from),
                            city_short(*to),
                            cargo_total
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    // Başlık — kompakt özet.
    let mut out = Vec::new();
    let mut summary_parts: Vec<String> = Vec::new();
    if total_matches > 0 {
        summary_parts.push(format!("🔄 {total_matches} match"));
    }
    if human_production + npc_production > 0 {
        summary_parts.push(format!("🏭 +{} üretim", human_production + npc_production));
    }
    if human_arrivals + npc_arrivals > 0 {
        summary_parts.push(format!("🚚 {} varış", human_arrivals + npc_arrivals));
    }
    if contracts_events > 0 {
        summary_parts.push(format!("📜 {contracts_events} kontrat"));
    }
    if rejects > 0 {
        summary_parts.push(format!("✗ {rejects} reject"));
    }
    let header = if summary_parts.is_empty() {
        format!("── Tick {tick} ── (sakin)")
    } else {
        format!("── Tick {tick} ── {}", summary_parts.join(" · "))
    };
    out.push(header);

    // Human'ın işlemleri detay olarak.
    out.extend(human_lines);

    // NPC trafiği tek satır özet (sadece NPC-NPC match ya da NPC üretim/varış varsa).
    let npc_matches = total_matches.saturating_sub(human_matches);
    if npc_matches > 0 || npc_production > 0 || npc_arrivals > 0 {
        let mut npc_parts: Vec<String> = Vec::new();
        if npc_matches > 0 {
            npc_parts.push(format!("{npc_matches} match"));
        }
        if npc_production > 0 {
            npc_parts.push(format!("+{npc_production} üretim"));
        }
        if npc_arrivals > 0 {
            npc_parts.push(format!("{npc_arrivals} varış"));
        }
        out.push(format!("  · NPC trafik: {}", npc_parts.join(", ")));
    }

    // Yeni olaylar (haber) — son, nadir ama önemli.
    out.extend(events_lines);

    out
}
