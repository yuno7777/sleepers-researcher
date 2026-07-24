$ErrorActionPreference = "Stop"

$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$TauriDir = Join-Path $Root "src-tauri"
$OutDir = Join-Path $Root "release"
$ExeSource = Join-Path $TauriDir "target\release\sleepers-researcher.exe"
$ExeDest = Join-Path $OutDir "Sleepers Researcher.exe"

function Require-Command($Name, $InstallHint) {
    $cmd = Get-Command $Name -ErrorAction SilentlyContinue
    if (-not $cmd) {
        throw "Missing required command '$Name'. $InstallHint"
    }
}

Write-Host "Sleepers Researcher release build" -ForegroundColor Cyan
Write-Host "Project: $Root"
Write-Host ""

Require-Command "cargo" "Install Rust from https://rustup.rs/ and make sure Cargo is on PATH."

Push-Location $TauriDir
try {
    Write-Host "Checking Tauri CLI..."
    $tauriOk = $true
    cargo tauri --version *> $null
    if ($LASTEXITCODE -ne 0) { $tauriOk = $false }

    if (-not $tauriOk) {
        Write-Host "Tauri CLI not found. Installing tauri-cli v2..." -ForegroundColor Yellow
        cargo install tauri-cli --version "^2.0.0"
    }

    Write-Host ""
    Write-Host "Building Windows desktop app..." -ForegroundColor Cyan
    cargo tauri build
    if ($LASTEXITCODE -ne 0) {
        throw "cargo tauri build failed."
    }
}
finally {
    Pop-Location
}

if (-not (Test-Path $ExeSource)) {
    throw "Build finished, but the expected executable was not found: $ExeSource"
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Copy-Item -Force $ExeSource $ExeDest

$InstallerDir = Join-Path $TauriDir "target\release\bundle\nsis"
if (Test-Path $InstallerDir) {
    Get-ChildItem $InstallerDir -Filter "*.exe" | ForEach-Object {
        Copy-Item -Force $_.FullName (Join-Path $OutDir $_.Name)
    }
}

Write-Host ""
Write-Host "Release ready" -ForegroundColor Green
Write-Host "Portable exe: $ExeDest"
if (Test-Path $InstallerDir) {
    Write-Host "Installer(s):  $OutDir"
}
Write-Host ""
Write-Host "Before first launch, put your API keys in one of these files:"
Write-Host "  - $Root\.env"
Write-Host "  - beside the exe as release\.env"
Write-Host "  - $env:APPDATA\com.sleepers.researcher\.env"
