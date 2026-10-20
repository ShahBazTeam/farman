# Architecture and dependency decisions

Status: Phase 1/2 implementation, verified on Windows 10.0.26200 (x86_64) with
Rust 1.93.1. Decisions below are based on the actual APIs inspected and the
code compiled/run in this checkout, not on assumed versions.

## 1. Runtime stack

| Concern | Choice | Verified reason |
| --- | --- | --- |
| Window/event loop | `winit 0.30.x` | Stable `ApplicationHandler` + `EventLoopProxy`; DPI-correct physical pixels; IME events. Cargo resolves 0.30.13 for the `0.30.12` request. |
| GPU | `wgpu 30.0.1` (DX12) | Native surface, offscreen readback, WGSL. DX12 backend selected by the adapter on the test machine. |
| Text | `glyphon 0.12` + `cosmic-text 0.19` | Glyphon 0.12 declares `cosmic-text 0.19` and `wgpu 30`, so the three crates are API-compatible. Cosmic-text provides HarfRust shaping, font fallback and Unicode bidi. |
| VT/terminal state | `alacritty_terminal 0.26` | Mature VT parser/state machine, alternate screen, scrollback, selection, 16/256/true color. Rendering is ours. |
| ConPTY | `portable-pty 0.9` | Source inspection confirms it loads `CreatePseudoConsole`, `ResizePseudoConsole`, `ClosePseudoConsole` from `kernel32.dll` (with optional sideloaded `conpty.dll`). Integration tests prove real CMD/PowerShell sessions. |
| Config | `serde` + `toml` | Versioned TOML, defaults for missing fields, validation, invalid-file preservation. |
| Logging | `tracing` + file appender | Structured diagnostics; no shell content. |
| Clipboard | `arboard` | Unicode text get/set on Windows. OSC 52 handled by alacritty events. |
| Windows integration | `windows-sys 0.61` | Borderless DWM chrome (dark mode, rounded corners), work-area fit, fatal-error dialog. |

## 2. Layering

```
main.rs
  ├─ app.rs             winit lifecycle, tabs, input dispatch, scene assembly
  ├─ session.rs         ConPTY + alacritty Term + async pipes
  ├─ snapshot.rs        immutable render data (logical grid coordinates)
  ├─ text.rs            shaping, bidi policy, visual cell map
  ├─ renderer.rs        wgpu quads + glyphon text, glyph atlas
  ├─ ui.rs              custom chrome and command palette (quads + text blocks)
  ├─ config.rs          TOML, validation, AppData paths
  ├─ profile.rs         discovered + explicit shell profiles
  ├─ input.rs           key/mouse protocol encoding, shortcut parser
  ├─ theme.rs           built-in themes and 256-color palette
  └─ platform.rs        Win32 borderless chrome / work area / message box
```

The terminal state (`alacritty_terminal`) is never mutated to make text look
right. Rendering consumes an immutable `Snapshot`; visual ordering is derived in
`text.rs`.

## 3. ConPTY transport and threading

- `portable-pty` opens the pseudoconsole at the initial grid size and spawns the
  configured child.
- A **bounded** `SyncSender<Vec<u8>>` carries output from a reader thread to the
  UI thread. Backpressure prevents unbounded memory growth under command floods.
- Input uses a separate writer thread with a bounded channel, so the winit thread
  never blocks on a full pipe.
- A child-wait thread reports exit status. `ChildKiller` is cloned and used on
  shutdown; dropping the master closes the pseudoconsole. Closing the window
  terminates PowerShell, conhost helpers and pipes (verified manually).
- VT parsing (`vte::ansi::Processor`) runs on the UI thread with split UTF-8
  handled by the parser across reads.
- Resize updates `Term` dimensions and `ResizePseudoConsole` together.

No PTY output stays in a queue after a tab is closed; sessions own their
channels and threads, and are dropped with the tab.

## 4. RTL/bidi policy

### Logical data

The terminal grid stores the exact characters, cells and cursor positions from
the shell. Copy uses `alacritty_terminal`'s logical selection serialization, not
the visual order.

### Visual shaping

For each row:

1. Build the row text from logical cells, including zero-width combining marks.
2. Respect the configured paragraph direction (`auto`, `ltr`, `rtl`) using
   Unicode direction controls.
3. Shape the row with cosmic-text/HarfRust.
4. Record `LayoutGlyph.x`/`level` and map glyph clusters back to logical cell
   indices.
5. Build a per-row visual map: logical cell → visual x/width, plus an inverse
   visual column → logical cell map.
6. Paint backgrounds, selection, cursor and text at the visual positions. Hit
   testing and selection use the inverse map.

`Align::Left` is used for terminal paragraphs so a row remains anchored to its
logical start columns. The visual order of the text itself is still Unicode
bidi order.

### Paths and leading indentation

Backslashes and colons are bidi-neutral, so a Windows path with Persian
component names can be visually reordered. FARMAN detects drive paths
(`C:\...`), UNC paths (`\\server\...`), relative paths and common URL schemes,
and wraps them in zero-width LTR isolates (`U+2066`/`U+2069`) **for shaping
only**. The terminal cells and clipboard still contain the original bytes.

Rows beginning with spaces get an LTR anchoring mark in `auto` mode so terminal
indentation stays on the left rather than resolving to the RTL paragraph side.
This is a deliberate terminal-grid policy, documented as a UBA deviation for
leading whitespace.

### Cursor

The cursor visual x is derived from the logical cursor cell's bidi level:
RTL cells put the caret on their right edge; an insertion point after an RTL run
goes to the run's left edge. This is a best-effort mapping; see limitations.

## 5. Rendering

- One terminal `cosmic_text::Buffer` is shaped for the visible grid and cached
  until the snapshot generation/font/grid changes.
- UI text blocks are shaped separately with Pinar (proportional, Persian
  capable), with Vazirmatn as a fallback for symbols Pinar does not cover, so UI
  labels can be right-aligned without affecting terminal cells.
- Two `TextRenderer` instances share one `TextAtlas`: one for terminal text,
  one for UI text. This allows overlay chrome to be painted above terminal glyphs
  without terminal text bleeding through.
- Colored rectangles (cell backgrounds, selection, cursor, panel chrome,
  borders) use a small wgpu quad pipeline with an sRGB-aware color path.
- The FARMAN emblem is an embedded PNG uploaded once to an sRGB texture and
  drawn through a premultiplied-alpha image pipeline, so the title bar and empty
  state use the real artwork instead of a letter badge.
- Render order:
  1. clear + base quads (background gradient, glass terminal pane,
     header/status cards, cell backgrounds, cursor/selection)
  2. embedded mark images
  3. terminal glyphs (clipped to the terminal rect)
  4. overlay quads (settings/menu/toast/banner/empty-state card/command palette)
  5. UI glyphs
- Modal hit regions are registered full-screen first, then the panel as
  `Noop`, then individual controls. Reverse-order hit testing lets a palette row
  win over the dismiss layer without duplicating state.
- Base UI text stays in the scene while Settings or the command palette is
  open; the overlay scrim and opaque sheet suppress it visually without the
  chrome popping in or out. Terminal glyphs remain underneath the scrim.
- ANSI named/indexed/true colors are resolved from the active theme, with
  session-level color overrides honored.
- Inverse, bold, italic, underline, strikeout and wide-cell flags are mapped
  into the snapshot; backgrounds and decorations are painted separately.

### Glass geometry

The quad shader evaluates a signed distance field for a rounded rectangle and
feathers the edge in physical pixels. Each quad also supports:

- vertical top/bottom colors for subtle glass gradients;
- a radius, used by all cards/tabs/pills and the rounded cursor;
- a softness parameter for dropped shadows;
- a radial-light mode kept for future effects but currently unused; the
  backdrop relies on band-free vertical glass gradients to avoid 8-bit banding;
- a frosted-grain flag, applied only to translucent glass fills.

The large glass surfaces are layered: a dark vertical-gradient backdrop, a
translucent terminal pane, then floating header/status cards. Modal
sheets receive an additional dimming layer and a near-opaque tint so terminal
text cannot bleed through and harm readability. This is a deliberate
"readable glass" tradeoff.

The wgpu DX12 surface on the tested machine exposes only
`CompositeAlphaMode::Opaque`, so desktop-behind-window blur/transparency is not
available through the swapchain. The borderless window uses DWM dark mode and
rounded corners; acrylic is not applied to the client area.

## 6. Window and UI

- Borderless winit window (`with_decorations(false)`) with a custom title bar:
  brand, tab strip, profile chip, settings and minimize/maximize/close controls.
  The client area also contains the terminal viewport and status bar.
- Dragging the title-bar background calls `Window::drag_window`; the outer 6
  logical pixels of the window are resize handles using `drag_resize_window`.
- DWM is configured for immersive dark mode, rounded corners and a dark theme
  border. The window icon is loaded from the executable resource.
- The window is centered/clamped to the primary monitor work area at startup so
  the status bar is not hidden under the taskbar on high-DPI displays.
- Visual language: floating glass header/status cards, a rounded terminal
  pane with a right-hand session inspector, pill tabs and controls, and
  theme-driven accents. The terminal receives the vast majority of the window.
- Header: brand mark, tabs with status dots/close buttons, new-tab and
  settings, plus a search field.
- Workspace: terminal card on the left and a narrow session inspector on the
  right with the active shell, encoding, renderer, theme and RTL shaping, plus
  a grid summary footer.
- Status: profile selector chip, grid, UTF-8, RTL shaping and version.
  Redundant "Ready" status is suppressed for a quieter default state.
- Settings: a six-theme visual gallery with swatches, font size, default
  shell, bidi toggle, config path and version. The UI is English-first and
  uses Pinar for interface typography; Persian terminal text is shaped through
  the same glyph pipeline.
- Empty state, starting state, exit banner and error banner are rendered even
  when no terminal text exists.

## 7. Security decisions

- Built-in PowerShell profiles: `-NoLogo -NoProfile`; CMD: `/D` (disables
  AutoRun registry commands). This avoids implicit startup execution.
- User-defined profiles execute only the explicit program/args from config.
- No network access, telemetry, shell-content logging or embedded credentials.
- OSC 52 handling uses alacritty's default copy-only policy; FARMAN limits
  clipboard payload size.
- Clipboard paste normalizes line endings, removes NUL and strips an embedded
  bracketed-paste terminator to prevent premature paste termination.
- Invalid/malformed VT sequences are handled by the mature parser; no custom
  fake ANSI parser.
- Child processes are killed on window close, not orphaned.

## 8. Rejected or deferred options

- **`alacritty` itself as a dependency**: it is an application, not a reusable
  GUI terminal library; `alacritty_terminal` is the correct engine.
- **A web-based terminal UI**: explicitly disallowed by the product brief.
- **Custom VT parser**: unnecessary risk; alacritty's engine is proven.
- **Per-cell isolated Arabic shaping**: would break contextual joining; we shape
  whole runs instead.
- **Right-to-left shell protocol**: never. Commands and paths are sent as the
  user typed them; only presentation is reordered.
- **Kitty keyboard protocol**: not implemented in this milestone; legacy CSI
  encoding and common modifiers are supported.
- **Cursor blink animation and window opacity**: config fields exist/reserved,
  but the first milestone renders a solid cursor and an opaque window.
