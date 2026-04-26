//! Backup + restore for the Minecraft world folder.
//!
//! Before the very first tile of a new job is processed, the entire world
//! folder is recursively copied to a sibling directory named
//! `<world>_backup_<unix-timestamp>`. The "Restore Backup" button in the admin
//! panel copies that folder back over the live world. Restore is only allowed
//! while the job is paused or canceled (never mid-run), and the tile engine
//! enforces this.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::manifest::unix_now;

pub struct SnapshotManager;

impl SnapshotManager {
    /// Create a backup of `world_path` next to it. If `world_path` does not yet
    /// exist (a fresh-world job), nothing is copied and `Ok(None)` is returned
    /// — there's nothing to back up.
    pub fn create_backup(world_path: &Path) -> io::Result<Option<PathBuf>> {
        if !world_path.exists() {
            return Ok(None);
        }
        let parent = world_path.parent().unwrap_or_else(|| Path::new("."));
        let name = world_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "world".to_string());
        let backup = parent.join(format!("{name}_backup_{}", unix_now()));
        copy_dir_recursive(world_path, &backup)?;
        Ok(Some(backup))
    }

    /// Restore `world_path` from `backup_path` by deleting the live world and
    /// copying the backup back into place.
    pub fn restore(world_path: &Path, backup_path: &Path) -> io::Result<()> {
        if !backup_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("backup path does not exist: {}", backup_path.display()),
            ));
        }
        if world_path.exists() {
            fs::remove_dir_all(world_path)?;
        }
        copy_dir_recursive(backup_path, world_path)?;
        Ok(())
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    if !src.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("source is not a directory: {}", src.display()),
        ));
    }
    fs::create_dir_all(dst)?;
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        for entry in fs::read_dir(&from)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let from_path = entry.path();
            let to_path = to.join(entry.file_name());
            if ft.is_dir() {
                fs::create_dir_all(&to_path)?;
                stack.push((from_path, to_path));
            } else if ft.is_file() {
                fs::copy(&from_path, &to_path)?;
            }
            // Symlinks intentionally skipped — Minecraft worlds shouldn't have any
            // and following them risks copying outside the source tree.
        }
    }
    Ok(())
}
