//! Shared state for the admin-panel web server.

use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use crate::tile_engine::executor::{ExecutorEvent, JobControl, LogBuffer};
use crate::tile_engine::manifest::JobManifest;

/// Capacity of the WS event channel. 256 is enough headroom that a slow client
/// can fall behind for a few tiles without lagging the executor (which uses a
/// non-blocking `send`).
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// All web-server state. Wrapped in `Arc` so it can be cloned cheaply into
/// every handler.
pub struct AppState {
    /// Currently-loaded manifest. `None` until /api/job/create is called or a
    /// manifest is discovered on disk.
    pub manifest: Mutex<Option<Arc<Mutex<JobManifest>>>>,
    pub control: Mutex<Option<Arc<JobControl>>>,
    pub events: broadcast::Sender<ExecutorEvent>,
    pub logs: Arc<LogBuffer>,
    /// `Some` while the executor thread is alive.
    pub executor_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl AppState {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            manifest: Mutex::new(None),
            control: Mutex::new(None),
            events: tx,
            logs: LogBuffer::new(),
            executor_handle: Mutex::new(None),
        }
    }

    /// Best-effort startup hook. The web panel can call /api/job/status with a
    /// manifest path to load existing state explicitly; we don't try to scan
    /// the filesystem here because there's no canonical world location.
    pub fn try_resume_existing(&self) {
        // Intentionally empty — actual resume happens via the admin panel's
        // "Resume" button which posts to /api/job/start.
    }

    pub fn current_manifest(&self) -> Option<Arc<Mutex<JobManifest>>> {
        self.manifest.lock().unwrap().clone()
    }

    pub fn current_control(&self) -> Option<Arc<JobControl>> {
        self.control.lock().unwrap().clone()
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
