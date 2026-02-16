//! Diamond Spinner - Beautiful progress indicators
//!
//! Provides:
//! - Animated diamond spinner for scanning
//! - Pulsing progress bar for exports
//! - Color-coded status indicators
//! - ETA calculations

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use colored::Colorize;

/// Diamond spinner animation frames
const DIAMOND_FRAMES: &[&str] = &["💎", "◇ ", "◈ ", "◆ ", "◈ ", "◇ "];

/// Pulse bar animation frames
const PULSE_FRAMES: &[&str] = &[
    "▰▱▱▱▱▱▱▱▱▱",
    "▰▰▱▱▱▱▱▱▱▱",
    "▰▰▰▱▱▱▱▱▱▱",
    "▱▰▰▰▱▱▱▱▱▱",
    "▱▱▰▰▰▱▱▱▱▱",
    "▱▱▱▰▰▰▱▱▱▱",
    "▱▱▱▱▰▰▰▱▱▱",
    "▱▱▱▱▱▰▰▰▱▱",
    "▱▱▱▱▱▱▰▰▰▱",
    "▱▱▱▱▱▱▱▰▰▰",
    "▱▱▱▱▱▱▱▱▰▰",
    "▱▱▱▱▱▱▱▱▱▰",
];

/// Status indicators
pub struct StatusIcons;

impl StatusIcons {
    pub const SUCCESS: &'static str = "✓";
    pub const ERROR: &'static str = "✗";
    pub const WARNING: &'static str = "⚠";
    pub const INFO: &'static str = "ℹ";
    pub const SCAN: &'static str = "🔍";
    pub const EXPORT: &'static str = "📤";
    pub const VERIFY: &'static str = "🔐";
    pub const HEAL: &'static str = "🩹";
    pub const DIAMOND: &'static str = "💎";
}

/// Animated diamond spinner
pub struct DiamondSpinner {
    running: Arc<AtomicBool>,
    message: Arc<parking_lot::RwLock<String>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl DiamondSpinner {
    /// Create and start a new spinner
    pub fn new(message: &str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let message = Arc::new(parking_lot::RwLock::new(message.to_string()));

        let r = Arc::clone(&running);
        let m = Arc::clone(&message);

        let handle = thread::spawn(move || {
            let mut frame = 0;
            while r.load(Ordering::Relaxed) {
                let msg = m.read().clone();
                print!(
                    "\r{} {} ",
                    DIAMOND_FRAMES[frame % DIAMOND_FRAMES.len()].cyan(),
                    msg
                );
                let _ = io::stdout().flush();

                frame += 1;
                thread::sleep(Duration::from_millis(120));
            }
        });

        Self {
            running,
            message,
            handle: Some(handle),
        }
    }

    /// Update the spinner message
    pub fn set_message(&self, msg: &str) {
        *self.message.write() = msg.to_string();
    }

    /// Stop with success message
    pub fn success(self, msg: &str) {
        self.stop();
        println!("\r{} {}", StatusIcons::SUCCESS.green(), msg.green());
    }

    /// Stop with error message
    pub fn error(self, msg: &str) {
        self.stop();
        println!("\r{} {}", StatusIcons::ERROR.red(), msg.red());
    }

    /// Stop with warning message
    pub fn warn(self, msg: &str) {
        self.stop();
        println!("\r{} {}", StatusIcons::WARNING.yellow(), msg.yellow());
    }

    /// Stop the spinner
    fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        print!("\r{}\r", " ".repeat(80)); // Clear line
        let _ = io::stdout().flush();
    }
}

impl Drop for DiamondSpinner {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Pulsing progress bar
pub struct PulseProgress {
    running: Arc<AtomicBool>,
    current: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    message: Arc<parking_lot::RwLock<String>>,
    start_time: Instant,
    handle: Option<thread::JoinHandle<()>>,
}

impl PulseProgress {
    /// Create a new progress bar
    pub fn new(total: u64, message: &str) -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let current = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(total));
        let message = Arc::new(parking_lot::RwLock::new(message.to_string()));
        let start_time = Instant::now();

        let r = Arc::clone(&running);
        let c = Arc::clone(&current);
        let t = Arc::clone(&total);
        let m = Arc::clone(&message);

        let handle = thread::spawn(move || {
            let mut frame = 0;
            while r.load(Ordering::Relaxed) {
                let cur = c.load(Ordering::Relaxed);
                let tot = t.load(Ordering::Relaxed);
                let msg = m.read().clone();

                let pct = if tot > 0 {
                    (cur as f64 / tot as f64 * 100.0).min(100.0)
                } else {
                    0.0
                };

                // Build progress bar
                let bar_width = 30;
                let filled = (pct / 100.0 * bar_width as f64) as usize;
                let bar: String = (0..bar_width)
                    .map(|i| {
                        if i < filled {
                            "█"
                        } else if i == filled {
                            &PULSE_FRAMES[frame % PULSE_FRAMES.len()][..1]
                        } else {
                            "░"
                        }
                    })
                    .collect();

                print!(
                    "\r{} [{}] {:>5.1}% {} ",
                    StatusIcons::DIAMOND.cyan(),
                    bar.cyan(),
                    pct,
                    msg
                );
                let _ = io::stdout().flush();

                frame += 1;
                thread::sleep(Duration::from_millis(100));
            }
        });

        Self {
            running,
            current,
            total,
            message,
            start_time,
            handle: Some(handle),
        }
    }

    /// Update progress
    pub fn set(&self, current: u64) {
        self.current.store(current, Ordering::Relaxed);
    }

    /// Increment progress
    pub fn inc(&self, delta: u64) {
        self.current.fetch_add(delta, Ordering::Relaxed);
    }

    /// Update message
    pub fn set_message(&self, msg: &str) {
        *self.message.write() = msg.to_string();
    }

    /// Get elapsed time
    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    /// Calculate ETA
    pub fn eta(&self) -> Option<Duration> {
        let current = self.current.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);

        if current == 0 || current >= total {
            return None;
        }

        let elapsed = self.start_time.elapsed();
        let rate = current as f64 / elapsed.as_secs_f64();
        let remaining = (total - current) as f64 / rate;

        Some(Duration::from_secs_f64(remaining))
    }

    /// Format ETA for display
    pub fn eta_string(&self) -> String {
        match self.eta() {
            Some(eta) => {
                let secs = eta.as_secs();
                if secs < 60 {
                    format!("{}s", secs)
                } else if secs < 3600 {
                    format!("{}m {}s", secs / 60, secs % 60)
                } else {
                    format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
                }
            }
            None => "---".to_string(),
        }
    }

    /// Stop with success
    pub fn success(self, msg: &str) {
        self.stop();
        let elapsed = self.elapsed();
        println!(
            "\r{} {} ({})",
            StatusIcons::SUCCESS.green(),
            msg.green(),
            format_duration(elapsed)
        );
    }

    /// Stop with error
    pub fn error(self, msg: &str) {
        self.stop();
        println!("\r{} {}", StatusIcons::ERROR.red(), msg.red());
    }

    fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        print!("\r{}\r", " ".repeat(80));
        let _ = io::stdout().flush();
    }
}

impl Drop for PulseProgress {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Format duration for display
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{:.1}s", d.as_secs_f64())
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m {}s", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
}

/// Print a styled header
pub fn print_header(title: &str) {
    let width = 60;
    let padding = (width - title.len() - 4) / 2;

    println!();
    println!("{}", "═".repeat(width).cyan());
    println!(
        "{}  {}  {}",
        " ".repeat(padding),
        title.bright_white().bold(),
        " ".repeat(padding)
    );
    println!("{}", "═".repeat(width).cyan());
    println!();
}

/// Print a styled subheader
pub fn print_subheader(title: &str) {
    println!("\n{} {}", "▶".bright_cyan(), title.bright_white());
    println!("{}", "─".repeat(40).bright_black());
}

/// Print a key-value pair
pub fn print_kv(key: &str, value: &str) {
    println!("  {}: {}", key.bright_black(), value.white());
}

/// Print a success message
pub fn print_success(msg: &str) {
    println!("{} {}", StatusIcons::SUCCESS.green(), msg.green());
}

/// Print an error message
pub fn print_error(msg: &str) {
    println!("{} {}", StatusIcons::ERROR.red(), msg.red());
}

/// Print a warning message
pub fn print_warning(msg: &str) {
    println!("{} {}", StatusIcons::WARNING.yellow(), msg.yellow());
}

/// Print an info message
pub fn print_info(msg: &str) {
    println!("{} {}", StatusIcons::INFO.cyan(), msg.cyan());
}

/// Print file type gauge (colored bar showing distribution)
pub fn print_type_gauge(types: &[(String, usize, u64)]) {
    let total: usize = types.iter().map(|(_, c, _)| c).sum();
    if total == 0 {
        return;
    }

    let bar_width = 50;
    let colors = [
        "\x1b[35m", // Magenta - Images
        "\x1b[36m", // Cyan - Videos
        "\x1b[33m", // Yellow - Audio
        "\x1b[32m", // Green - Documents
        "\x1b[34m", // Blue - Archives
        "\x1b[31m", // Red - Code
        "\x1b[37m", // White - Other
    ];

    print!("  [");
    for (i, (_, count, _)) in types.iter().enumerate() {
        let width = (*count as f64 / total as f64 * bar_width as f64) as usize;
        let color = colors[i % colors.len()];
        print!("{}{}\x1b[0m", color, "█".repeat(width.max(1)));
    }
    println!("]");

    // Legend
    for (i, (name, count, bytes)) in types.iter().enumerate() {
        let color = colors[i % colors.len()];
        println!(
            "  {}█\x1b[0m {} {} ({})",
            color,
            name,
            count,
            humansize::format_size(*bytes, humansize::BINARY)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::from_secs(30)), "30.0s");
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30s");
        assert_eq!(format_duration(Duration::from_secs(3700)), "1h 1m 40s");
    }

    #[test]
    fn test_eta_calculation() {
        let progress = PulseProgress::new(100, "test");
        progress.set(50);
        thread::sleep(Duration::from_millis(100));

        // ETA should be roughly equal to elapsed time (50% done)
        let eta = progress.eta();
        assert!(eta.is_some());
    }
}
