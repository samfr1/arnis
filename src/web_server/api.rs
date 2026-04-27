//! REST handlers for the admin panel.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::tile_engine::executor::{JobControl, JobExecutor};
use crate::tile_engine::grid::{
    build_grid, BBox, GenerationSettings, DEFAULT_TILE_SIZE_KM, MAX_TILE_SIZE_KM, MIN_TILE_SIZE_KM,
};
use crate::tile_engine::manifest::{JobManifest, JobStatus, TileStatus};
use crate::tile_engine::snapshot::SnapshotManager;

use super::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateJobReq {
    /// `[min_lat, min_lng, max_lat, max_lng]`.
    pub bbox: [f64; 4],
    #[serde(default = "default_tile_size")]
    pub tile_size_km: f64,
    pub world_path: String,
    #[serde(default)]
    pub settings: GenerationSettings,
}

fn default_tile_size() -> f64 {
    DEFAULT_TILE_SIZE_KM
}

#[derive(Debug, Serialize)]
pub struct CreateJobResp {
    pub job_id: String,
    pub total: usize,
    pub backup_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ApiError { error: msg.into() })).into_response()
}

pub async fn create_job(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateJobReq>,
) -> Response {
    // Reject if a job is already running or paused.
    if let Some(m) = state.current_manifest() {
        let s = m.lock().unwrap().status;
        if matches!(s, JobStatus::Running | JobStatus::Paused) {
            return err(
                StatusCode::CONFLICT,
                "another job is currently running or paused",
            );
        }
    }

    if !(MIN_TILE_SIZE_KM..=MAX_TILE_SIZE_KM).contains(&req.tile_size_km) {
        return err(
            StatusCode::BAD_REQUEST,
            format!("tile_size_km must be between {MIN_TILE_SIZE_KM} and {MAX_TILE_SIZE_KM}"),
        );
    }

    let bbox = BBox {
        min_lat: req.bbox[0],
        min_lng: req.bbox[1],
        max_lat: req.bbox[2],
        max_lng: req.bbox[3],
    };
    if bbox.dlat() <= 0.0 || bbox.dlng() <= 0.0 {
        return err(StatusCode::BAD_REQUEST, "bbox has non-positive extent");
    }

    let world_path = PathBuf::from(&req.world_path);
    if !world_path.is_absolute() {
        return err(
            StatusCode::BAD_REQUEST,
            format!("world_path must be absolute: {}", req.world_path),
        );
    }

    let grid = match build_grid(&bbox, req.tile_size_km) {
        Ok(g) => g,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };

    let job_id = format!("job-{}", crate::tile_engine::manifest::unix_now());
    let mut manifest = JobManifest::new(
        job_id.clone(),
        bbox,
        req.tile_size_km,
        world_path.clone(),
        grid.tiles,
        req.settings,
    );

    // Backup the existing world (if any) before any tile is written.
    let backup = match SnapshotManager::create_backup(&world_path) {
        Ok(p) => p,
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("backup failed: {e}"),
            )
        }
    };
    manifest.backup_path = backup.clone();

    if let Err(e) = manifest.save_atomic() {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to save manifest: {e}"),
        );
    }

    let total = manifest.total;
    let m_arc = Arc::new(Mutex::new(manifest));
    *state.manifest.lock().unwrap() = Some(Arc::clone(&m_arc));
    *state.control.lock().unwrap() = Some(JobControl::new());

    Json(CreateJobResp {
        job_id,
        total,
        backup_path: backup.map(|p| p.display().to_string()),
    })
    .into_response()
}

pub async fn start_job(State(state): State<Arc<AppState>>) -> Response {
    let manifest = match state.current_manifest() {
        Some(m) => m,
        None => return err(StatusCode::BAD_REQUEST, "no job exists; create one first"),
    };

    {
        let m = manifest.lock().unwrap();
        match m.status {
            JobStatus::Running => {
                return err(StatusCode::CONFLICT, "job is already running");
            }
            JobStatus::Done => {
                return err(StatusCode::CONFLICT, "job is already done");
            }
            JobStatus::Canceled => {
                return err(StatusCode::CONFLICT, "job was canceled; create a new one");
            }
            _ => {}
        }
    }

    // (Re)create the control flags and clear pause.
    let control = state.current_control().unwrap_or_else(JobControl::new);
    control.clear_pause();
    *state.control.lock().unwrap() = Some(Arc::clone(&control));

    // Reset any tile that was Running (from a previous interrupted run) to
    // Pending so the executor will retry it.
    {
        let mut m = manifest.lock().unwrap();
        m.reset_running_to_pending();
        m.touch();
        let _ = m.save_atomic();
    }

    let exec = JobExecutor::new(
        Arc::clone(&manifest),
        Arc::clone(&control),
        state.events.clone(),
        Arc::clone(&state.logs),
    );
    let handle = exec.spawn();
    *state.executor_handle.lock().unwrap() = Some(handle);

    Json(serde_json::json!({"started": true})).into_response()
}

pub async fn pause_job(State(state): State<Arc<AppState>>) -> Response {
    let Some(control) = state.current_control() else {
        return err(StatusCode::BAD_REQUEST, "no job exists");
    };
    control.request_pause();
    Json(serde_json::json!({"pause_requested": true})).into_response()
}

pub async fn resume_job(State(state): State<Arc<AppState>>) -> Response {
    let Some(_manifest) = state.current_manifest() else {
        return err(StatusCode::BAD_REQUEST, "no job exists");
    };
    // Resume == start again; reuses start logic so the executor thread is
    // recreated cleanly (the previous thread finishes after a paused tile).
    start_job(State(state)).await
}

pub async fn cancel_job(State(state): State<Arc<AppState>>) -> Response {
    let Some(control) = state.current_control() else {
        return err(StatusCode::BAD_REQUEST, "no job exists");
    };
    control.request_cancel();
    Json(serde_json::json!({"cancel_requested": true})).into_response()
}

pub async fn restore_backup(State(state): State<Arc<AppState>>) -> Response {
    let Some(manifest) = state.current_manifest() else {
        return err(StatusCode::BAD_REQUEST, "no job exists");
    };
    let (status, world_path, backup_path) = {
        let m = manifest.lock().unwrap();
        (m.status, m.world_path.clone(), m.backup_path.clone())
    };
    if matches!(status, JobStatus::Running) {
        return err(
            StatusCode::CONFLICT,
            "cannot restore backup while job is running; pause or cancel first",
        );
    }
    let Some(backup) = backup_path else {
        return err(StatusCode::BAD_REQUEST, "no backup recorded for this job");
    };

    if let Err(e) = SnapshotManager::restore(&world_path, &backup) {
        return err(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("restore failed: {e}"),
        );
    }
    Json(serde_json::json!({"restored": true})).into_response()
}

pub async fn job_status(State(state): State<Arc<AppState>>) -> Response {
    let Some(manifest) = state.current_manifest() else {
        return Json(serde_json::json!({"manifest": null})).into_response();
    };
    let m = manifest.lock().unwrap();
    Json(serde_json::json!({"manifest": &*m})).into_response()
}

#[derive(Serialize)]
struct TileGridDto {
    rows: u32,
    cols: u32,
    tiles: Vec<TileGridCell>,
}

#[derive(Serialize)]
struct TileGridCell {
    id: String,
    row: u32,
    col: u32,
    status: TileStatus,
    started_at: Option<u64>,
    finished_at: Option<u64>,
    error: Option<String>,
    bbox: BBox,
}

pub async fn job_tiles(State(state): State<Arc<AppState>>) -> Response {
    let Some(manifest) = state.current_manifest() else {
        return Json(serde_json::json!({"tiles": null})).into_response();
    };
    let m = manifest.lock().unwrap();
    let rows = m.tiles.iter().map(|t| t.task.row).max().unwrap_or(0) + 1;
    let cols = m.tiles.iter().map(|t| t.task.col).max().unwrap_or(0) + 1;
    let cells = m
        .tiles
        .iter()
        .map(|t| TileGridCell {
            id: t.task.id.clone(),
            row: t.task.row,
            col: t.task.col,
            status: t.status,
            started_at: t.started_at,
            finished_at: t.finished_at,
            error: t.error.clone(),
            bbox: t.task.bbox,
        })
        .collect();
    Json(TileGridDto {
        rows,
        cols,
        tiles: cells,
    })
    .into_response()
}

pub async fn logs(State(state): State<Arc<AppState>>) -> Response {
    Json(serde_json::json!({"lines": state.logs.snapshot()})).into_response()
}

/// Sensible defaults the admin panel uses to pre-fill its inputs so the user
/// never has to type a path or bbox by hand. "Generate Entire World" mode
/// uses `world_bbox` and `default_world_tile_km`; the regular preview map
/// uses `default_tile_size_km`.
pub async fn defaults() -> Response {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    let default_path = cwd.join("arnis-world");
    Json(serde_json::json!({
        "default_world_path": default_path.display().to_string(),
        "world_bbox": [-85.0_f64, -180.0_f64, 85.0_f64, 180.0_f64],
        "default_tile_size_km": DEFAULT_TILE_SIZE_KM,
        "default_world_tile_km": 100.0_f64,
        "min_tile_size_km": MIN_TILE_SIZE_KM,
        "max_tile_size_km": MAX_TILE_SIZE_KM
    }))
    .into_response()
}
