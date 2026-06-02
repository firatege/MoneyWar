//! MoneyWar web platformu — actix backend.
//!
//! Sonsuz sezon loop'u arka planda her `TICK_SECONDS` saniyede bir tick
//! ilerletir; her tick snapshot'ı broadcast kanalına yollar. İzleyiciler
//! `WS /ws` ile canlı snapshot alır, `GET /api/snapshot` ile anlık durumu,
//! `GET /api/series` ile fiyat zaman serisini çeker.

mod debuglog;
mod driver;
mod dto;
mod world;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use actix_files::Files;
use actix_web::{App, HttpRequest, HttpResponse, HttpServer, Responder, web};
use futures_util::StreamExt;
use moneywar_npc::Difficulty;
use serde::Deserialize;
use tokio::sync::{RwLock, broadcast};

use debuglog::{LogSink, format_tick_block};
use driver::SimDriver;

/// Sezon uzunluğu (tick). Plan: 90 tick ≈ 3 dk.
const SEASON_TICKS: u32 = 90;
/// Tick aralığı (saniye). 5 sn/tick — sezon ≈ 7.5 dk.
const TICK_SECONDS: u64 = 5;
/// WS broadcast kanal kapasitesi — yavaş izleyici taşarsa eski mesaj düşer.
const BROADCAST_CAPACITY: usize = 64;

/// Paylaşılan uygulama durumu.
struct AppState {
    /// Sürücü — arka plan loop yazar, endpoint'ler okur.
    driver: Arc<RwLock<SimDriver>>,
    /// Her tick serileştirilmiş snapshot yayını.
    tx: broadcast::Sender<Arc<str>>,
    /// Tick-by-tick ekonomi debug log'u (ring buffer + opsiyonel dosya).
    log: Arc<LogSink>,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let base_seed = std::env::var("MONEYWAR_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0x4D6F_6E65_7957_6172);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let driver = Arc::new(RwLock::new(SimDriver::new(
        base_seed,
        SEASON_TICKS,
        u32::try_from(TICK_SECONDS).unwrap_or(2),
        Difficulty::Hard,
    )));
    let (tx, _rx) = broadcast::channel::<Arc<str>>(BROADCAST_CAPACITY);

    // Debug log: MONEYWAR_LOG env > /app/debug/economy.log (dizin varsa) > sadece bellek.
    let log_file: Option<PathBuf> = std::env::var("MONEYWAR_LOG")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            if std::path::Path::new("/app/debug").is_dir() {
                Some(PathBuf::from("/app/debug/economy.log"))
            } else {
                None
            }
        });
    let log = Arc::new(LogSink::new(log_file));

    spawn_tick_loop(driver.clone(), tx.clone(), log.clone());

    let app_state = web::Data::new(AppState { driver, tx, log });

    // Frontend dist dizini: STATIC_DIR env > /app/web/dist (Docker) > web/dist (dev)
    let static_dir = std::env::var("STATIC_DIR").unwrap_or_else(|_| {
        if std::path::Path::new("/app/web/dist").is_dir() {
            "/app/web/dist".into()
        } else {
            "web/dist".into()
        }
    });
    let static_dir_exists = std::path::Path::new(&static_dir).is_dir();
    tracing::info!(port, base_seed, static_dir, static_dir_exists, "moneywar-web başlıyor");

    HttpServer::new(move || {
        let mut app = App::new()
            .app_data(app_state.clone())
            .route("/api/snapshot", web::get().to(get_snapshot))
            .route("/api/series", web::get().to(get_series))
            .route("/api/log", web::get().to(get_log))
            .route("/ws", web::get().to(ws_handler));

        if static_dir_exists {
            app = app.service(
                Files::new("/", static_dir.clone()).index_file("index.html"),
            );
        }
        app
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}

/// Arka plan sezon loop'u — her `TICK_SECONDS` bir tick ilerletir, yayınlar
/// ve debug log'a yazar.
fn spawn_tick_loop(
    driver: Arc<RwLock<SimDriver>>,
    tx: broadcast::Sender<Arc<str>>,
    log: Arc<LogSink>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(TICK_SECONDS));
        loop {
            interval.tick().await;
            let (json, block) = {
                let mut d = driver.write().await;
                d.step();
                let block = format_tick_block(&d.state, &d.last_report, d.season);
                (serde_json::to_string(&d.snapshot()), block)
            };
            log.push_block(&block);
            match json {
                Ok(s) => {
                    // Alıcı yoksa hata önemsiz (kimse izlemiyor).
                    let _ = tx.send(Arc::from(s.as_str()));
                }
                Err(e) => tracing::error!(error = %e, "snapshot serileştirme hatası"),
            }
        }
    });
}

async fn get_snapshot(state: web::Data<AppState>) -> impl Responder {
    let snapshot = state.driver.read().await.snapshot();
    HttpResponse::Ok().json(snapshot)
}

#[derive(Debug, Deserialize)]
struct SeriesQuery {
    city: String,
    product: String,
}

async fn get_series(state: web::Data<AppState>, q: web::Query<SeriesQuery>) -> impl Responder {
    let (Some(city), Some(product)) = (dto::parse_city(&q.city), dto::parse_product(&q.product))
    else {
        return HttpResponse::BadRequest().body("geçersiz city/product");
    };
    let series = dto::build_series(&state.driver.read().await.state, city, product);
    HttpResponse::Ok().json(series)
}

#[derive(Debug, Deserialize)]
struct LogQuery {
    /// Son kaç satır (varsayılan 2000, 0 → tümü).
    tail: Option<usize>,
}

/// Tick-by-tick ekonomi debug log'unu düz metin döndürür (sim formatı).
async fn get_log(state: web::Data<AppState>, q: web::Query<LogQuery>) -> impl Responder {
    let n = q.tail.unwrap_or(2000);
    HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body(state.log.tail(n))
}

/// WS upgrade — bağlanan izleyiciye önce mevcut snapshot, sonra her tick yayını.
async fn ws_handler(
    req: HttpRequest,
    body: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse, actix_web::Error> {
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, body)?;

    let mut rx = state.tx.subscribe();
    let initial = serde_json::to_string(&state.driver.read().await.snapshot()).ok();

    actix_web::rt::spawn(async move {
        // İlk yükleme: anlık snapshot.
        if let Some(s) = initial {
            if session.text(s).await.is_err() {
                return;
            }
        }

        loop {
            tokio::select! {
                // Sunucudan yayın → istemciye ilet.
                msg = rx.recv() => match msg {
                    Ok(payload) => {
                        if session.text(payload.as_ref()).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                // İstemciden mesaj → ping/close yönet.
                client = msg_stream.next() => match client {
                    Some(Ok(actix_ws::Message::Ping(bytes))) => {
                        if session.pong(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(actix_ws::Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                },
            }
        }

        let _ = session.close(None).await;
    });

    Ok(response)
}
