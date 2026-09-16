# Native MSVC packaging with bundled source-built media tools.
param([switch]$SkipBuild)
$ErrorActionPreference = "Stop"
Set-Location (Join-Path $PSScriptRoot "..")
if (-not $IsWindows -and $env:OS -ne "Windows_NT") { throw "Run on Windows in an x64 Visual Studio developer shell." }
$arguments = @("scripts/package-desktop.py")
if ($SkipBuild) { $arguments += "--skip-build" }
python @arguments
if ($LASTEXITCODE -ne 0) { throw "Windows package build or verification failed." }
