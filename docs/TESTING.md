# Testing and verification record

All commands below were actually run in this checkout on 2026-10-19/20, Windows
10.0.26200 (x86_64), Rust 1.93.1 MSVC, NVIDIA RTX 4060, DX12 backend.

## Automated tests

```powershell
cargo test
```

Result: **52 passed, 0 failed, 0 ignored** (the exact count is visible in the
test output; the suite covers the groups below).

### Unicode, shaping and bidi

- Persian-only shaping and RTL visual order.
- Arabic contextual shaping path (bundled Vazir Code Hack, HarfRust).
- Mixed Persian/English: `خطا در main.rs:42`.
- Windows paths, including `C:\Users\User\Desktop\Project` and
  `D:\پروژه‌ها\فارسی\main.rs` component order.
- Numbers/identifiers: `نسخه Python: 3.13`.
- Combining marks.
- Logical text preservation (no string reversal).
- Visual→logical inverse map roundtrip for content cells.
- Leading indentation in RTL rows.
- Forced LTR policy changes visual ordering only.
- Bundled font coverage and monospace advance verification.

### Terminal core (alacritty_terminal)

- 16-color, 256-color and true-color SGR.
- Cursor positioning and relative movement.
- Wide characters (CJK) and spacer flags; Persian width.
- Wrapping, scrollback history and display offset.
- Resize and content preservation.
- Alternate screen enter/leave.
- Backspace, tab, carriage return, line feed.
- Malformed/unterminated sequences do not panic.

### Configuration and UI

- Default config roundtrip and TOML serialization.
- Invalid values clamped/validated.
- Invalid config is preserved and backed up, never overwritten.
- Custom profiles deserialize.
- Empty-state chrome construction.
- Terminal layout occupies most of the viewport; hit-test precedence.
- Glass-card geometry uses rounded quads; overlay radii >= 15 px.
- Theme resolution and 256-color palette.
- Command palette multi-term filtering (case-insensitive, out-of-order terms).
- Palette action hit regions are registered above the dismiss layer.

### Windows ConPTY integration (Windows-only)

- Real `cmd.exe /D /C echo ...` through ConPTY: output received.
- Real `powershell.exe -NoLogo -NoProfile -Command ...`: output received.
- Keyboard input roundtrip into interactive CMD (`echo ...` + CR).
- Resize reaches both VT state and the ConPTY.
- `close()` terminates the child and completes.
- Program discovery finds `cmd.exe`.

## GPU render probe

```powershell
cargo run -- --render-test target\rtl-render.png
```

Observed:

```
render-test OK: target\rtl-render.png (1280x800), adapter=NVIDIA GeForce RTX 4060 Laptop GPU,
non-bg-pixels=90093, bright-pixels=17259, cell=10.84x24.00, grid=113x28
fonts: terminal=Vazir Code Hack, ui=Pinar; bidi=auto; scale=1.0
```

The resulting image was visually inspected. It contains correct Persian
shaping, RTL word order, mixed-direction punctuation, an intact path with Persian
directory names, ANSI-colored text, and a visible block cursor. The probe is a
real wgpu render and readback, not a mockup. The checked-in artifact is
[images/rtl-render.png](images/rtl-render.png).

## Manual runtime checks

These were performed against the actual GUI process:

1. **Window + shell**: launched `target\debug\farman.exe`; the borderless
   window showed the custom title bar, brand, tab pills, profile chip, settings
   icon, window controls, live PowerShell prompt `PS C:\Users\Omen>` and the
   floating status card.
2. **Glass visual pass**: verified at 150% DPI that the rounded terminal pane,
   header/status cards, vertical glass gradients, borders and frosted grain
   render without banding, seams or ringing; the embedded FARMAN emblem has a
   clean transparent background in both the title bar and empty state, and modal
   sheets use a stronger tint so terminal text does not bleed through. Screenshots:
   [farman-window](images/farman-window.png), [settings](images/settings.png),
   [profile-menu](images/profile-menu.png),
   [persian-session](images/persian-session.png).
3. **ASCII input/output**: pasted `echo runtimeok` via `Ctrl+Shift+V`; shell
   echoed and executed it; output appeared.
4. **Persian input/output**: pasted `echo "خطا در main.rs:42"`; shell received
   the logical UTF-8 text and output rendered correctly in RTL visual order.
5. **Selection/copy**: dragged across the rendered Persian output and pressed
   `Ctrl+Shift+C`; Windows clipboard contained exactly
   `خطا در main.rs:42` in logical order.
6. **Font/theme**: settings panel changed font size and theme; the config file
   updated and the UI re-rendered with the glass palette.
7. **Tabs**: `Ctrl+Shift+T` opened a second live PowerShell tab; `Ctrl+Shift+W`
   closed it and left the remaining tab.
8. **Profile menu**: status-bar selector opened and listed Windows PowerShell,
   PowerShell 7, Command Prompt, WSL and Settings. A full-screen dim originally
   made the base UI look blank; it was removed, and chrome text now remains
   visible behind the menu. Verified with a real mouse click and captured in
   [profile-menu.png](images/profile-menu.png).
9. **Settings overlay**: opened with a real mouse click on the title-bar settings
   icon; screenshot checked at full 1770x1080 physical resolution.
10. **Window work area**: at 150% DPI the startup window is 1770x1080 physical at
    (395,144); the work area is 2560x1368, and the window is centered/clamped so
    the status bar remains above the taskbar.
11. **Command palette**: `Ctrl+K` and the title-bar search field opened the
    palette; typing filtered commands, arrows moved the selection, `Enter` ran
    New tab/Settings/theme/bidi/shell/reload actions, and `Esc` dismissed it.
    The scrim keeps the session readable behind the card and captures the
    pointer. Screenshot: [command-palette.png](images/command-palette.png).
12. **Clean shutdown**: `CloseMainWindow()` exited FARMAN; its `powershell.exe`
    child and both conhost helpers were no longer running.
13. **Startup safety**: before switching to `-NoProfile`, a user PowerShell
    profile launched an external application (OpenCode). Built-in profiles now
    pass `-NoProfile`, and a clean launch shows only the prompt.

## Installer smoke test

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
# then install silently into a temporary directory:
dist\FARMAN-Terminal-0.3.0-win64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /NOICONS /LANG=farsi /DIR=target\installer-smoke-030
```

Observed:

- Inno Setup 6.7.1 compiled the installer successfully.
- The language dialog offers English, Persian and Arabic; the Persian wizard
  pages were checked for Vazirmatn typography and RTL layout.
- Exit code 0; `farman.exe`, docs, licenses and `config.example.toml` were
  installed under the requested directory with `/LANG=farsi`.
- `HKCU\...\App Paths\farman.exe` pointed at the installed executable.
- The per-user uninstaller appeared under Apps & features and removed the
  directory and App Paths key when run with `/VERYSILENT`.

## Release artifacts

```powershell
cargo build --release
powershell -ExecutionPolicy Bypass -File scripts\package.ps1
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
```

- `target\release\farman.exe` — 11,593,216 bytes; embedded FARMAN emblem
  artwork/icon and Windows version metadata (`FileVersion=0.3.0`,
  `ProductVersion=0.3.0`, `ProductName=FARMAN Terminal`,
  `CompanyName=Shahbaz Team`).
- `dist\FARMAN-Terminal-0.3.0-win64.zip` — portable release package with
  updated docs, screenshots, licenses (including Pinar) and the example config.
- `dist\FARMAN-Terminal-0.3.0-win64-setup.exe` — per-user trilingual Inno Setup
  installer.
- SHA-256:
  - executable `6B629DA4419482A2868369D9B3B04E6948A92FB58E916226A0ED420685FAAFFA`
  - the archive checksums are written to `dist\SHA256SUMS.txt` and published
    with the GitHub Release (embedding them in docs would change the archives
    they describe).
- The release executable is launched and closed successfully after packaging.

## Not tested / not claimed

- Interactive full-screen TUI applications (vim, htop, etc.) with Persian
  content.
- Live WSL session; only discovery is covered.
- Multi-monitor mixed-DPI scenarios.
- High-throughput output stress benchmark.
- Authenticode signing and MSI/MSIX packaging.
- Windows 10 1809 or ARM64.

See [LIMITATIONS.md](LIMITATIONS.md).
