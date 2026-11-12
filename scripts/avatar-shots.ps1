# Renders a scripted preview of the avatar to contact-sheet PNGs, without the app or a window.
#
#   .\scripts\avatar-shots.ps1                                   # the full tour
#   .\scripts\avatar-shots.ps1 -Script 'happy@ground@0,0@2@4'    # one step
#   .\scripts\avatar-shots.ps1 -Script 'idle@ground@150,0@3@8' -Framing full -Size 390x555
#   .\scripts\avatar-shots.ps1 -Model C:\models\other.vrm        # a custom VRM
#
# A step is emotion@mode@vx,vy@seconds@shots[@persona]; see aria-avatar/src/demo.rs.
# Pictures land in target\shots (inside the repo, so a sandboxed process may write there).
param(
    [string]$Script = '',
    [string]$Framing = 'full',
    [string]$Size = '260x370',
    [string]$Lighting = 'day',
    [string]$Model = '',
    [string]$Out = '',
    [switch]$Release,
    [int]$TimeoutSeconds = 300
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$sub = if ($Release) { 'release' } else { 'debug' }
$exe = Join-Path $root "target\$sub\aria-avatar.exe"
if (-not (Test-Path $exe)) { throw "missing $exe (run cargo build first)" }
if (-not $Out) { $Out = Join-Path $root 'target\shots' }
Remove-Item -Recurse -Force $Out -ErrorAction SilentlyContinue

$env:ARIA_AVATAR_SHOTS = $Out
$env:ARIA_AVATAR_SCRIPT = $Script
$env:ARIA_AVATAR_FRAMING = $Framing
$env:ARIA_AVATAR_SIZE = $Size
$env:ARIA_AVATAR_LIGHTING = $Lighting
$log = Join-Path $root 'target\shots.log'
$arguments = if ($Model) { @('--model', $Model) } else { @() }
try {
    $start = @{
        FilePath               = $exe
        WorkingDirectory       = $root
        PassThru               = $true
        WindowStyle            = 'Hidden'
        RedirectStandardError  = $log
        RedirectStandardOutput = "$log.out"
    }
    if ($arguments.Count) { $start.ArgumentList = $arguments }
    $process = Start-Process @start
    if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
        Stop-Process -Id $process.Id -Force
        Write-Warning "timed out after $TimeoutSeconds s"
    }
}
finally {
    foreach ($name in 'SHOTS', 'SCRIPT', 'FRAMING', 'SIZE', 'LIGHTING') {
        Remove-Item "Env:ARIA_AVATAR_$name" -ErrorAction SilentlyContinue
    }
}
Get-Content $log, "$log.out" -ErrorAction SilentlyContinue |
    Select-String -Pattern 'ERROR|WARN|panic|rig resolved|demo: cannot' |
    Select-Object -First 20 | ForEach-Object { $_.Line }
Get-ChildItem $Out -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
