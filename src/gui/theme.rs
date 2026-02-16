//! Dark theme constants for the Diamond Drill GUI

/// Background colors (dark mode)
pub const BG_PRIMARY: [f32; 3] = [0.11, 0.12, 0.14];
pub const BG_SECONDARY: [f32; 3] = [0.15, 0.16, 0.18];
pub const BG_SURFACE: [f32; 3] = [0.18, 0.19, 0.22];

/// Foreground / text colors
pub const FG_PRIMARY: [f32; 3] = [0.92, 0.93, 0.95];
pub const FG_SECONDARY: [f32; 3] = [0.62, 0.65, 0.70];

/// Accent colors
pub const ACCENT_CYAN: [f32; 3] = [0.0, 0.75, 0.85];
pub const ACCENT_GREEN: [f32; 3] = [0.25, 0.80, 0.40];
pub const ACCENT_RED: [f32; 3] = [0.90, 0.30, 0.30];

/// File type colors
pub const COLOR_IMAGE: [f32; 3] = [0.80, 0.40, 0.80];
pub const COLOR_VIDEO: [f32; 3] = ACCENT_CYAN;
pub const COLOR_AUDIO: [f32; 3] = [0.95, 0.80, 0.25];
pub const COLOR_DOCUMENT: [f32; 3] = ACCENT_GREEN;
pub const COLOR_ARCHIVE: [f32; 3] = [0.30, 0.55, 0.90];
pub const COLOR_CODE: [f32; 3] = ACCENT_RED;
