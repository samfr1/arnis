//! Local HTTP + WebSocket admin panel for tile-based world generation.
//!
//! Bound to `127.0.0.1` only — no LAN exposure, no auth needed. Runs on the
//! tokio runtime; the tile executor itself runs on a stdlib thread (see
//! `tile_engine::executor`) and communicates with this server via channels.

pub mod api;
pub mod assets;
pub mod state;
pub mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

pub use state::AppState;

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
        .route("/ws/events", get(ws::ws_handler))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

/// Run the server until Ctrl+C. Opens the admin panel in the user's default
/// browser as soon as the listener is bound.
pub async fn run(port: u16) -> Result<(), String> {
    let state = Arc::new(AppState::new());

    // Try to load any pre-existing manifest so a restarted process shows the
    // last job's state immediately. Discovery is best-effort: we look for a
    // `*.arnis-job.json` next to a typical world folder via the user's saved
    // setting cache, which doesn't exist yet on first run — so this just no-ops
    // until /api/job/create is called.
    state.try_resume_existing();

    let app = router(Arc::clone(&state));
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            return Err(format!(
                "Failed to bind {addr}: {e}. Is another arnis --web instance already running?"
            ));
        }
    };

    let url = format!("http://127.0.0.1:{port}/");
    println!("Arnis admin panel listening on {url}");
    if let Err(e) = open::that(&url) {
        eprintln!("Could not auto-open browser ({e}); navigate to {url} manually.");
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
