//! MoneyWar — terminal izleyici / playtest TUI.
//!
//! Ratatui + crossterm. Tek ekran, 4 panel + alt bar.
//!
//! # Tuşlar
//!
//! - `Space` — bir tick ilerlet (NPC komutları otomatik).
//! - `s`      — auto-sim aç/kapa (her 300ms tick).
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

use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use moneywar_domain::{
    CityId, Command, GameState, MarketOrder, Money, NewsItem, NewsTier, OrderId, OrderSide, Player,
    PlayerId, ProductKind, Role, RoomConfig, RoomId, Tick,
};
use moneywar_engine::{LogEvent, PlayerScore, advance_tick, leaderboard, rng_for, score_player};
use moneywar_npc::decide_all_npcs;
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

fn main() -> Result<()> {
    let mut app = App::new();

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
            // Wizard kısayolları — tek tuşla aksiyon menüsü.
            KeyCode::Char('b') => {
                app.mode = Mode::Wizard {
                    wizard: Wizard::new(ActionKind::Buy),
                };
            }
            KeyCode::Char('S') => {
                // Büyük S: SAT (küçük 's' auto-sim için ayrılı, alttaki branch).
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
            KeyCode::Char('r') => {
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
            // Auto-sim — küçük s.
            KeyCode::Char('s') => {
                app.auto_sim = !app.auto_sim;
                app.set_status_info(if app.auto_sim {
                    "Auto-sim AÇIK (s ile kapat)"
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
        Mode::Help | Mode::Info | Mode::Holdings | Mode::NewsInbox => match code {
            // Overlay'i kapat — Esc burada güvenli (oyundan çıkmaz, panel kapanır).
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char(' ') => {
                app.mode = Mode::Normal;
            }
            _ => {}
        },
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
    /// Sezon sonu reveal — 90. tick'te otomatik açılır, tüm skorlar görünür.
    GameOver,
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
            Self::Buy | Self::Sell => &[City, Product, QtyU32, PriceLira],
            Self::Build => &[City, FinishedProduct],
            Self::Caravan => &[City],
            Self::Ship => &[CaravanId, CityFrom, CityTo, Product, QtyU32],
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
    CityFrom,
    CityTo,
    Product,
    FinishedProduct,
    QtyU32,
    PriceLira,
    AmountLira,
    DurationTicks,
    DeliveryTick,
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
            Self::CityFrom => "Nereden",
            Self::CityTo => "Nereye",
            Self::Product => "Ürün",
            Self::FinishedProduct => "Bitmiş ürün",
            Self::QtyU32 => "Miktar (birim)",
            Self::PriceLira => "Birim fiyat (₺)",
            Self::AmountLira => "Tutar (₺)",
            Self::DurationTicks => "Vade (tick)",
            Self::DeliveryTick => "Teslimat tick'i",
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
}

impl Wizard {
    fn new(kind: ActionKind) -> Self {
        Self {
            kind,
            fields: Vec::new(),
            text_buf: String::new(),
        }
    }

    /// Şu an doldurulması gereken alan. `None` → tüm alanlar tamam, confirm.
    fn current(&self) -> Option<FieldKind> {
        self.kind.schema().get(self.fields.len()).copied()
    }

    fn is_done(&self) -> bool {
        self.fields.len() >= self.kind.schema().len()
    }
}

struct App {
    state: GameState,
    last_tick_log: Vec<String>,
    recent_news: Vec<NewsItem>,
    prev_prices: std::collections::BTreeMap<(CityId, ProductKind), Money>,
    auto_sim: bool,
    mode: Mode,
    /// Startup ekranında seçilen preset (oyun başlayınca `state.config`'e yazılır).
    selected_preset: PresetChoice,
    /// İnsan komutları — tick ilerledikçe NPC komutlarıyla birlikte advance'e iletilir.
    pending_human_cmds: Vec<Command>,
    /// Tek seferlik status satırı (başarı/hata). Bir sonraki tick'te temizlenir.
    status: Option<StatusMsg>,
    /// Human `OrderId` sayacı — her yeni order için monoton artar.
    next_human_order_id: u64,
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

impl App {
    fn new() -> Self {
        // Boş state; gerçek dünya `start_game(role)` ile kurulur.
        let state = GameState::new(RoomId::new(1), RoomConfig::hizli());
        Self {
            state,
            last_tick_log: Vec::new(),
            recent_news: Vec::new(),
            prev_prices: std::collections::BTreeMap::new(),
            auto_sim: false,
            mode: Mode::Startup,
            selected_preset: PresetChoice::Hizli,
            pending_human_cmds: Vec::new(),
            status: None,
            next_human_order_id: 1,
        }
    }

    fn start_game(&mut self, role: Role) {
        self.state = seed_world(role, self.selected_preset.config());
        self.mode = Mode::Normal;
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
        let npc_cmds = decide_all_npcs(&self.state, &mut rng, next_tick);

        // İnsan komutları önce (sıra fark etmez ama insan kararını önce
        // göstermek log'da daha okunur).
        let mut cmds: Vec<Command> = self.pending_human_cmds.drain(..).collect();
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
        self.last_tick_log = summarize_report(&report);
        self.harvest_news();
        // Eski status mesajı varsa temizle (yeni tick'in kendi mesajı olsun).
        self.status = None;
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

fn seed_world(human_role: Role, config: RoomConfig) -> GameState {
    let mut s = GameState::new(RoomId::new(1), config);

    // İnsan oyuncu — rol'e göre özelleştirilmiş başlangıç paketi.
    let (starting_cash, human_name) = match human_role {
        // Sanayici: daha az nakit, İstanbul'da pamuk stoğu (fabrika beslemesi için).
        Role::Sanayici => (50_000_i64, "Sen (Sanayici)"),
        // Tüccar: bol nakit (kervan + arbitraj sermayesi), az stok.
        Role::Tuccar => (80_000_i64, "Sen (Tüccar)"),
    };
    let mut human = Player::new(
        HUMAN_ID,
        human_name,
        human_role,
        Money::from_lira(starting_cash).unwrap(),
        false,
    )
    .unwrap();
    // Sanayici fabrikasını besleyebilmek için pamuk stoğu ile başlar.
    if matches!(human_role, Role::Sanayici) {
        human
            .inventory
            .add(CityId::Istanbul, ProductKind::Pamuk, 100)
            .unwrap();
    }
    s.players.insert(human.id, human);

    // NPC #1 — Tüccar satıcı (bol stok).
    let mut npc1 = Player::new(
        PlayerId::new(100),
        "NPC-Tüccar",
        Role::Tuccar,
        Money::from_lira(15_000).unwrap(),
        true,
    )
    .unwrap();
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            npc1.inventory.add(city, product, 80).unwrap();
        }
    }
    s.players.insert(npc1.id, npc1);

    // NPC #2 — Sanayici (biraz stok + bol nakit).
    let mut npc2 = Player::new(
        PlayerId::new(101),
        "NPC-Sanayici",
        Role::Sanayici,
        Money::from_lira(30_000).unwrap(),
        true,
    )
    .unwrap();
    npc2.inventory
        .add(CityId::Istanbul, ProductKind::Kumas, 50)
        .unwrap();
    npc2.inventory
        .add(CityId::Ankara, ProductKind::Un, 50)
        .unwrap();
    s.players.insert(npc2.id, npc2);

    // NPC #3 — Nakit-ağırlık alıcı.
    let npc3 = Player::new(
        PlayerId::new(102),
        "NPC-Alıcı",
        Role::Tuccar,
        Money::from_lira(50_000).unwrap(),
        true,
    )
    .unwrap();
    s.players.insert(npc3.id, npc3);

    s
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

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(10),   // middle (panels)
            Constraint::Length(3), // leaderboard
            Constraint::Length(1), // footer
        ])
        .split(area);

    render_header(f, chunks[0], app);
    render_middle(f, chunks[1], app);
    // Command mode'dayken leaderboard yerine komut hint'i göster —
    // oyuncu hangi parametreleri girmesi gerektiğini bilsin.
    if matches!(app.mode, Mode::Command { .. }) {
        render_command_hint(f, chunks[2], app);
    } else {
        render_leaderboard(f, chunks[2], app);
    }
    render_footer(f, chunks[3], app);

    // Overlay'ler — ortada popup, arkaplanı temizle.
    match app.mode {
        Mode::Help => render_help_overlay(f, area, app),
        Mode::Info => render_info_overlay(f, area, app),
        Mode::Holdings => render_holdings_overlay(f, area, app),
        Mode::NewsInbox => render_news_inbox_overlay(f, area, app),
        Mode::GameOver => render_game_over_overlay(f, area, app),
        Mode::Wizard { ref wizard } => render_wizard_overlay(f, area, app, wizard),
        _ => {}
    }
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
        help_kv("s", "Auto-sim aç/kapa (300ms tick)"),
        help_kv(
            "m",
            "Varlıklarım (emir / fabrika / kervan / kontrat / kredi)",
        ),
        help_kv("n", "Haber kutusu"),
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
        help_kv("b", "🛒 AL — şehir × ürün × miktar × fiyat"),
        help_kv("S", "💸 SAT (büyük S — küçük 's' auto-sim)"),
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
        help_kv("r", "💳 Kredi öde"),
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
        Line::from("  1) `:` ile komut yaz (buy/sell/...) — tick içinde biriker, kilitli"),
        Line::from("  2) SPACE bas → komutlar uygulanır, NPC'ler hamle yapar, piyasa temizlenir"),
        Line::from("  3) Üretim / kervan varış / kontrat / kredi otomatik işlenir"),
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
            FieldKind::City | FieldKind::CityFrom | FieldKind::CityTo => {
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
            | FieldKind::DeliveryTick => {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{}_", wizard.text_buf),
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                let hint = match field {
                    FieldKind::QtyU32 => "rakam tuşları → birim sayısı, Enter onay",
                    FieldKind::PriceLira => "rakam tuşları → ₺ cinsinden birim fiyat, Enter onay",
                    FieldKind::AmountLira => "rakam tuşları → toplam ₺, Enter onay",
                    FieldKind::DurationTicks => "rakam tuşları → kaç tick, Enter onay",
                    FieldKind::DeliveryTick => "rakam tuşları → teslimat tick'i, Enter onay",
                    _ => "",
                };
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("  💡 {hint}"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
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

    let board = leaderboard(&app.state);
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
        "  q veya Esc ile çık",
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
            format!("Sezon {}", season),
            Style::default().fg(Color::Green),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{}", app.state.config.preset),
            Style::default().fg(Color::Magenta),
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

fn render_middle(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // Sol sütun: player paneli (üst) + haber (alt)
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(cols[0]);
    render_player_panel(f, left[0], app);
    render_news_panel(f, left[1], app);

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

    // Satır 5+: en büyük 3 stok satırı (inline)
    let mut entries: Vec<(CityId, ProductKind, u32)> = player
        .inventory
        .entries()
        .filter(|(_, _, q)| *q > 0)
        .collect();
    entries.sort_by_key(|(_, _, q)| std::cmp::Reverse(*q));
    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (stok yok — :buy ile al)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  en çok stok:",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        for (city, product, qty) in entries.iter().take(3) {
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
        if entries.len() > 3 {
            lines.push(Line::from(Span::styled(
                format!("  … +{} satır ( m ile tam liste)", entries.len() - 3),
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
    // Hücre = fiyat + ok (↑/↓/—) rengiyle.
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
            let last = app
                .state
                .price_history
                .get(&key)
                .and_then(|v| v.last())
                .map(|(_, p)| *p);
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
            let cell = Line::from(vec![
                Span::styled(
                    format!("{}", price),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {arrow}"), Style::default().fg(color)),
            ]);
            cells.push(ratatui::text::Text::from(cell));
        }
        rows.push(Row::new(cells));
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
    let table = Table::new(rows, widths).header(header_row).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Pazar (fiyatlar / tick) ")
            .border_style(Style::default().fg(Color::Yellow)),
    );
    f.render_widget(table, area);
}

fn render_news_panel(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let items: Vec<ListItem> = if app.recent_news.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (henüz haber yok)",
            Style::default().fg(Color::DarkGray),
        )))]
    } else {
        app.recent_news
            .iter()
            .rev()
            .map(|n| {
                let tier_icon = match n.tier {
                    NewsTier::Bronze => "🥉",
                    NewsTier::Silver => "🥈",
                    NewsTier::Gold => "🥇",
                };
                let tier_style = Style::default()
                    .fg(tier_color(n.tier))
                    .add_modifier(Modifier::BOLD);
                let line = Line::from(vec![
                    Span::styled(format!("{tier_icon} "), tier_style),
                    Span::styled(
                        format!("tick {:>2} ", n.event_tick.value()),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw(format_event(&n.event)),
                ]);
                ListItem::new(line)
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Haber ")
            .border_style(Style::default().fg(Color::Magenta)),
    );
    f.render_widget(list, area);
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

fn render_leaderboard(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let board = leaderboard(&app.state);
    let mut spans: Vec<Span> = vec![Span::styled(
        " ★ Leaderboard ",
        Style::default()
            .fg(Color::Black)
            .bg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    spans.push(Span::raw("  "));
    for (idx, sc) in board.iter().take(5).enumerate() {
        spans.push(rank_span(idx, sc, &app.state));
        spans.push(Span::raw("  "));
    }
    let line = Line::from(spans);
    let block = Block::default().borders(Borders::TOP | Borders::BOTTOM);
    let para = Paragraph::new(line).block(block);
    f.render_widget(para, area);
}

fn rank_span<'a>(idx: usize, sc: &PlayerScore, state: &GameState) -> Span<'a> {
    let name = state
        .players
        .get(&sc.player_id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| format!("{}", sc.player_id));
    let medal = match idx {
        0 => "🥇",
        1 => "🥈",
        2 => "🥉",
        _ => "  ",
    };
    let is_human = sc.player_id == HUMAN_ID;
    let style = if is_human {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    Span::styled(format!("{medal} {name:<14} {}", sc.total), style)
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
            spans.extend_from_slice(&[
                hotkey("SPACE"),
                Span::raw(" tick  "),
                hotkey("b"),
                Span::raw("/"),
                hotkey("S"),
                Span::raw(" al/sat  "),
                hotkey("f"),
                Span::raw(" fab  "),
                hotkey("c"),
                Span::raw("/"),
                hotkey("d"),
                Span::raw(" kervan  "),
                hotkey("m"),
                Span::raw(" varlık  "),
                hotkey("?"),
                Span::raw(" tüm tuşlar  "),
                hotkey("q"),
                Span::raw(" çık"),
            ]);
            f.render_widget(Paragraph::new(Line::from(spans)), area);
        }
    }
}

fn hotkey(label: &str) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White)
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
        match code {
            KeyCode::Char(c) if c.is_ascii_digit() => wizard.text_buf.push(c),
            KeyCode::Enter => {
                let parsed: Result<u64, _> = wizard.text_buf.parse();
                match parsed {
                    Ok(n) if n > 0 => {
                        wizard.fields.push(FieldValue::Number(n));
                        wizard.text_buf.clear();
                    }
                    _ => {
                        return WizardOutcome::Error("geçerli bir pozitif sayı gir".into());
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
        FieldKind::City | FieldKind::CityFrom | FieldKind::CityTo => {
            CityId::ALL.get(idx).map(|c| FieldValue::City(*c))
        }
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
            let price_lira = i64::try_from(pick_number(3)?).map_err(|_| "fiyat çok büyük")?;
            let price = Money::from_lira(price_lira).map_err(|e| format!("{e}"))?;
            let side = if matches!(wizard.kind, ActionKind::Buy) {
                OrderSide::Buy
            } else {
                OrderSide::Sell
            };
            let order = MarketOrder::new(
                app.next_order_id(),
                HUMAN_ID,
                city,
                product,
                side,
                qty,
                price,
                tick,
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
            let from = pick_city(1)?;
            let to = pick_city(2)?;
            let product = pick_product(3)?;
            let qty = u32::try_from(pick_number(4)?).map_err(|_| "miktar çok büyük")?;
            let mut cargo = moneywar_domain::CargoSpec::new();
            cargo.add(product, qty).map_err(|e| format!("{e}"))?;
            Ok(Command::DispatchCaravan {
                caravan_id: cid,
                from,
                to,
                cargo,
            })
        }
        ActionKind::Loan => {
            let amount_lira = i64::try_from(pick_number(0)?).map_err(|_| "tutar çok büyük")?;
            let duration = u32::try_from(pick_number(1)?).map_err(|_| "vade çok büyük")?;
            let amount = Money::from_lira(amount_lira).map_err(|e| format!("{e}"))?;
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
            let price_lira = i64::try_from(pick_number(2)?).map_err(|_| "fiyat")?;
            let unit_price = Money::from_lira(price_lira).map_err(|e| format!("{e}"))?;
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
    let price_lira: i64 = args[2]
        .parse()
        .map_err(|_| format!("geçersiz fiyat: {}", args[2]))?;
    let unit_price = Money::from_lira(price_lira).map_err(|e| format!("{e}"))?;
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
        let d: i64 = args[5]
            .parse()
            .map_err(|_| format!("geçersiz deposit: {}", args[5]))?;
        Money::from_lira(d).map_err(|e| format!("{e}"))?
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
    if args.len() != 4 {
        return Err("kullanım: buy/sell <şehir> <ürün> <miktar> <fiyat>".into());
    }
    let city = parse_city(args[0])?;
    let product = parse_product(args[1])?;
    let qty: u32 = args[2]
        .parse()
        .map_err(|_| format!("geçersiz miktar: {}", args[2]))?;
    let price_lira: i64 = args[3]
        .parse()
        .map_err(|_| format!("geçersiz fiyat: {}", args[3]))?;
    let price = Money::from_lira(price_lira).map_err(|e| format!("fiyat hatası: {e}"))?;
    let order = MarketOrder::new(
        app.next_order_id(),
        HUMAN_ID,
        city,
        product,
        side,
        qty,
        price,
        tick,
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
    let amount_lira: i64 = args[0]
        .parse()
        .map_err(|_| format!("geçersiz miktar: {}", args[0]))?;
    let amount = Money::from_lira(amount_lira).map_err(|e| format!("miktar hatası: {e}"))?;
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
        } => format!(
            "yeni pazar {} / {} (+{} talep)",
            city_short(*city),
            product,
            extra_demand
        ),
    }
}

fn summarize_report(report: &moneywar_engine::TickReport) -> Vec<String> {
    let tick = report.tick.value();
    let mut out = Vec::new();
    out.push(format!("── Tick {tick} ──"));

    let mut matches = 0;
    let mut rejects = 0;
    let mut events = Vec::<String>::new();
    let mut production = 0;
    let mut caravan_arrivals = 0;
    let mut contracts_settled = Vec::<String>::new();
    let mut loans = Vec::<String>::new();

    for entry in &report.entries {
        match &entry.event {
            LogEvent::OrderMatched {
                quantity, price, ..
            } => {
                matches += 1;
                out.push(format!("  • {} adet eşleşti @ {}", quantity, price));
            }
            LogEvent::CommandRejected { reason, .. } => {
                rejects += 1;
                out.push(format!("  ✗ REJECT: {reason}"));
            }
            LogEvent::EventScheduled {
                game_event,
                event_tick,
                ..
            } => {
                events.push(format!(
                    "tick {}'te {}",
                    event_tick.value(),
                    format_event(game_event)
                ));
            }
            LogEvent::ProductionCompleted { units, product, .. } => {
                production += *units;
                out.push(format!("  ✓ Üretim: {} {} envantere", units, product));
            }
            LogEvent::CaravanArrived {
                city, cargo_total, ..
            } => {
                caravan_arrivals += 1;
                out.push(format!(
                    "  🚚 Kervan {} vardı ({} birim)",
                    city_short(*city),
                    cargo_total
                ));
            }
            LogEvent::ContractSettled { final_state, .. } => match final_state {
                moneywar_domain::ContractState::Fulfilled => {
                    contracts_settled.push("fulfilled".into());
                    out.push("  ✓ Kontrat teslim edildi".into());
                }
                moneywar_domain::ContractState::Breached { .. } => {
                    contracts_settled.push("breach".into());
                    out.push("  ⚠ Kontrat caydı (breach)".into());
                }
                _ => {}
            },
            LogEvent::LoanRepaid { amount_paid, .. } => {
                loans.push(format!("ödendi {amount_paid}"));
                out.push(format!("  💰 Kredi ödendi: {amount_paid}"));
            }
            LogEvent::LoanDefaulted { seized, .. } => {
                loans.push(format!("default ({seized})"));
                out.push(format!("  ⚠ Kredi DEFAULT (el koydu: {seized})"));
            }
            _ => {}
        }
    }

    if !events.is_empty() {
        for e in events {
            out.push(format!("  🎲 Yeni olay: {e}"));
        }
    }
    if matches == 0 && rejects == 0 && production == 0 && caravan_arrivals == 0 {
        out.push("  (sakin tick)".into());
    }

    out
}
