//! Configuration Module - User preferences from ~/.ddrill/config.toml
//!
//! Supports:
//! - Default export destination
//! - Theme preferences (dark/light/auto)
//! - Keyboard shortcuts customization
//! - Read-only enforcement settings

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Diamond Drill Configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// General settings
    pub general: GeneralConfig,
    /// Export settings
    pub export: ExportConfig,
    /// TUI settings
    pub tui: TuiConfig,
    /// Scan settings
    pub scan: ScanConfig,
    /// Custom keyboard shortcuts
    #[serde(default)]
    pub keys: HashMap<String, String>,
}

/// General application settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Theme: dark, light, auto
    pub theme: String,
    /// Enforce read-only mode (panic if write access detected)
    pub enforce_readonly: bool,
    /// Log level: trace, debug, info, warn, error
    pub log_level: String,
    /// Check for updates on startup
    pub check_updates: bool,
    /// Show tips on startup
    pub show_tips: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            theme: "auto".to_string(),
            enforce_readonly: true,
            log_level: "info".to_string(),
            check_updates: false,
            show_tips: true,
        }
    }
}

/// Export settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportConfig {
    /// Default destination directory
    pub default_dest: Option<PathBuf>,
    /// Preserve directory structure by default
    pub preserve_structure: bool,
    /// Create manifest by default
    pub create_manifest: bool,
    /// Verify hashes by default
    pub verify_hash: bool,
    /// Continue on errors by default
    pub continue_on_error: bool,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            default_dest: dirs::document_dir().map(|d| d.join("Diamond Drill Exports")),
            preserve_structure: true,
            create_manifest: true,
            verify_hash: true,
            continue_on_error: true,
        }
    }
}

/// TUI settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Show file sizes
    pub show_sizes: bool,
    /// Show file dates
    pub show_dates: bool,
    /// Show file type icons
    pub show_icons: bool,
    /// Tree indent width
    pub indent_width: usize,
    /// Enable vim keybindings
    pub vim_mode: bool,
    /// Show hidden files
    pub show_hidden: bool,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            show_sizes: true,
            show_dates: true,
            show_icons: true,
            indent_width: 2,
            vim_mode: true,
            show_hidden: false,
        }
    }
}

/// Scan settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// Number of parallel workers (0 = auto)
    pub workers: usize,
    /// Skip hidden files by default
    pub skip_hidden: bool,
    /// Checkpoint interval (0 = disabled)
    pub checkpoint_interval: usize,
    /// Block size for bad sector detection
    pub block_size: usize,
    /// Default file extensions to filter (empty = all)
    pub default_extensions: Vec<String>,
    /// Max depth (0 = unlimited)
    pub max_depth: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            workers: 0, // auto-detect
            skip_hidden: true,
            checkpoint_interval: 1000,
            block_size: 4096,
            default_extensions: Vec::new(),
            max_depth: 0,
        }
    }
}

impl Config {
    /// Load config from default path or return defaults
    pub fn load() -> Self {
        Self::load_from(&Self::default_path()).unwrap_or_default()
    }

    /// Load config from a specific path
    pub fn load_from(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config: {}", path.display()))?;

        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config: {}", path.display()))?;

        Ok(config)
    }

    /// Save config to default path
    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::default_path())
    }

    /// Save config to a specific path
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(path, content)
            .with_context(|| format!("Failed to write config: {}", path.display()))?;

        Ok(())
    }

    /// Get default config path
    pub fn default_path() -> PathBuf {
        directories::ProjectDirs::from("com", "tunclon", "diamond-drill")
            .map(|dirs| dirs.config_dir().join("config.toml"))
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".ddrill")
                    .join("config.toml")
            })
    }

    /// Check if config file exists
    pub fn exists() -> bool {
        Self::default_path().exists()
    }

    /// Create default config file if it doesn't exist
    pub fn ensure_exists() -> Result<()> {
        let path = Self::default_path();
        if !path.exists() {
            let config = Config::default();
            config.save_to(&path)?;
            tracing::info!("Created default config at {}", path.display());
        }
        Ok(())
    }

    /// Get keybinding or default
    pub fn get_key(&self, action: &str, default: &str) -> String {
        self.keys
            .get(action)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }
}

/// Generate a sample config file with comments
pub fn generate_sample_config() -> String {
    r#"# Diamond Drill Configuration
# Location: ~/.ddrill/config.toml (or %APPDATA%\diamond-drill\config.toml on Windows)

[general]
# Theme: "dark", "light", or "auto"
theme = "auto"

# Enforce read-only mode (recommended - prevents accidental writes)
enforce_readonly = true

# Log level: trace, debug, info, warn, error
log_level = "info"

# Show helpful tips on startup
show_tips = true

[export]
# Default destination for exports (optional)
# default_dest = "/home/user/Recovered"

# Preserve original directory structure
preserve_structure = true

# Create verification manifest
create_manifest = true

# Verify blake3 hashes after copy
verify_hash = true

# Continue exporting on individual file errors
continue_on_error = true

[tui]
# Show file sizes in tree
show_sizes = true

# Show modification dates
show_dates = true

# Show file type icons (emoji)
show_icons = true

# Tree indent width
indent_width = 2

# Enable vim-style navigation (j/k/g/G)
vim_mode = true

# Show hidden files
show_hidden = false

[scan]
# Number of parallel workers (0 = auto-detect CPU count)
workers = 0

# Skip hidden files and directories
skip_hidden = true

# Auto-save checkpoint every N files (0 = disabled)
checkpoint_interval = 1000

# Block size for bad sector detection (bytes)
block_size = 4096

# Default file extensions filter (empty = all files)
# Example: ["jpg", "png", "pdf", "doc"]
default_extensions = []

# Maximum scan depth (0 = unlimited)
max_depth = 0

[keys]
# Custom keybindings (action = key)
# Available actions: quit, nav_up, nav_down, select, select_all, search, help
# quit = "q"
# nav_up = "k"
# nav_down = "j"
# select = "space"
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.general.enforce_readonly);
        assert_eq!(config.general.theme, "auto");
        assert!(config.tui.vim_mode);
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_config.toml");

        let config = Config::default();
        config.save_to(&path).unwrap();

        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.general.theme, config.general.theme);
        assert_eq!(
            loaded.export.preserve_structure,
            config.export.preserve_structure
        );
    }

    #[test]
    fn test_parse_sample_config() {
        let sample = generate_sample_config();
        let _config: Config = toml::from_str(&sample).unwrap();
    }

    #[test]
    fn test_custom_keybinding() {
        let mut config = Config::default();
        config.keys.insert("quit".to_string(), "x".to_string());

        assert_eq!(config.get_key("quit", "q"), "x");
        assert_eq!(config.get_key("nav_up", "k"), "k");
    }
}
