param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $root

$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    $candidate = Join-Path $env:USERPROFILE ".rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe"
    if (Test-Path $candidate) {
        $cargo = $candidate
    } else {
        throw "cargo was not found. Install the Rust MSVC toolchain first."
    }
}

$versionLine = Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
$version = $versionLine.Matches[0].Groups[1].Value

if (-not $SkipBuild) {
    Write-Host "Building FARMAN $version (release)..."
    & $cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$exe = Join-Path $root "target\release\farman.exe"
if (-not (Test-Path $exe)) {
    throw "Release executable not found at $exe. Run scripts\build-installer.ps1 without -SkipBuild."
}

$iscc = Get-Command iscc -ErrorAction SilentlyContinue
if (-not $iscc) {
    $candidate = "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
    if (Test-Path $candidate) {
        $iscc = $candidate
    } else {
        throw "Inno Setup 6 (ISCC.exe) was not found. Install it from https://jrsoftware.org/isdl.php."
    }
}

$dist = Join-Path $root "dist"
New-Item -ItemType Directory -Force -Path $dist | Out-Null

$script = Join-Path $root "installer\farman.iss"
Write-Host "Compiling installer with $iscc..."
& $iscc "/DAppVersion=$version" $script
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed" }

$setup = Join-Path $dist "FARMAN-Terminal-$version-win64-setup.exe"
if (-not (Test-Path $setup)) {
    throw "Installer was not produced at $setup"
}

Write-Host "Created $setup"
Write-Host "Publisher: Shahbaz Team (https://shahbazteam.ir)"
