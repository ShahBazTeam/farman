# Contributing to FARMAN

Thanks for helping improve a terminal with real Persian and Arabic support.
Small, focused pull requests are easiest to review.

## Before you start

- Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
  [docs/LIMITATIONS.md](docs/LIMITATIONS.md).
- Search existing issues before opening a new one.
- For security issues, follow [SECURITY.md](SECURITY.md) instead of opening a
  public issue.

## Development setup

Requirements: Windows 10 1809+ or Windows 11, Rust stable MSVC, Visual Studio
Build Tools, Windows SDK, and PowerShell 7 for the helper scripts.

```powershell
$env:Path = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin;$env:Path"
cargo test
cargo run
```

The test suite includes Windows-only ConPTY integration tests. Run it before
opening a pull request:

```powershell
cargo test
cargo check --all-targets
```

Inno Setup 6 is only needed when changing `installer/farman.iss` or the
translations under `installer/languages`:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
```

## Pull request expectations

- Keep the terminal grid in logical order. Visual bidi reordering belongs in
  the shaping/paint path, never in the VT buffer.
- Preserve copy/paste, selection, mouse coordinates and cursor mapping for RTL
  rows.
- Add or update tests for parser, shaping, bidi, input or UI changes.
- Do not add network calls, telemetry or shell content to logs.
- Do not add fonts or other assets unless their license allows redistribution
  and the license text is included.
- Update `docs/LIMITATIONS.md` or `docs/TESTING.md` when behaviour changes.
- Keep UI text in English for now; the installer is intentionally English,
  Persian and Arabic.

## Commit style

Use a short imperative subject and explain why in the body when it is not
obvious:

```text
Add command palette filtering tests

Covers multi-term queries and verifies that palette rows sit above the
full-screen dismiss region.
```

## Reporting bugs

Include:

- Windows version and display scale.
- FARMAN version (visible in Settings).
- Shell/profile and whether the issue is RTL-specific.
- Exact reproduction steps.
- Relevant logs from `%LOCALAPPDATA%\FARMAN\logs` with secrets removed.
- A screenshot or short recording when the issue is visual.

Please do not paste tokens, credentials, private paths or sensitive shell
output.
