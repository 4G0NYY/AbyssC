<#
.SYNOPSIS
    Build the AbyssC release binaries and compile the Windows installer.

.DESCRIPTION
    1. Reads the single-source-of-truth version from the workspace Cargo.toml.
    2. Builds abyssc.exe + abyssc-gui.exe in release mode.
    3. Invokes the Inno Setup compiler (ISCC), injecting that version.

    The finished installer lands in installer\dist\AbyssC-<version>-Setup.exe.

.NOTES
    Requires Inno Setup 6 (https://jrsoftware.org/isdl.php). If ISCC.exe is not
    on PATH, the script looks in the usual Program Files locations.
#>
[CmdletBinding()]
param(
    # Skip `cargo build --release` (use existing binaries).
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot   # repo root (installer\ is one level down)

# --- 1. Version from Cargo.toml (SSOT) -------------------------------------
$cargoToml = Join-Path $root 'Cargo.toml'
$versionLine = Select-String -Path $cargoToml -Pattern '^\s*version\s*=\s*"([^"]+)"' |
    Select-Object -First 1
if (-not $versionLine) { throw "Could not find a version in $cargoToml" }
$version = $versionLine.Matches[0].Groups[1].Value
Write-Host "AbyssC version: $version" -ForegroundColor Cyan

# --- 2. Build the release binaries -----------------------------------------
if (-not $NoBuild) {
    Write-Host "Building release binaries..." -ForegroundColor Cyan
    Push-Location $root
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

foreach ($bin in @('abyssc.exe', 'abyssc-gui.exe')) {
    $p = Join-Path $root "target\release\$bin"
    if (-not (Test-Path $p)) { throw "Missing $p - build first (or drop -NoBuild)." }
}

# --- 3. Locate the Inno Setup compiler -------------------------------------
$iscc = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
if (-not $iscc) {
    foreach ($candidate in @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
        "${env:LOCALAPPDATA}\Programs\Inno Setup 6\ISCC.exe"
    )) {
        if ($candidate -and (Test-Path $candidate)) { $iscc = $candidate; break }
    }
}
if (-not $iscc) {
    throw "ISCC.exe (Inno Setup 6) not found. Install it from https://jrsoftware.org/isdl.php"
}
Write-Host "Using compiler: $iscc" -ForegroundColor Cyan

# --- 4. Compile the installer ----------------------------------------------
$iss = Join-Path $PSScriptRoot 'abyssc.iss'
& $iscc "/DAppVersion=$version" $iss
if ($LASTEXITCODE -ne 0) { throw "ISCC failed (exit $LASTEXITCODE)" }

$out = Join-Path $PSScriptRoot "dist\AbyssC-$version-Setup.exe"
Write-Host "`nInstaller built: $out" -ForegroundColor Green
