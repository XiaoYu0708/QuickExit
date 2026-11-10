# Build release exe and package it into dist\QuickExit-v<version>.zip
# Usage:  powershell -ExecutionPolicy Bypass -File .\build.ps1
# cargo prints progress to stderr; with 'Stop' PowerShell 5.1 would treat that as an error,
# so native calls run under 'Continue' and are checked via $LASTEXITCODE.
$ErrorActionPreference = 'Continue'
Set-Location $PSScriptRoot

$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path $cargo)) { $cargo = 'cargo' }

# The mingw ld bundled with rustup cannot handle non-ASCII paths. Pick the build output dir:
#   1. $env:CARGO_TARGET_DIR if set
#   2. target-dir from a local (untracked) .cargo\config.toml
#   3. if the project path is pure ASCII: .\target
#   4. otherwise an ASCII folder under %LOCALAPPDATA% (or the system drive)
function Test-Ascii([string]$s) { return $s -match '^[\x00-\x7F]*$' }
$targetDir = $env:CARGO_TARGET_DIR
if (-not $targetDir -and (Test-Path .cargo\config.toml)) {
    $m = Select-String -Path .cargo\config.toml -Pattern '^target-dir\s*=\s*"(.+)"'
    if ($m) { $targetDir = $m.Matches[0].Groups[1].Value }
}
if (-not $targetDir) {
    if (Test-Ascii $PSScriptRoot) {
        $targetDir = Join-Path $PSScriptRoot 'target'
    } else {
        $base = $env:LOCALAPPDATA
        if (-not (Test-Ascii $base)) { $base = "$env:SystemDrive\" }
        $targetDir = Join-Path $base 'quick-exit-target'
        $env:CARGO_TARGET_DIR = $targetDir
    }
}

# Strip local paths (user name, project folder) from the exe's embedded panic/source locations
$remap = @()
foreach ($p in @($env:USERPROFILE, $targetDir, $PSScriptRoot)) {
    if ($p) {
        $remap += "--remap-path-prefix=$p=~"
        $remap += "--remap-path-prefix=$($p -replace '\\', '/')=~"
    }
}
$env:RUSTFLAGS = (@($env:RUSTFLAGS) + $remap | Where-Object { $_ }) -join ' '

& $cargo test
if ($LASTEXITCODE -ne 0) { throw 'tests failed' }
& $cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'build failed' }
$ErrorActionPreference = 'Stop'

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
$exe = Join-Path $targetDir 'release\QuickExit.exe'

$name = "QuickExit-v$version"
$stage = Join-Path $PSScriptRoot "dist\$name"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage | Out-Null
Copy-Item $exe, README.md, CHANGELOG.md $stage

$zip = Join-Path $PSScriptRoot "dist\$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path "$stage\*" -DestinationPath $zip

Write-Host "Packaged: $zip"
