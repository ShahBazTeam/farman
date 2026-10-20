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
- offers the setup wizard in **English**, **Persian (فارسی)** and
  **Arabic (العربية)**, including RTL wizard layout;
- adds Start Menu and optional desktop shortcuts;
- registers an uninstaller in Apps & features;
- registers `farman.exe` under `HKCU\Software\Microsoft\Windows\CurrentVersion\App Paths`;
- includes docs, licenses and the example config;
- leaves `%APPDATA%\FARMAN` and `%LOCALAPPDATA%\FARMAN` user data in place on
  uninstall.

Language behavior:

- The language dialog appears before the wizard and lists all three languages.
- Arabic uses the official Inno Setup translation
  (`compiler:Languages\Arabic.isl`).
- Persian uses the bundled translation at `installer\languages\Farsi.isl`
  (UTF-8 with BOM, `LanguageID=$0429`, `RightToLeft=yes`).
- Automatic selection follows the Windows UI language where possible; the user
  can still choose another language in the dialog.
- `scripts\build-installer.ps1` and CI compile all three language files, so a
  broken translation fails the build.

Publisher metadata: **Shahbaz Team**, <https://shahbazteam.ir>.

## Create a release ZIP

```powershell
powershell -ExecutionPolicy Bypass -File scripts\package.ps1
```

The script builds release mode and writes
`dist\FARMAN-Terminal-<version>-win64.zip` containing the executable, docs,
licenses and an example config. It does not upload anything.

## Automated release

Pushing a tag such as `v0.3.0` runs
[.github/workflows/release.yml](../.github/workflows/release.yml). The workflow:

1. verifies that the tag matches the version in `Cargo.toml`;
2. runs the test suite;
3. builds the portable ZIP;
4. installs Inno Setup and compiles the trilingual installer;
5. writes `dist\SHA256SUMS.txt`;
6. publishes a GitHub Release with the ZIP, installer and checksums.

The repository CI also compiles the installer on every push and pull request,
so a missing or broken language file is caught before release.

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
