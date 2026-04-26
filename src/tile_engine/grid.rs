//! Tile grid construction.
//!
//! Splits an input geographic bounding box into a rectangular grid of tiles
//! sized in kilometers. Each tile carries an inner bbox (the area committed to
//! the world) and a buffered bbox (250 m of overlap on every side, fetched and
//! generated to give neighboring tiles enough context to render features that
//! cross tile borders without visible seams).

use serde::{Deserialize, Serialize};

/// Approximate meters per degree of latitude. Treated as constant — the
/// curvature error is well under a meter inside any reasonable city/region.
pub const METERS_PER_DEG_LAT: f64 = 111_320.0;

/// Overlap buffer applied around each tile's bbox in meters.
pub const TILE_BUFFER_METERS: f64 = 250.0;

/// Default tile size when the user doesn't override it.
pub const DEFAULT_TILE_SIZE_KM: f64 = 5.0;

/// Minimum and maximum allowed tile size in km.
pub const MIN_TILE_SIZE_KM: f64 = 1.0;
pub const MAX_TILE_SIZE_KM: f64 = 100.0;

/// Plain bounding box used by the tile engine. Mirrors `LLBBox` but is
/// `Serialize`/`Deserialize` so it can live in the on-disk manifest.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub min_lat: f64,
    pub min_lng: f64,
    pub max_lat: f64,
    pub max_lng: f64,
}

impl BBox {
    /// Format the bbox as the `min_lat,min_lng,max_lat,max_lng` string accepted
    /// by `LLBBox::from_str` and the existing `--bbox` CLI flag.
    pub fn to_arnis_arg(self) -> String {
        format!(
            "{},{},{},{}",
            self.min_lat, self.min_lng, self.max_lat, self.max_lng
        )
    }

    /// Lat span in degrees.
    pub fn dlat(&self) -> f64 {
        self.max_lat - self.min_lat
    }

    /// Lng span in degrees.
    pub fn dlng(&self) -> f64 {
        self.max_lng - self.min_lng
    }
}

/// Optional generation knobs forwarded to each child invocation. Mirrors a
/// subset of `Args` so the admin panel can configure jobs.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GenerationSettings {
    #[serde(default)]
    pub terrain: bool,
    #[serde(default = "default_true")]
    pub interior: bool,
    #[serde(default = "default_true")]
    pub roof: bool,
    #[serde(default)]
    pub fillground: bool,
    #[serde(default = "default_true")]
    pub land_cover: bool,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub bedrock: bool,
}

fn default_true() -> bool {
    true
}
fn default_scale() -> f64 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TileTask {
    /// Stable per-tile id of the form `r{row:03}-c{col:03}`.
    pub id: String,
    pub row: u32,
    pub col: u32,
    /// Inner bbox — the area whose generated blocks should be considered the
    /// authoritative output for this tile.
    pub bbox: BBox,
    /// Buffered bbox — the area actually fetched and generated. Includes
    /// `TILE_BUFFER_METERS` of overlap on every side (clamped to the planet).
    pub buffered_bbox: BBox,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TileGrid {
    pub tiles: Vec<TileTask>,
    pub rows: u32,
    pub cols: u32,
    pub tile_size_km: f64,
}

/// Validate `tile_size_km` and split `bbox` into a left-to-right, top-to-bottom
/// grid of tiles approximately `tile_size_km` on a side.
pub fn build_grid(bbox: &BBox, tile_size_km: f64) -> Result<TileGrid, String> {
    if !(MIN_TILE_SIZE_KM..=MAX_TILE_SIZE_KM).contains(&tile_size_km) {
        return Err(format!(
            "tile_size_km {tile_size_km} out of range {MIN_TILE_SIZE_KM}..={MAX_TILE_SIZE_KM}"
        ));
    }
    if bbox.dlat() <= 0.0 || bbox.dlng() <= 0.0 {
        return Err("bbox has non-positive extent".to_string());
    }

    let tile_size_m = tile_size_km * 1000.0;
    let center_lat = (bbox.min_lat + bbox.max_lat) / 2.0;
    let m_per_deg_lng = METERS_PER_DEG_LAT * center_lat.to_radians().cos().abs().max(1e-6);

    let dlat = tile_size_m / METERS_PER_DEG_LAT;
    let dlng = tile_size_m / m_per_deg_lng;
    let buf_lat = TILE_BUFFER_METERS / METERS_PER_DEG_LAT;
    let buf_lng = TILE_BUFFER_METERS / m_per_deg_lng;

    let rows = (bbox.dlat() / dlat).ceil().max(1.0) as u32;
    let cols = (bbox.dlng() / dlng).ceil().max(1.0) as u32;

    let mut tiles = Vec::with_capacity((rows as usize) * (cols as usize));
    for row in 0..rows {
        // Row 0 is the northernmost (top) strip.
        let r_max = (bbox.max_lat - dlat * row as f64).min(bbox.max_lat);
        let r_min = (r_max - dlat).max(bbox.min_lat);
        for col in 0..cols {
            let c_min = (bbox.min_lng + dlng * col as f64).max(bbox.min_lng);
            let c_max = (c_min + dlng).min(bbox.max_lng);
            if r_max - r_min <= 0.0 || c_max - c_min <= 0.0 {
                continue;
            }
            let inner = BBox {
                min_lat: r_min,
                min_lng: c_min,
                max_lat: r_max,
                max_lng: c_max,
            };
            let buffered = BBox {
                min_lat: clamp_lat(inner.min_lat - buf_lat),
                min_lng: clamp_lng(inner.min_lng - buf_lng),
                max_lat: clamp_lat(inner.max_lat + buf_lat),
                max_lng: clamp_lng(inner.max_lng + buf_lng),
            };
            tiles.push(TileTask {
                id: format!("r{:03}-c{:03}", row, col),
                row,
                col,
                bbox: inner,
                buffered_bbox: buffered,
            });
        }
    }

    Ok(TileGrid {
        tiles,
        rows,
        cols,
        tile_size_km,
    })
}

fn clamp_lat(v: f64) -> f64 {
    v.clamp(-89.9999, 89.9999)
}
fn clamp_lng(v: f64) -> f64 {
    v.clamp(-179.9999, 179.9999)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_tile_size() {
        let b = BBox {
            min_lat: 0.0,
            min_lng: 0.0,
            max_lat: 0.1,
            max_lng: 0.1,
        };
        assert!(build_grid(&b, 0.5).is_err());
        assert!(build_grid(&b, 200.0).is_err());
    }

    #[test]
    fn small_bbox_yields_one_tile() {
        let b = BBox {
            min_lat: 48.10,
            min_lng: 11.50,
            max_lat: 48.11,
            max_lng: 11.52,
        };
        let g = build_grid(&b, 5.0).unwrap();
        assert_eq!(g.tiles.len(), 1);
        assert_eq!(g.rows, 1);
        assert_eq!(g.cols, 1);
        assert_eq!(g.tiles[0].row, 0);
        assert_eq!(g.tiles[0].col, 0);
        // Buffered bbox strictly contains the inner bbox.
        let t = &g.tiles[0];
        assert!(t.buffered_bbox.min_lat < t.bbox.min_lat);
        assert!(t.buffered_bbox.max_lat > t.bbox.max_lat);
        assert!(t.buffered_bbox.min_lng < t.bbox.min_lng);
        assert!(t.buffered_bbox.max_lng > t.bbox.max_lng);
    }

    #[test]
    fn larger_bbox_yields_grid() {
        // ~30 km wide × ~20 km tall around Munich at ~5 km tiles → 6×4 ish.
        let b = BBox {
            min_lat: 48.05,
            min_lng: 11.40,
            max_lat: 48.23,
            max_lng: 11.80,
        };
        let g = build_grid(&b, 5.0).unwrap();
        assert!(
            g.tiles.len() > 4,
            "expected multiple tiles, got {}",
            g.tiles.len()
        );
        // Tiles should be ordered left-to-right, top-to-bottom.
        let first = &g.tiles[0];
        let last = &g.tiles[g.tiles.len() - 1];
        assert_eq!(first.row, 0);
        assert_eq!(first.col, 0);
        assert_eq!(last.row, g.rows - 1);
        assert_eq!(last.col, g.cols - 1);
    }
}
