//! GUI module - Optional graphical interface using iced
//!
//! Provides a modern, responsive GUI for Diamond Drill.
//! Build with `--features gui` to enable.

#[cfg(feature = "gui")]
mod app;
#[cfg(feature = "gui")]
pub mod theme;

#[cfg(feature = "gui")]
pub use app::run_gui;

/// Placeholder for GUI when feature is disabled
#[cfg(not(feature = "gui"))]
pub fn run_gui(_args: ()) -> anyhow::Result<()> {
    anyhow::bail!("GUI feature not enabled. Rebuild with --features gui")
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_size() {
        fn parse_size(size: &str) -> (u32, u32) {
            let parts: Vec<&str> = size.split('x').collect();
            if parts.len() == 2 {
                let w = parts[0].parse().unwrap_or(1280);
                let h = parts[1].parse().unwrap_or(800);
                (w, h)
            } else {
                (1280, 800)
            }
        }

        assert_eq!(parse_size("1280x800"), (1280, 800));
        assert_eq!(parse_size("1920x1080"), (1920, 1080));
        assert_eq!(parse_size("invalid"), (1280, 800));
    }
}
