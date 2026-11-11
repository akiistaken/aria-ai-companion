# One-time developer machine setup for faster Rust builds. Run from an ELEVATED PowerShell:
#
#   powershell -ExecutionPolicy Bypass -File .\scripts\dev-setup.ps1
#
# Windows Defender scans every object file, rlib and executable the compiler writes. On a Bevy +
# Slint workspace that is tens of thousands of files per clean build, and real-time scanning
# commonly adds 20-40% to build times. This excludes only build output and toolchain folders,
# never source or downloads.

$ErrorActionPreference = 'Stop'

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this script from an elevated (Administrator) PowerShell."
}

$root = Split-Path -Parent $PSScriptRoot
$paths = @(
    (Join-Path $root 'target'),            # build output
    (Join-Path $root '.tmp'),              # build-script temp (see .cargo/config.toml)
    (Join-Path $env:USERPROFILE '.cargo'), # registry sources and cached crates
    (Join-Path $env:USERPROFILE '.rustup') # toolchains (rustc, rust-lld)
)
$processes = @('cargo.exe', 'rustc.exe', 'rust-lld.exe', 'ld.lld.exe', 'gcc.exe', 'ld.exe')

foreach ($p in $paths) {
    Add-MpPreference -ExclusionPath $p
    Write-Host "excluded path    $p"
}
foreach ($p in $processes) {
    Add-MpPreference -ExclusionProcess $p
    Write-Host "excluded process $p"
}
Write-Host "Done. Undo with Remove-MpPreference -ExclusionPath/-ExclusionProcess for the same values."
