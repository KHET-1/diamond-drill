# Diamond Drill

<p align="center">
  <strong>Ultra-fast forensic disk recovery — built with Rust, powered by purpose.</strong>
</p>

<p align="center">
  <a href="#benchmarks">Benchmarks</a> •
  <a href="#features">Features</a> •
  <a href="#quick-start">Quick Start</a> •
  <a href="#architecture">Architecture</a> •
  <a href="#swarm-agent-orchestration">Swarm Agents</a> •
  <a href="#license">License</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/language-Rust-orange?style=flat-square" alt="Rust">
  <img src="https://img.shields.io/badge/tests-109_passing-brightgreen?style=flat-square" alt="Tests">
  <img src="https://img.shields.io/badge/clippy-zero_warnings-brightgreen?style=flat-square" alt="Clippy">
  <img src="https://img.shields.io/badge/binary-3.3_MB-blue?style=flat-square" alt="Binary Size">
  <img src="https://img.shields.io/badge/license-MIT-purple?style=flat-square" alt="License">
</p>

---

Diamond Drill is an **extreme-speed offline disk recovery tool** for forensic
investigators, public defenders, and anyone who needs to index, preview,
deduplicate, and safely export files from disk images — without ever modifying
the source.

Built for the [CaseStar](https://github.com/your-org/CaseStar-Turbo) forensic
intelligence platform. Part of the **Soft Justice** initiative: high-performance
tools for public defenders, 100% local, 100% private, 100% free.

## Benchmarks

Real numbers. No asterisks.

| Operation                   | Throughput      | Notes                            |
| --------------------------- | --------------- | -------------------------------- |
| **Blake3 hashing (1 MB)**   | **1.43 GiB/s**  | Streaming, 8 KB buffer           |
| **Blake3 hashing (100 KB)** | 410 MiB/s       | Single-threaded                  |
| **Blake3 hashing (10 KB)**  | 77 MiB/s        | I/O bound at small sizes         |
| **File entry creation**     | 39 µs/file      | Path + metadata + type detection |
| **Dedup scan**              | 100K+ files/min | Parallel via rayon               |
| **Index resume**            | Instant         | Checkpoint-based, zero re-scan   |

> Measured on Windows 11 / Ryzen / NVMe. Run `cargo bench` to reproduce.

## Features

### 🔍 Parallel Indexing

Multi-threaded file scanning with rayon. Indexes 100K+ files per minute with
automatic checkpoint/resume — crash mid-scan, pick up exactly where you left
off.

### 🖼️ Progressive Thumbnails

Two-pass thumbnail generation: instant 64×64 previews, background 512×512
upscale. EXIF-aware rotation (all 8 orientations). Batch parallel generation via
rayon.

### 🔐 Read-Only Safety

**Never modifies source data.** Every operation is read-only against the source.
Exports go to a separate destination with Blake3 verification.

### Content-Addressable Deduplication

- **Exact dedup**: Blake3 content hashing with partial-hash optimization for
  large files (>8 MB)
- **Fuzzy dedup**: Levenshtein + Jaccard similarity for renamed copies
- **Smart master selection**: Keeps cleanest filename, most recent, or
  shallowest path
- **Dry-run mode**: Preview before any destructive action

### Bad Sector Recovery

Block-level reads with exponential backoff retry. Zero-fills unrecoverable
sectors. Generates heatmap visualizations and detailed JSON/human reports.

### Swarm Agent Orchestration

7-agent pipeline for automated processing:

| Agent             | Role                                         |
| ----------------- | -------------------------------------------- |
| **Scan Agent**    | Parallel file discovery with walkdir         |
| **Chunk Agent**   | Dynamic chunking (text/code/image/PDF)       |
| **Embed Agent**   | Vector embeddings (LM Studio / Ollama / GPU) |
| **Verify Agent**  | Blake3 hash verification on all outputs      |
| **Export Agent**  | Safe copy with bad-sector handling           |
| **Heal Agent**    | Auto-recovery from agent failures (3x retry) |
| **Summary Agent** | Pipeline statistics and reporting            |

### 🖥️ Terminal UI

Full ratatui-powered TUI with:

- Vim keybindings (j/k/g/G)
- File tree with type-colored icons
- Tab switching (Files / Search / Export / Dedup / Bad Sectors)
- Fuzzy search with live filtering
- Multi-select for batch export

### 📦 Export & Proof

- Verified copy with Blake3 checksums
- Proof manifest (JSON) with machine info, timestamps, file inventory
- Chain-of-custody metadata for legal admissibility

## Quick Start

```bash
# Clone and build
git clone https://github.com/your-org/CaseStar-Turbo.git
cd CaseStar-Turbo/diamond-drill
cargo build --release

# Easy Mode - Grandma-friendly guided recovery
./target/release/diamond-drill --easy
./target/release/diamond-drill -E  # shorthand

# Index a disk image or directory
./target/release/diamond-drill index /path/to/source

# Browse with TUI
./target/release/diamond-drill tui /path/to/source

# Interactive guided workflow
./target/release/diamond-drill interactive

# Run deduplication analysis
./target/release/diamond-drill dedup /path/to/source --report human

# Export selected files with verification
./target/release/diamond-drill export /path/to/source --dest ./recovered --verify

# Run 5-agent swarm pipeline for parallel document processing
./target/release/diamond-drill swarm ./documents --output manifest.json
```

### Easy Mode 🎯

For non-technical users, Easy Mode provides a step-by-step wizard:

```text
Step 1: What happened?
  - I accidentally deleted files
  - My drive is corrupted
  - I lost photos from a camera/phone
  - Scan everything

Step 2: Where are your files?
  - Browse folders
  - Select connected drives

Step 3: What files to recover?
  - Photos & Images
  - Videos
  - Documents
  - Everything

Step 4: Where to save?
Step 5: Recover with progress bar
Step 6: Verification complete!
```

## Architecture

```text
diamond-drill/
├── src/
│   ├── core/           # DrillEngine, FileIndex, Scanner
│   │   ├── engine.rs   # Central orchestrator (index, search, export)
│   │   ├── index.rs    # File index with search + persistence
│   │   └── scanner.rs  # Parallel file discovery
│   ├── badsector/      # Block-level I/O with retry + heatmaps
│   ├── checkpoint/     # Crash-safe index resume
│   ├── cli/            # Clap-powered CLI + interactive wizard
│   │   ├── mod.rs      # CLI definitions (clap)
│   │   ├── easy_mode.rs # Grandma-friendly step-by-step wizard
│   │   └── interactive.rs # Interactive shell
│   ├── config.rs       # TOML config (~/.ddrill/config.toml)
│   ├── spinner.rs      # Diamond spinner + pulsing progress bars
│   ├── dedup/          # Exact + fuzzy deduplication engine
│   ├── export/         # Verified file copy with proof manifests
│   ├── preview/        # Progressive thumbnails + EXIF rotation
│   ├── proof/          # Chain-of-custody proof generation
│   ├── swarm/          # 5-agent pipeline orchestration
│   │   ├── agents.rs   # Agent implementations (Scan, Chunk, Embed, Heal, Export)
│   │   ├── orchestrator.rs  # Pipeline coordination with crossbeam channels
│   │   ├── heal.rs     # Auto-recovery + retry logic (3-strike rule)
│   │   ├── chunker.rs  # Media-aware chunking (text/code/PDF/image)
│   │   ├── embedder.rs # GPU/CPU vector embeddings with fallback
│   │   ├── searcher.rs # Hybrid keyword + vector semantic search
│   │   └── session.rs  # State persistence + resume
│   ├── tui/            # Full terminal UI (ratatui)
│   │   ├── app.rs      # State machine + vim keybindings
│   │   ├── ui.rs       # Rendering (tabs, file tree, details)
│   │   └── file_tree.rs # Navigable tree with filtering
│   ├── lib.rs          # Public API exports
│   └── main.rs         # CLI entry point (<200 lines)
├── benches/            # Criterion benchmarks (6 suites)
├── tests/              # Integration tests
└── Cargo.toml
```

### Configuration

Diamond Drill reads config from `~/.config/diamond-drill/config.toml`:

```toml
[general]
theme = "auto"           # dark, light, auto
enforce_readonly = true  # NEVER modify source data

[export]
preserve_structure = true
create_manifest = true
verify_hash = true

[tui]
vim_mode = true
show_icons = true

[scan]
workers = 0              # 0 = auto-detect CPU count
skip_hidden = true
```

### LM Studio Integration

Diamond Drill auto-detects local embedding servers for semantic search:

```text
Auto-detection order:
1. LM Studio (localhost:1234) — Best GPU performance
2. Ollama (localhost:11434)   — Good alternative
3. Blake3 pseudo-embeddings   — Fast CPU fallback
```

**Quick Setup with LM Studio 4.0:**

```bash
# 1. Install LM Studio from https://lmstudio.ai
# 2. Download an embedding model (e.g., nomic-embed-text, bge-small)
# 3. Start the local server (bottom right → Start Server)
# 4. Run Diamond Drill — it auto-detects!

./target/release/diamond-drill swarm ./documents --output manifest.json
# Output: "Auto-detected LM Studio at http://localhost:1234/v1 with model: nomic-embed-text"
```

**Performance (RTX 4080 Laptop):**

| Model                  | Dimension | Speed       |
| ---------------------- | --------- | ----------- |
| nomic-embed-text       | 768       | ~1000 emb/s |
| bge-small-en-v1.5      | 384       | ~2000 emb/s |
| text-embedding-3-small | 1536      | ~500 emb/s  |

## Quality Gates

Every commit passes:

```bash
cargo clippy -- -D warnings   # Zero warnings
cargo fmt                      # Consistent formatting
cargo test -- --test-threads=1 # 109 tests, single-threaded for WASAPI
cargo bench                    # 6 benchmark suites
cargo build --release          # 3.3 MB optimized binary
```

## Development

```bash
# Run all quality gates
cargo fmt && cargo clippy -- -D warnings && cargo test -- --test-threads=1

# Run benchmarks
cargo bench

# Build release binary
cargo build --release

# Run with verbose logging
RUST_LOG=debug cargo run -- index /path/to/source
```

## The Mission: Soft Justice 💎

Diamond Drill exists because **public defenders deserve the same tools as
prosecutors.**

When a case involves 30,000+ pages of discovery — medical records, police
reports, surveillance footage — and the defense has one overworked attorney with
no budget for forensic software, that's not justice. That's a system designed to
lose.

Diamond Drill is the engine inside
[CaseStar](https://github.com/your-org/CaseStar-Turbo), a forensic intelligence
platform built specifically for:

- **Public defenders** drowning in discovery
- **Innocence projects** (Arizona Innocence Project, ACLU AZ)
- **Legal aid organizations** with zero tech budget
- **Anyone** who believes access to justice shouldn't depend on access to
  capital

100% local. 100% private. 100% free. No cloud. No telemetry. No compromise.

> _"High-performance visualization for public defenders. Diamond hands,
> unbreakable execution."_

## License

MIT — because justice tools should be free.

---

<p align="center">
  <strong>Built with 🦀 Rust by the CaseStar team</strong><br>
  <em>Forged in AI collaboration. Tempered by purpose.</em>
</p>
