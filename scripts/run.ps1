# Build and run Aria.
#
#   .\scripts\run.ps1            # debug build, then launch
#   .\scripts\run.ps1 -Release   # release build, then launch
#   .\scripts\run.ps1 -BuildOnly # build without launching
param(
    [switch]$Release,
    [switch]$BuildOnly
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $profileArgs = if ($Release) { @('--release') } else { @() }

    Write-Host "Building aria-avatar and aria..." -ForegroundColor Cyan
    cargo build @profileArgs
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }

    if ($BuildOnly) { return }

    $sub = if ($Release) { 'release' } else { 'debug' }
    $exe = Join-Path $root "target\$sub\aria.exe"
    if (-not (Test-Path $exe)) { throw "missing $exe" }

    Write-Host "Launching $exe" -ForegroundColor Cyan
    Start-Process -FilePath $exe -WorkingDirectory $root
}
finally {
    Pop-Location
}
