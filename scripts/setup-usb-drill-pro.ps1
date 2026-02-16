# Setup Diamond Drill Pro + Meta-Learning Swarm for USB (D:\)
# Read-only from sources: copies CaseStar-Turbo diamond-drill and swarm_simulation to D:\DiamondDrillPro.
# Run this on the machine where D:\ is your USB (or staging) drive.

$ErrorActionPreference = "Stop"
$SourceDrill = "X:\Github Repos\CaseStar-Turbo\diamond-drill"
$SourceSwarm = "X:\Github Repos\diamond-drill\swarm_simulation"
$DestRoot = "D:\DiamondDrillPro"

# Ensure D:\ exists and destination exists
if (-not (Test-Path "D:\")) {
    New-Item -ItemType Directory -Path "D:\" -Force
}
New-Item -ItemType Directory -Path $DestRoot -Force -ErrorAction SilentlyContinue

Write-Host "Copying CaseStar-Turbo Diamond Drill (read-only from source) to $DestRoot ..."
Copy-Item -Path "$SourceDrill\*" -Destination $DestRoot -Recurse -Force

Write-Host "Copying Meta-Learning Swarm simulation to $DestRoot\swarm_simulation ..."
New-Item -ItemType Directory -Path "$DestRoot\swarm_simulation" -Force -ErrorAction SilentlyContinue
Copy-Item -Path "$SourceSwarm\*" -Destination "$DestRoot\swarm_simulation" -Recurse -Force

# Write run-from-USB guide
$RunGuide = @"
# Run from USB — Diamond Drill Pro + Meta-Learning Swarm

This folder contains **CaseStar-Turbo Diamond Drill** (recovery) and the **Meta-Learning Swarm** simulation. Use it from this drive or copy to another PC.

---

## Diamond Drill Pro (recovery)

- **Source:** CaseStar-Turbo/diamond-drill (read-only copy).
- **Build** (need Rust installed): `` `cargo build --release` `` 
- **Run:** `` `.\target\release\diamond-drill.exe` `` or `` `-E` `` (easy), `` `tui` `` (full-screen TUI)
- If you have a pre-built `` `diamond-drill.exe` ``, drop it here and run with no install.

---

## Meta-Learning Swarm (simulation)

- **What it is:** Agents use Neural Architecture Search to rewrite their own networks in real time; adversarial chaos scenarios get harder as they improve; a hive mind shares solutions instantly; the run ends with a Singularity Report.
- **Run:** From `` `swarm_simulation` ``: `` `pip install -r requirements.txt` `` then `` `python run_simulation.py --portable` ``, or `` `run.bat` `` / `` `run.sh` ``.

---

## USB use

1. Run this setup script on the PC where the USB is mounted (e.g. as D:\).
2. Build once: `` `cargo build --release` `` (or copy a pre-built exe).
3. On any PC: run `` `diamond-drill.exe` `` from `` `DiamondDrillPro\target\release\` ``.
4. No install needed on the target PC if you use the release exe; for the swarm, Python + numpy are required.

"@
Set-Content -Path "$DestRoot\RUN_FROM_USB.md" -Value $RunGuide -Encoding UTF8

Write-Host "Done. Contents at $DestRoot :"
Get-ChildItem $DestRoot -Name
Write-Host ""
Write-Host "Next: cd $DestRoot ; cargo build --release"
Write-Host "Then run: .\target\release\diamond-drill.exe"
