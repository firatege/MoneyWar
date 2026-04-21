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
    clippy::comparison_chain
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
    CityId, GameState, Money, NewsItem, NewsTier, Player, PlayerId, ProductKind, Role, RoomConfig,
    RoomId,
};
use moneywar_engine::{LogEvent, PlayerScore, advance_tick, leaderboard, rng_for, score_player};
use moneywar_npc::decide_all_npcs;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Row, Table};

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

        // Auto-sim tetikleme.
        if app.auto_sim && last_auto_tick.elapsed() >= Duration::from_millis(300) {
            app.step_one_tick();
            last_auto_tick = Instant::now();
        }

        // Event polling — kısa timeout ki auto-sim çalışsın.
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char(' ') => app.step_one_tick(),
                    KeyCode::Char('s') => app.auto_sim = !app.auto_sim,
                    _ => {}
                }
            }
        }

        if app.game_over() {
            // Son ekranı görsün, `q` ile çık.
            app.auto_sim = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Uygulama durumu
// ---------------------------------------------------------------------------

struct App {
    state: GameState,
    /// Son tick'in ham event'leri (özet için kaydedilir).
    last_tick_log: Vec<String>,
    /// İnsan oyuncunun son N haberi (tier ile birlikte).
    recent_news: Vec<NewsItem>,
    /// Son tick'te her `(city, product)` için clearing fiyatı delta hesabı
    /// (yukarı/aşağı rengini boyarken kullanılır).
    prev_prices: std::collections::BTreeMap<(CityId, ProductKind), Money>,
    auto_sim: bool,
}

impl App {
    fn new() -> Self {
        let state = seed_world();
        Self {
            state,
            last_tick_log: Vec::new(),
            recent_news: Vec::new(),
            prev_prices: std::collections::BTreeMap::new(),
            auto_sim: false,
        }
    }

    fn step_one_tick(&mut self) {
        if self.game_over() {
            return;
        }
        let next_tick = self.state.current_tick.next();
        let mut rng = rng_for(self.state.room_id, next_tick);
        let npc_cmds = decide_all_npcs(&self.state, &mut rng, next_tick);

        let Ok((new_state, report)) = advance_tick(&self.state, &npc_cmds) else {
            self.last_tick_log
                .push("[ENGINE HATASI] advance_tick başarısız".into());
            return;
        };

        // Önceki fiyatları sakla — delta için.
        self.prev_prices = new_state
            .price_history
            .iter()
            .filter_map(|(k, v)| v.last().map(|(_, p)| (*k, *p)))
            .collect();

        self.state = new_state;
        self.last_tick_log = summarize_report(&report);
        self.harvest_news();
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

fn seed_world() -> GameState {
    let mut s = GameState::new(RoomId::new(1), RoomConfig::hizli());

    // İnsan — Sanayici, İstanbul'da pamuk stoğu + nakit.
    let mut human = Player::new(
        HUMAN_ID,
        "Sen",
        Role::Sanayici,
        Money::from_lira(50_000).unwrap(),
        false,
    )
    .unwrap();
    human
        .inventory
        .add(CityId::Istanbul, ProductKind::Pamuk, 100)
        .unwrap();
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
    render_leaderboard(f, chunks[2], app);
    render_footer(f, chunks[3], app);
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

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            player.name.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  ("),
        Span::styled(
            format!("{}", player.role),
            Style::default().fg(role_color(player.role)),
        ),
        Span::raw(")"),
    ]));
    lines.push(Line::from(""));
    lines.push(kv("Nakit", &format!("{}", player.cash), Color::Green));
    lines.push(kv("Skor", &format!("{}", score.total), Color::Yellow));
    lines.push(Line::from(vec![
        Span::styled("  └ Nakit ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{}", score.cash), Style::default().fg(Color::Gray)),
        Span::raw("  "),
        Span::styled("Stok ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", score.stock_value),
            Style::default().fg(Color::Gray),
        ),
        Span::raw("  "),
        Span::styled("Fabrika ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", score.factory_value),
            Style::default().fg(Color::Gray),
        ),
        Span::raw("  "),
        Span::styled("Escrow ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}", score.escrow_value),
            Style::default().fg(Color::Gray),
        ),
    ]));

    lines.push(Line::from(""));
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
    lines.push(kv("Fabrika", &format!("{factory_count}"), Color::Magenta));
    lines.push(kv("Kervan", &format!("{caravan_count}"), Color::Magenta));

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Stok",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    let mut stock_lines = 0;
    for (city, product, qty) in player.inventory.entries() {
        if qty == 0 {
            continue;
        }
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("{:<9}", city_short(city)),
                Style::default().fg(Color::Blue),
            ),
            Span::styled(
                format!("{:<10}", product),
                Style::default().fg(product_color(product)),
            ),
            Span::styled(
                format!("{qty:>5}"),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        stock_lines += 1;
        if stock_lines >= 6 {
            lines.push(Line::from(Span::styled(
                "  …",
                Style::default().fg(Color::DarkGray),
            )));
            break;
        }
    }
    if stock_lines == 0 {
        lines.push(Line::from(Span::styled(
            "  (boş)",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Oyuncu ")
        .border_style(Style::default().fg(Color::Cyan));
    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

fn render_market_panel(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let header_row = Row::new(vec!["Şehir", "Ürün", "Son Fiyat", "Δ", "Son5 Avg"]).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    );

    let mut rows: Vec<Row> = Vec::new();
    for city in CityId::ALL {
        for product in ProductKind::ALL {
            let key = (city, product);
            let hist = app.state.price_history.get(&key);
            let last = hist.and_then(|v| v.last()).map(|(_, p)| *p);
            let avg = app.state.rolling_avg_price(city, product, 5);
            let Some(price) = last else {
                continue;
            };
            let prev = app.prev_prices.get(&key).copied();
            let delta = prev.map_or(0, |p| price.as_cents() - p.as_cents());
            let delta_style = if delta > 0 {
                Style::default().fg(Color::Green)
            } else if delta < 0 {
                Style::default().fg(Color::Red)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let delta_str = if delta == 0 {
                "—".to_string()
            } else {
                format!(
                    "{}{:.2}",
                    if delta > 0 { "+" } else { "-" },
                    (delta.abs() as f64) / 100.0
                )
            };
            rows.push(Row::new(vec![
                ratatui::text::Text::from(Span::styled(
                    city_short(city),
                    Style::default().fg(Color::Blue),
                )),
                ratatui::text::Text::from(Span::styled(
                    format!("{product}"),
                    Style::default().fg(product_color(product)),
                )),
                ratatui::text::Text::from(format!("{price}")),
                ratatui::text::Text::from(Span::styled(delta_str, delta_style)),
                ratatui::text::Text::from(avg.map_or("—".to_string(), |m| format!("{m}"))),
            ]));
        }
    }

    if rows.is_empty() {
        rows.push(Row::new(vec!["—", "(Henüz clearing olmadı)", "", "", ""]));
    }

    let widths = [
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths).header(header_row).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Pazar ")
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

fn render_footer(f: &mut ratatui::Frame<'_>, area: Rect, _app: &App) {
    let line = Line::from(vec![
        Span::styled(
            " SPACE ",
            Style::default().bg(Color::DarkGray).fg(Color::White),
        ),
        Span::raw(" tick ilerle   "),
        Span::styled(" s ", Style::default().bg(Color::DarkGray).fg(Color::White)),
        Span::raw(" auto-sim   "),
        Span::styled(" q ", Style::default().bg(Color::DarkGray).fg(Color::White)),
        Span::raw(" çık"),
    ]);
    let para = Paragraph::new(line);
    f.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn kv(label: &str, value: &str, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<10}"), Style::default().fg(Color::DarkGray)),
        Span::styled(
            value.to_string(),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

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
