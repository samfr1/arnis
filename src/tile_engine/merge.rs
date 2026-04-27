//! Merge per-tile Anvil region directories into a single master world.
//!
//! Each tile's child process writes a complete Minecraft world into its own
//! `<world>_tiles/<tile_id>/Arnis World 1/` directory. After a tile finishes
//! the parent process calls `merge_tile_into_master` here to copy each
//! generated chunk from the tile's `region/r.X.Z.mca` files into the
//! master world's `region/r.X.Z.mca` files. Existing chunks at the same
//! `(region, chunk)` address are overwritten by the new tile's data
//! (last-tile-wins).
//!
//! The first successful merge also copies `level.dat` (and a few sibling
//! files Minecraft expects) into the master world root so the user can
//! actually open it.
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use fastanvil::Region;

/// Region template (8 KiB of empty header + sector marker) embedded in the
/// existing world editor — reuse it so newly created master region files
/// have the right shape.
const REGION_TEMPLATE: &[u8] = include_bytes!("../../assets/minecraft/region.template");

/// Merge `tile_world_dir` (`<tile_dir>/Arnis World N/`) into `master_world_dir`.
/// Returns the number of chunks copied.
///
/// On the first call into a new master world, world-level files such as
/// `level.dat` are also copied so the master directory is a valid
/// Minecraft save.
pub fn merge_tile_into_master(
    tile_world_dir: &Path,
    master_world_dir: &Path,
) -> Result<usize, String> {
    fs::create_dir_all(master_world_dir.join("region"))
        .map_err(|e| format!("create master region dir: {e}"))?;

    // Copy level.dat / icon.png / etc once.
    seed_master_metadata(tile_world_dir, master_world_dir)?;

    let src_region_dir = tile_world_dir.join("region");
    if !src_region_dir.exists() {
        return Ok(0);
    }

    let mut chunks_copied = 0usize;
    for entry in fs::read_dir(&src_region_dir).map_err(|e| format!("read tile region dir: {e}"))? {
        let entry = entry.map_err(|e| format!("read tile region entry: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("mca") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let dst_path = master_world_dir.join("region").join(name);
        chunks_copied += merge_region_file(&path, &dst_path)
            .map_err(|e| format!("merge {}: {e}", name))?;
    }
    Ok(chunks_copied)
}

/// Locate the Minecraft world subdir written by the child arnis process.
///
/// The child writes to `--output-dir` and arnis creates a numbered subdir
/// like `Arnis World 1/`. We pick the first one we find — there should
/// only ever be one per per-tile output directory because each tile gets
/// a fresh empty parent.
pub fn find_child_world_dir(child_output_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(child_output_dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && p.join("level.dat").exists() {
            return Some(p);
        }
    }
    None
}

fn seed_master_metadata(tile_world_dir: &Path, master_world_dir: &Path) -> Result<(), String> {
    for name in ["level.dat", "icon.png", "session.lock"] {
        let src = tile_world_dir.join(name);
        let dst = master_world_dir.join(name);
        if src.exists() && !dst.exists() {
            fs::copy(&src, &dst).map_err(|e| format!("copy {name}: {e}"))?;
        }
    }
    // Also create the standard subdirs Minecraft expects.
    for sub in ["data", "playerdata", "DIM-1/region", "DIM1/region"] {
        let _ = fs::create_dir_all(master_world_dir.join(sub));
    }
    Ok(())
}

/// Copy every populated chunk from `src` into `dst`. If `dst` doesn't yet
/// exist, the entire source file is copied as-is (faster than per-chunk
/// rewrite). Otherwise each source chunk is read and re-written into the
/// destination; existing chunks at the same coords are replaced.
fn merge_region_file(src: &Path, dst: &Path) -> Result<usize, String> {
    if !dst.exists() {
        fs::copy(src, dst).map_err(|e| format!("fast-path copy: {e}"))?;
        return Ok(count_populated_chunks(src).unwrap_or(0));
    }

    let src_file = std::fs::File::open(src).map_err(|e| format!("open src: {e}"))?;
    let mut src_region =
        Region::from_stream(src_file).map_err(|e| format!("parse src region: {e}"))?;

    let dst_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dst)
        .map_err(|e| format!("open dst: {e}"))?;
    let mut dst_region =
        Region::from_stream(dst_file).map_err(|e| format!("parse dst region: {e}"))?;

    let mut copied = 0usize;
    for x in 0..32usize {
        for z in 0..32usize {
            match src_region.read_chunk(x, z) {
                Ok(Some(data)) => {
                    dst_region
                        .write_chunk(x, z, &data)
                        .map_err(|e| format!("write chunk ({x},{z}): {e}"))?;
                    copied += 1;
                }
                Ok(None) => {}
                Err(e) => return Err(format!("read src chunk ({x},{z}): {e}")),
            }
        }
    }
    Ok(copied)
}

fn count_populated_chunks(path: &Path) -> Result<usize, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let mut r = Region::from_stream(f).map_err(|e| format!("parse: {e}"))?;
    let mut n = 0;
    for x in 0..32usize {
        for z in 0..32usize {
            if matches!(r.read_chunk(x, z), Ok(Some(_))) {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Initialize an empty master region directory layout with no .mca files.
/// Currently only ensures the `region/` subdir exists; called eagerly so
/// snapshot logic always finds a stable target directory.
#[allow(dead_code)]
pub fn ensure_master_world_dir(master: &Path) -> Result<(), String> {
    fs::create_dir_all(master.join("region"))
        .map_err(|e| format!("ensure master region dir: {e}"))?;
    let _ = REGION_TEMPLATE; // silence unused-const if no .mca yet
    let mut sentinel = fs::File::create(master.join(".arnis-master")).ok();
    if let Some(f) = sentinel.as_mut() {
        let _ = f.write_all(b"arnis master world\n");
    }
    Ok(())
}
