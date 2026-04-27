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
/// never has to type a path or bbox by hand. The "main" pre-fill is the
/// configured default region (St. Petersburg out of the box, configurable
/// via `~/.config/arnis/config.toml`); the world-wide button uses the
/// hard-coded planetary bbox.
pub async fn defaults(State(state): State<Arc<AppState>>) -> Response {
    let cfg = &state.config;
    let default_path = if cfg.default_world_path.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
        cwd.join("arnis-world").display().to_string()
    } else {
        cfg.default_world_path.clone()
    };
    Json(serde_json::json!({
        "default_world_path": default_path,
        "default_bbox": cfg.default_bbox,
        "world_bbox": [-85.0_f64, -180.0_f64, 85.0_f64, 180.0_f64],
        "default_tile_size_km": cfg.default_tile_size_km,
        "default_world_tile_km": 100.0_f64,
        "min_tile_size_km": MIN_TILE_SIZE_KM,
        "max_tile_size_km": MAX_TILE_SIZE_KM,
        "bind": cfg.bind,
        "port": cfg.port,
    }))
    .into_response()
}

#[derive(Debug, Serialize)]
struct SnapshotResp {
    snapshot_path: String,
    chunks_estimated: u64,
    queued_at_unix: u64,
}

/// Take a non-blocking snapshot of the master world directory. The actual
/// copy runs on a worker thread so the executor can keep generating tiles;
/// we copy the live world tree to a sibling `<world>_snapshot_<ts>/`.
///
/// The snapshot is best-effort: if the executor happens to be merging a
/// tile concurrently, individual region files may be partially copied for
/// that one tile but the rest of the world is fine. Practically users
/// should pause first if they want a perfectly consistent snapshot.
pub async fn snapshot_world(State(state): State<Arc<AppState>>) -> Response {
    let manifest = match state.current_manifest() {
        Some(m) => m,
        None => return err(StatusCode::BAD_REQUEST, "no job loaded"),
    };
    let (world_path, ts, job_id) = {
        let m = manifest.lock().unwrap();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        (m.world_path.clone(), ts, m.job_id.clone())
    };

    if !world_path.exists() {
        return err(
            StatusCode::BAD_REQUEST,
            "master world directory has not been written yet — generate at least one tile first",
        );
    }

    let snap_path = world_path.with_file_name(format!(
        "{}_snapshot_{ts}",
        world_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "world".to_string())
    ));

    let logs = state.logs.clone();
    let events = state.events.clone();
    let snap_clone = snap_path.clone();
    let world_clone = world_path.clone();
    let job_id_clone = job_id.clone();

    std::thread::Builder::new()
        .name("arnis-snapshot".into())
        .spawn(move || {
            let line = format!(
                "[snapshot] starting copy {} -> {}",
                world_clone.display(),
                snap_clone.display()
            );
            logs.push(line.clone());
            let _ = events.send(crate::tile_engine::executor::ExecutorEvent::LogLine { line });
            match copy_dir_recursive(&world_clone, &snap_clone) {
                Ok(bytes) => {
                    let line = format!(
                        "[snapshot] done ({} MiB) -> {}",
                        bytes / (1024 * 1024),
                        snap_clone.display()
                    );
                    logs.push(line.clone());
                    let _ = events.send(
                        crate::tile_engine::executor::ExecutorEvent::LogLine { line },
                    );
                }
                Err(e) => {
                    let line = format!("[snapshot] FAILED: {e}");
                    logs.push(line.clone());
                    let _ = events.send(
                        crate::tile_engine::executor::ExecutorEvent::LogLine { line },
                    );
                }
            }
            let _ = job_id_clone;
        })
        .ok();

    Json(SnapshotResp {
        snapshot_path: snap_path.display().to_string(),
        chunks_estimated: 0,
        queued_at_unix: ts,
    })
    .into_response()
}

#[derive(Debug, Serialize)]
struct SnapshotEntry {
    path: String,
    timestamp: u64,
    size_mib: u64,
}

/// List all snapshot directories that match `<world>_snapshot_<ts>` next to
/// the current job's master world.
pub async fn list_snapshots(State(state): State<Arc<AppState>>) -> Response {
    let empty: Vec<SnapshotEntry> = Vec::new();
    let Some(manifest) = state.current_manifest() else {
        return Json(serde_json::json!({ "snapshots": empty })).into_response();
    };
    let world_path = manifest.lock().unwrap().world_path.clone();
    let Some(parent) = world_path.parent().map(|p| p.to_path_buf()) else {
        return Json(serde_json::json!({ "snapshots": empty })).into_response();
    };
    let prefix = format!(
        "{}_snapshot_",
        world_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    );

    let mut entries = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&parent) {
        for e in rd.flatten() {
            let name = e.file_name();
            let name_s = name.to_string_lossy().into_owned();
            if let Some(ts_str) = name_s.strip_prefix(&prefix) {
                if let Ok(ts) = ts_str.parse::<u64>() {
                    let size = dir_size_bytes(&e.path()).unwrap_or(0);
                    entries.push(SnapshotEntry {
                        path: e.path().display().to_string(),
                        timestamp: ts,
                        size_mib: size / (1024 * 1024),
                    });
                }
            }
        }
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.timestamp));
    Json(serde_json::json!({ "snapshots": entries })).into_response()
}

fn dir_size_bytes(p: &std::path::Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(p)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += dir_size_bytes(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<u64> {
    std::fs::create_dir_all(dst)?;
    let mut total = 0u64;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            total += copy_dir_recursive(&entry.path(), &dst_path)?;
        } else if ty.is_file() {
            total += std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(total)
}
