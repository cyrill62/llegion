<#
.SYNOPSIS
    Regenerates the README screenshots (docs/screenshots/*.png) from a debug build,
    against the mock Ollama (examples/mock_ollama.rs) so no real data appears.

.DESCRIPTION
    Windows counterpart of scripts/screenshots.sh. Only needs Rust.

.EXAMPLE
    .\scripts\screenshots.ps1                      # writes docs/screenshots
    .\scripts\screenshots.ps1 C:\temp\shots
    $env:LLEGION_URL = 'http://127.0.0.1:11434'; .\scripts\screenshots.ps1   # use a real server instead
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string] $Out = 'docs/screenshots'
)

$ErrorActionPreference = 'Stop'
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Port = 11500
$AppExe = Join-Path $RepoRoot 'target\debug\llegion.exe'
$MockExe = Join-Path $RepoRoot 'target\debug\examples\mock_ollama.exe'
$mockOut = Join-Path $env:TEMP "llegion-mock-$PID.out.log"
$mockErr = Join-Path $env:TEMP "llegion-mock-$PID.err.log"
$hadUrl = Test-Path Env:LLEGION_URL
$mock = $null

Push-Location $RepoRoot
try {
    cargo build --bin llegion --example mock_ollama
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }

    if (-not $hadUrl) {
        $mock = Start-Process -FilePath $MockExe -ArgumentList @([string]$Port) -PassThru -WindowStyle Hidden `
            -RedirectStandardOutput $mockOut -RedirectStandardError $mockErr
        $ready = $false
        for ($i = 0; $i -lt 50; $i++) {
            if ($mock.HasExited) {
                $details = if (Test-Path $mockErr) { Get-Content $mockErr -Raw } else { '' }
                throw "Mock server exited before it started (port $Port in use?). $details"
            }
            if (Select-String -Path $mockOut -SimpleMatch "listening on 127.0.0.1:$Port" -Quiet -ErrorAction SilentlyContinue) {
                $ready = $true
                break
            }
            Start-Sleep -Milliseconds 100
        }
        if (-not $ready) { throw 'Mock server did not start listening.' }
        $env:LLEGION_URL = "http://127.0.0.1:$Port"
    }

    $env:LLEGION_SCREENSHOTS = $Out
    & $AppExe
    if ($LASTEXITCODE -ne 0) { throw "llegion exited with $LASTEXITCODE" }
    $shots = @(Get-ChildItem -Path (Join-Path $Out '*.png') -File)
    if ($shots.Count -eq 0) { throw "No PNG files were written to $Out." }
    $shots | Format-Table Name, Length, LastWriteTime
}
finally {
    if ($null -ne $mock -and -not $mock.HasExited) {
        Stop-Process -Id $mock.Id -Force
        $mock.WaitForExit()
    }
    Remove-Item Env:LLEGION_SCREENSHOTS -ErrorAction SilentlyContinue
    if (-not $hadUrl) { Remove-Item Env:LLEGION_URL -ErrorAction SilentlyContinue }
    Remove-Item $mockOut, $mockErr -Force -ErrorAction SilentlyContinue
    Pop-Location
}
