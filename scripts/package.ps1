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
Write-Host "Building FARMAN $version (release)..."

& $cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$distRoot = Join-Path $root "dist"
$stage = Join-Path $distRoot "FARMAN-Terminal-$version-win64"
$zip = Join-Path $distRoot "FARMAN-Terminal-$version-win64.zip"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
if (Test-Path $zip) { Remove-Item $zip -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

Copy-Item "target\release\farman.exe" (Join-Path $stage "farman.exe")
Copy-Item "README.md" $stage
Copy-Item "LICENSE" $stage
Copy-Item "config.example.toml" $stage
Copy-Item "docs" $stage -Recurse
New-Item -ItemType Directory -Force -Path (Join-Path $stage "licenses") | Out-Null
Copy-Item "assets\fonts\OFL-Vazir-Code.txt" (Join-Path $stage "licenses")
Copy-Item "assets\fonts\OFL-Vazirmatn.txt" (Join-Path $stage "licenses")
Copy-Item "assets\fonts\NOTICE-Pinar.txt" (Join-Path $stage "licenses")

Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip
Write-Host "Created $zip"
Write-Host "Executable: target\release\farman.exe"
