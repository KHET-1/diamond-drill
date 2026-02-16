# Minimal copy: CaseStar-Turbo diamond-drill to D:\DiamondDrillPro (your one-liner, expanded)
if (-not (Test-Path "D:\")) { New-Item -ItemType Directory -Path "D:\" -Force }
New-Item -ItemType Directory -Path "D:\DiamondDrillPro" -Force -ErrorAction SilentlyContinue
Copy-Item -Path "X:\Github Repos\CaseStar-Turbo\diamond-drill\*" -Destination "D:\DiamondDrillPro" -Recurse -Force
Write-Host "Done. D:\DiamondDrillPro is ready. Build: cd D:\DiamondDrillPro ; cargo build --release"
