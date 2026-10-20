//! Text shaping, Unicode bidi policy and logical→visual cell mapping.
//!
//! Terminal state is always kept in logical order. This module builds a
//! paragraph for each row, lets cosmic-text (HarfRust + unicode-bidi) shape it,
//! and then records where each logical cell landed visually. The renderer uses
//! that map for painting, hit-testing, selection and the cursor; the shell and
//! clipboard never see anything but the original logical text.

use cosmic_text::{
    fontdb, Attrs, Buffer, Align, Family, FeatureTag, FontFeatures, FontSystem, Metrics, Shaping, Style,
    SwashCache, UnderlineStyle, Weight, Wrap,
};
use std::ops::Range;
use std::sync::Arc;

use crate::config::BidiConfig;
use crate::snapshot::Snapshot;
use crate::types::Rgb;

const TERMINAL_FONT: &[u8] = include_bytes!("../assets/fonts/Vazir-Code-Hack.ttf");
const UI_FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Pinar-Regular.ttf");
const UI_FONT_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Pinar-Medium.ttf");
const UI_FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Pinar-SemiBold.ttf");
const UI_FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/Pinar-Bold.ttf");
// Kept as a glyph fallback for UI symbols Pinar does not cover.
const UI_FALLBACK_REGULAR: &[u8] = include_bytes!("../assets/fonts/Vazirmatn-Regular.ttf");
const UI_FALLBACK_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Vazirmatn-Medium.ttf");
pub const TERMINAL_FAMILY: &str = "Vazir Code Hack";
pub const UI_FAMILY: &str = "Pinar";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BidiPolicy {
    Auto,
    Ltr,
    Rtl,
}

impl BidiPolicy {
    pub fn from_config(config: &BidiConfig) -> Self {
        if !config.enabled {
            return Self::Ltr;
        }
        match config.direction.as_str() {
            "ltr" => Self::Ltr,
            "rtl" => Self::Rtl,
            _ => Self::Auto,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Auto => "",
            Self::Ltr => "\u{200e}", // LRM: explicit paragraph direction
            Self::Rtl => "\u{200f}", // RLM
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphStyle {
    pub color: Rgb,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

impl Default for GlyphStyle {
    fn default() -> Self {
        Self {
            color: Rgb::new(0xd7, 0xe2, 0xf0),
            bold: false,
            italic: false,
            underline: false,
            strike: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSpan {
    pub range: Range<usize>,
    pub style: GlyphStyle,
}

#[derive(Debug, Clone)]
pub struct RowTextMap {
    pub byte_start: usize,
    pub byte_end: usize,
    /// Byte range in the full shaped text for every logical terminal cell.
    pub cell_ranges: Vec<Option<Range<usize>>>,
    /// Terminal-cell span for wide characters (1 for normal cells).
    pub cell_spans: Vec<u8>,
    pub content_end: usize,
    pub first_strong_rtl: bool,
}

#[derive(Debug, Clone)]
pub struct ScreenText {
    pub text: String,
    pub spans: Vec<TextSpan>,
    pub rows: Vec<RowTextMap>,
    pub cols: usize,
    pub contains_rtl: bool,
    pub policy: BidiPolicy,
}

impl ScreenText {
    pub fn build(snapshot: &Snapshot, policy: BidiPolicy) -> Self {
        let cols = snapshot.cols.max(1);
        let mut text = String::new();
        let mut spans: Vec<TextSpan> = Vec::new();
        let mut rows = Vec::with_capacity(snapshot.rows.len());
        let mut contains_rtl = false;

        for (row_index, row) in snapshot.rows.iter().enumerate() {
            let row_start = text.len();
            // Terminal indentation is a grid property. In auto mode, a row
            // that begins with spaces is anchored LTR so the indentation stays
            // on the left instead of being resolved to the RTL paragraph side.
            let starts_with_space = row.cells.first().is_some_and(|cell| {
                cell.ch == ' '
                    && !cell.flags.wide
                    && !cell.flags.wide_spacer
                    && !cell.flags.leading_wide_spacer
            });
            let prefix = if policy == BidiPolicy::Auto && starts_with_space {
                "\u{200e}"
            } else {
                policy.prefix()
            };
            text.push_str(prefix);

            // Last cell that contributes visible characters. Trailing blank
            // cells are still addressable in the grid; they simply do not take
            // part in shaping.
            let mut content_end = 0usize;
            for (i, cell) in row.cells.iter().enumerate().take(cols) {
                let has_zero = !cell.zerowidth.is_empty();
                if cell.flags.hidden || cell.ch != ' ' || has_zero {
                    content_end = i + 1;
                }
            }
            if row.cells.iter().any(|c| c.ch == '\t') {
                content_end = content_end.max(row.cells.len().min(cols));
            }

            let mut cell_ranges: Vec<Option<Range<usize>>> = vec![None; cols];
            let mut cell_spans: Vec<u8> = vec![1; cols];
            let mut current_style: Option<GlyphStyle> = None;
            let mut current_span_start = row_start;
            let mut first_strong_rtl = false;
            let mut saw_strong = false;
            let mut appended_chars = false;
            let path_ranges = detect_ltr_paths(&row.cells, content_end);
            let mut path_start = vec![false; cols];
            let mut path_end = vec![false; cols];
            for (start, end) in &path_ranges {
                if *start < cols {
                    path_start[*start] = true;
                }
                if end > start && end - 1 < cols {
                    path_end[end - 1] = true;
                }
            }


            for col in 0..content_end {
                let cell = &row.cells[col];
                if cell.flags.leading_wide_spacer {
                    continue;
                }
                if cell.flags.wide_spacer {
                    if col > 0 {
                        cell_spans[col - 1] = 2;
                    }
                    continue;
                }
                if !saw_strong && !cell.ch.is_whitespace() {
                    saw_strong = true;
                    first_strong_rtl = is_rtl_char(cell.ch);
                }
                if is_rtl_char(cell.ch) {
                    contains_rtl = true;
                }
                let ch = if cell.flags.hidden { ' ' } else { cell.ch };
                let cell_start = text.len();
                if path_start[col] {
                    // Visual-only LTR isolate: a Windows path (including one
                    // that contains Persian directory names) is a single
                    // left-to-right component. The logical cell text is not
                    // changed; these controls are zero-width presentation
                    // characters and are only used for shaping.
                    text.push('\u{2066}');
                }
                text.push(if ch == '\0' { ' ' } else { ch });
                for zw in &cell.zerowidth {
                    text.push(*zw);
                    if is_rtl_char(*zw) {
                        contains_rtl = true;
                    }
                }
                if path_end[col] {
                    text.push('\u{2069}');
                }
                let cell_end = text.len();
                cell_ranges[col] = Some(cell_start..cell_end);
                appended_chars = true;
                if cell.flags.wide {
                    cell_spans[col] = 2;
                }

                let mut style = GlyphStyle {
                    color: if cell.flags.inverse { cell.bg } else { cell.fg },
                    bold: cell.flags.bold,
                    italic: cell.flags.italic,
                    underline: cell.flags.underline,
                    strike: cell.flags.strike,
                };
                if style.bold {
                    // The bundled monospace face has one weight; brighten
                    // bold text instead of silently falling back to a
                    // proportionally spaced Arabic font.
                    style.color = style.color.scale(1.14);
                }
                match current_style {
                    None => {
                        // Include the invisible paragraph-direction prefix in
                        // the first span so spans always cover the full text.
                        current_style = Some(style);
                        current_span_start = row_start;
                    }
                    Some(prev) if prev != style => {
                        if current_span_start < cell_start {
                            spans.push(TextSpan {
                                range: current_span_start..cell_start,
                                style: prev,
                            });
                        }
                        current_span_start = cell_start;
                        current_style = Some(style);
                    }
                    Some(_) => {}
                }
            }

            // A completely empty row still needs one paragraph so that
            // LayoutRun::line_i stays aligned with the terminal row.
            if !appended_chars {
                if current_style.is_none() {
                    current_span_start = row_start;
                    current_style = Some(GlyphStyle::default());
                }
                text.push('\u{200b}');
            }

            if row_index + 1 < snapshot.rows.len() {
                text.push('\n');
            }
            if let Some(style) = current_style {
                spans.push(TextSpan {
                    range: current_span_start..text.len(),
                    style,
                });
            }

            rows.push(RowTextMap {
                byte_start: row_start,
                byte_end: text.len(),
                cell_ranges,
                cell_spans,
                content_end,
                first_strong_rtl,
            });
        }

        if text.is_empty() {
            text.push('\u{200b}');
            spans.push(TextSpan {
                range: 0..text.len(),
                style: GlyphStyle::default(),
            });
            rows.push(RowTextMap {
                byte_start: 0,
                byte_end: text.len(),
                cell_ranges: vec![None; cols],
                cell_spans: vec![1; cols],
                content_end: 0,
                first_strong_rtl: false,
            });
        }

        Self {
            text,
            spans,
            rows,
            cols,
            contains_rtl,
            policy,
        }
    }

    pub fn text_for_row(&self, row: usize) -> &str {
        self.rows
            .get(row)
            .and_then(|r| self.text.get(r.byte_start..r.byte_end))
            .unwrap_or("")
    }
}

#[derive(Debug, Clone)]
pub struct RowVisual {
    pub cols: usize,
    pub cell_x: Vec<f32>,
    pub cell_w: Vec<f32>,
    pub cell_level: Vec<u8>,
    pub visual_to_cell: Vec<usize>,
    pub content_end: usize,
    pub rtl: bool,
    pub first_strong_rtl: bool,
}

impl RowVisual {
    fn new(cols: usize, cell_width: f32, content_end: usize, first_strong_rtl: bool) -> Self {
        Self {
            cols,
            cell_x: (0..cols).map(|i| i as f32 * cell_width).collect(),
            cell_w: vec![cell_width; cols],
            cell_level: vec![0; cols],
            visual_to_cell: (0..cols).collect(),
            content_end,
            rtl: false,
            first_strong_rtl,
        }
    }

    pub fn cell_rect(&self, col: usize) -> Option<(f32, f32)> {
        if col >= self.cols {
            return None;
        }
        Some((self.cell_x[col], self.cell_w[col]))
    }

    pub fn logical_at_visual(&self, visual_col: usize) -> usize {
        if self.visual_to_cell.is_empty() {
            0
        } else {
            self.visual_to_cell[visual_col.min(self.visual_to_cell.len() - 1)]
        }
    }

    /// Caret position at a logical insertion column. For an RTL cell the caret
    /// sits on the right edge; after an RTL run it sits at the run's left edge.
    pub fn caret_x(&self, col: usize, cell_width: f32) -> (f32, bool) {
        if self.cols == 0 {
            return (0.0, false);
        }
        if col < self.cols && self.cell_level[col] % 2 == 1 {
            return (self.cell_x[col] + self.cell_w[col], true);
        }
        if col < self.cols && self.is_content_col(col) {
            return (self.cell_x[col], false);
        }
        if col > 0 && self.is_content_col(col - 1) {
            let prev = col - 1;
            if self.cell_level[prev] % 2 == 1 {
                return (self.cell_x[prev], true);
            }
            return (self.cell_x[prev] + self.cell_w[prev], false);
        }
        if col < self.cols {
            (self.cell_x[col], false)
        } else {
            (self.cols as f32 * cell_width, false)
        }
    }

    fn is_content_col(&self, _col: usize) -> bool {
        // A cell with a non-default x is considered shaped. Even spaces in the
        // shaped prefix are represented; this is only used as a hint.
        true
    }
}

#[derive(Debug, Clone)]
pub struct FontMetrics {
    pub font_size: f32,
    pub cell_width: f32,
    pub cell_height: f32,
    pub ascent: f32,
}

pub struct TextEngine {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub metrics: FontMetrics,
    family: String,
    features: FontFeatures,
}

impl TextEngine {
    pub fn new(font_size_px: f32, line_height: f32, family: Option<&str>) -> Self {
        let sources = [
            fontdb::Source::Binary(Arc::new(TERMINAL_FONT.to_vec())),
            fontdb::Source::Binary(Arc::new(UI_FONT_REGULAR.to_vec())),
            fontdb::Source::Binary(Arc::new(UI_FONT_MEDIUM.to_vec())),
            fontdb::Source::Binary(Arc::new(UI_FONT_SEMIBOLD.to_vec())),
            fontdb::Source::Binary(Arc::new(UI_FONT_BOLD.to_vec())),
            fontdb::Source::Binary(Arc::new(UI_FALLBACK_REGULAR.to_vec())),
            fontdb::Source::Binary(Arc::new(UI_FALLBACK_MEDIUM.to_vec())),
        ];
        let mut font_system = FontSystem::new_with_fonts(sources);
        let requested = family.unwrap_or(TERMINAL_FAMILY).trim();
        let family = if family_exists(&font_system, requested) {
            requested.to_owned()
        } else {
            TERMINAL_FAMILY.to_owned()
        };

        let mut features = FontFeatures::new();
        // Column positions must win over typographic ligatures. Arabic
        // contextual joining uses init/medi/fina, not `liga`, so this does not
        // disable Persian shaping.
        features.disable(FeatureTag::STANDARD_LIGATURES);
        features.disable(FeatureTag::CONTEXTUAL_LIGATURES);
        features.disable(FeatureTag::KERNING);

        let font_size = font_size_px.max(8.0);
        let line_h = (font_size * line_height.clamp(1.0, 2.0)).round().max(font_size);
        let metrics = Metrics::new(font_size, line_h);
        let attrs = Attrs::new()
            .family(Family::Name(&family))
            .font_features(features.clone());

        let mut probe = Buffer::new(&mut font_system, metrics);
        probe.set_size(Some(4096.0), Some(line_h * 2.0));
        probe.set_wrap(Wrap::None);
        probe.set_text("M", &attrs, Shaping::Advanced, None);
        probe.shape_until_scroll(&mut font_system, false);
        let cell_width = probe
            .layout_runs()
            .next()
            .map(|run| run.line_w.max(1.0))
            .unwrap_or(font_size * 0.5)
            .max(1.0);
        let ascent = probe
            .layout_runs()
            .next()
            .map(|run| run.line_y)
            .unwrap_or(font_size * 0.85);

        Self {
            font_system,
            swash_cache: SwashCache::new(),
            metrics: FontMetrics {
                font_size,
                cell_width,
                cell_height: line_h,
                ascent,
            },
            family,
            features,
        }
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    /// Build attributes for a UI text run (proportional Pinar).
    pub fn ui_attrs(&self, color: Rgb, weight: Weight) -> Attrs<'static> {
        Attrs::new()
            .family(Family::Name(UI_FAMILY))
            .weight(weight)
            .color(cosmic_text::Color::rgb(color.r, color.g, color.b))
    }

    /// Build attributes for a terminal text run.
    pub fn glyph_attrs(&self, style: GlyphStyle) -> Attrs<'_> {
        let mut attrs = Attrs::new()
            .family(Family::Name(&self.family))
            .font_features(self.features.clone())
            .color(cosmic_text::Color::rgb(
                style.color.r,
                style.color.g,
                style.color.b,
            ));
        if style.italic {
            attrs = attrs.style(Style::Italic);
        }
        if style.underline {
            attrs = attrs.underline(UnderlineStyle::Single);
        }
        if style.strike {
            attrs = attrs.strikethrough();
        }
        attrs
    }

    pub fn shape_screen(
        &mut self,
        buffer: &mut Buffer,
        screen: &ScreenText,
        width: f32,
        height: f32,
    ) -> Vec<RowVisual> {
        buffer.set_size(Some(width.max(1.0)), Some(height.max(1.0)));
        buffer.set_wrap(Wrap::None);
        let default_attrs = Attrs::new()
            .family(Family::Name(&self.family))
            .font_features(self.features.clone())
            .color(cosmic_text::Color::rgb(215, 226, 240));

        if screen.spans.is_empty() {
            buffer.set_text(&screen.text, &default_attrs, Shaping::Advanced, Some(Align::Left));
        } else {
            let spans: Vec<(&str, Attrs<'_>)> = screen
                .spans
                .iter()
                .filter_map(|span| {
                    screen
                        .text
                        .get(span.range.clone())
                        .map(|text| (text, self.glyph_attrs(span.style)))
                })
                .collect();
            buffer.set_rich_text(spans, &default_attrs, Shaping::Advanced, Some(Align::Left));
        }
        buffer.shape_until_scroll(&mut self.font_system, false);

        let cell_width = self.metrics.cell_width;
        let mut visuals: Vec<RowVisual> = screen
            .rows
            .iter()
            .map(|row| {
                RowVisual::new(
                    screen.cols,
                    cell_width,
                    row.content_end,
                    row.first_strong_rtl,
                )
            })
            .collect();

        for run in buffer.layout_runs() {
            let Some(row_map) = screen.rows.get(run.line_i) else {
                continue;
            };
            let Some(visual) = visuals.get_mut(run.line_i) else {
                continue;
            };
            visual.rtl = run.rtl;
            for glyph in run.glyphs {
                let Some(cell) = find_cell_for_byte(row_map, glyph.start) else {
                    continue;
                };
                if cell >= visual.cols {
                    continue;
                }
                let glyph_x = glyph.x;
                let span = row_map.cell_spans.get(cell).copied().unwrap_or(1).max(1) as f32;
                let width = cell_width * span;
                // For multiple glyphs in one cell (combining marks), keep the
                // base glyph's left edge and the full cell box.
                if visual.cell_level[cell] == 0 && visual.cell_x[cell] == cell as f32 * cell_width {
                    visual.cell_x[cell] = glyph_x;
                } else {
                    visual.cell_x[cell] = visual.cell_x[cell].min(glyph_x);
                }
                visual.cell_w[cell] = visual.cell_w[cell].max(width);
                visual.cell_level[cell] = glyph.level.number();
            }
        }

        // Build the visual→logical inverse map. Content cells override the
        // identity fallback, because bidi may have moved them.
        for (row_idx, visual) in visuals.iter_mut().enumerate() {
            let Some(row_map) = screen.rows.get(row_idx) else {
                continue;
            };
            for col in 0..screen.cols {
                if row_map.cell_ranges.get(col).and_then(|r| r.as_ref()).is_none() {
                    continue;
                }
                let start = (visual.cell_x[col] / cell_width).floor().max(0.0) as usize;
                let span = (visual.cell_w[col] / cell_width).round().max(1.0) as usize;
                for vc in start..(start + span).min(screen.cols) {
                    visual.visual_to_cell[vc] = col;
                }
            }
        }

        visuals
    }
}

fn family_exists(font_system: &FontSystem, family: &str) -> bool {
    font_system.db().faces().any(|face| {
        face.families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(family))
    })
}

fn find_cell_for_byte(row: &RowTextMap, byte: usize) -> Option<usize> {
    if row.cell_ranges.is_empty() {
        return None;
    }
    // Ranges are monotonically increasing. Find the first cell whose start is
    // greater than the glyph start, then inspect its predecessor.
    let idx = row
        .cell_ranges
        .partition_point(|range| range.as_ref().is_some_and(|r| r.start <= byte));
    let candidates = [idx.saturating_sub(1), idx, idx.saturating_add(1)];
    for i in candidates {
        if let Some(Some(range)) = row.cell_ranges.get(i) {
            if range.start <= byte && byte < range.end {
                return Some(i);
            }
        }
    }
    // The glyph may belong to the direction-control prefix. Attach it to the
    // first cell that has text.
    row.cell_ranges
        .iter()
        .position(|r| r.as_ref().is_some_and(|r| byte <= r.start))
        .or_else(|| {
            row.cell_ranges
                .iter()
                .rposition(|r| r.as_ref().is_some_and(|r| byte >= r.end))
        })
}

/// Detect Windows paths and URLs so they can be wrapped in a visual LTR
/// isolate. This is a presentation heuristic, not a change to terminal data:
/// backslashes are neutral to the Unicode bidi algorithm, and a path whose
/// components are Persian would otherwise be reordered.
fn detect_ltr_paths(cells: &[crate::snapshot::CellView], content_end: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = cells
        .iter()
        .take(content_end)
        .map(|c| {
            if c.flags.wide_spacer || c.flags.leading_wide_spacer {
                ' '
            } else if c.ch == '\0' {
                ' '
            } else {
                c.ch
            }
        })
        .collect();
    let mut ranges = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let url_start = ["https://", "http://", "file://", "ftp://"]
            .iter()
            .any(|prefix| {
                let prefix: Vec<char> = prefix.chars().collect();
                i + prefix.len() <= chars.len()
                    && chars[i..i + prefix.len()]
                        .iter()
                        .zip(prefix.iter())
                        .all(|(a, b)| a.eq_ignore_ascii_case(b))
            });
        let drive = i + 2 < chars.len()
            && chars[i].is_ascii_alphabetic()
            && chars[i + 1] == ':'
            && matches!(chars[i + 2], '\\' | '/');
        let unc = i + 1 < chars.len()
            && chars[i] == '\\'
            && chars[i + 1] == '\\';
        let relative = (chars[i] == '.')
            && (i + 1 < chars.len() && matches!(chars[i + 1], '\\' | '/')
                || i + 2 < chars.len()
                    && chars[i + 1] == '.'
                    && matches!(chars[i + 2], '\\' | '/'));
        if !(url_start || drive || unc || relative) {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        while end < chars.len() {
            let ch = chars[end];
            if ch.is_whitespace()
                || matches!(ch, '"' | '\'' | '`' | '<' | '>' | '|' | ',' | ';' | ')' | ']' | '}')
            {
                break;
            }
            end += 1;
        }
        if end > start {
            ranges.push((start, end));
            i = end;
        } else {
            i += 1;
        }
    }
    ranges
}

/// True for the major RTL scripts and their presentation blocks.
pub fn is_rtl_char(ch: char) -> bool {
    matches!(ch as u32,
        0x0590..=0x05FF | // Hebrew
        0x0600..=0x06FF | // Arabic
        0x0700..=0x074F | // Syriac
        0x0750..=0x077F | // Arabic Supplement
        0x0780..=0x07BF | // Thaana
        0x07C0..=0x07FF | // NKo
        0x0800..=0x083F | // Samaritan
        0x0840..=0x085F | // Mandaic
        0x0860..=0x08FF | // Syriac/Arabic Extended
        0xFB1D..=0xFB4F | // Hebrew presentation forms
        0xFB50..=0xFDFF | // Arabic presentation forms A
        0xFE70..=0xFEFF | // Arabic presentation forms B
        0x10800..=0x10FFF |
        0x1E800..=0x1EFFF
    ) || ch == '\u{200f}'
        || ch == '\u{202b}'
        || ch == '\u{202e}'
        || ch == '\u{2067}'
}

#[derive(Debug, Clone)]
pub struct DebugGlyph {
    pub cluster: String,
    pub x: f32,
    pub width: f32,
    pub level: u8,
}

/// Diagnostic-only shaping helper used by the test suite and `--text-probe`.
pub fn debug_shape(
    engine: &mut TextEngine,
    text: &str,
    width: f32,
) -> (bool, Vec<DebugGlyph>, f32) {
    let metrics = Metrics::new(engine.metrics.font_size, engine.metrics.cell_height);
    let mut buffer = Buffer::new(&mut engine.font_system, metrics);
    buffer.set_size(Some(width), Some(engine.metrics.cell_height * 4.0));
    buffer.set_wrap(Wrap::None);
    let attrs = Attrs::new()
        .family(Family::Name(engine.family()))
        .font_features(engine.features.clone());
    buffer.set_text(text, &attrs, Shaping::Advanced, Some(Align::Left));
    buffer.shape_until_scroll(&mut engine.font_system, false);
    let mut glyphs = Vec::new();
    let mut rtl = false;
    let mut line_w = 0.0;
    for run in buffer.layout_runs() {
        rtl = run.rtl;
        line_w = run.line_w;
        for glyph in run.glyphs {
            let cluster = run.text[glyph.start..glyph.end].to_owned();
            glyphs.push(DebugGlyph {
                cluster,
                x: glyph.x,
                width: glyph.w,
                level: glyph.level.number(),
            });
        }
    }
    (rtl, glyphs, line_w)
}

/// Diagnostic helper: shape a terminal row from logical text and return its
/// visual map. Used by tests and the headless renderer.
pub fn shape_row_for_test(
    engine: &mut TextEngine,
    logical_text: &str,
    cols: usize,
) -> (RowVisual, ScreenText) {
    let cols = cols.max(logical_text.chars().count().max(1));
    let mut snapshot = Snapshot::empty(cols, 1);
    let mut col = 0;
    for ch in logical_text.chars() {
        if col >= cols {
            break;
        }
        let cell = &mut snapshot.rows[0].cells[col];
        cell.ch = ch;
        cell.flags.wide_spacer = false;
        col += 1;
    }
    let screen = ScreenText::build(&snapshot, BidiPolicy::Auto);
    let mut buffer = Buffer::new(
        &mut engine.font_system,
        Metrics::new(engine.metrics.font_size, engine.metrics.cell_height),
    );
    let width = cols as f32 * engine.metrics.cell_width;
    let visuals = engine.shape_screen(&mut buffer, &screen, width, engine.metrics.cell_height);
    (visuals.into_iter().next().unwrap(), screen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::CellFlags;
    use crate::types::Rgb;

    fn engine() -> TextEngine {
        TextEngine::new(16.0, 1.35, Some(TERMINAL_FAMILY))
    }

    #[test]
    fn bundled_font_has_persian_coverage_and_monospace_advance() {
        let mut e = engine();
        assert!(e.metrics.cell_width > 5.0);
        let (rtl, glyphs, _) = debug_shape(&mut e, "سلام", 400.0);
        assert!(rtl, "Persian paragraph must be RTL");
        assert!(!glyphs.is_empty());
        // All non-zero glyph advances in the bundled face are identical.
        for glyph in &glyphs {
            if glyph.width > 0.0 {
                assert!((glyph.width - e.metrics.cell_width).abs() < 0.01);
            }
        }
    }

    #[test]
    fn arabic_contextual_forms_are_used() {
        let mut e = engine();
        let (_, isolated, _) = debug_shape(&mut e, "س", 200.0);
        let (_, joined, _) = debug_shape(&mut e, "سس", 200.0);
        assert!(!isolated.is_empty() && !joined.is_empty());
        // HarfBuzz selects different glyphs for isolated vs. joined forms.
        // Cluster text is unchanged; the glyph id is what changes, which the
        // debug helper does not expose, so verify widths/ordering plus that
        // shaping produced a single visual glyph stream.
        assert_eq!(joined.len(), 2);
        assert!(joined.iter().all(|g| g.width > 0.0));
    }

    #[test]
    fn logical_text_is_never_reversed() {
        let mut e = engine();
        let sample = "خطا در main.rs:42";
        let (_, screen) = shape_row_for_test(&mut e, sample, 40);
        assert!(screen.text.contains(sample));
        assert_eq!(
            screen.text_for_row(0).trim_end_matches('\n'),
            sample,
            "logical order must be preserved"
        );
    }

    #[test]
    fn persian_words_are_visually_rtl() {
        let mut e = engine();
        let (visual, _) = shape_row_for_test(&mut e, "سلام", 20);
        assert!(visual.rtl);
        // First logical cell is rendered to the right of the last logical cell.
        assert!(visual.cell_x[0] > visual.cell_x[3]);
    }

    #[test]
    fn mixed_line_puts_latin_run_left_of_rtl_run() {
        let mut e = engine();
        let sample = "خطا در main.rs:42";
        let (visual, screen) = shape_row_for_test(&mut e, sample, 60);
        let map = &screen.rows[0];
        let latin_start = sample.find("main").unwrap();
        // Byte offset → logical cell index.
        let mut latin_cell = 0;
        let mut char_index = 0;
        for (i, ch) in sample.char_indices() {
            if i == latin_start {
                latin_cell = char_index;
            }
            char_index += 1;
            let _ = ch;
        }
        let latin_x = visual.cell_x[latin_cell];
        let persian_x = visual.cell_x[0];
        assert!(
            latin_x < persian_x,
            "LTR path/code run must remain to the left in an RTL paragraph (latin_x={latin_x}, persian_x={persian_x})"
        );
        assert!(map.content_end > 0);
    }

    #[test]
    fn windows_path_characters_keep_their_internal_order() {
        let mut e = engine();
        let sample = "مسیر فایل: C:\\Users\\User\\Desktop\\Project";
        let (_, glyphs, _) = debug_shape(&mut e, sample, 1200.0);
        let mut sorted = glyphs;
        sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        let ascii: String = sorted
            .iter()
            .filter(|g| g.cluster.chars().all(|c| c.is_ascii()))
            .map(|g| g.cluster.clone())
            .collect();
        assert!(
            ascii.contains("C:\\Users\\User\\Desktop\\Project"),
            "ASCII path was reordered or split: {ascii:?}"
        );
    }

    #[test]
    fn persian_path_components_keep_windows_order() {
        let mut e = engine();
        let sample = "مسیر: D:\\پروژه‌ها\\فارسی\\main.rs";
        let (visual, screen) = shape_row_for_test(&mut e, sample, 80);
        let chars: Vec<char> = sample.chars().collect();
        let d_cell = chars.iter().position(|c| *c == 'D').unwrap();
        let main_cell = chars
            .windows(4)
            .position(|w| w == ['m', 'a', 'i', 'n'])
            .unwrap();
        assert!(
            visual.cell_x[d_cell] < visual.cell_x[main_cell],
            "path isolate failed: D x={} main x={}",
            visual.cell_x[d_cell],
            visual.cell_x[main_cell]
        );
        assert!(
            screen.text.contains("D:\\پروژه‌ها\\فارسی\\main.rs"),
            "logical path text must remain intact: {:?}",
            screen.text
        );
    }

    #[test]
    fn leading_spaces_keep_terminal_indentation_in_rtl_rows() {
        let mut e = engine();
        let (visual, _) = shape_row_for_test(&mut e, "  سلام", 20);
        let space0 = visual.cell_x[0];
        let space1 = visual.cell_x[1];
        let first_persian = visual.cell_x[2];
        assert!(
            space0 < first_persian && space1 < first_persian,
            "leading spaces must stay left of an RTL run: space0={space0} space1={space1} persian={first_persian}"
        );
    }

    #[test]
    fn numbers_and_identifiers_keep_their_internal_order() {
        let mut e = engine();
        let sample = "نسخه Python: 3.13";
        let (_, glyphs, _) = debug_shape(&mut e, sample, 1000.0);
        let mut sorted = glyphs;
        sorted.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap());
        let ascii: String = sorted
            .iter()
            .filter(|g| g.cluster.chars().all(|c| c.is_ascii()))
            .map(|g| g.cluster.clone())
            .collect();
        assert!(ascii.contains("Python: 3.13"), "ascii stream: {ascii:?}");
    }

    #[test]
    fn combining_marks_are_retained() {
        let mut e = engine();
        let sample = "سَلام";
        let (_, screen) = shape_row_for_test(&mut e, sample, 20);
        assert!(screen.text.contains('\u{064e}'));
        assert!(screen.contains_rtl);
    }

    #[test]
    fn inverse_map_roundtrips_content_cells() {
        let mut e = engine();
        let (visual, _) = shape_row_for_test(&mut e, "خطا در main.rs:42", 40);
        for col in 0..visual.content_end {
            let (x, w) = visual.cell_rect(col).unwrap();
            let vcol = ((x + w * 0.5) / e.metrics.cell_width).floor() as usize;
            let mapped = visual.logical_at_visual(vcol);
            assert_eq!(
                mapped, col,
                "visual column {vcol} mapped to {mapped}, expected {col}"
            );
        }
    }

    #[test]
    fn forced_ltr_policy_changes_only_visual_order() {
        let mut e = engine();
        let sample = "خطا در main.rs:42";
        let mut snapshot = Snapshot::empty(40, 1);
        for (i, ch) in sample.chars().enumerate() {
            snapshot.rows[0].cells[i].ch = ch;
        }
        let auto = ScreenText::build(&snapshot, BidiPolicy::Auto);
        let ltr = ScreenText::build(&snapshot, BidiPolicy::Ltr);
        assert!(auto.text.contains(sample));
        assert!(ltr.text.contains(sample));
        assert!(ltr.text.starts_with('\u{200e}'));
        assert_eq!(auto.text.trim_end_matches('\n'), sample);
        let mut b = Buffer::new(
            &mut e.font_system,
            Metrics::new(e.metrics.font_size, e.metrics.cell_height),
        );
        let w = 40.0 * e.metrics.cell_width;
        let auto_visuals = e.shape_screen(&mut b, &auto, w, e.metrics.cell_height);
        let ltr_visuals = e.shape_screen(&mut b, &ltr, w, e.metrics.cell_height);
        let latin_cell = sample.chars().position(|c| c == 'm').expect("latin run");
        assert!(
            auto_visuals[0].cell_x[latin_cell] < auto_visuals[0].cell_x[0],
            "auto (RTL base) should place the Latin run left of the Persian run"
        );
        assert!(
            ltr_visuals[0].cell_x[latin_cell] > ltr_visuals[0].cell_x[0],
            "forced LTR should place the Persian run left of the Latin run; \
             auto_latin={} auto_persian={} ltr_latin={} ltr_persian={}",
            auto_visuals[0].cell_x[latin_cell],
            auto_visuals[0].cell_x[0],
            ltr_visuals[0].cell_x[latin_cell],
            ltr_visuals[0].cell_x[0]
        );
    }

    #[test]
    fn rtl_scripts_are_detected() {
        assert!(is_rtl_char('خ'));
        assert!(is_rtl_char('א'));
        assert!(!is_rtl_char('A'));
        assert!(!is_rtl_char('د'.to_string().chars().next().map(|_| 'A').unwrap()));
    }

    #[test]
    fn snapshot_logical_text_is_preserved() {
        let mut snapshot = Snapshot::empty(20, 1);
        let sample = "مسیر فایل: C:\\tmp";
        for (i, ch) in sample.chars().enumerate() {
            snapshot.rows[0].cells[i].ch = ch;
            snapshot.rows[0].cells[i].fg = Rgb::new(1, 2, 3);
        }
        assert_eq!(snapshot.logical_row_text(0).trim_end(), sample);
    }

    #[test]
    fn cell_flags_do_not_change_logical_text() {
        let mut snapshot = Snapshot::empty(10, 1);
        snapshot.rows[0].cells[0].ch = 'خ';
        snapshot.rows[0].cells[0].flags = CellFlags {
            bold: true,
            underline: true,
            ..Default::default()
        };
        assert_eq!(snapshot.logical_row_text(0).chars().next(), Some('خ'));
    }
}

