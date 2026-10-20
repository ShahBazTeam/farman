# Packaging and installation

FARMAN is distributed as a single portable executable, a portable ZIP, and a
per-user Inno Setup installer. All artifacts are unsigned development builds
unless a release is explicitly signed.

## Portable build

```powershell
cargo build --release
# target\release\farman.exe
```

The executable embeds:

- the FARMAN icon,
- Vazir Code Hack (terminal),
- Pinar Regular/Medium/SemiBold/Bold (UI),
- Vazirmatn (UI symbol fallback).

No runtime asset folder is required. Configuration and logs are created under
`%APPDATA%\FARMAN` and `%LOCALAPPDATA%\FARMAN`.

## Windows installer

Requirements: Inno Setup 6 (`ISCC.exe`), normally installed at
`C:\Program Files (x86)\Inno Setup 6\ISCC.exe`.

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
# dist\FARMAN-Terminal-<version>-win64-setup.exe
```

`build-installer.ps1` reads the version from `Cargo.toml`, builds release mode,
and compiles [installer/farman.iss](../installer/farman.iss). Use `-SkipBuild`
to package an existing `target\release\farman.exe`.

The installer:

- installs per-user to `%LOCALAPPDATA%\Programs\FARMAN Terminal` (no elevation);
- adds Start Menu and optional desktop shortcuts;
- registers an uninstaller in Apps & features;
- registers `farman.exe` under `HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths`;
- includes docs, licenses and the example config;
- leaves `%APPDATA%\FARMAN` and `%LOCALAPPDATA%\FARMAN` user data in place on
  uninstall.

Publisher metadata: **Shahbaz Team**, <https://shahbazteam.ir>.

## Create a release ZIP

```powershell
powershell -ExecutionPolicy Bypass -File scripts\package.ps1
```

The script builds release mode and writes
`dist\FARMAN-Terminal-<version>-win64.zip` containing the executable, docs,
licenses and an example config. It does not upload anything.

## Per-user installation without Inno Setup

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install.ps1
```

This copies the release executable to
`%LOCALAPPDATA%\FARMAN\bin\farman.exe` and creates a Start Menu shortcut. It
does not modify the machine-wide PATH and does not require elevation. Uninstall
by deleting that directory and the shortcut.

## Code signing

This milestone does not include Authenticode signing, MSI/MSIX packaging or
auto-update. Windows SmartScreen may warn on an unsigned locally built binary.
For production distribution, sign the final executable and the installer with a
trusted code-signing certificate.
