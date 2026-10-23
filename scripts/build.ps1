<#
.SYNOPSIS
    Release build + packaging for Windows 11 (x64 / ARM64), and Linux when cross tools are present.

.DESCRIPTION
    Targets (short names or full Rust triples):
      windows-x64  x86_64-pc-windows-msvc       windows-arm64  aarch64-pc-windows-msvc
      linux-x64    x86_64-unknown-linux-gnu     linux-arm64    aarch64-unknown-linux-gnu
      macos-x64    x86_64-apple-darwin          macos-arm64    aarch64-apple-darwin
    Groups: windows, linux, macos, all. Default: both Windows architectures.

    Requirements on Windows:
      - Rust (https://rustup.rs) with the MSVC toolchain
      - Visual Studio 2022 Build Tools, "Desktop development with C++",
        plus "MSVC ARM64 build tools" for the ARM64 target
      - LLVM clang in PATH for ARM64 (the `ring` crate needs it): winget install LLVM.LLVM
    Linux targets need cargo-zigbuild + zig, or cross + Docker. macOS targets must be built on macOS.

.EXAMPLE
    .\scripts\build.ps1                      # windows-x64 + windows-arm64
    .\scripts\build.ps1 windows-arm64
    .\scripts\build.ps1 -NoPackage all
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0, ValueFromRemainingArguments = $true)]
    [string[]] $Target,
    [switch] $NoPackage
)

$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$App = 'ollama-egui'
$Version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"(.+)"' | Select-Object -First 1).Matches.Groups[1].Value
$Dist = 'dist'

function Have([string] $cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

function Resolve-Target([string] $name) {
    switch ($name) {
        'windows-x64'   { 'x86_64-pc-windows-msvc' }
        'windows-arm64' { 'aarch64-pc-windows-msvc' }
        'linux-x64'     { 'x86_64-unknown-linux-gnu' }
        'linux-arm64'   { 'aarch64-unknown-linux-gnu' }
        'macos-x64'     { 'x86_64-apple-darwin' }
        'macos-arm64'   { 'aarch64-apple-darwin' }
        'windows'       { 'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc' }
        'linux'         { 'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu' }
        'macos'         { 'x86_64-apple-darwin', 'aarch64-apple-darwin' }
        'all'           { 'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc', 'x86_64-unknown-linux-gnu',
                          'aarch64-unknown-linux-gnu', 'x86_64-apple-darwin', 'aarch64-apple-darwin' }
        { $_ -match '^\w+-\w+-\w+' } { $name }
        default { throw "unknown target '$name' (see Get-Help .\scripts\build.ps1)" }
    }
}

$targets = @()
if ($Target) { foreach ($t in $Target) { $targets += Resolve-Target $t } }
else { $targets = Resolve-Target 'windows' }

if (-not (Have cargo)) { throw 'cargo not found - install Rust from https://rustup.rs' }

$onWindows = ($env:OS -eq 'Windows_NT')

# Returns the cargo subcommand for a target, or 'skip:<reason>'.
function Get-Builder([string] $t) {
    if ($t -like '*-windows-msvc') {
        if ($onWindows) {
            if ($t -like 'aarch64-*' -and -not (Have clang)) {
                return 'skip:ARM64 needs LLVM clang in PATH (winget install LLVM.LLVM) and the "MSVC ARM64 build tools" VS component'
            }
            return 'build'
        }
        if (Have cargo-xwin) { return 'xwin' }
        return 'skip:build on Windows or install cargo-xwin'
    }
    if ($t -like '*-linux-gnu') {
        if ((Have cargo-zigbuild) -and (Have zig)) { return 'zigbuild' }
        if (Have cross) { return 'cross' }
        return 'skip:install cargo-zigbuild + zig (cargo install cargo-zigbuild; pip install ziglang) or cross + Docker'
    }
    if ($t -like '*-apple-darwin') {
        if ($IsMacOS) { return 'build' }
        return 'skip:macOS targets must be built on macOS (scripts/build.sh) or by the GitHub release workflow'
    }
    return 'skip:unsupported target'
}

New-Item -ItemType Directory -Force -Path $Dist | Out-Null
$built = @()
$skipped = @()

foreach ($t in $targets) {
    $builder = Get-Builder $t
    if ($builder.StartsWith('skip:')) { $skipped += "${t}: $($builder.Substring(5))"; continue }

    if (Have rustup) { rustup target add $t *> $null }
    Write-Host "==> building $t (cargo $builder)"
    switch ($builder) {
        'build'    { cargo build --release --locked --target $t }
        'zigbuild' { cargo zigbuild --release --locked --target "$t.2.31" }
        'cross'    { cross build --release --locked --target $t }
        'xwin'     { cargo xwin build --release --locked --target $t }
    }
    if ($LASTEXITCODE -ne 0) { throw "build failed for $t" }

    $exe = if ($t -like '*-windows-msvc') { "$App.exe" } else { $App }
    $bin = "target/$t/release/$exe"
    if (-not (Test-Path $bin)) { throw "expected binary not found: $bin" }
    $built += $t

    if (-not $NoPackage) {
        $name = "$App-$Version-$t"
        $stage = Join-Path $Dist $name
        Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
        New-Item -ItemType Directory -Path $stage | Out-Null
        Copy-Item $bin, README.md $stage
        if ($t -like '*-windows-msvc') {
            $archive = "$stage.zip"
            Remove-Item -Force $archive -ErrorAction SilentlyContinue
            Compress-Archive -Path $stage -DestinationPath $archive
        } else {
            $archive = "$stage.tar.gz"
            tar -C $Dist -czf $archive $name
        }
        Remove-Item -Recurse -Force $stage
        Write-Host "==> packaged $archive"
    }
}

Write-Host ''
Write-Host ("Built: " + $(if ($built) { $built -join ' ' } else { 'none' }))
foreach ($s in $skipped) { Write-Host "Skipped $s" }
if (-not $built) { exit 1 }
