# Build release exe and package it into dist\QuickExit-v<version>.zip
# Usage:  powershell -ExecutionPolicy Bypass -File .\build.ps1
# cargo prints progress to stderr; with 'Stop' PowerShell 5.1 would treat that as an error,
# so native calls run under 'Continue' and are checked via $LASTEXITCODE.
$ErrorActionPreference = 'Continue'
Set-Location $PSScriptRoot

$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path $cargo)) { $cargo = 'cargo' }

& $cargo test
if ($LASTEXITCODE -ne 0) { throw 'tests failed' }
& $cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'build failed' }
$ErrorActionPreference = 'Stop'

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
$targetDir = (& $cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
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
