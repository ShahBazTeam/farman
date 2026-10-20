//! Immutable, renderer-facing snapshot of one terminal viewport.
//!
//! The snapshot is derived from `alacritty_terminal` but contains only plain
//! data. The logical grid, cursor coordinates and selection coordinates are all
//! stored unchanged; visual bidi ordering is computed later by the text layer.

use crate::types::Rgb;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellFlags {
    pub inverse: bool,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub wide: bool,
    pub wide_spacer: bool,
    pub leading_wide_spacer: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CellView {
    pub ch: char,
    pub zerowidth: Vec<char>,
    pub fg: Rgb,
    pub bg: Rgb,
    pub flags: CellFlags,
}

impl Default for CellView {
    fn default() -> Self {
        Self {
            ch: ' ',
            zerowidth: Vec::new(),
            fg: Rgb::new(0xd7, 0xe2, 0xf0),
            bg: Rgb::new(0x08, 0x0d, 0x18),
            flags: CellFlags::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowView {
    pub cells: Vec<CellView>,
    pub wrapped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShapeView {
    Block,
    Beam,
    Underline,
    HollowBlock,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorView {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShapeView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionView {
    /// Viewport-relative start (top/left in logical coordinates).
    pub start_row: usize,
    pub start_col: usize,
    /// Viewport-relative end (bottom/right in logical coordinates).
    pub end_row: usize,
    pub end_col: usize,
    pub block: bool,
}

impl SelectionView {
    pub fn contains(&self, row: usize, col: usize) -> bool {
        if row < self.start_row || row > self.end_row {
            return false;
        }
        if self.block {
            return col >= self.start_col.min(self.end_col)
                && col <= self.start_col.max(self.end_col);
        }
        let after_start = row > self.start_row || col >= self.start_col;
        let before_end = row < self.end_row || col <= self.end_col;
        after_start && before_end
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: Vec<RowView>,
    pub cursor: Option<CursorView>,
    pub selection: Option<SelectionView>,
    pub display_offset: usize,
    pub alt_screen: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
    pub mouse_reporting: bool,
    pub mouse_motion: bool,
    pub sgr_mouse: bool,
    pub utf8_mouse: bool,
    pub generation: u64,
    /// Stable id of the session this snapshot came from. Renderer caches must
    /// not confuse two tabs whose local generation counters happen to match.
    pub session_id: u64,
}

impl Snapshot {
    pub fn empty(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows: vec![
                RowView {
                    cells: vec![CellView::default(); cols],
                    wrapped: false,
                };
                rows
            ],
            cursor: None,
            selection: None,
            display_offset: 0,
            alt_screen: false,
            application_cursor: false,
            bracketed_paste: false,
            mouse_reporting: false,
            mouse_motion: false,
            sgr_mouse: false,
            utf8_mouse: false,
            generation: 0,
            session_id: 0,
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn cell(&self, row: usize, col: usize) -> Option<&CellView> {
        self.rows.get(row).and_then(|r| r.cells.get(col))
    }

    pub fn is_selected(&self, row: usize, col: usize) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|s| s.contains(row, col))
    }

    /// Extract a row's visible text in logical order. This is used by tests and
    /// diagnostics; it is never used to reorder shell input.
    pub fn logical_row_text(&self, row: usize) -> String {
        let Some(r) = self.rows.get(row) else {
            return String::new();
        };
        let mut out = String::new();
        for cell in &r.cells {
            if cell.flags.wide_spacer || cell.flags.leading_wide_spacer {
                continue;
            }
            if cell.flags.hidden {
                out.push(' ');
                continue;
            }
            out.push(cell.ch);
            for ch in &cell.zerowidth {
                out.push(*ch);
            }
        }
        out
    }
}
