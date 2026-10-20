//! A real Windows pseudoconsole session.
//!
//! `portable-pty` is verified to use `CreatePseudoConsole`/`ResizePseudoConsole`
//! on Windows. This module owns the ConPTY, the child process, the asynchronous
//! pipe transport and the `alacritty_terminal` VT state machine. No output is
//! synthesized and no command is rewritten.

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use alacritty_terminal::vte::ansi::{
    Color, CursorShape, CursorStyle, NamedColor, Processor, Rgb as VteRgb,
};
use portable_pty::{
    native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize,
};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use tracing::{debug, warn};

use crate::snapshot::{
    CellFlags, CellView, CursorShapeView, CursorView, RowView, SelectionView, Snapshot,
};
use crate::theme::Theme;
use crate::types::Rgb;

#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub id: u64,
    pub profile_name: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub cols: u16,
    pub rows: u16,
    pub cell_width_px: u16,
    pub cell_height_px: u16,
    pub scrollback: usize,
    pub cursor_style: String,
    pub cursor_blink: bool,
}

#[derive(Debug, Clone)]
pub struct ChildExitInfo {
    pub code: Option<u32>,
    pub success: bool,
    pub signal: Option<String>,
    pub at: Instant,
}

pub type WakeFn = Arc<dyn Fn(u64) + Send + Sync>;

#[derive(Clone)]
struct Listener {
    id: u64,
    events: Sender<Event>,
    wake: WakeFn,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Wakeup | Event::MouseCursorDirty => {}
            event => {
                if self.events.send(event).is_err() {
                    return;
                }
            }
        }
        (self.wake)(self.id);
    }
}

struct Size {
    rows: usize,
    cols: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct Session {
    pub id: u64,
    pub profile_name: String,
    pub title: String,
    term: Term<Listener>,
    processor: Processor,
    master: Option<Box<dyn MasterPty + Send>>,
    /// Input is user-rate-limited and paste is capped, so an unbounded queue
    /// here cannot be grown by untrusted terminal output. Unlike a bounded
    /// queue, it can never deadlock the UI thread if a child stops reading.
    writer_tx: Option<Sender<Vec<u8>>>,
    output_rx: Receiver<Vec<u8>>,
    events_rx: Receiver<Event>,
    exit_rx: Receiver<ChildExitInfo>,
    killer: Option<Arc<Mutex<Box<dyn ChildKiller + Send + Sync>>>>,
    pending_events: Vec<Event>,
    pub child_pid: Option<u32>,
    pub exit: Option<ChildExitInfo>,
    pub generation: u64,
    pub cols: usize,
    pub rows: usize,
    pub cell_width_px: u16,
    pub cell_height_px: u16,
    pub started_at: Instant,
    pub last_output_at: Instant,
    pub read_error: Option<String>,
    read_error_shared: Arc<Mutex<Option<String>>>,
    pub shutdown: bool,
}

impl Session {
    pub fn spawn(options: SpawnOptions, wake: WakeFn) -> anyhow::Result<Self> {
        let cols = options.cols.max(2);
        let rows = options.rows.max(2);
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: (cols as u32 * options.cell_width_px as u32).min(u16::MAX as u32) as u16,
            pixel_height: (rows as u32 * options.cell_height_px as u32).min(u16::MAX as u32) as u16,
        })?;

        let mut command = CommandBuilder::new(&options.program);
        for arg in &options.args {
            command.arg(arg);
        }
        if let Some(cwd) = options.cwd.as_ref().filter(|p| p.is_dir()) {
            command.cwd(cwd);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("FARMAN_TERMINAL", "1");

        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let child_pid = child.process_id();
        let killer = Arc::new(Mutex::new(child.clone_killer()));

        let master = pair.master;
        let writer = master.take_writer()?;
        let mut reader = master.try_clone_reader()?;

        let shape = match options.cursor_style.as_str() {
            "beam" => CursorShape::Beam,
            "underline" => CursorShape::Underline,
            _ => CursorShape::Block,
        };
        let config = TermConfig {
            scrolling_history: options.scrollback,
            default_cursor_style: CursorStyle {
                shape,
                blinking: options.cursor_blink,
            },
            ..Default::default()
        };

        let (events_tx, events_rx) = mpsc::channel();
        let listener = Listener {
            id: options.id,
            events: events_tx,
            wake: wake.clone(),
        };
        let term = Term::new(config, &Size { rows: rows as usize, cols: cols as usize }, listener);
        let processor = Processor::new();

        let (output_tx, output_rx) = mpsc::sync_channel::<Vec<u8>>(256);
        let reader_wake = wake.clone();
        let reader_id = options.id;
        let read_error = Arc::new(Mutex::new(None::<String>));
        let read_error_thread = read_error.clone();
        thread::Builder::new()
            .name(format!("farman-pty-reader-{reader_id}"))
            .spawn(move || {
                let mut buf = [0u8; 32 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if output_tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                            reader_wake(reader_id);
                        }
                        Err(err) => {
                            // A closed ConPTY often reports a broken pipe on
                            // shutdown; retain the message for diagnostics.
                            if err.kind() != std::io::ErrorKind::BrokenPipe {
                                *read_error_thread.lock().unwrap() = Some(err.to_string());
                                warn!(session = reader_id, error = %err, "ConPTY reader stopped");
                            }
                            break;
                        }
                    }
                }
            })?;

        let (writer_tx, writer_rx) = mpsc::channel::<Vec<u8>>();
        let writer_id = options.id;
        thread::Builder::new()
            .name(format!("farman-pty-writer-{writer_id}"))
            .spawn(move || {
                let mut writer = writer;
                while let Ok(bytes) = writer_rx.recv() {
                    if writer.write_all(&bytes).is_err() {
                        break;
                    }
                    if writer.flush().is_err() {
                        break;
                    }
                }
                // Dropping the writer closes the ConPTY input side.
            })?;

        let (exit_tx, exit_rx) = mpsc::channel::<ChildExitInfo>();
        let exit_wake = wake;
        let exit_id = options.id;
        thread::Builder::new()
            .name(format!("farman-child-wait-{exit_id}"))
            .spawn(move || {
                let mut child = child;
                match child.wait() {
                    Ok(status) => {
                        let info = ChildExitInfo {
                            code: Some(status.exit_code()),
                            success: status.success(),
                            signal: status.signal().map(str::to_owned),
                            at: Instant::now(),
                        };
                        let _ = exit_tx.send(info);
                    }
                    Err(err) => {
                        let _ = exit_tx.send(ChildExitInfo {
                            code: None,
                            success: false,
                            signal: Some(format!("wait failed: {err}")),
                            at: Instant::now(),
                        });
                    }
                }
                exit_wake(exit_id);
            })?;

        let now = Instant::now();
        debug!(
            session = options.id,
            profile = %options.profile_name,
            pid = child_pid.unwrap_or(0),
            cols,
            rows,
            "ConPTY session started"
        );

        Ok(Self {
            id: options.id,
            profile_name: options.profile_name,
            title: String::new(),
            term,
            processor,
            master: Some(master),
            writer_tx: Some(writer_tx),
            output_rx,
            events_rx,
            exit_rx,
            killer: Some(killer),
            pending_events: Vec::new(),
            child_pid,
            exit: None,
            generation: 1,
            cols: cols as usize,
            rows: rows as usize,
            cell_width_px: options.cell_width_px,
            cell_height_px: options.cell_height_px,
            started_at: now,
            last_output_at: now,
            read_error: None,
            read_error_shared: read_error,
            shutdown: false,
        })
    }

    /// Process available PTY output and terminal events without blocking.
    /// Returns true if the viewport needs repainting.
    pub fn poll(&mut self, theme: &Theme) -> bool {
        let mut changed = false;
        loop {
            match self.output_rx.try_recv() {
                Ok(chunk) => {
                    self.processor.advance(&mut self.term, &chunk);
                    self.last_output_at = Instant::now();
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.read_error = self.read_error_shared.lock().unwrap().clone();
                    break;
                }
            }
        }

        while let Ok(event) = self.events_rx.try_recv() {
            match event {
                Event::Title(title) => {
                    self.title = title;
                    changed = true;
                }
                Event::ResetTitle => {
                    self.title.clear();
                    changed = true;
                }
                Event::PtyWrite(text) => {
                    self.write(text.into_bytes());
                }
                Event::TextAreaSizeRequest(formatter) => {
                    let size = WindowSize {
                        num_lines: self.rows as u16,
                        num_cols: self.cols as u16,
                        cell_width: self.cell_width_px,
                        cell_height: self.cell_height_px,
                    };
                    let response = formatter(size);
                    self.write(response.into_bytes());
                }
                Event::ColorRequest(index, formatter) => {
                    let color = theme.indexed(index.min(255) as u8);
                    let response = formatter(VteRgb {
                        r: color.r,
                        g: color.g,
                        b: color.b,
                    });
                    self.write(response.into_bytes());
                }
                Event::ChildExit(status) => {
                    self.exit = Some(ChildExitInfo {
                        code: status.code().map(|c| c as u32),
                        success: status.success(),
                        signal: None,
                        at: Instant::now(),
                    });
                    changed = true;
                }
                Event::Exit => {
                    changed = true;
                }
                Event::Bell => {
                    self.pending_events.push(Event::Bell);
                }
                Event::ClipboardStore(ty, text) => {
                    self.pending_events.push(Event::ClipboardStore(ty, text));
                }
                Event::ClipboardLoad(ty, formatter) => {
                    self.pending_events.push(Event::ClipboardLoad(ty, formatter));
                }
                Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
            }
        }

        loop {
            match self.exit_rx.try_recv() {
                Ok(info) => {
                    self.exit = Some(info);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        if changed {
            self.generation = self.generation.wrapping_add(1);
        }
        changed
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.pending_events)
    }

    pub fn write(&self, bytes: impl AsRef<[u8]>) {
        if let Some(tx) = &self.writer_tx {
            let _ = tx.send(bytes.as_ref().to_vec());
        }
    }

    pub fn write_text(&self, text: &str) {
        self.write(text.as_bytes());
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_width_px: u16, cell_height_px: u16) {
        let cols = cols.max(2) as usize;
        let rows = rows.max(2) as usize;
        if cols == self.cols && rows == self.rows {
            self.cell_width_px = cell_width_px;
            self.cell_height_px = cell_height_px;
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.cell_width_px = cell_width_px;
        self.cell_height_px = cell_height_px;
        self.term.resize(Size { rows, cols });
        if let Some(master) = &self.master {
            let _ = master.resize(PtySize {
                rows: rows as u16,
                cols: cols as u16,
                pixel_width: (cols as u32 * cell_width_px as u32).min(u16::MAX as u32) as u16,
                pixel_height: (rows as u32 * cell_height_px as u32).min(u16::MAX as u32) as u16,
            });
        }
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn scroll(&mut self, delta: i32) {
        if delta == 0 {
            return;
        }
        self.term.scroll_display(Scroll::Delta(delta));
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn scroll_page(&mut self, up: bool) {
        self.term
            .scroll_display(if up { Scroll::PageUp } else { Scroll::PageDown });
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn selection_start(&mut self, viewport_row: usize, col: usize, side: Side) {
        let point = self.viewport_to_point(viewport_row, col);
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
    }

    pub fn selection_update(&mut self, viewport_row: usize, col: usize, side: Side) {
        let point = self.viewport_to_point(viewport_row, col);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn selection_clear(&mut self) {
        self.term.selection = None;
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    pub fn has_selection(&self) -> bool {
        self.term
            .selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
    }

    fn viewport_to_point(&self, viewport_row: usize, col: usize) -> Point {
        let offset = self.term.grid().display_offset() as i32;
        let line = Line(viewport_row as i32 - offset);
        let col = Column(col.min(self.cols.saturating_sub(1)));
        Point::new(line, col)
    }

    pub fn snapshot(&self, theme: &Theme) -> Snapshot {
        let content = self.term.renderable_content();
        let cols = self.cols.max(1);
        let visible_rows = self.rows.max(1);
        let mut rows: Vec<RowView> = (0..visible_rows)
            .map(|_| RowView {
                cells: vec![CellView::default(); cols],
                wrapped: false,
            })
            .collect();

        for indexed in content.display_iter {
            let point = indexed.point;
            let viewport_row = point.line.0 + content.display_offset as i32;
            if viewport_row < 0 {
                continue;
            }
            let row_idx = viewport_row as usize;
            if row_idx >= rows.len() || point.column.0 >= cols {
                continue;
            }
            let cell: &Cell = indexed.cell;
            let view = &mut rows[row_idx].cells[point.column.0];
            view.ch = if cell.c == '\0' || (cell.c.is_control() && cell.c != '\t') {
                ' '
            } else {
                cell.c
            };
            view.zerowidth = cell.zerowidth().map(|z| z.to_vec()).unwrap_or_default();
            view.fg = resolve_color(cell.fg, content.colors, theme);
            view.bg = resolve_color(cell.bg, content.colors, theme);
            view.flags = CellFlags {
                inverse: cell.flags.contains(Flags::INVERSE),
                bold: cell.flags.contains(Flags::BOLD),
                italic: cell.flags.contains(Flags::ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                strike: cell.flags.contains(Flags::STRIKEOUT),
                wide: cell.flags.contains(Flags::WIDE_CHAR),
                wide_spacer: cell.flags.contains(Flags::WIDE_CHAR_SPACER),
                leading_wide_spacer: cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER),
                hidden: cell.flags.contains(Flags::HIDDEN),
            };
            if cell.flags.contains(Flags::WRAPLINE) {
                rows[row_idx].wrapped = true;
            }
        }

        let offset = content.display_offset as i32;
        let cursor = {
            let point = content.cursor.point;
            let row = point.line.0 + offset;
            let shape = match content.cursor.shape {
                CursorShape::Block => CursorShapeView::Block,
                CursorShape::Beam => CursorShapeView::Beam,
                CursorShape::Underline => CursorShapeView::Underline,
                CursorShape::HollowBlock => CursorShapeView::HollowBlock,
                CursorShape::Hidden => CursorShapeView::Hidden,
            };
            if row >= 0 && (row as usize) < visible_rows && point.column.0 < cols {
                Some(CursorView {
                    row: row as usize,
                    col: point.column.0,
                    shape,
                })
            } else {
                None
            }
        };

        let selection = content.selection.map(|range| {
            let mut start_row = range.start.line.0 + offset;
            let mut end_row = range.end.line.0 + offset;
            let mut start_col = range.start.column.0;
            let mut end_col = range.end.column.0;
            if start_row < 0 {
                start_row = 0;
                start_col = 0;
            }
            if end_row < 0 {
                return SelectionView {
                    start_row: 0,
                    start_col: 0,
                    end_row: 0,
                    end_col: 0,
                    block: range.is_block,
                };
            }
            if start_row as usize >= visible_rows {
                start_row = visible_rows.saturating_sub(1) as i32;
                start_col = 0;
            }
            if end_row as usize >= visible_rows {
                end_row = visible_rows.saturating_sub(1) as i32;
                end_col = cols.saturating_sub(1);
            }
            SelectionView {
                start_row: start_row as usize,
                start_col: start_col.min(cols.saturating_sub(1)),
                end_row: end_row as usize,
                end_col: end_col.min(cols.saturating_sub(1)),
                block: range.is_block,
            }
        });

        let mode = content.mode;
        Snapshot {
            cols,
            rows,
            cursor,
            selection,
            display_offset: content.display_offset,
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            application_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse_reporting: mode.intersects(TermMode::MOUSE_MODE),
            mouse_motion: mode.intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG),
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            utf8_mouse: mode.contains(TermMode::UTF8_MOUSE),
            generation: self.generation,
            session_id: self.id,
        }
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn is_running(&self) -> bool {
        self.exit.is_none()
    }

    pub fn application_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    pub fn close(&mut self) {
        self.shutdown = true;
        self.writer_tx.take();
        if let Some(killer) = self.killer.take() {
            if let Ok(mut killer) = killer.lock() {
                let _ = killer.kill();
            }
        }
        // Closing the ConPTY handle is what causes the reader thread to observe
        // EOF. The child killer above guarantees the process tree is asked to
        // terminate even if it ignores Ctrl+C.
        self.master.take();
    }

    #[cfg(test)]
    fn wait_for_text(
        &mut self,
        theme: &Theme,
        needle: &str,
        timeout: std::time::Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            self.poll(theme);
            let snapshot = self.snapshot(theme);
            for row in 0..snapshot.rows.len() {
                if snapshot.logical_row_text(row).contains(needle) {
                    return true;
                }
            }
            thread::sleep(std::time::Duration::from_millis(25));
        }
        false
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

fn resolve_color(color: Color, colors: &Colors, theme: &Theme) -> Rgb {
    match color {
        Color::Spec(rgb) => Rgb::new(rgb.r, rgb.g, rgb.b),
        Color::Indexed(index) => {
            if let Some(rgb) = colors[index as usize] {
                Rgb::new(rgb.r, rgb.g, rgb.b)
            } else {
                theme.indexed(index)
            }
        }
        Color::Named(named) => {
            if let Some(rgb) = colors[named] {
                return Rgb::new(rgb.r, rgb.g, rgb.b);
            }
            match named {
                NamedColor::Black => theme.ansi[0],
                NamedColor::Red => theme.ansi[1],
                NamedColor::Green => theme.ansi[2],
                NamedColor::Yellow => theme.ansi[3],
                NamedColor::Blue => theme.ansi[4],
                NamedColor::Magenta => theme.ansi[5],
                NamedColor::Cyan => theme.ansi[6],
                NamedColor::White => theme.ansi[7],
                NamedColor::BrightBlack => theme.ansi[8],
                NamedColor::BrightRed => theme.ansi[9],
                NamedColor::BrightGreen => theme.ansi[10],
                NamedColor::BrightYellow => theme.ansi[11],
                NamedColor::BrightBlue => theme.ansi[12],
                NamedColor::BrightMagenta => theme.ansi[13],
                NamedColor::BrightCyan => theme.ansi[14],
                NamedColor::BrightWhite => theme.ansi[15],
                NamedColor::Foreground => theme.foreground,
                NamedColor::BrightForeground => theme.bright_foreground,
                NamedColor::DimForeground => theme.dim_foreground,
                NamedColor::Background => theme.background,
                NamedColor::Cursor => theme.cursor,
                NamedColor::DimBlack => theme.dim(theme.ansi[0]),
                NamedColor::DimRed => theme.dim(theme.ansi[1]),
                NamedColor::DimGreen => theme.dim(theme.ansi[2]),
                NamedColor::DimYellow => theme.dim(theme.ansi[3]),
                NamedColor::DimBlue => theme.dim(theme.ansi[4]),
                NamedColor::DimMagenta => theme.dim(theme.ansi[5]),
                NamedColor::DimCyan => theme.dim(theme.ansi[6]),
                NamedColor::DimWhite => theme.dim(theme.ansi[7]),
            }
        }
    }
}

/// Check whether a Windows command-line executable exists.
pub fn find_program(program: &str) -> Option<PathBuf> {
    if program.contains('\\') || program.contains('/') {
        let path = PathBuf::from(program);
        return path.is_file().then_some(path);
    }
    let path_var = std::env::var_os("PATH")?;
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_owned());
    for dir in std::env::split_paths(&path_var) {
        let direct = dir.join(program);
        if direct.is_file() {
            return Some(direct);
        }
        for ext in pathext.split(';') {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FARMAN_DARK;
    use std::time::Duration;

    fn test_options(program: &str, args: &[&str]) -> SpawnOptions {
        SpawnOptions {
            id: 99,
            profile_name: "test".to_owned(),
            program: program.to_owned(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            cwd: None,
            cols: 100,
            rows: 30,
            cell_width_px: 10,
            cell_height_px: 20,
            scrollback: 10_000,
            cursor_style: "block".to_owned(),
            cursor_blink: false,
        }
    }

    #[test]
    fn conpty_cmd_roundtrip() {
        let mut session = Session::spawn(
            test_options(
                "cmd.exe",
                &["/D", "/C", "echo FARMAN_CMD_CONPTY_OK"],
            ),
            Arc::new(|_| {}),
        )
        .expect("spawn cmd through ConPTY");
        assert!(session.child_pid.is_some());
        assert!(
            session.wait_for_text(&FARMAN_DARK, "FARMAN_CMD_CONPTY_OK", Duration::from_secs(15)),
            "CMD output was not received through ConPTY"
        );
    }

    #[test]
    fn conpty_powershell_roundtrip() {
        let mut session = Session::spawn(
            test_options(
                "powershell.exe",
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-Command",
                    "Write-Output FARMAN_PS_CONPTY_OK",
                ],
            ),
            Arc::new(|_| {}),
        )
        .expect("spawn PowerShell through ConPTY");
        assert!(
            session.wait_for_text(&FARMAN_DARK, "FARMAN_PS_CONPTY_OK", Duration::from_secs(25)),
            "PowerShell output was not received through ConPTY"
        );
    }

    #[test]
    fn keyboard_input_roundtrip() {
        let mut session = Session::spawn(
            test_options("cmd.exe", &["/D", "/Q"]),
            Arc::new(|_| {}),
        )
        .expect("spawn interactive cmd");
        // Give cmd time to initialize, then send a command with CR exactly as
        // the keyboard encoder would.
        thread::sleep(Duration::from_millis(500));
        session.write_text("echo FARMAN_INPUT_ROUNDTRIP\r");
        assert!(
            session.wait_for_text(
                &FARMAN_DARK,
                "FARMAN_INPUT_ROUNDTRIP",
                Duration::from_secs(15)
            ),
            "keystrokes did not reach the shell"
        );
        session.write_text("exit\r");
    }

    #[test]
    fn resize_reaches_vt_state_and_pty() {
        let mut session = Session::spawn(
            test_options("cmd.exe", &["/D", "/Q"]),
            Arc::new(|_| {}),
        )
        .expect("spawn cmd");
        session.resize(120, 40, 11, 22);
        let snapshot = session.snapshot(&FARMAN_DARK);
        assert_eq!(snapshot.cols, 120);
        assert_eq!(snapshot.rows.len(), 40);
        session.close();
    }

    #[test]
    fn shutdown_kills_child() {
        let mut session = Session::spawn(
            test_options("cmd.exe", &["/D", "/Q"]),
            Arc::new(|_| {}),
        )
        .expect("spawn cmd");
        thread::sleep(Duration::from_millis(400));
        session.close();
        let deadline = Instant::now() + Duration::from_secs(8);
        while Instant::now() < deadline && session.exit.is_none() {
            session.poll(&FARMAN_DARK);
            thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            session.exit.is_some(),
            "child process did not terminate after close()"
        );
    }

    #[test]
    fn program_discovery_finds_cmd() {
        assert!(find_program("cmd.exe").is_some());
    }

    mod vt {
        use super::*;
        use alacritty_terminal::event::VoidListener;
        use alacritty_terminal::term::cell::Flags;
        use alacritty_terminal::vte::ansi::{Color, NamedColor};

        fn term(cols: usize, rows: usize) -> Term<VoidListener> {
            Term::new(
                TermConfig::default(),
                &Size { rows, cols },
                VoidListener,
            )
        }

        fn feed(term: &mut Term<VoidListener>, bytes: &[u8]) {
            let mut processor: Processor = Processor::new();
            processor.advance(term, bytes);
        }

        #[test]
        fn sgr_16_256_and_truecolor_are_stored_in_the_grid() {
            let mut t = term(20, 3);
            feed(&mut t, b"\x1b[31;44mA\x1b[38;5;196mB\x1b[38;2;1;2;3mC");
            assert_eq!(t.grid()[Line(0)][Column(0)].fg, Color::Named(NamedColor::Red));
            assert_eq!(t.grid()[Line(0)][Column(0)].bg, Color::Named(NamedColor::Blue));
            assert_eq!(t.grid()[Line(0)][Column(1)].fg, Color::Indexed(196));
            assert_eq!(
                t.grid()[Line(0)][Column(2)].fg,
                Color::Spec(VteRgb { r: 1, g: 2, b: 3 })
            );
        }

        #[test]
        fn cursor_positioning_and_movement_follow_vt_sequences() {
            let mut t = term(20, 5);
            feed(&mut t, b"\x1b[2;3H");
            assert_eq!(t.grid().cursor.point, Point::new(Line(1), Column(2)));
            feed(&mut t, b"\x1b[2A\x1b[4D");
            assert_eq!(t.grid().cursor.point, Point::new(Line(0), Column(0)));
            feed(&mut t, b"abc");
            assert_eq!(t.grid().cursor.point, Point::new(Line(0), Column(3)));
            assert_eq!(t.grid()[Line(0)][Column(1)].c, 'b');
        }

        #[test]
        fn wide_characters_advance_two_columns_and_mark_spacers() {
            let mut t = term(20, 3);
            feed(&mut t, "界".as_bytes());
            assert_eq!(t.grid().cursor.point.column, Column(2));
            assert!(t.grid()[Line(0)][Column(0)].flags.contains(Flags::WIDE_CHAR));
            assert!(t.grid()[Line(0)][Column(1)]
                .flags
                .contains(Flags::WIDE_CHAR_SPACER));
            feed(&mut t, "خ".as_bytes());
            assert_eq!(t.grid().cursor.point.column, Column(3));
            assert_eq!(t.grid()[Line(0)][Column(2)].c, 'خ');
        }

        #[test]
        fn wrapping_and_scrolling_work_at_the_right_edge() {
            let mut t = term(10, 3);
            for i in 0..(10 * 3 + 5) {
                feed(&mut t, format!("{i}").as_bytes());
            }
            assert!(t.grid().history_size() > 0);
            assert!(t.grid().display_offset() == 0);
            t.scroll_display(Scroll::PageUp);
            assert!(t.grid().display_offset() > 0);
            t.scroll_display(Scroll::Bottom);
            assert_eq!(t.grid().display_offset(), 0);
        }

        #[test]
        fn resize_updates_vt_dimensions_and_preserves_text() {
            let mut t = term(20, 3);
            feed(&mut t, b"hello");
            t.resize(Size { rows: 10, cols: 40 });
            assert_eq!(t.columns(), 40);
            assert_eq!(t.screen_lines(), 10);
            assert_eq!(t.grid()[Line(0)][Column(0)].c, 'h');
            assert_eq!(t.grid()[Line(0)][Column(4)].c, 'o');
        }

        #[test]
        fn alternate_screen_mode_is_parsed() {
            let mut t = term(20, 3);
            feed(&mut t, b"\x1b[?1049h");
            assert!(t.mode().contains(TermMode::ALT_SCREEN));
            feed(&mut t, b"alt");
            assert_eq!(t.grid()[Line(0)][Column(0)].c, 'a');
            feed(&mut t, b"\x1b[?1049l");
            assert!(!t.mode().contains(TermMode::ALT_SCREEN));
        }

        #[test]
        fn malformed_sequences_do_not_corrupt_or_panic() {
            let mut t = term(20, 3);
            feed(&mut t, b"\x1b[999;999H\x1b[38;5m\x1b]0;terminated\x07");
            feed(&mut t, b"\x1b[?25l\x1b[?25h");
            feed(&mut t, b"\x1b[H");
            feed(&mut t, b"ok");
            assert!(t.grid()[Line(0)][Column(0)].c == 'o');
        }

        #[test]
        fn copy_serialization_preserves_logical_persian_order() {
            let mut t = term(40, 3);
            let logical = "خطا در main.rs:42";
            feed(&mut t, logical.as_bytes());
            let copied = t.bounds_to_string(
                Point::new(Line(0), Column(0)),
                Point::new(Line(0), Column(logical.chars().count() - 1)),
            );
            assert_eq!(copied, logical);
        }

        #[test]
        fn backspace_tab_carriage_return_and_line_feed() {
            let mut t = term(20, 3);
            feed(&mut t, b"abc\x08Z");
            assert_eq!(t.grid()[Line(0)][Column(2)].c, 'Z');
            feed(&mut t, b"\r\nX");
            assert_eq!(t.grid()[Line(1)][Column(0)].c, 'X');
            feed(&mut t, b"\tY");
            assert_eq!(t.grid()[Line(1)][Column(8)].c, 'Y');
        }
    }
}

