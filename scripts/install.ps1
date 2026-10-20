param(
    [string]$ExePath = ""
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
if (-not $ExePath) {
    $ExePath = Join-Path $root "target\release\farman.exe"
}
if (-not (Test-Path $ExePath)) {
    throw "FARMAN executable not found at $ExePath. Run 'cargo build --release' or scripts\package.ps1 first."
}

$installDir = Join-Path $env:LOCALAPPDATA "FARMAN\bin"
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$target = Join-Path $installDir "farman.exe"
Copy-Item $ExePath $target -Force

$startMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
New-Item -ItemType Directory -Force -Path $startMenu | Out-Null
$shortcutPath = Join-Path $startMenu "FARMAN Terminal.lnk"
$ws = New-Object -ComObject WScript.Shell
$shortcut = $ws.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $target
$shortcut.WorkingDirectory = $env:USERPROFILE
$shortcut.IconLocation = "$target,0"
$shortcut.Description = "FARMAN Terminal — Persian/Arabic capable Windows terminal"
$shortcut.Save()

Write-Host "Installed FARMAN to $target"
Write-Host "Start Menu shortcut: $shortcutPath"
Write-Host "No machine-wide PATH or registry changes were made."
