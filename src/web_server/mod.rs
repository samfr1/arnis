//! Local HTTP + WebSocket admin panel for tile-based world generation.
//!
//! Bound to `127.0.0.1` only — no LAN exposure, no auth needed. Runs on the
//! tokio runtime; the tile executor itself runs on a stdlib thread (see
//! `tile_engine::executor`) and communicates with this server via channels.

pub mod api;
pub mod assets;
pub mod config;
pub mod state;
pub mod ws;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

pub use config::AdminConfig;
pub use state::AppState;

/// Configuration handed to `run` from the CLI/main glue. Allows callers to
/// override `host`/`port` from CLI flags while still reading the rest from
/// the on-disk config file.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub config: AdminConfig,
}

/// Build the axum router. Public so it can be used in tests.
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(assets::serve_index))
        .route("/index.html", get(assets::serve_index))
        .route("/api/job/create", post(api::create_job))
        .route("/api/job/start", post(api::start_job))
        .route("/api/job/pause", post(api::pause_job))
        .route("/api/job/resume", post(api::resume_job))
        .route("/api/job/cancel", post(api::cancel_job))
        .route("/api/job/restore-backup", post(api::restore_backup))
        .route("/api/job/status", get(api::job_status))
        .route("/api/job/tiles", get(api::job_tiles))
        .route("/api/logs", get(api::logs))
        .route("/api/defaults", get(api::defaults))
        .route("/api/job/snapshot", post(api::snapshot_world))
        .route("/api/job/snapshots", get(api::list_snapshots))
        .route("/ws/events", get(ws::ws_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

/// Run the admin server until Ctrl+C.
pub async fn run(opts: ServerOptions) -> Result<(), String> {
    let cfg = opts.config.clone();
    let state = Arc::new(AppState::new(cfg.clone()));
    state.try_resume_existing();

    let app = router(Arc::clone(&state));
    let host: IpAddr = cfg
        .bind
        .parse()
        .map_err(|e| format!("invalid bind address {:?}: {e}", cfg.bind))?;
    let addr = SocketAddr::from((host, cfg.port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            return Err(format!(
                "Failed to bind {addr}: {e}. Is another arnis --web instance already running?"
            ));
        }
    };

    let local_url = format!("http://127.0.0.1:{}/", cfg.port);
    println!("Arnis admin panel listening on http://{addr}/");
    if cfg.bind != "127.0.0.1" {
        println!("Local URL:   {local_url}");
    }
    if cfg.auto_open_browser {
        if let Err(e) = open::that(&local_url) {
            eprintln!("Could not auto-open browser ({e}); navigate to {local_url} manually.");
        }
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("server error: {e}"))?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    println!("\nShutting down arnis admin panel...");
}
