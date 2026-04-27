//! Tile engine: split a bounding box into a grid of small tiles, generate each
//! tile in sequence by re-invoking arnis as a child process, and persist a
//! crash-safe job manifest so generation can be paused, canceled, and resumed.
//!
//! Memory containment: each tile runs as a separate child process so all
//! intermediate buffers (parsed OSM data, ground caches, region buffers) are
//! freed by the OS when the child exits. This means selecting a 100 km × 100 km
//! region cannot OOM the parent — it just runs longer.

pub mod executor;
pub mod grid;
pub mod manifest;
pub mod merge;
pub mod snapshot;

// Re-exports kept for ergonomic access from `web_server` and tests. The
// binary itself routes through full paths, so suppress the dead-import lint.
#[allow(unused_imports)]
pub use executor::{ExecutorEvent, JobControl, JobExecutor};
#[allow(unused_imports)]
pub use grid::{build_grid, BBox, GenerationSettings, TileGrid, TileTask, TILE_BUFFER_METERS};
#[allow(unused_imports)]
pub use manifest::{JobManifest, JobStatus, TileStatus};
#[allow(unused_imports)]
pub use snapshot::SnapshotManager;
