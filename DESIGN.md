# Diamond Drill — GUI Architecture & Design Specification

> **Status:** Design Phase — Approved, not yet implemented
> **Date:** 2026-02-08
> **Authors:** Ryan Cashmoney + AI collab
> **Version:** 1.0 (Pre-Implementation)

______________________________________________________________________

## Table of Contents

1. [Vision](#vision)
1. [Binary Architecture](#binary-architecture)
1. [Mission System](#mission-system)
1. [GUI Layout & UX Flow](#gui-layout--ux-flow)
1. [Diamond Design System](#diamond-design-system)
1. [Real-Time Event Architecture](#real-time-event-architecture)
1. [Error Handling Policy](#error-handling-policy)
1. [Shared Module Rule](#shared-module-rule)
1. [File Structure](#file-structure)
1. [Implementation Phases](#implementation-phases)
1. [Standalone Repo Borrowing](#standalone-repo-borrowing)
1. [Open Questions](#open-questions)

______________________________________________________________________

## Vision

Diamond Drill is not just a file recovery tool — it's a **forensic intelligence platform**
built on general-purpose primitives (scanning, hashing, exporting, dedup, proof chains).
File recovery is the first _composition_ of these primitives. The architecture must support
future expansions without re-architecture.

The GUI ("Diamond Drill Studio") should feel like **NASA mission control meets Bloomberg
Terminal** — premium, authoritative, information-dense, with real-time data visualization
at its core. The user should be able to watch their data being recovered in real-time,
file by file, with full visibility into what's happening at every stage.

### Core Principles

- **Recovery takes main stage** — the scan and recovery UX is the star, not the settings
- **Real-time visibility** — users see files as they're discovered and recovered
- **Forward-if-possible** — read-only operations never halt on errors
- **Modular missions** — different workflows compose the same tool modules
- **CLI parity** — every GUI feature has a CLI text fallback

______________________________________________________________________

## Binary Architecture

Two distinct binaries from the same crate:

```
diamond-drill        → CLI/TUI (lean, ~3.3 MB, always built)
diamond-drill-studio → GUI (full visual, ~18 MB, feature-gated)
```

### Cargo.toml Structure

```toml
[[bin]]
name = "diamond-drill"
path = "src/main.rs"

[[bin]]
name = "diamond-drill-studio"
path = "src/studio.rs"
required-features = ["gui"]
```

### Build Commands

```bash
cargo build --release                    # CLI only (~3.3 MB)
cargo build --release --features gui     # CLI + Studio (~18 MB)
```

### Dependency Notes

- **iced 0.13** (upgrade from 0.12) — new styling API, better custom theme support
- **iced_aw** — additional widgets
- **rfd** — native file picker dialogs
- TUI dependencies (ratatui, crossterm) remain always-on (not feature-gated)

______________________________________________________________________

## Mission System

### Concept

A "Mission" defines what tools are active and what the user sees. The GUI adapts
its UI based on the selected mission. The CLI can also use missions via subcommands.

### Mission Definition

```rust
pub struct Mission {
    pub id: MissionId,
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,

    /// Which tool modules this mission activates
    pub tools: Vec<ToolModule>,

    /// The pipeline stages the user will see
    pub stages: Vec<PipelineStage>,

    /// What inputs this mission needs
    pub requires_source: bool,
    pub requires_destination: bool,
    pub requires_case_id: bool,

    /// Error policy default
    pub error_policy: ErrorPolicy,
}
```

### Available Tool Modules

```rust
pub enum ToolModule {
    Scanner,       // Always available — file discovery
    Exporter,      // File export with Blake3 verification
    Dedup,         // Exact + fuzzy duplicate detection
    BadSector,     // Disk health scanning & heatmap
    Proof,         // Chain-of-custody Merkle proof
    Swarm,         // Document intelligence pipeline
    Preview,       // Thumbnail generation
}
```

### Pre-Defined Missions

| Mission | Icon | Tools | Stages | Error Policy |
| --------------- | ---- | -------------------------- | ----------------------------------- | -------------- |
| **Recover** | 🔄 | Scanner, Exporter, Preview | Scan → Select → Recover → Verify | ForwardOnError |
| **Investigate** | 🔍 | Scanner, Proof, Exporter | Scan → Proof → Export → Report | ForwardOnError |
| **Diagnose** | 🏥 | Scanner, BadSector | Scan → BadSector → Heatmap → Report | ForwardOnError |
| **Clean** | 🧹 | Scanner, Dedup | Scan → Dedup → Review → Purge | HaltOnError |
| **More...** | 📋 | varies | varies | varies |
| **Custom** | ⚙️ | user selects | user selects | user selects |

### Future Missions (Not Yet Implemented)

- **Data Migration** — old drive → new drive with verification
- **Backup Verification** — hash comparison against existing backup
- **eDiscovery / FOIA** — legal document collection with proof chain
- **Digital Estate** — categorize + preview + selective export
- **Document Intelligence** — CaseStar swarm integration

### CLI Mapping

Missions map to CLI subcommands:

```bash
diamond-drill recover -s E:\Backup -d C:\Recovered
diamond-drill diagnose -s E:\Backup
diamond-drill clean -s ~/Documents
diamond-drill investigate -s /evidence --case-id 2026-CR-1234
```

______________________________________________________________________

## GUI Layout & UX Flow

### Screen Flow

```
Mission Select → Source/Dest Setup → [Live Phase View] → Report/Done
                                          │
                     ┌────────────────────┼────────────────────┐
                     │                    │                    │
                  RECOVER              DIAGNOSE             CLEAN
                  (Scan→Select→        (Scan→BadSector→     (Scan→Dedup→
                   Export→Verify)       Heatmap→Report)      Review→Purge)
```

### Screen 1: Mission Select (Opening Screen)

Six mission cards arranged in a grid. Each card has:

- Diamond icon (subtle category tint)
- Mission name (large text)
- Brief description (smaller text)
- Diamond-cut chamfered corners

The Diamond Drill logo at top center has a brief shimmer/sparkle
animation on app open that fades over 20-30 seconds.

### Screen 2: Source/Dest Setup

Adapts based on selected mission:

- **Recover:** Source path + destination path
- **Diagnose:** Source path only
- **Clean:** Source path only
- **Investigate:** Source path + output folder + case ID

Native file picker via `rfd` for path selection. Drag-and-drop support.

### Screen 3: Live Scan (Discovery Feed)

Split layout:

- **Left sidebar:** Live stats — file type counters ticking up in real-time
- **Top center:** Disk map visualization (blocks lighting up as sectors are read)
- **Bottom center:** Auto-scrolling live file feed (files appear as discovered)
- **Bottom bar:** Overall progress, scan speed, ETA

### Screen 4: Select (Triage Table) — Recover Mission Only

Three-column layout:

- **Left:** Filter sidebar (type checkboxes, size range, date range)
- **Center:** Sortable virtualized file table with live fuzzy search
- **Right:** Preview panel (thumbnail + metadata + hash for selected file)
- **Bottom:** Batch action buttons (Select All Photos, Select All, Clear, → Recover)

### Screen 5: Live Recovery (Pipeline View)

The star of the show:

- **Left sidebar:** Progress stats (%, files done, speed, ETA, errors)
- **Top center:** Pipeline strip — 4 boxes showing file counts per stage:
  - Queue (gray) → Copy (blue pulse) → Verify (amber) → Done (green)
  - Failed files go to a red "Failed" state
- **Center:** Live recovery log — each file transitions through states visually
- **Bottom:** Overall progress bar with bytes transferred

### Screen 6: Report / Done

Summary of what happened:

- Files recovered / failed / skipped
- Total bytes transferred
- Verification results
- Error log (expandable)
- "Open Destination Folder" button

______________________________________________________________________

## Diamond Design System

### Visual Philosophy

| Principle | Implementation |
| ------------------- | -------------------------------------------- |
| Class, not flash | Dark surfaces, subtle gradients, no neon |
| Tint, not paint | Diamond colors as undertones, not full fills |
| Logo moment | Opening shimmer, fades in 20-30s, then calm |
| Diamond-cut corners | 45° chamfered edges on all panels/cards |
| Information density | Every pixel earns its place |
| Minimal glow | Just enough to make glassmorphism work |

### Color Palette

#### Backgrounds

```
BG_PRIMARY:     #121218   Deep space black
BG_SECONDARY:   #1A1A22   Panel background
BG_SURFACE:     #22222C   Card/elevated surface
BG_HOVER:       #2A2A36   Hovered element
```

#### Diamond Category Colors (Subtle Tints)

These appear as icon colors and very subtle background washes:

| Category | Diamond Name | Color | Tinted BG |
| ----------- | ------------ | --------- | --------- |
| Recover | Sapphire | `#2D7DD2` | `#1E2028` |
| Investigate | Amethyst | `#9B59B6` | `#221E24` |
| Diagnose | Topaz | `#E6A817` | `#24221E` |
| Clean | Emerald | `#27AE60` | `#1E2420` |
| Custom | Clear | `#BDC3C7` | `#202022` |

#### File Type Diamond Colors

| Type | Diamond | Color |
| --------- | -------- | --------- |
| Images | Rose | `#E84393` |
| Documents | Jade | `#00B894` |
| Audio | Canary | `#FDCB6E` |
| Video | Aqua | `#00CEC9` |
| Code | Ruby | `#D63031` |
| Archives | Lapis | `#6C5CE7` |
| Error | Obsidian | `#2D3436` |

#### Text

```
TEXT_PRIMARY:    #E8E8EE   Main text
TEXT_SECONDARY:  #8888A0   Subtitles, descriptions
TEXT_MUTED:      #555566   Disabled, hints
```

### Corner Treatment

All panels/cards use 45° chamfered corners (diamond-cut):

- Small elements: 4px cut
- Cards: 8px cut
- Panels: 12px cut
- Active/focused elements get a tiny filled diamond ◆ at the cut point

### Typography

- Headers: Inter Bold or similar geometric sans
- Body: Inter Regular
- Monospace (for paths, hashes): JetBrains Mono

### The Drill Animation

When actively working (scanning, recovering):

- Diamond icon rotates like a drill bit (canvas widget)
- Rotation speed scales with throughput
- Conveys "I'm crushing it" energy
- Stops when operation completes

______________________________________________________________________

## Real-Time Event Architecture

### Channel-Based Event Streaming

```rust
/// Events emitted by the engine during operations
pub enum DrillEvent {
    // Scan events
    FileFound { path: PathBuf, size: u64, file_type: FileType },
    SectorRead { offset: u64, status: SectorStatus },
    ScanProgress { scanned: usize, total_estimate: usize },
    ScanComplete { stats: ScanStats },

    // Recovery events
    FileQueued { path: PathBuf },
    FileCopying { path: PathBuf, bytes_done: u64, bytes_total: u64 },
    FileVerifying { path: PathBuf },
    FileRecovered { path: PathBuf, hash: String, duration: Duration },
    FileFailed { path: PathBuf, error: String },

    // Dedup events
    DuplicateFound { group: DuplicateGroup },
    DedupComplete { report: DedupReport },

    // General
    Error { message: String, recoverable: bool },
    OperationComplete { summary: OperationSummary },
}
```

### GUI Integration

```
DrillEngine (tokio async)
    │
    └──→ mpsc::Sender<DrillEvent>
              │
              └──→ iced::Subscription<Message>
                        │
                        └──→ App::update() → re-render at 60fps
```

The engine emits granular per-file events. The GUI subscribes to the channel
via iced's `Subscription` mechanism. Each event triggers an `update()` call
that modifies the app state. The GUI renders at whatever rate iced allows.

### Operation Handle

```rust
pub struct OperationHandle {
    /// User pressed Stop
    cancel: CancellationToken,
    /// Live events to UI
    progress: mpsc::Sender<DrillEvent>,
}
```

The cancel token is checked between file operations. When triggered:

1. Finish current file (don't corrupt mid-write)
1. Write checkpoint
1. Emit `OperationComplete` with partial summary
1. UI shows "Stopped at file X of Y — Resume later?"

______________________________________________________________________

## Error Handling Policy

### Two Policies

```rust
pub enum ErrorPolicy {
    /// Read-only: log error, mark file as failed, continue to next
    ForwardOnError,
    /// Destructive: halt immediately with detailed explanation
    HaltOnError,
}
```

### Policy by Mission

| Mission | Policy | Rationale |
| ------------- | -------------- | ------------------------------------------------ |
| Recover | ForwardOnError | Source is read-only; recover everything possible |
| Investigate | ForwardOnError | Read-only; document everything findable |
| Diagnose | ForwardOnError | Errors ARE the data |
| Clean (purge) | HaltOnError | Destructive; stop on unexpected errors |
| Custom | User selects | Power user decides |

### Forward-On-Error Flow

```
Error occurs during read-only operation
  → Log error with full context
  → Mark file as FAILED with reason
  → Continue to next file
  → At end: display error summary panel
  → Error log saved to file
```

### Halt-On-Error Flow

```
Error occurs during destructive operation
  → STOP immediately
  → Show detailed explanation:
    • What operation was attempted
    • What failed and why
    • What completed successfully
    • What remains unprocessed
    • What the user should do next
  → Write checkpoint for potential resume
```

### User Override (Emergency Halt)

- **CLI:** Ctrl+C → graceful shutdown via `tokio::signal`
- **GUI:** Red ⏹ STOP button (always visible during ops) + Escape key
- Both: finish current file, write checkpoint, show partial summary

______________________________________________________________________

## Shared Module Rule

### Architecture

```
┌─────────────────────────────────────────────────┐
│           diamond_drill (library)                │
│                                                   │
│  ┌─────────┐ ┌────────┐ ┌─────┐ ┌──────────┐   │
│  │ Scanner │ │Exporter│ │Dedup│ │BadSector │   │
│  └────┬────┘ └───┬────┘ └──┬──┘ └────┬─────┘   │
│       └──────────┼─────────┼─────────┘           │
│            ┌─────▼─────────▼──────┐              │
│            │   Mission System     │              │
│            └──┬────────────────┬──┘              │
│               │                │                  │
│         ┌─────▼──────┐  ┌─────▼──────┐          │
│         │CLI Renderer│  │GUI Renderer│          │
│         └────────────┘  └────────────┘          │
│               │                │                  │
│        diamond-drill    diamond-drill-studio      │
└─────────────────────────────────────────────────┘
```

### The Rule

> Every tool produces **structured output** (data types, not strings).
> CLI renders those types as colored text/tables.
> GUI renders those types as visual widgets.
> The core library never imports any UI framework.

### Example

```rust
// In library (no UI imports):
pub struct DedupReport {
    pub groups: Vec<DuplicateGroup>,
    pub total_wasted: u64,
    pub stats: DedupStats,
}

// CLI side:
impl DedupReport {
    pub fn print_cli(&self) { /* colored text tables */ }
}

// GUI side (in gui/ module, feature-gated):
fn view_dedup_report(report: &DedupReport) -> Element<Message> {
    /* iced widgets */
}
```

______________________________________________________________________

## File Structure

### Target Architecture

```
src/
├── main.rs              # CLI entry point (diamond-drill binary)
├── studio.rs            # GUI entry point (diamond-drill-studio binary)
├── lib.rs               # Library root
├── mission/             # Mission system (NOT feature-gated)
│   ├── mod.rs           # Mission trait, MissionId, ToolModule enums
│   ├── recover.rs       # Recover mission definition
│   ├── investigate.rs   # Investigate mission definition
│   ├── diagnose.rs      # Diagnose mission definition
│   └── clean.rs         # Clean mission definition
├── core/                # (existing) Engine, scanner, index
├── cli/                 # (existing) CLI commands + easy mode
├── tui/                 # (existing) Terminal UI
├── gui/                 # GUI module (feature-gated)
│   ├── mod.rs           # Feature gate + run_studio()
│   ├── app.rs           # Main application state + update
│   ├── theme.rs         # DiamondTheme: custom palette, styles
│   ├── subscriptions.rs # Engine event → iced Subscription bridge
│   ├── views/
│   │   ├── mod.rs
│   │   ├── mission_select.rs  # Opening screen
│   │   ├── source_setup.rs    # Source/dest configuration
│   │   ├── live_scan.rs       # Real-time discovery feed
│   │   ├── triage.rs          # File selection table
│   │   ├── live_recovery.rs   # Pipeline extraction view
│   │   ├── report.rs          # Completion summary
│   │   ├── dedup.rs           # Dedup analysis view
│   │   ├── badsector.rs       # Heatmap canvas
│   │   └── proof.rs           # Chain of custody viewer
│   └── widgets/
│       ├── mod.rs
│       ├── diamond_card.rs    # Diamond-cut card container
│       ├── stat_counter.rs    # Animated stat counter
│       ├── pipeline_strip.rs  # 4-stage pipeline visualization
│       ├── file_row.rs        # File table row with status
│       ├── disk_map.rs        # Canvas-based disk visualization
│       └── drill_spinner.rs   # Rotating diamond animation
├── dedup/               # (existing)
├── export/              # (existing)
├── proof/               # (existing)
├── badsector/           # (existing)
├── swarm/               # (existing)
├── checkpoint/          # (existing)
├── preview/             # (existing)
├── config.rs            # (existing)
├── readonly.rs          # (existing)
└── spinner.rs           # (existing)
```

______________________________________________________________________

## Implementation Phases

### Phase 0: Housekeeping (This Session)

- [x] `cargo fix` — unused import
- [x] README fixes (tables, test count)
- [x] Write DESIGN.md (this document)
- [ ] Git cleanup (commit, prune branches)
- [ ] Port `tour` command from standalone repo

### Phase 1: GUI Foundation (Next Session)

- [ ] Create `src/studio.rs` entry point
- [ ] Add second `[[bin]]` to Cargo.toml
- [ ] Upgrade iced 0.12 → 0.13
- [ ] Create `mission/` module with types (not feature-gated)
- [ ] Create `gui/theme.rs` — DiamondTheme
- [ ] Create `gui/views/mission_select.rs` — opening screen
- [ ] Create `gui/views/source_setup.rs` with rfd
- [ ] Wire up mission select → source setup → basic scan

### Phase 2: Live Recovery Experience

- [ ] Implement `DrillEvent` enum and channel system
- [ ] Create `gui/subscriptions.rs` — engine → GUI bridge
- [ ] Create `gui/views/live_scan.rs` — discovery feed
- [ ] Create `gui/views/live_recovery.rs` — pipeline view
- [ ] Create `gui/widgets/pipeline_strip.rs`
- [ ] Create `gui/widgets/stat_counter.rs`
- [ ] Create `gui/views/report.rs` — completion summary

### Phase 3: Visual Polish & Advanced Views

- [ ] Create `gui/widgets/disk_map.rs` — canvas visualization
- [ ] Create `gui/widgets/drill_spinner.rs` — rotating diamond
- [ ] Create `gui/widgets/diamond_card.rs` — chamfered containers
- [ ] Virtual scrolling for file table (100K+ files)
- [ ] Create `gui/views/dedup.rs`
- [ ] Create `gui/views/badsector.rs`
- [ ] Create `gui/views/proof.rs`

### Phase 4: Integration & Launch

- [ ] CLI mission subcommands (`diamond-drill recover`, etc.)
- [ ] State persistence (`~/.ddrill/gui_state.json`)
- [ ] Keyboard shortcut map for GUI
- [ ] Port `tour` command to work with missions
- [ ] Comprehensive testing
- [ ] Binary size optimization
- [ ] Release build + README update

______________________________________________________________________

## Standalone Repo Borrowing

Scanned `X:\Github Repos\diamond-drill` and compared with CaseStar-Turbo version.

### Worth Borrowing

| Feature | Source | Notes |
| ----------------- | --------------------------------- | -------------------------------------------------- |
| `tour` command | `src/cli/commands/tour.rs` | Auto-advancing guided walkthrough, great for demos |
| USB deploy script | `scripts/setup-usb-drill-pro.ps1` | Portable forensic field deployment |

### Already Superior in CaseStar-Turbo

Everything else: dedup, proof, bad sector, swarm, checkpoint/resume, TUI (5 tabs vs 3),
richer CLI args, fuzzy search, GPU support. The standalone is a strict subset.

### VERSION_COMPARISON.md

The standalone repo already has a detailed comparison doc at
`docs/VERSION_COMPARISON.md` (276 lines) confirming CaseStar-Turbo is strictly superior.

______________________________________________________________________

## Open Questions

1. **iced 0.13 breaking changes** — Need to audit exact API changes before upgrading
1. **Canvas widget** — Required for disk map and drill animation; verify available in iced features
1. **Font loading** — Need Inter + JetBrains Mono embedded or loaded at runtime
1. **Accessibility** — Tab order and screen reader support in iced (limited, may need future work)
1. **Voice control** — Noted as potential future feature; not in scope for Phase 1-4
1. **Cross-platform testing** — iced works on Win/Mac/Linux but rfd dialogs may differ
1. **GPU rendering** — iced uses wgpu; may conflict with candle GPU features?

______________________________________________________________________

_This document is the single source of truth for Diamond Drill GUI architecture.
Update it as decisions change. Code should match what's described here._
