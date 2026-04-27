//! Persistent admin-panel configuration.
//!
//! Read once at server startup from a TOML file in the user's config dir
//! (`~/.config/arnis/config.toml` on Linux/macOS, `%APPDATA%\arnis\config.toml`
//! on Windows). All fields are optional; missing values fall back to the
//! built-in defaults (5 km tile centered on St. Petersburg, bind 127.0.0.1
//! on port 7373).
//!
//! The CLI flags `--host` / `--port` always override the config file so the
//! user can spin up an alternate instance without editing on disk.
//!
//! The config file is created lazily the first time the panel runs; users
//! can edit it freely afterwards.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Built-in default bbox: a 5 km square centered on St. Petersburg
/// (Невский проспект). Picked because it has dense OSM coverage and is a
/// good first run for users who just want to click "Generate".
pub const DEFAULT_CENTER_LAT: f64 = 59.9343;
pub const DEFAULT_CENTER_LNG: f64 = 30.3351;
pub const DEFAULT_TILE_SIZE_KM_FALLBACK: f64 = 5.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AdminConfig {
    /// Address to bind the HTTP server to. Use "127.0.0.1" for local-only
    /// access (default) or "0.0.0.0" to expose on the LAN. The CLI flag
    /// `--host` overrides this.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// TCP port. The CLI flag `--port` overrides this.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Default bbox pre-filled in the admin panel. `[min_lat, min_lng, max_lat, max_lng]`.
    #[serde(default = "default_bbox")]
    pub default_bbox: [f64; 4],
    /// Default Minecraft world folder. If empty, falls back to
    /// `<cwd>/arnis-world`.
    #[serde(default)]
    pub default_world_path: String,
    /// Default tile size in km. Range 1..=100.
    #[serde(default = "default_tile_size")]
    pub default_tile_size_km: f64,
    /// Whether to automatically open the admin panel in the system browser
    /// after server startup.
    #[serde(default = "default_auto_open")]
    pub auto_open_browser: bool,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}
fn default_port() -> u16 {
    7373
}
fn default_bbox() -> [f64; 4] {
    // 5 km square around St. Petersburg center.
    let half_lat_deg = 2_500.0 / 111_320.0;
    let half_lng_deg = 2_500.0
        / (111_320.0 * (DEFAULT_CENTER_LAT.to_radians().cos().abs()).max(1e-6));
    [
        DEFAULT_CENTER_LAT - half_lat_deg,
        DEFAULT_CENTER_LNG - half_lng_deg,
        DEFAULT_CENTER_LAT + half_lat_deg,
        DEFAULT_CENTER_LNG + half_lng_deg,
    ]
}
fn default_tile_size() -> f64 {
    DEFAULT_TILE_SIZE_KM_FALLBACK
}
fn default_auto_open() -> bool {
    true
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            default_bbox: default_bbox(),
            default_world_path: String::new(),
            default_tile_size_km: default_tile_size(),
            auto_open_browser: default_auto_open(),
        }
    }
}

/// Resolve the platform-appropriate config file path.
pub fn config_path() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join("arnis");
    Some(dir.join("config.toml"))
}

/// Load the config from disk, or return defaults if the file is missing or
/// malformed. On the very first call we also write the defaults to disk so
/// the user has a starting template to edit.
pub fn load_or_create() -> AdminConfig {
    let Some(path) = config_path() else {
        return AdminConfig::default();
    };
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<AdminConfig>(&text) {
                Ok(cfg) => return cfg,
                Err(e) => {
                    eprintln!(
                        "warning: failed to parse {}: {}. Using defaults.",
                        path.display(),
                        e
                    );
                    return AdminConfig::default();
                }
            },
            Err(e) => {
                eprintln!(
                    "warning: failed to read {}: {}. Using defaults.",
                    path.display(),
                    e
                );
                return AdminConfig::default();
            }
        }
    }

    let cfg = AdminConfig::default();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let header = "# Arnis admin panel configuration\n\
        # See https://github.com/louis-e/arnis for documentation.\n\
        # Override defaults below; CLI flags --host/--port still take priority.\n\n";
    if let Ok(body) = toml::to_string_pretty(&cfg) {
        let _ = std::fs::write(&path, format!("{header}{body}"));
    }
    cfg
}
