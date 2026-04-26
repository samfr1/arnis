//! On-disk job manifest with crash-safe atomic updates.
//!
//! The manifest is the single source of truth for an in-progress generation
//! run. Every tile completion (success or failure) flushes the manifest to
//! disk via write-to-tmp + rename so the file is never observed in a torn
//! state, even if the process is force-killed mid-flush.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::grid::{BBox, GenerationSettings, TileTask};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TileStatus {
    Pending,
    Running,
    Done,
    Failed,
    Canceled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Paused,
    Done,
    Canceled,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TileEntry {
    pub task: TileTask,
    pub status: TileStatus,
    #[serde(default)]
    pub started_at: Option<u64>,
    #[serde(default)]
    pub finished_at: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
    /// Per-tile output directory, relative to the job's `world_path` parent.
    pub tile_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobManifest {
    pub job_id: String,
    pub bbox: BBox,
    pub tile_size_km: f64,
    pub world_path: PathBuf,
    pub backup_path: Option<PathBuf>,
    pub manifest_path: PathBuf,
    pub tiles: Vec<TileEntry>,
    pub status: JobStatus,
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub canceled: usize,
    pub started_at: u64,
    pub updated_at: u64,
    pub settings: GenerationSettings,
}

impl JobManifest {
    pub fn manifest_path_for(world_path: &Path) -> PathBuf {
        let parent = world_path.parent().unwrap_or_else(|| Path::new("."));
        let stem = world_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "world".to_string());
        parent.join(format!("{stem}.arnis-job.json"))
    }

    pub fn new(
        job_id: String,
        bbox: BBox,
        tile_size_km: f64,
        world_path: PathBuf,
        tasks: Vec<TileTask>,
        settings: GenerationSettings,
    ) -> Self {
        let manifest_path = Self::manifest_path_for(&world_path);
        let now = unix_now();
        let total = tasks.len();
        let job_root = world_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{}_tiles", file_stem(&world_path)));
        let tiles = tasks
            .into_iter()
            .map(|t| {
                let dir = job_root.join(&t.id);
                TileEntry {
                    tile_dir: dir,
                    task: t,
                    status: TileStatus::Pending,
                    started_at: None,
                    finished_at: None,
                    error: None,
                }
            })
            .collect();
        Self {
            job_id,
            bbox,
            tile_size_km,
            world_path,
            backup_path: None,
            manifest_path,
            tiles,
            status: JobStatus::Pending,
            total,
            done: 0,
            failed: 0,
            canceled: 0,
            started_at: now,
            updated_at: now,
            settings,
        }
    }

    /// Atomically write the manifest to disk: serialize → write to a `.tmp`
    /// sibling → fsync → rename over the real path. This is the only way the
    /// manifest is ever updated; readers see either the previous full copy or
    /// the new full copy, never a partial write.
    pub fn save_atomic(&self) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        if let Some(parent) = self.manifest_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.manifest_path.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &self.manifest_path)?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let bytes = fs::read(path)?;
        let mut m: JobManifest = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        m.manifest_path = path.to_path_buf();
        Ok(m)
    }

    /// On startup, any tile that was `Running` when the previous process died
    /// is reset to `Pending` so the executor will retry it.
    pub fn reset_running_to_pending(&mut self) {
        for t in &mut self.tiles {
            if t.status == TileStatus::Running {
                t.status = TileStatus::Pending;
                t.started_at = None;
                t.finished_at = None;
            }
        }
        if matches!(self.status, JobStatus::Running) {
            self.status = JobStatus::Paused;
        }
        self.recount();
    }

    pub fn touch(&mut self) {
        self.updated_at = unix_now();
    }

    pub fn recount(&mut self) {
        self.done = self
            .tiles
            .iter()
            .filter(|t| t.status == TileStatus::Done)
            .count();
        self.failed = self
            .tiles
            .iter()
            .filter(|t| t.status == TileStatus::Failed)
            .count();
        self.canceled = self
            .tiles
            .iter()
            .filter(|t| t.status == TileStatus::Canceled)
            .count();
        self.total = self.tiles.len();
    }

    /// Index of the next tile to run, if any.
    pub fn next_pending(&self) -> Option<usize> {
        self.tiles
            .iter()
            .position(|t| t.status == TileStatus::Pending)
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn file_stem(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "world".to_string())
}
