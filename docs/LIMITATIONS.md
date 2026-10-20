# Known limitations

FARMAN is an early production-oriented terminal, not a universal terminal or a
complete replacement for Windows Terminal. The following limitations are
explicit and testable.

## RTL and terminal semantics

1. **Terminal rows are not a general-purpose RTL paragraph editor.**
   Unicode bidi is applied per row. Interactive programs that position cells
   individually, use arbitrary bidi text across line wraps, or draw TUIs with
   Persian labels may see visual cell shifts. Standard English/Latin TUIs are
   unaffected because their rows remain LTR and map one-to-one to cells.

2. **Cursor positions at bidi boundaries are heuristic.**
   The cursor is anchored to the visual edge of its logical cell, and insertion
   after an RTL run goes to the run's left edge. A few mixed-direction editing
   positions can still be ambiguous. Cursor movement is reported honestly by
   the shell; FARMAN does not rewrite arrow keys.

3. **Leading whitespace uses a deliberate LTR anchor in `auto` mode.**
   `U+200E` is inserted visually at the start of a row that begins with spaces
   so terminal indentation stays on the left. This intentionally overrides the
   strict UBA paragraph direction for leading whitespace.

4. **Path detection is lexical.**
   Drive, UNC, relative and common URL-like tokens are isolated LTR. Paths with
   spaces are only isolated up to the first whitespace, so a mixed Persian path
   containing spaces can still be reordered. Quoted paths or unusual drive
   syntaxes may not be detected.

5. **Emoji, CJK, and complex grapheme clusters.**
   The bundled Persian font is monospaced for Persian/Latin. Wide or emoji
   glyphs may come from system fallback fonts whose advance widths are not
   guaranteed to equal an integer number of terminal cells. Backgrounds and
   selection use the terminal's wide-cell span, but the glyph itself may not fill
   it exactly.

6. **Per-cell ANSI color boundaries can split shaping.**
   Colors are represented as spans inside one shaped paragraph, so ordinary
   color changes are fine. A program that changes color in the middle of a
   Persian joining sequence can split a ligature/joining context.

7. **No universal RTL guarantee.**
   Persian shaping, bidi punctuation/numbers, Windows paths and mixed
   Persian/English rows are covered by tests and the GPU probe. That is not the
   same as having tested every terminal application. Interactive TUI
   compatibility remains a Phase 2/3 hardening task.

## Terminal features

- **Keyboard protocol:** legacy CSI/SS3 sequences, application-cursor mode,
  modifier parameters, F1–F12 and common keys are implemented. The Kitty
  keyboard protocol and some exotic modifier combinations are not.
- **Mouse protocols:** SGR and X10 reporting, wheel and motion are encoded.
  Middle-button reporting and some extended/urxvt mouse protocols are not
  implemented.
- **Cursor blinking:** the configuration field exists and alacritty's cursor
  style is applied, but FARMAN currently renders a solid cursor; there is no
  blink timer.
- **Window opacity:** the config field is validated but not applied. The window
  is opaque in this milestone.
- **Desktop glass/blur:** the wgpu DX12 swapchain reports only
  `CompositeAlphaMode::Opaque` on the tested machine, so the app cannot blur the
  desktop behind the window. The glass effect is an in-app GPU glassmorphism
  layer (translucency over the app's own backdrop, rounded SDF cards, light
  fields, soft shadows and frosted grain).
- **Frosted grain:** this is a fixed screen-space material grain, not a true
  backdrop blur. It is intentionally subtle to preserve glyph legibility.
- **Custom window chrome:** FARMAN uses a borderless custom title bar with its
  own drag, edge resize and minimize/maximize/close controls. Native caption
  behavior and some shell integrations (for example window-shake snap or the
  classic system menu) may differ; DWM rounded corners, taskbar and Alt+Tab are
  retained.
- **Selection:** simple drag selection and logical copy are implemented.
  Block/semantic/line selection, double-click word selection and search are not
  exposed in the UI yet.
- **UI language:** application labels and command-palette entries are English.
  The terminal renders Persian/Arabic correctly and the installer is
  trilingual, but in-app UI localization is not implemented yet.
- **Hyperlinks:** alacritty stores hyperlink metadata, but OSC 8 activation is
  not exposed as clickable links yet.
- **Tab persistence / window layout restore:** not implemented. Tabs and
  sessions are per-run.
- **Installer/signing:** a portable release ZIP, a per-user install script and
  an Inno Setup per-user installer are provided; there is no MSI/MSIX, code
  signing, or auto-update.

## Environment coverage

- Compiled and runtime-tested on **one** Windows 10.0.26200 machine with an
  NVIDIA RTX 4060 using the DX12 backend. Other GPUs/backends, Windows 10 1809
  machines, ARM64, and multi-monitor mixed-DPI layouts were not tested.
- WSL integration depends on `wsl.exe` and a configured distribution; only the
  profile discovery is covered here, not a live WSL session test.
- PowerShell 7 profile discovery is tested structurally; the live ConPTY tests
  use Windows PowerShell 5.1 and CMD.
- The current environment is Windows, so all code is compiled for
  `x86_64-pc-windows-msvc`. Cross-platform fallbacks are not a supported target.

## Configuration

- Font family changes are accepted, but the renderer assumes a monospaced,
  Persian-capable face for exact grid alignment. If the requested family is
  missing, FARMAN falls back to bundled Vazir Code Hack. A present but
  proportional custom font can shift cell positions.
- Some settings require a restart conceptually (default profile, startup
  directory, scrollback) and are simply used for new sessions.
- `opacity` and the Kitty keyboard flags are reserved/forward-compatible fields.

## Honest verification statement

The test suite and probes in this repository actually ran. See
[TESTING.md](TESTING.md) for exact commands and results. Anything not listed
there as tested should be treated as unverified, even if implemented.
