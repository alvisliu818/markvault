# Build release NSIS installer for markvault (Windows).
# Usage: pwsh scripts\build-release.ps1 [-SkipInstaller]
# Steps:
#   1. cargo build --release (kernel)
#   2. copy kernel to app/src-tauri/binaries/markvault-kernel-<target-triple>.exe (sidecar)
#   3. npm run build (frontend -> dist)
#   4. npx tauri build (NSIS installer + exe)

param(
    [switch]$SkipInstaller
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$triple = rustc -Vv | Select-String "host:" | ForEach-Object { ($_ -split ":\s+")[1].Trim() }
Write-Host "target triple: $triple"

Write-Host "== 1/4 kernel release build =="
cargo build --release --manifest-path crates/kernel/Cargo.toml
if ($LASTEXITCODE -ne 0) { throw "kernel build failed" }

Write-Host "== 2/4 stage kernel sidecar =="
$sidecarDir = Join-Path $root "app\src-tauri\binaries"
New-Item -ItemType Directory -Force -Path $sidecarDir | Out-Null
$src = Join-Path $root "target\release\markvault-kernel.exe"
$dst = Join-Path $sidecarDir "markvault-kernel-$triple.exe"
Copy-Item $src $dst -Force
Write-Host "sidecar: $dst"

Write-Host "== 3/4 frontend build =="
Push-Location (Join-Path $root "frontend")
npm run build
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "frontend build failed" }
Pop-Location

if ($SkipInstaller) {
    Write-Host "== done (no installer) =="
    exit 0
}

Write-Host "== 4/4 tauri build (NSIS) =="
Push-Location (Join-Path $root "app\src-tauri")
npx --yes @tauri-apps/cli@latest build
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "tauri build failed" }
Pop-Location

$bundleDir = Join-Path $root "app\src-tauri\target\release\bundle\nsis"
Write-Host "== installer output: $bundleDir =="
Get-ChildItem $bundleDir -ErrorAction SilentlyContinue | Select-Object Name, Length
