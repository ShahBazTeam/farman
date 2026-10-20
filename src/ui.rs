//! Native window chrome built from GPU quads and shaped text blocks.
//!
//! The layout is deliberately quiet: a frameless dark window with a custom
//! title bar, a compact tab strip, one restrained terminal card and a minimal
//! status line. All controls are drawn in physical pixels and mirrored by hit
//! regions so keyboard and terminal semantics stay unchanged.

use cosmic_text::Weight;

use crate::profile::Profile;
use crate::renderer::{Quad, TextAlign, TextBlock};
use crate::theme::Theme;
use crate::types::{Rgb, Rgba, Rect};

const TITLE_H: f32 = 36.0;
const TOOLBAR_H: f32 = 42.0;
const STATUS_H: f32 = 26.0;
const PAGE_PAD: f32 = 8.0;
const PANEL_INSET_X: f32 = 12.0;
const PANEL_INSET_Y: f32 = 10.0;
const WINDOW_BUTTON_W: f32 = 46.0;

#[derive(Debug, Clone)]
pub struct TabInfo<'a> {
    pub title: &'a str,
    pub starting: bool,
    pub exited: bool,
}

#[derive(Debug, Clone)]
pub enum UiAction {
    NewTab(Option<String>),
    CloseTab(usize),
    ActivateTab(usize),
    ToggleSettings,
    ToggleProfileMenu,
    CloseOverlay,
    FontDelta(i32),
    SetTheme(String),
    CycleDefaultProfile,
    ToggleBidi,
    ReloadConfig,
    TogglePalette,
    MinimizeWindow,
    ToggleMaximize,
    CloseWindow,
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteCommand {
    NewTab,
    Settings,
    ThemeDark,
    ThemeMidnight,
    ThemeGraphite,
    Bidi,
    Profiles,
    Reload,
}

impl PaletteCommand {
    pub fn action(self) -> UiAction {
        match self {
            Self::NewTab => UiAction::NewTab(None),
            Self::Settings => UiAction::ToggleSettings,
            Self::ThemeDark => UiAction::SetTheme("farman-dark".to_owned()),
            Self::ThemeMidnight => UiAction::SetTheme("midnight".to_owned()),
            Self::ThemeGraphite => UiAction::SetTheme("graphite".to_owned()),
            Self::Bidi => UiAction::ToggleBidi,
            Self::Profiles => UiAction::ToggleProfileMenu,
            Self::Reload => UiAction::ReloadConfig,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PaletteItem {
    pub title: &'static str,
    pub hint: &'static str,
    pub command: PaletteCommand,
}

const PALETTE_ITEMS: [PaletteItem; 8] = [
    PaletteItem {
        title: "New tab",
        hint: "Ctrl+Shift+T",
        command: PaletteCommand::NewTab,
    },
    PaletteItem {
        title: "Settings",
        hint: "Ctrl+,",
        command: PaletteCommand::Settings,
    },
    PaletteItem {
        title: "Theme: FARMAN Dark",
        hint: "Default",
        command: PaletteCommand::ThemeDark,
    },
    PaletteItem {
        title: "Theme: Midnight",
        hint: "Blue",
        command: PaletteCommand::ThemeMidnight,
    },
    PaletteItem {
        title: "Theme: Graphite",
        hint: "Neutral",
        command: PaletteCommand::ThemeGraphite,
    },
    PaletteItem {
        title: "Toggle bidi shaping",
        hint: "Persian / Arabic",
        command: PaletteCommand::Bidi,
    },
    PaletteItem {
        title: "Choose default shell",
        hint: "Profiles",
        command: PaletteCommand::Profiles,
    },
    PaletteItem {
        title: "Reload configuration",
        hint: "config.toml",
        command: PaletteCommand::Reload,
    },
];

pub fn palette_matches(query: &str) -> Vec<PaletteItem> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| term.to_lowercase())
        .collect();
    PALETTE_ITEMS
        .iter()
        .filter(|item| {
            let haystack = format!("{} {}", item.title, item.hint).to_lowercase();
            terms.iter().all(|term| haystack.contains(term))
        })
        .copied()
        .collect()
}

#[derive(Debug, Clone)]
pub struct HitRegion {
    pub rect: Rect,
    pub action: UiAction,
}

pub struct ChromeInput<'a> {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub theme: &'a Theme,
    pub tabs: &'a [TabInfo<'a>],
    pub active: usize,
    pub settings_open: bool,
    pub profile_menu_open: bool,
    pub palette_open: bool,
    pub palette_query: &'a str,
    pub palette_selected: usize,
    pub profiles: &'a [Profile],
    pub selected_profile: &'a str,
    pub font_size: f32,
    pub theme_name: &'a str,
    pub bidi_enabled: bool,
    pub cols: usize,
    pub rows: usize,
    pub status: &'a str,
    pub toast: Option<&'a str>,
    pub mouse: (f32, f32),
    pub config_path: &'a str,
    pub version: &'a str,
    pub active_profile: &'a str,
    pub starting: bool,
    pub session_banner: Option<&'a str>,
    pub maximized: bool,
}

pub struct Chrome {
    pub base_quads: Vec<Quad>,
    pub overlay_quads: Vec<Quad>,
    pub text: Vec<TextBlock>,
    pub images: Vec<Rect>,
    pub hits: Vec<HitRegion>,
    pub terminal_rect: Rect,
    pub drag_region: Rect,
}

#[derive(Debug, Clone, Copy)]
struct Layout {
    title: Rect,
    toolbar: Rect,
    panel: Rect,
    terminal: Rect,
    status: Rect,
}

fn layout(width: f32, height: f32, s: f32) -> Layout {
    let title = Rect::new(0.0, 0.0, width.max(1.0), TITLE_H * s);
    let toolbar = Rect::new(
        0.0,
        title.h,
        width.max(1.0),
        TOOLBAR_H * s,
    );
    let status_h = STATUS_H * s;
    let pad = PAGE_PAD * s;
    let status = Rect::new(0.0, height - status_h, width.max(1.0), status_h);
    let panel_y = toolbar.bottom() + 6.0 * s;
    let panel_bottom = status.y - 6.0 * s;
    let panel = Rect::new(
        pad,
        panel_y,
        (width - pad * 2.0).max(1.0),
        (panel_bottom - panel_y).max(1.0),
    );
    let terminal = Rect::new(
        panel.x + PANEL_INSET_X * s,
        panel.y + PANEL_INSET_Y * s,
        (panel.w - PANEL_INSET_X * 2.0 * s).max(1.0),
        (panel.h - PANEL_INSET_Y * 2.0 * s).max(1.0),
    );
    Layout {
        title,
        toolbar,
        panel,
        terminal,
        status,
    }
}

#[derive(Debug, Clone, Copy)]
struct ToolbarRects {
    profile: Rect,
    settings: Rect,
    tabs_start: f32,
    tabs_end: f32,
}

fn toolbar_rects(width: f32, height: f32, s: f32) -> ToolbarRects {
    let l = layout(width, height, s);
    let size = 30.0 * s;
    let settings = Rect::new(
        width - 12.0 * s - size,
        l.toolbar.y + (l.toolbar.h - size) * 0.5,
        size,
        size,
    );
    let pill_w = (180.0 * s).min((width * 0.24).max(120.0 * s));
    let profile = Rect::new(
        settings.x - 8.0 * s - pill_w,
        l.toolbar.y + (l.toolbar.h - 30.0 * s) * 0.5,
        pill_w,
        30.0 * s,
    );
    ToolbarRects {
        profile,
        settings,
        tabs_start: 12.0 * s,
        tabs_end: (profile.x - 12.0 * s).max(12.0 * s + 1.0),
    }
}

pub fn terminal_rect(width: f32, height: f32, scale: f32) -> Rect {
    layout(width, height, scale.max(0.1)).terminal
}

pub fn build(input: ChromeInput<'_>) -> Chrome {
    let ChromeInput {
        width,
        height,
        scale,
        theme,
        tabs,
        active,
        settings_open,
        profile_menu_open,
        palette_open,
        palette_query,
        palette_selected,
        profiles,
        selected_profile,
        font_size,
        theme_name,
        bidi_enabled,
        cols,
        rows,
        status,
        toast,
        mouse,
        config_path,
        version,
        active_profile,
        starting,
        session_banner,
        maximized,
    } = input;
    let s = scale.max(0.1);
    let l = layout(width, height, s);

    let mut base_quads = Vec::new();
    let mut overlay_quads = Vec::new();
    let mut base_text = Vec::new();
    let mut text = Vec::new();
    let mut images = Vec::new();
    let mut hits = Vec::new();

    paint_backdrop(&mut base_quads, width, height, theme, s);
    paint_title_bar(
        &mut base_quads,
        &mut base_text,
        &mut images,
        &mut hits,
        &l,
        width,
        theme,
        mouse,
        maximized,
        s,
    );
    paint_toolbar(
        &mut base_quads,
        &mut base_text,
        &mut hits,
        &l,
        width,
        height,
        theme,
        tabs,
        active,
        selected_profile,
        settings_open,
        profile_menu_open,
        mouse,
        s,
    );

    if tabs.is_empty() {
        paint_empty_state(
            &mut base_quads,
            &mut base_text,
            &mut images,
            &mut hits,
            l.terminal,
            theme,
            mouse,
            s,
        );
    } else if let Some(banner) = session_banner.filter(|_| !palette_open) {
        paint_session_banner(
            &mut overlay_quads,
            &mut base_text,
            &mut hits,
            l.terminal,
            theme,
            mouse,
            banner,
            s,
        );
    } else if starting && !palette_open {
        paint_starting_pill(
            &mut overlay_quads,
            &mut base_text,
            l.terminal,
            theme,
            active_profile,
            s,
        );
    }

    paint_status_bar(
        &mut base_quads,
        &mut base_text,
        &l,
        theme,
        cols,
        rows,
        bidi_enabled,
        status,
        s,
    );

    if let Some(message) = toast.filter(|_| !palette_open) {
        paint_toast(
            &mut overlay_quads,
            &mut base_text,
            l,
            theme,
            message,
            s,
        );
    }

    if !settings_open && !palette_open {
        text.extend(base_text);
    }

    if profile_menu_open {
        let rects = toolbar_rects(width, height, s);
        paint_profile_menu(
            &mut overlay_quads,
            &mut text,
            &mut hits,
            rects.profile,
            l.toolbar.bottom() + 6.0 * s,
            profiles,
            selected_profile,
            theme,
            mouse,
            width,
            height,
            s,
        );
    }

    if settings_open {
        paint_settings(
            &mut overlay_quads,
            &mut text,
            &mut hits,
            width,
            height,
            theme,
            font_size,
            theme_name,
            bidi_enabled,
            profiles,
            selected_profile,
            config_path,
            version,
            mouse,
            s,
        );
    }

    if palette_open {
        paint_palette(
            &mut overlay_quads,
            &mut text,
            &mut hits,
            width,
            height,
            theme,
            palette_query,
            palette_selected,
            mouse,
            s,
        );
    }

    let drag_region = Rect::new(
        170.0 * s,
        0.0,
        (width - 170.0 * s - WINDOW_BUTTON_W * 3.0 * s).max(1.0),
        TITLE_H * s,
    );

    Chrome {
        base_quads,
        overlay_quads,
        text,
        images,
        hits,
        terminal_rect: l.terminal,
        drag_region,
    }
}

fn alpha(color: Rgb, value: f32) -> Rgba {
    Rgba::with_alpha(color, value)
}

fn paint_backdrop(
    quads: &mut Vec<Quad>,
    width: f32,
    height: f32,
    theme: &Theme,
    s: f32,
) {
    let top = theme.background.blend(theme.header_bg, 0.55);
    let bottom = theme.background.scale(0.42);
    quads.push(Quad::glass(
        Rect::new(0.0, 0.0, width, height),
        Rgba::opaque(top),
        Rgba::opaque(bottom),
        0.0,
    ));
    let border = alpha(theme.border, 0.45);
    quads.push(Quad::rounded(
        Rect::new(0.0, 0.0, width, 1.0 * s),
        border,
        0.0,
    ));
    quads.push(Quad::rounded(
        Rect::new(0.0, height - 1.0 * s, width, 1.0 * s),
        border,
        0.0,
    ));
    quads.push(Quad::rounded(
        Rect::new(0.0, 0.0, 1.0 * s, height),
        border,
        0.0,
    ));
    quads.push(Quad::rounded(
        Rect::new(width - 1.0 * s, 0.0, 1.0 * s, height),
        border,
        0.0,
    ));
}

fn paint_title_bar(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    images: &mut Vec<Rect>,
    hits: &mut Vec<HitRegion>,
    l: &Layout,
    width: f32,
    theme: &Theme,
    mouse: (f32, f32),
    maximized: bool,
    s: f32,
) {
    let title = l.title;
    quads.push(Quad::glass(
        title,
        alpha(theme.header_bg, 0.94),
        alpha(theme.header_bg.blend(theme.background, 0.55), 0.92),
        0.0,
    ));
    quads.push(Quad::rounded(
        Rect::new(0.0, title.bottom() - 1.0 * s, width, 1.0 * s),
        alpha(theme.border, 0.55),
        0.0,
    ));

    let mark = Rect::new(14.0 * s, 6.0 * s, 24.0 * s, 24.0 * s);
    paint_mark(quads, images, mark, theme, s);

    text.push(
        TextBlock::new(
            "FARMAN",
            Rect::new(46.0 * s, 9.0 * s, 76.0 * s, 16.0 * s),
            11.5 * s,
            theme.foreground,
        )
        .weight(Weight::BOLD)
        .width(76.0 * s),
    );
    quads.push(Quad::rounded(
        Rect::new(118.0 * s, 12.0 * s, 1.0 * s, 12.0 * s),
        alpha(theme.border, 0.9),
        0.0,
    ));
    text.push(
        TextBlock::new(
            "TERMINAL",
            Rect::new(127.0 * s, 12.0 * s, 74.0 * s, 12.0 * s),
            7.5 * s,
            theme.muted,
        )
        .weight(Weight::MEDIUM)
        .width(74.0 * s),
    );

    let button_w = WINDOW_BUTTON_W * s;
    let controls_left = width - button_w * 3.0 - 10.0 * s;
    let search_start = 205.0 * s;
    let search_avail = controls_left - search_start - 10.0 * s;
    if search_avail > 170.0 * s {
        let pill_w = (270.0 * s).min(search_avail);
        let pill = Rect::new(
            search_start + (search_avail - pill_w) * 0.5,
            7.0 * s,
            pill_w,
            22.0 * s,
        );
        let hot = pill.contains(mouse.0, mouse.1);
        quads.push(Quad::rounded(
            pill,
            alpha(if hot { theme.accent } else { theme.border }, 0.55),
            7.0 * s,
        ));
        quads.push(Quad::rounded(
            Rect::new(
                pill.x + 1.0 * s,
                pill.y + 1.0 * s,
                (pill.w - 2.0 * s).max(1.0),
                (pill.h - 2.0 * s).max(1.0),
            ),
            alpha(
                theme.foreground,
                if hot { 0.075 } else { 0.040 },
            ),
            6.0 * s,
        ));
        quads.push(Quad::rounded(
            Rect::new(pill.x + 11.0 * s, pill.y + 6.0 * s, 2.0 * s, 10.0 * s),
            alpha(if hot { theme.accent } else { theme.muted }, 0.90),
            1.0 * s,
        ));
        let badge = Rect::new(
            pill.right() - 46.0 * s,
            pill.y + 4.0 * s,
            38.0 * s,
            14.0 * s,
        );
        quads.push(Quad::rounded(
            badge,
            alpha(theme.foreground, 0.055),
            4.0 * s,
        ));
        text.push(
            TextBlock::new(
                "Search commands",
                Rect::new(
                    pill.x + 22.0 * s,
                    pill.y + 5.0 * s,
                    (badge.x - pill.x - 30.0 * s).max(1.0),
                    12.0 * s,
                ),
                8.6 * s,
                if hot { theme.foreground } else { theme.muted },
            )
            .weight(Weight::MEDIUM)
            .width((badge.x - pill.x - 30.0 * s).max(1.0)),
        );
        text.push(
            TextBlock::new(
                "Ctrl K",
                Rect::new(badge.x, badge.y + 1.0 * s, badge.w, 12.0 * s),
                7.2 * s,
                theme.muted,
            )
            .align(TextAlign::Center)
            .weight(Weight::MEDIUM)
            .width(badge.w),
        );
        hits.push(HitRegion {
            rect: pill,
            action: UiAction::TogglePalette,
        });
    }

    let close = Rect::new(width - button_w, 0.0, button_w, title.h);
    let max = Rect::new(width - button_w * 2.0, 0.0, button_w, title.h);
    let min = Rect::new(width - button_w * 3.0, 0.0, button_w, title.h);

    window_button(
        quads,
        text,
        hits,
        min,
        WindowButton::Minimize,
        theme,
        mouse,
        s,
    );
    window_button(
        quads,
        text,
        hits,
        max,
        if maximized {
            WindowButton::Restore
        } else {
            WindowButton::Maximize
        },
        theme,
        mouse,
        s,
    );
    window_button(
        quads,
        text,
        hits,
        close,
        WindowButton::Close,
        theme,
        mouse,
        s,
    );
}

#[derive(Debug, Clone, Copy)]
enum WindowButton {
    Minimize,
    Maximize,
    Restore,
    Close,
}

fn window_button(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    rect: Rect,
    kind: WindowButton,
    theme: &Theme,
    mouse: (f32, f32),
    s: f32,
) {
    let hot = rect.contains(mouse.0, mouse.1);
    let is_close = matches!(kind, WindowButton::Close);
    let background = if hot && is_close {
        alpha(theme.error, 0.90)
    } else if hot {
        alpha(theme.foreground, 0.085)
    } else {
        alpha(theme.foreground, 0.0)
    };
    let button = Rect::new(
        rect.x + 7.0 * s,
        rect.y + 5.0 * s,
        rect.w - 14.0 * s,
        rect.h - 10.0 * s,
    );
    quads.push(Quad::rounded(button, background, 6.0 * s));
    let color = if hot && is_close {
        theme.foreground
    } else if hot {
        theme.bright_foreground
    } else {
        theme.muted
    };
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    match kind {
        WindowButton::Minimize => {
            quads.push(Quad::rounded(
                Rect::new(cx - 5.0 * s, cy - 0.5 * s, 10.0 * s, 1.3 * s),
                Rgba::opaque(color),
                0.5 * s,
            ));
        }
        WindowButton::Maximize => {
            outline(
                quads,
                Rect::new(cx - 5.0 * s, cy - 5.0 * s, 10.0 * s, 10.0 * s),
                color,
                1.1 * s,
                s,
            );
        }
        WindowButton::Restore => {
            outline(
                quads,
                Rect::new(cx - 3.0 * s, cy - 2.0 * s, 8.0 * s, 8.0 * s),
                color,
                1.0 * s,
                s,
            );
            quads.push(Quad::rounded(
                Rect::new(cx - 7.0 * s, cy - 5.5 * s, 6.0 * s, 6.0 * s),
                alpha(theme.header_bg, 0.96),
                0.0,
            ));
            outline(
                quads,
                Rect::new(cx - 7.0 * s, cy - 5.5 * s, 8.0 * s, 8.0 * s),
                color,
                1.0 * s,
                s,
            );
        }
        WindowButton::Close => {
            text.push(
                TextBlock::new(
                    "×",
                    Rect::new(cx - 9.0 * s, cy - 10.0 * s, 18.0 * s, 20.0 * s),
                    17.0 * s,
                    color,
                )
                .align(TextAlign::Center)
                .width(18.0 * s),
            );
        }
    }
    hits.push(HitRegion {
        rect,
        action: match kind {
            WindowButton::Minimize => UiAction::MinimizeWindow,
            WindowButton::Maximize | WindowButton::Restore => UiAction::ToggleMaximize,
            WindowButton::Close => UiAction::CloseWindow,
        },
    });
}

fn outline(quads: &mut Vec<Quad>, rect: Rect, color: Rgb, thickness: f32, _s: f32) {
    let c = Rgba::opaque(color);
    quads.push(Quad::rounded(
        Rect::new(rect.x, rect.y, rect.w, thickness),
        c,
        0.3,
    ));
    quads.push(Quad::rounded(
        Rect::new(rect.x, rect.bottom() - thickness, rect.w, thickness),
        c,
        0.3,
    ));
    quads.push(Quad::rounded(
        Rect::new(rect.x, rect.y, thickness, rect.h),
        c,
        0.3,
    ));
    quads.push(Quad::rounded(
        Rect::new(rect.right() - thickness, rect.y, thickness, rect.h),
        c,
        0.3,
    ));
}

#[allow(clippy::too_many_arguments)]
fn paint_toolbar(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    l: &Layout,
    width: f32,
    height: f32,
    theme: &Theme,
    tabs: &[TabInfo<'_>],
    active: usize,
    selected_profile: &str,
    settings_open: bool,
    profile_menu_open: bool,
    mouse: (f32, f32),
    s: f32,
) {
    let rects = toolbar_rects(width, height, s);
    quads.push(Quad::rounded(
        Rect::new(0.0, l.toolbar.bottom() - 1.0 * s, width, 1.0 * s),
        alpha(theme.border, 0.40),
        0.0,
    ));

    let new_size = 30.0 * s;
    let gap = 4.0 * s;
    let tab_area_end = (rects.tabs_end - new_size - gap).max(rects.tabs_start + 1.0);
    let available = (tab_area_end - rects.tabs_start).max(1.0);
    let count = tabs.len().max(1) as f32;
    let tab_w = (available / count)
        .clamp(84.0 * s, 190.0 * s)
        .min(available);
    let mut x = rects.tabs_start;
    for (index, tab) in tabs.iter().enumerate() {
        if x + tab_w > tab_area_end + 0.5 {
            break;
        }
        let tab_rect = Rect::new(x, l.toolbar.y + 6.0 * s, tab_w, l.toolbar.h - 12.0 * s);
        paint_tab(
            quads, text, hits, tab_rect, index, tab, index == active, theme, mouse, s,
        );
        x += tab_w + gap;
    }
    let new_x = x.min(rects.tabs_end - new_size).max(rects.tabs_start);
    let new_rect = Rect::new(
        new_x,
        l.toolbar.y + (l.toolbar.h - new_size) * 0.5,
        new_size,
        new_size,
    );
    paint_plus_button(quads, text, hits, new_rect, theme, mouse, s);

    paint_profile_pill(
        quads,
        text,
        hits,
        rects.profile,
        selected_profile,
        profile_menu_open,
        theme,
        mouse,
        s,
    );
    paint_settings_button(
        quads,
        hits,
        rects.settings,
        theme,
        mouse,
        settings_open,
        s,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_tab(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    rect: Rect,
    index: usize,
    tab: &TabInfo<'_>,
    selected: bool,
    theme: &Theme,
    mouse: (f32, f32),
    s: f32,
) {
    let hovered = rect.contains(mouse.0, mouse.1);
    if selected {
        quads.push(Quad::glass(
            rect,
            alpha(theme.panel_alt.blend(theme.accent, 0.10), 0.96),
            alpha(theme.panel_bg.blend(theme.accent, 0.05), 0.94),
            9.0 * s,
        ));
        quads.push(Quad::rounded(
            Rect::new(rect.x + 10.0 * s, rect.bottom() - 1.4 * s, (rect.w - 20.0 * s).max(1.0), 1.4 * s),
            alpha(theme.accent, 0.55),
            0.7 * s,
        ));
    } else if hovered {
        quads.push(Quad::rounded(rect, alpha(theme.foreground, 0.055), 9.0 * s));
    }
    quads.push(Quad::rounded(
        rect,
        alpha(if selected { theme.accent } else { theme.border }, if selected { 0.22 } else { 0.45 }),
        9.0 * s,
    ));
    quads.push(Quad::rounded(
        Rect::new(rect.x + 10.0 * s, rect.y + 1.0 * s, (rect.w - 20.0 * s).max(1.0), 1.0 * s),
        alpha(theme.foreground, if selected { 0.08 } else { 0.035 }),
        0.0,
    ));

    let dot_color = if tab.exited {
        theme.error
    } else if tab.starting {
        theme.warning
    } else {
        theme.success
    };
    quads.push(Quad::rounded(
        Rect::new(rect.x + 12.0 * s, rect.y + rect.h * 0.5 - 2.5 * s, 5.0 * s, 5.0 * s),
        Rgba::opaque(dot_color),
        2.5 * s,
    ));
    let title_w = (rect.w - 58.0 * s).max(10.0 * s);
    text.push(
        TextBlock::new(
            truncate(tab.title, 24),
            Rect::new(rect.x + 25.0 * s, rect.y + 8.0 * s, title_w, 15.0 * s),
            10.2 * s,
            theme.foreground,
        )
        .weight(if selected { Weight::MEDIUM } else { Weight::NORMAL })
        .width(title_w),
    );
    hits.push(HitRegion {
        rect: Rect::new(rect.x, rect.y, rect.w, rect.h),
        action: UiAction::ActivateTab(index),
    });
    if selected || hovered {
        let close = Rect::new(
            rect.right() - 23.0 * s,
            rect.y + rect.h * 0.5 - 9.0 * s,
            18.0 * s,
            18.0 * s,
        );
        quads.push(Quad::rounded(
            close,
            alpha(theme.foreground, if close.contains(mouse.0, mouse.1) { 0.10 } else { 0.0 }),
            4.0 * s,
        ));
        text.push(
            TextBlock::new(
                "×",
                Rect::new(close.x, close.y + 1.0 * s, close.w, close.h),
                12.0 * s,
                theme.muted,
            )
            .align(TextAlign::Center)
            .width(close.w),
        );
        hits.push(HitRegion {
            rect: close,
            action: UiAction::CloseTab(index),
        });
    }
}

fn paint_plus_button(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    rect: Rect,
    theme: &Theme,
    mouse: (f32, f32),
    s: f32,
) {
    let hot = rect.contains(mouse.0, mouse.1);
    quads.push(Quad::rounded(
        rect,
        alpha(theme.foreground, if hot { 0.075 } else { 0.0 }),
        8.0 * s,
    ));
    if hot {
        quads.push(Quad::rounded(rect, alpha(theme.border, 0.55), 8.0 * s));
    }
    text.push(
        TextBlock::new(
            "+",
            Rect::new(rect.x, rect.y - 1.0 * s, rect.w, rect.h),
            17.0 * s,
            if hot { theme.foreground } else { theme.muted },
        )
        .align(TextAlign::Center)
        .weight(Weight::MEDIUM)
        .width(rect.w),
    );
    hits.push(HitRegion {
        rect,
        action: UiAction::NewTab(None),
    });
}

fn paint_profile_pill(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    rect: Rect,
    selected_profile: &str,
    open: bool,
    theme: &Theme,
    mouse: (f32, f32),
    s: f32,
) {
    let hot = rect.contains(mouse.0, mouse.1);
    let bg = if open {
        alpha(theme.accent, 0.12)
    } else if hot {
        alpha(theme.foreground, 0.075)
    } else {
        alpha(theme.foreground, 0.04)
    };
    quads.push(Quad::rounded(rect, bg, 9.0 * s));
    quads.push(Quad::rounded(
        rect,
        alpha(if open { theme.accent } else { theme.border }, if open { 0.45 } else { 0.70 }),
        9.0 * s,
    ));
    quads.push(Quad::rounded(
        Rect::new(rect.x + 1.0 * s, rect.y + 1.0 * s, (rect.w - 2.0 * s).max(1.0), 1.0 * s),
        alpha(theme.foreground, 0.055),
        0.0,
    ));
    quads.push(Quad::rounded(
        Rect::new(rect.x + 12.0 * s, rect.y + rect.h * 0.5 - 2.5 * s, 5.0 * s, 5.0 * s),
        Rgba::opaque(theme.success),
        2.5 * s,
    ));
    let name = if selected_profile.is_empty() {
        "Shell"
    } else {
        selected_profile
    };
    let label_w = (rect.w - 52.0 * s).max(10.0 * s);
    text.push(
        TextBlock::new(
            truncate(name, 22),
            Rect::new(rect.x + 24.0 * s, rect.y + 8.0 * s, label_w, 15.0 * s),
            9.8 * s,
            theme.foreground,
        )
        .weight(Weight::MEDIUM)
        .width(label_w),
    );
    text.push(
        TextBlock::new(
            "v",
            Rect::new(rect.right() - 24.0 * s, rect.y + 9.0 * s, 12.0 * s, 12.0 * s),
            8.0 * s,
            theme.muted,
        )
        .align(TextAlign::Center)
        .width(12.0 * s),
    );
    hits.push(HitRegion {
        rect,
        action: UiAction::ToggleProfileMenu,
    });
}

fn paint_settings_button(
    quads: &mut Vec<Quad>,
    hits: &mut Vec<HitRegion>,
    rect: Rect,
    theme: &Theme,
    mouse: (f32, f32),
    open: bool,
    s: f32,
) {
    let hot = rect.contains(mouse.0, mouse.1);
    quads.push(Quad::rounded(
        rect,
        alpha(if open { theme.accent } else { theme.foreground }, if open { 0.13 } else if hot { 0.075 } else { 0.0 }),
        8.0 * s,
    ));
    if open || hot {
        quads.push(Quad::rounded(
            rect,
            alpha(if open { theme.accent } else { theme.border }, if open { 0.42 } else { 0.55 }),
            8.0 * s,
        ));
    }
    let color = if open {
        theme.accent
    } else if hot {
        theme.foreground
    } else {
        theme.muted
    };
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    for i in 0..3 {
        let y = cy + (i as f32 - 1.0) * 4.4 * s;
        quads.push(Quad::rounded(
            Rect::new(cx - 7.0 * s, y - 0.55 * s, 14.0 * s, 1.1 * s),
            Rgba::opaque(color),
            0.55 * s,
        ));
        let knob_x = if i == 1 { cx + 3.2 * s } else { cx - 4.6 * s };
        quads.push(Quad::rounded(
            Rect::new(knob_x, y - 1.9 * s, 3.8 * s, 3.8 * s),
            Rgba::opaque(color),
            1.9 * s,
        ));
    }
    hits.push(HitRegion {
        rect,
        action: UiAction::ToggleSettings,
    });
}

fn paint_empty_state(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    images: &mut Vec<Rect>,
    hits: &mut Vec<HitRegion>,
    terminal: Rect,
    theme: &Theme,
    mouse: (f32, f32),
    s: f32,
) {
    let cx = terminal.x + terminal.w * 0.5;
    let center_y = terminal.y + terminal.h * 0.43;
    let mark = Rect::new(cx - 27.0 * s, center_y - 88.0 * s, 54.0 * s, 54.0 * s);
    paint_mark(quads, images, mark, theme, s);
    text.push(
        TextBlock::new(
            "FARMAN",
            Rect::new(cx - 160.0 * s, center_y - 18.0 * s, 320.0 * s, 26.0 * s),
            18.0 * s,
            theme.foreground,
        )
        .align(TextAlign::Center)
        .weight(Weight::BOLD)
        .width(320.0 * s),
    );
    text.push(
        TextBlock::new(
            "No active sessions",
            Rect::new(cx - 160.0 * s, center_y + 14.0 * s, 320.0 * s, 16.0 * s),
            11.0 * s,
            theme.foreground,
        )
        .align(TextAlign::Center)
        .weight(Weight::MEDIUM)
        .width(320.0 * s),
    );
    text.push(
        TextBlock::new(
            "Open a shell to start working.",
            Rect::new(cx - 180.0 * s, center_y + 35.0 * s, 360.0 * s, 14.0 * s),
            9.0 * s,
            theme.muted,
        )
        .align(TextAlign::Center)
        .width(360.0 * s),
    );
    let button = Rect::new(cx - 88.0 * s, center_y + 64.0 * s, 176.0 * s, 34.0 * s);
    accent_button(
        quads,
        text,
        hits,
        button,
        "New session",
        UiAction::NewTab(None),
        theme,
        mouse,
        s,
    );
    text.push(
        TextBlock::new(
            "Ctrl+Shift+T   Ctrl+,   Ctrl+K",
            Rect::new(cx - 160.0 * s, center_y + 108.0 * s, 320.0 * s, 12.0 * s),
            8.0 * s,
            theme.muted.scale(0.86),
        )
        .align(TextAlign::Center)
        .width(320.0 * s),
    );
}

fn paint_mark(
    _quads: &mut Vec<Quad>,
    images: &mut Vec<Rect>,
    rect: Rect,
    _theme: &Theme,
    _s: f32,
) {
    images.push(rect);
}

#[allow(clippy::too_many_arguments)]
fn paint_status_bar(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    l: &Layout,
    theme: &Theme,
    cols: usize,
    rows: usize,
    bidi_enabled: bool,
    status: &str,
    s: f32,
) {
    let rect = l.status;
    quads.push(Quad::glass(
        rect,
        alpha(theme.status_bg, 0.86),
        alpha(theme.status_bg.blend(theme.background, 0.3), 0.92),
        0.0,
    ));
    quads.push(Quad::rounded(
        Rect::new(0.0, rect.y, rect.w, 1.0 * s),
        alpha(theme.border, 0.40),
        0.0,
    ));
    quads.push(Quad::rounded(
        Rect::new(14.0 * s, rect.y + rect.h * 0.5 - 2.0 * s, 4.0 * s, 4.0 * s),
        Rgba::opaque(theme.success),
        2.0 * s,
    ));
    let quiet = status.is_empty() || status.eq_ignore_ascii_case("ready");
    text.push(
        TextBlock::new(
            truncate(if quiet { "Ready" } else { status }, 58),
            Rect::new(24.0 * s, rect.y + 6.0 * s, (rect.w * 0.55).max(80.0), 14.0 * s),
            8.8 * s,
            if quiet { theme.muted } else { theme.foreground },
        )
        .width((rect.w * 0.55).max(80.0)),
    );
    let info = format!(
        "{cols} × {rows}   UTF-8   Bidi {}",
        if bidi_enabled { "auto" } else { "off" }
    );
    let info_w = 210.0 * s;
    text.push(
        TextBlock::new(
            info,
            Rect::new(rect.right() - info_w - 14.0 * s, rect.y + 6.0 * s, info_w, 14.0 * s),
            8.4 * s,
            theme.muted,
        )
        .align(TextAlign::Right)
        .width(info_w),
    );
}

fn paint_starting_pill(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    terminal: Rect,
    theme: &Theme,
    profile: &str,
    s: f32,
) {
    let message = format!("Starting {}...", profile);
    let w = (message.chars().count() as f32 * 5.4 + 46.0).clamp(170.0, 330.0) * s;
    let rect = Rect::new(terminal.x + 14.0 * s, terminal.y + 14.0 * s, w, 30.0 * s);
    quads.push(Quad::glass(
        rect,
        alpha(theme.panel_alt, 0.94),
        alpha(theme.panel_bg, 0.94),
        9.0 * s,
    ));
    quads.push(Quad::rounded(rect, alpha(theme.warning, 0.35), 9.0 * s));
    quads.push(Quad::rounded(
        Rect::new(rect.x + 12.0 * s, rect.y + rect.h * 0.5 - 2.5 * s, 5.0 * s, 5.0 * s),
        Rgba::opaque(theme.warning),
        2.5 * s,
    ));
    text.push(
        TextBlock::new(
            message,
            Rect::new(rect.x + 24.0 * s, rect.y + 8.0 * s, rect.w - 34.0 * s, 14.0 * s),
            9.0 * s,
            theme.foreground,
        )
        .weight(Weight::MEDIUM)
        .width(rect.w - 34.0 * s),
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_session_banner(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    terminal: Rect,
    theme: &Theme,
    mouse: (f32, f32),
    message: &str,
    s: f32,
) {
    let w = 390.0 * s;
    let h = 128.0 * s;
    let rect = Rect::new(
        terminal.x + (terminal.w - w) * 0.5,
        terminal.y + (terminal.h - h) * 0.5,
        w,
        h,
    );
    glass_card(quads, rect, theme, 14.0 * s, true, s);
    text.push(
        TextBlock::new(
            "Session ended",
            Rect::new(rect.x + 22.0 * s, rect.y + 18.0 * s, w - 44.0 * s, 18.0 * s),
            12.0 * s,
            theme.foreground,
        )
        .weight(Weight::SEMIBOLD)
        .width(w - 44.0 * s),
    );
    text.push(
        TextBlock::new(
            truncate(message, 62),
            Rect::new(rect.x + 22.0 * s, rect.y + 42.0 * s, w - 44.0 * s, 28.0 * s),
            9.0 * s,
            theme.muted,
        )
        .width(w - 44.0 * s),
    );
    accent_button(
        quads,
        text,
        hits,
        Rect::new(rect.x + 22.0 * s, rect.bottom() - 46.0 * s, 146.0 * s, 30.0 * s),
        "New session",
        UiAction::NewTab(None),
        theme,
        mouse,
        s,
    );
    text.push(
        TextBlock::new(
            "Ctrl+Shift+T",
            Rect::new(rect.x + 180.0 * s, rect.bottom() - 36.0 * s, 160.0 * s, 12.0 * s),
            8.0 * s,
            theme.muted,
        )
        .width(160.0 * s),
    );
}

fn paint_toast(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    l: Layout,
    theme: &Theme,
    message: &str,
    s: f32,
) {
    let message = truncate(message, 74);
    let w = (message.chars().count() as f32 * 5.3 + 52.0).clamp(180.0, 560.0) * s;
    let rect = Rect::new(
        (l.terminal.x + (l.terminal.w - w) * 0.5).max(l.terminal.x),
        l.status.y - 44.0 * s,
        w,
        32.0 * s,
    );
    quads.push(Quad::soft(
        Rect::new(rect.x - 5.0 * s, rect.y + 5.0 * s, rect.w + 10.0 * s, rect.h + 6.0 * s),
        alpha(theme.background, 0.35),
        12.0 * s,
        9.0 * s,
    ));
    quads.push(Quad::glass(
        rect,
        alpha(theme.panel_alt, 0.97),
        alpha(theme.panel_bg, 0.97),
        10.0 * s,
    ));
    quads.push(Quad::rounded(rect, alpha(theme.border, 0.85), 10.0 * s));
    quads.push(Quad::rounded(
        Rect::new(rect.x + 14.0 * s, rect.y + rect.h * 0.5 - 2.5 * s, 5.0 * s, 5.0 * s),
        Rgba::opaque(theme.accent),
        2.5 * s,
    ));
    text.push(
        TextBlock::new(
            message,
            Rect::new(rect.x + 26.0 * s, rect.y + 9.0 * s, rect.w - 38.0 * s, 14.0 * s),
            8.8 * s,
            theme.foreground,
        )
        .width(rect.w - 38.0 * s),
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_profile_menu(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    anchor: Rect,
    top: f32,
    profiles: &[Profile],
    selected_profile: &str,
    theme: &Theme,
    mouse: (f32, f32),
    width: f32,
    height: f32,
    s: f32,
) {
    let row_h = 34.0 * s;
    let menu_w = 286.0 * s;
    let menu_h = 26.0 * s + profiles.len().max(1) as f32 * row_h + 42.0 * s;
    let x = (anchor.right() - menu_w).clamp(12.0 * s, (width - menu_w - 12.0 * s).max(12.0 * s));
    let y = top.min((height - menu_h - 12.0 * s).max(12.0 * s));
    let menu = Rect::new(x, y, menu_w, menu_h);

    hits.push(HitRegion {
        rect: Rect::new(0.0, 0.0, width, height),
        action: UiAction::CloseOverlay,
    });

    glass_card(quads, menu, theme, 12.0 * s, true, s);
    hits.push(HitRegion {
        rect: menu,
        action: UiAction::Noop,
    });
    text.push(
        TextBlock::new(
            "NEW SESSION",
            Rect::new(menu.x + 14.0 * s, menu.y + 9.0 * s, menu.w - 28.0 * s, 11.0 * s),
            7.8 * s,
            theme.muted,
        )
        .weight(Weight::SEMIBOLD)
        .width(menu.w - 28.0 * s),
    );

    if profiles.is_empty() {
        text.push(
            TextBlock::new(
                "No shells found",
                Rect::new(menu.x + 14.0 * s, menu.y + 28.0 * s, menu.w - 28.0 * s, 20.0 * s),
                9.0 * s,
                theme.muted,
            )
            .width(menu.w - 28.0 * s),
        );
    } else {
        for (index, profile) in profiles.iter().enumerate() {
            let row = Rect::new(
                menu.x + 6.0 * s,
                menu.y + 27.0 * s + index as f32 * row_h,
                menu.w - 12.0 * s,
                row_h - 2.0 * s,
            );
            let hot = row.contains(mouse.0, mouse.1);
            let selected = profile.id == selected_profile;
            if hot {
                quads.push(Quad::rounded(row, alpha(theme.foreground, 0.065), 7.0 * s));
            }
            let dot = if selected { theme.accent } else { theme.border };
            quads.push(Quad::rounded(
                Rect::new(row.x + 8.0 * s, row.y + row.h * 0.5 - 2.5 * s, 5.0 * s, 5.0 * s),
                Rgba::opaque(dot),
                2.5 * s,
            ));
            text.push(
                TextBlock::new(
                    truncate(&profile.name, 28),
                    Rect::new(row.x + 20.0 * s, row.y + 8.0 * s, row.w - 32.0 * s, 15.0 * s),
                    9.4 * s,
                    if selected { theme.foreground } else { theme.dim_foreground },
                )
                .weight(if selected { Weight::MEDIUM } else { Weight::NORMAL })
                .width(row.w - 32.0 * s),
            );
            hits.push(HitRegion {
                rect: row,
                action: UiAction::NewTab(Some(profile.id.clone())),
            });
        }
    }

    let footer = Rect::new(
        menu.x + 6.0 * s,
        menu.bottom() - 37.0 * s,
        menu.w - 12.0 * s,
        30.0 * s,
    );
    let hot = footer.contains(mouse.0, mouse.1);
    if hot {
        quads.push(Quad::rounded(footer, alpha(theme.foreground, 0.06), 7.0 * s));
    }
    quads.push(Quad::rounded(
        Rect::new(menu.x + 14.0 * s, footer.y, menu.w - 28.0 * s, 1.0 * s),
        alpha(theme.border, 0.7),
        0.0,
    ));
    text.push(
        TextBlock::new(
            "Settings",
            Rect::new(footer.x + 14.0 * s, footer.y + 9.0 * s, 140.0 * s, 14.0 * s),
            9.2 * s,
            theme.foreground,
        )
        .width(140.0 * s),
    );
    hits.push(HitRegion {
        rect: footer,
        action: UiAction::ToggleSettings,
    });
}

#[allow(clippy::too_many_arguments)]
fn paint_palette(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    width: f32,
    height: f32,
    theme: &Theme,
    query: &str,
    selected: usize,
    mouse: (f32, f32),
    s: f32,
) {
    let matches = palette_matches(query);
    let rows = matches.len().max(1);
    let row_h = 34.0 * s;
    let panel_w = (560.0 * s).min((width - 36.0 * s).max(1.0));
    let panel_h = (112.0 * s + rows as f32 * row_h).min((height - 36.0 * s).max(1.0));
    let panel = Rect::new(
        (width - panel_w) * 0.5,
        ((height - panel_h) * 0.42).max(18.0 * s),
        panel_w,
        panel_h,
    );

    quads.push(Quad::rounded(
        Rect::new(0.0, 0.0, width, height),
        alpha(theme.background, 0.92),
        0.0,
    ));
    hits.push(HitRegion {
        rect: Rect::new(0.0, 0.0, width, height),
        action: UiAction::CloseOverlay,
    });

    glass_card(quads, panel, theme, 16.0 * s, true, s);
    hits.push(HitRegion {
        rect: panel,
        action: UiAction::Noop,
    });

    quads.push(Quad::glass(
        Rect::new(
            panel.x + 20.0 * s,
            panel.y + 1.0 * s,
            (panel.w - 40.0 * s).max(1.0),
            1.6 * s,
        ),
        alpha(theme.accent, 0.72),
        alpha(theme.accent_violet, 0.10),
        1.0 * s,
    ));

    let left = panel.x + 20.0 * s;
    let right = panel.right() - 20.0 * s;
    let w = (right - left).max(1.0);

    text.push(
        TextBlock::new(
            "COMMAND PALETTE",
            Rect::new(left, panel.y + 17.0 * s, w - 86.0 * s, 12.0 * s),
            7.8 * s,
            theme.muted,
        )
        .weight(Weight::SEMIBOLD)
        .width((w - 86.0 * s).max(1.0)),
    );
    text.push(
        TextBlock::new(
            "ESC",
            Rect::new(right - 34.0 * s, panel.y + 15.0 * s, 34.0 * s, 14.0 * s),
            7.0 * s,
            theme.muted.scale(0.92),
        )
        .align(TextAlign::Right)
        .weight(Weight::MEDIUM)
        .width(34.0 * s),
    );

    let search = Rect::new(left, panel.y + 38.0 * s, w, 40.0 * s);
    quads.push(Quad::rounded(search, alpha(theme.border, 0.70), 9.0 * s));
    quads.push(Quad::rounded(
        Rect::new(
            search.x + 1.0 * s,
            search.y + 1.0 * s,
            (search.w - 2.0 * s).max(1.0),
            (search.h - 2.0 * s).max(1.0),
        ),
        alpha(theme.background, 0.62),
        8.0 * s,
    ));
    quads.push(Quad::rounded(
        Rect::new(
            search.x + 12.0 * s,
            search.y + 11.0 * s,
            2.0 * s,
            18.0 * s,
        ),
        alpha(theme.accent, 0.92),
        1.0 * s,
    ));
    let query_text = if query.is_empty() {
        "Type a command..."
    } else {
        query
    };
    text.push(
        TextBlock::new(
            query_text,
            Rect::new(
                search.x + 22.0 * s,
                search.y + 11.0 * s,
                (search.w - 36.0 * s).max(1.0),
                16.0 * s,
            ),
            10.5 * s,
            if query.is_empty() {
                theme.muted.scale(0.9)
            } else {
                theme.foreground
            },
        )
        .weight(Weight::MEDIUM)
        .width((search.w - 36.0 * s).max(1.0)),
    );
    if !matches.is_empty() {
        let caret_x = (search.x
            + 22.0 * s
            + query.chars().count() as f32 * 5.45 * s
            + 1.0 * s)
            .min(search.right() - 14.0 * s);
        quads.push(Quad::rounded(
            Rect::new(caret_x, search.y + 12.0 * s, 1.5 * s, 16.0 * s),
            alpha(theme.accent, 0.90),
            0.75 * s,
        ));
    }

    let list_top = search.bottom() + 8.0 * s;
    if matches.is_empty() {
        text.push(
            TextBlock::new(
                "No matching commands",
                Rect::new(left, list_top + 9.0 * s, w, 16.0 * s),
                10.0 * s,
                theme.muted,
            )
            .align(TextAlign::Center)
            .width(w),
        );
    } else {
        for (index, item) in matches.iter().enumerate() {
            let row = Rect::new(
                left,
                list_top + index as f32 * row_h,
                w,
                row_h - 2.0 * s,
            );
            let hot = row.contains(mouse.0, mouse.1);
            let active = index == selected;
            if active || hot {
                quads.push(Quad::rounded(
                    row,
                    if active {
                        alpha(theme.accent, 0.13)
                    } else {
                        alpha(theme.foreground, 0.045)
                    },
                    7.0 * s,
                ));
            }
            if active {
                quads.push(Quad::rounded(
                    Rect::new(row.x, row.y + 6.0 * s, 2.0 * s, row.h - 12.0 * s),
                    alpha(theme.accent, 0.95),
                    1.0 * s,
                ));
            }
            text.push(
                TextBlock::new(
                    item.title,
                    Rect::new(row.x + 14.0 * s, row.y + 8.0 * s, row.w - 146.0 * s, 16.0 * s),
                    if active { 10.2 } else { 9.8 } * s,
                    if active { theme.foreground } else { theme.foreground.scale(0.86) },
                )
                .weight(if active { Weight::MEDIUM } else { Weight::NORMAL })
                .width((row.w - 146.0 * s).max(1.0)),
            );
            text.push(
                TextBlock::new(
                    item.hint,
                    Rect::new(row.right() - 126.0 * s, row.y + 9.0 * s, 112.0 * s, 14.0 * s),
                    8.0 * s,
                    theme.muted,
                )
                .align(TextAlign::Right)
                .width(112.0 * s),
            );
            hits.push(HitRegion {
                rect: row,
                action: item.command.action(),
            });
        }
    }

    let footer_y = (list_top + rows as f32 * row_h + 8.0 * s).min(panel.bottom() - 22.0 * s);
    quads.push(Quad::rounded(
        Rect::new(left, footer_y - 4.0 * s, w, 1.0 * s),
        alpha(theme.border, 0.70),
        0.0,
    ));
    text.push(
        TextBlock::new(
            "Up/Down move    Enter run    Esc close",
            Rect::new(left, footer_y + 3.0 * s, w - 54.0 * s, 12.0 * s),
            7.4 * s,
            theme.muted.scale(0.9),
        )
        .width((w - 54.0 * s).max(1.0)),
    );
    text.push(
        TextBlock::new(
            "Ctrl K",
            Rect::new(right - 50.0 * s, footer_y + 3.0 * s, 50.0 * s, 12.0 * s),
            7.4 * s,
            theme.muted.scale(0.9),
        )
        .align(TextAlign::Right)
        .width(50.0 * s),
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_settings(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    width: f32,
    height: f32,
    theme: &Theme,
    font_size: f32,
    theme_name: &str,
    bidi_enabled: bool,
    profiles: &[Profile],
    selected_profile: &str,
    config_path: &str,
    version: &str,
    mouse: (f32, f32),
    s: f32,
) {
    let panel_w = (520.0 * s).min((width - 32.0 * s).max(1.0));
    let panel_h = (504.0 * s).min((height - 32.0 * s).max(1.0));
    let panel = Rect::new(
        (width - panel_w) * 0.5,
        (height - panel_h) * 0.5,
        panel_w,
        panel_h,
    );

    quads.push(Quad::rounded(
        Rect::new(0.0, 0.0, width, height),
        alpha(theme.background, 0.62),
        0.0,
    ));
    hits.push(HitRegion {
        rect: Rect::new(0.0, 0.0, width, height),
        action: UiAction::CloseOverlay,
    });

    glass_card(quads, panel, theme, 16.0 * s, true, s);
    hits.push(HitRegion {
        rect: panel,
        action: UiAction::Noop,
    });

    let x = panel.x + 24.0 * s;
    let right = panel.right() - 24.0 * s;
    let w = (right - x).max(1.0);

    text.push(
        TextBlock::new(
            "Settings",
            Rect::new(x, panel.y + 18.0 * s, w - 40.0 * s, 24.0 * s),
            17.0 * s,
            theme.foreground,
        )
        .weight(Weight::SEMIBOLD)
        .width(w - 40.0 * s),
    );
    text.push(
        TextBlock::new(
            "Appearance, terminal behavior and configuration.",
            Rect::new(x, panel.y + 43.0 * s, w - 40.0 * s, 12.0 * s),
            8.4 * s,
            theme.muted,
        )
        .width(w - 40.0 * s),
    );
    let close = Rect::new(panel.right() - 46.0 * s, panel.y + 18.0 * s, 28.0 * s, 28.0 * s);
    let close_hot = close.contains(mouse.0, mouse.1);
    quads.push(Quad::rounded(
        close,
        alpha(theme.foreground, if close_hot { 0.08 } else { 0.0 }),
        6.0 * s,
    ));
    text.push(
        TextBlock::new(
            "×",
            Rect::new(close.x, close.y + 1.0 * s, close.w, close.h),
            15.0 * s,
            if close_hot { theme.foreground } else { theme.muted },
        )
        .align(TextAlign::Center)
        .width(close.w),
    );
    hits.push(HitRegion {
        rect: close,
        action: UiAction::CloseOverlay,
    });
    quads.push(Quad::rounded(
        Rect::new(x, panel.y + 70.0 * s, w, 1.0 * s),
        alpha(theme.border, 0.70),
        0.0,
    ));

    let mut y = panel.y + 86.0 * s;
    section_label(text, "APPEARANCE", x, y, w, theme, s);
    y += 18.0 * s;

    setting_label(
        text,
        "Theme",
        "Interface color scheme",
        x,
        y,
        w - 292.0 * s,
        theme,
        s,
    );
    let control_x = right - 276.0 * s;
    let button_w = 88.0 * s;
    for (index, name) in Theme::NAMES.iter().enumerate() {
        let rect = Rect::new(
            control_x + index as f32 * (button_w + 6.0 * s),
            y + 3.0 * s,
            button_w,
            28.0 * s,
        );
        let selected = *name == theme_name;
        select_button(
            quads,
            text,
            hits,
            rect,
            Theme::display_name_for(name),
            UiAction::SetTheme((*name).to_owned()),
            selected,
            theme,
            mouse,
            s,
        );
    }
    y += 44.0 * s;

    setting_label(
        text,
        "Font size",
        "Terminal grid text",
        x,
        y,
        w - 292.0 * s,
        theme,
        s,
    );
    let stepper_w = 144.0 * s;
    let step_x = right - stepper_w;
    let minus = Rect::new(step_x, y + 3.0 * s, 28.0 * s, 28.0 * s);
    let plus = Rect::new(step_x + 116.0 * s, y + 3.0 * s, 28.0 * s, 28.0 * s);
    let value = Rect::new(step_x + 32.0 * s, y + 3.0 * s, 80.0 * s, 28.0 * s);
    small_button(quads, text, hits, minus, "-", UiAction::FontDelta(-1), theme, mouse, s);
    quads.push(Quad::rounded(value, alpha(theme.foreground, 0.04), 7.0 * s));
    quads.push(Quad::rounded(value, alpha(theme.border, 0.65), 7.0 * s));
    text.push(
        TextBlock::new(
            format!("{} px", font_size.round() as i32),
            Rect::new(value.x, value.y + 8.0 * s, value.w, 14.0 * s),
            9.0 * s,
            theme.foreground,
        )
        .align(TextAlign::Center)
        .width(value.w),
    );
    small_button(quads, text, hits, plus, "+", UiAction::FontDelta(1), theme, mouse, s);
    y += 44.0 * s;

    divider(quads, x, y, w, theme, s);
    y += 20.0 * s;

    section_label(text, "TERMINAL", x, y, w, theme, s);
    y += 18.0 * s;

    setting_label(
        text,
        "Default shell",
        "Used for new tabs",
        x,
        y,
        w - 292.0 * s,
        theme,
        s,
    );
    let shell = Rect::new(right - 200.0 * s, y + 3.0 * s, 200.0 * s, 28.0 * s);
    let shell_hot = shell.contains(mouse.0, mouse.1);
    quads.push(Quad::rounded(
        shell,
        alpha(theme.foreground, if shell_hot { 0.075 } else { 0.045 }),
        7.0 * s,
    ));
    quads.push(Quad::rounded(shell, alpha(theme.border, 0.7), 7.0 * s));
    let shell_name = profiles
        .iter()
        .find(|p| p.id == selected_profile)
        .map(|p| p.name.as_str())
        .unwrap_or("Shell");
    text.push(
        TextBlock::new(
            truncate(shell_name, 24),
            Rect::new(shell.x + 11.0 * s, shell.y + 8.0 * s, shell.w - 34.0 * s, 14.0 * s),
            9.0 * s,
            theme.foreground,
        )
        .width(shell.w - 34.0 * s),
    );
    text.push(
        TextBlock::new(
            "v",
            Rect::new(shell.right() - 20.0 * s, shell.y + 9.0 * s, 12.0 * s, 12.0 * s),
            8.0 * s,
            theme.muted,
        )
        .align(TextAlign::Center)
        .width(12.0 * s),
    );
    hits.push(HitRegion {
        rect: shell,
        action: UiAction::CycleDefaultProfile,
    });
    y += 44.0 * s;

    setting_label(
        text,
        "Bidi text shaping",
        "Visual ordering for Persian and Arabic",
        x,
        y,
        w - 292.0 * s,
        theme,
        s,
    );
    let switch = Rect::new(right - 40.0 * s, y + 5.0 * s, 40.0 * s, 22.0 * s);
    paint_toggle(quads, switch, bidi_enabled, theme, s);
    hits.push(HitRegion {
        rect: switch,
        action: UiAction::ToggleBidi,
    });
    y += 44.0 * s;

    divider(quads, x, y, w, theme, s);
    y += 20.0 * s;

    section_label(text, "ABOUT", x, y, w, theme, s);
    y += 18.0 * s;
    about_row(text, "Version", version, x, y, w, theme, s);
    y += 22.0 * s;
    about_row(text, "Developer", "Shahbaz Team", x, y, w, theme, s);
    y += 22.0 * s;
    about_row(text, "Website", "shahbazteam.ir", x, y, w, theme, s);
    y += 26.0 * s;

    let card = Rect::new(x, y, w, 40.0 * s);
    quads.push(Quad::rounded(card, alpha(theme.foreground, 0.035), 8.0 * s));
    quads.push(Quad::rounded(card, alpha(theme.border, 0.65), 8.0 * s));
    text.push(
        TextBlock::new(
            "Configuration file",
            Rect::new(card.x + 12.0 * s, card.y + 7.0 * s, card.w - 130.0 * s, 10.0 * s),
            7.6 * s,
            theme.muted,
        )
        .width(card.w - 130.0 * s),
    );
    text.push(
        TextBlock::new(
            shorten_path(config_path, 62),
            Rect::new(card.x + 12.0 * s, card.y + 20.0 * s, card.w - 130.0 * s, 12.0 * s),
            8.0 * s,
            theme.dim_foreground,
        )
        .width(card.w - 130.0 * s),
    );
    let reload = Rect::new(card.right() - 112.0 * s, card.y + 7.0 * s, 100.0 * s, 26.0 * s);
    let reload_hot = reload.contains(mouse.0, mouse.1);
    quads.push(Quad::rounded(
        reload,
        alpha(theme.foreground, if reload_hot { 0.075 } else { 0.045 }),
        6.0 * s,
    ));
    quads.push(Quad::rounded(reload, alpha(theme.border, 0.7), 6.0 * s));
    text.push(
        TextBlock::new(
            "Reload config",
            Rect::new(reload.x, reload.y + 7.0 * s, reload.w, 12.0 * s),
            8.2 * s,
            theme.foreground,
        )
        .align(TextAlign::Center)
        .width(reload.w),
    );
    hits.push(HitRegion {
        rect: reload,
        action: UiAction::ReloadConfig,
    });
}

#[allow(clippy::too_many_arguments)]
fn setting_label(
    text: &mut Vec<TextBlock>,
    title: &str,
    subtitle: &str,
    x: f32,
    y: f32,
    w: f32,
    theme: &Theme,
    s: f32,
) {
    text.push(
        TextBlock::new(
            title,
            Rect::new(x, y + 3.0 * s, w.max(40.0), 14.0 * s),
            9.6 * s,
            theme.foreground,
        )
        .weight(Weight::MEDIUM)
        .width(w.max(40.0)),
    );
    text.push(
        TextBlock::new(
            subtitle,
            Rect::new(x, y + 18.0 * s, w.max(40.0), 11.0 * s),
            7.8 * s,
            theme.muted,
        )
        .width(w.max(40.0)),
    );
}

fn section_label(
    text: &mut Vec<TextBlock>,
    label: &str,
    x: f32,
    y: f32,
    w: f32,
    theme: &Theme,
    s: f32,
) {
    text.push(
        TextBlock::new(
            label,
            Rect::new(x, y, w, 11.0 * s),
            7.7 * s,
            theme.muted,
        )
        .weight(Weight::SEMIBOLD)
        .width(w),
    );
}

fn about_row(
    text: &mut Vec<TextBlock>,
    label: &str,
    value: &str,
    x: f32,
    y: f32,
    w: f32,
    theme: &Theme,
    s: f32,
) {
    text.push(
        TextBlock::new(
            label,
            Rect::new(x, y + 3.0 * s, 140.0 * s, 13.0 * s),
            8.8 * s,
            theme.muted,
        )
        .width(140.0 * s),
    );
    text.push(
        TextBlock::new(
            value,
            Rect::new(x + 140.0 * s, y + 3.0 * s, (w - 140.0 * s).max(1.0), 13.0 * s),
            8.8 * s,
            if label == "Website" { theme.accent } else { theme.foreground },
        )
        .align(TextAlign::Right)
        .width((w - 140.0 * s).max(1.0)),
    );
}

fn divider(quads: &mut Vec<Quad>, x: f32, y: f32, w: f32, theme: &Theme, s: f32) {
    quads.push(Quad::rounded(
        Rect::new(x, y, w, 1.0 * s),
        alpha(theme.border, 0.55),
        0.0,
    ));
}

fn paint_toggle(quads: &mut Vec<Quad>, rect: Rect, on: bool, theme: &Theme, s: f32) {
    quads.push(Quad::rounded(
        rect,
        alpha(if on { theme.accent } else { theme.muted }, if on { 0.55 } else { 0.16 }),
        rect.h * 0.5,
    ));
    let knob_size = rect.h - 6.0 * s;
    let knob_x = if on {
        rect.right() - knob_size - 3.0 * s
    } else {
        rect.x + 3.0 * s
    };
    quads.push(Quad::rounded(
        Rect::new(knob_x, rect.y + 3.0 * s, knob_size, knob_size),
        Rgba::opaque(theme.foreground),
        knob_size * 0.5,
    ));
}

#[allow(clippy::too_many_arguments)]
fn select_button(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    rect: Rect,
    label: &str,
    action: UiAction,
    selected: bool,
    theme: &Theme,
    mouse: (f32, f32),
    s: f32,
) {
    let hot = rect.contains(mouse.0, mouse.1);
    quads.push(Quad::rounded(
        rect,
        if selected {
            alpha(theme.accent, 0.20)
        } else {
            alpha(theme.foreground, if hot { 0.075 } else { 0.035 })
        },
        7.0 * s,
    ));
    quads.push(Quad::rounded(
        rect,
        alpha(if selected { theme.accent } else { theme.border }, if selected { 0.55 } else { 0.65 }),
        7.0 * s,
    ));
    text.push(
        TextBlock::new(
            label,
            Rect::new(rect.x + 2.0 * s, rect.y + 7.0 * s, rect.w - 4.0 * s, 14.0 * s),
            8.2 * s,
            if selected { theme.foreground } else { theme.dim_foreground },
        )
        .align(TextAlign::Center)
        .weight(if selected { Weight::MEDIUM } else { Weight::NORMAL })
        .width(rect.w - 4.0 * s),
    );
    hits.push(HitRegion { rect, action });
}

#[allow(clippy::too_many_arguments)]
fn small_button(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    rect: Rect,
    label: &str,
    action: UiAction,
    theme: &Theme,
    mouse: (f32, f32),
    s: f32,
) {
    let hot = rect.contains(mouse.0, mouse.1);
    quads.push(Quad::rounded(
        rect,
        alpha(theme.foreground, if hot { 0.08 } else { 0.04 }),
        7.0 * s,
    ));
    quads.push(Quad::rounded(rect, alpha(theme.border, 0.7), 7.0 * s));
    text.push(
        TextBlock::new(
            label,
            Rect::new(rect.x, rect.y - 1.0 * s, rect.w, rect.h),
            14.0 * s,
            theme.foreground,
        )
        .align(TextAlign::Center)
        .weight(Weight::MEDIUM)
        .width(rect.w),
    );
    hits.push(HitRegion { rect, action });
}

#[allow(clippy::too_many_arguments)]
fn accent_button(
    quads: &mut Vec<Quad>,
    text: &mut Vec<TextBlock>,
    hits: &mut Vec<HitRegion>,
    rect: Rect,
    label: &str,
    action: UiAction,
    theme: &Theme,
    mouse: (f32, f32),
    s: f32,
) {
    let hot = rect.contains(mouse.0, mouse.1);
    quads.push(Quad::soft(
        Rect::new(rect.x, rect.y + 3.0 * s, rect.w, rect.h),
        alpha(theme.accent, if hot { 0.25 } else { 0.15 }),
        9.0 * s,
        7.0 * s,
    ));
    quads.push(Quad::glass(
        rect,
        Rgba::opaque(theme.accent.blend(theme.foreground, if hot { 0.18 } else { 0.05 })),
        Rgba::opaque(theme.accent.scale(0.88)),
        9.0 * s,
    ));
    quads.push(Quad::rounded(
        Rect::new(rect.x + 1.0 * s, rect.y + 1.0 * s, (rect.w - 2.0 * s).max(1.0), (rect.h - 2.0 * s).max(1.0)),
        alpha(theme.foreground, 0.14),
        9.0 * s,
    ));
    text.push(
        TextBlock::new(
            label,
            Rect::new(rect.x, rect.y + (rect.h - 14.0 * s) * 0.5, rect.w, 14.0 * s),
            9.3 * s,
            theme.cursor_text,
        )
        .align(TextAlign::Center)
        .weight(Weight::SEMIBOLD)
        .width(rect.w),
    );
    hits.push(HitRegion { rect, action });
}

fn glass_card(
    quads: &mut Vec<Quad>,
    rect: Rect,
    theme: &Theme,
    radius: f32,
    strong: bool,
    s: f32,
) {
    quads.push(Quad::soft(
        Rect::new(
            rect.x - 6.0 * s,
            rect.y + 6.0 * s,
            rect.w + 12.0 * s,
            rect.h + 10.0 * s,
        ),
        alpha(theme.background, 0.42),
        radius + 6.0 * s,
        12.0 * s,
    ));
    let top = if strong {
        theme.panel_alt.blend(theme.header_active, 0.38)
    } else {
        theme.panel_alt.blend(theme.header_active, 0.18)
    };
    let bottom = theme.panel_bg.blend(theme.background, 0.25);
    quads.push(Quad::glass(
        rect,
        alpha(top, 0.985),
        alpha(bottom, 0.98),
        radius,
    ));
    quads.push(Quad::glass(
        rect,
        alpha(theme.foreground, 0.035),
        alpha(theme.foreground, 0.0),
        radius,
    ));
    quads.push(Quad::rounded(
        Rect::new(rect.x + radius * 0.7, rect.y + 1.0 * s, (rect.w - radius * 1.4).max(1.0), 1.0 * s),
        alpha(theme.foreground, 0.08),
        0.0,
    ));
    quads.push(Quad::rounded(rect, alpha(theme.border, 0.85), radius));
}

fn shorten_path(path: &str, max: usize) -> String {
    let count = path.chars().count();
    if count <= max {
        return path.to_owned();
    }
    let head: String = path.chars().take(max / 2 - 2).collect();
    let tail: String = path.chars().skip(count - (max / 2 - 2)).collect();
    format!("{head}...{tail}")
}

fn truncate(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

pub fn hit_test(hits: &[HitRegion], x: f32, y: f32) -> Option<UiAction> {
    hits.iter()
        .rev()
        .find(|hit| hit.rect.contains(x, y))
        .map(|hit| hit.action.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FARMAN_DARK;

    #[test]
    fn terminal_rect_dominates_window() {
        let rect = terminal_rect(1280.0, 800.0, 1.0);
        assert!(rect.h > 600.0);
        assert!(rect.w > 1180.0);
    }

    #[test]
    fn hit_test_prefers_overlay_last() {
        let hits = vec![
            HitRegion {
                rect: Rect::new(0.0, 0.0, 100.0, 100.0),
                action: UiAction::NewTab(None),
            },
            HitRegion {
                rect: Rect::new(10.0, 10.0, 20.0, 20.0),
                action: UiAction::CloseOverlay,
            },
        ];
        assert!(matches!(
            hit_test(&hits, 15.0, 15.0),
            Some(UiAction::CloseOverlay)
        ));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        assert_eq!(truncate("abcdefgh", 5), "ab...");
        assert_eq!(truncate("short", 10), "short");
    }

    #[test]
    fn builds_empty_state_without_panicking() {
        let tabs: [TabInfo<'_>; 0] = [];
        let profiles = Vec::new();
        let chrome = build(ChromeInput {
            width: 1024.0,
            height: 700.0,
            scale: 1.0,
            theme: &FARMAN_DARK,
            tabs: &tabs,
            active: 0,
            settings_open: false,
            profile_menu_open: false,
            palette_open: false,
            palette_query: "",
            palette_selected: 0,
            profiles: &profiles,
            selected_profile: "powershell",
            font_size: 16.0,
            theme_name: "farman-dark",
            bidi_enabled: true,
            cols: 100,
            rows: 30,
            status: "ready",
            toast: None,
            mouse: (0.0, 0.0),
            config_path: "C:\\Users\\Test\\AppData\\Roaming\\FARMAN\\config.toml",
            version: "v0.3.0",
            active_profile: "Windows PowerShell",
            starting: false,
            session_banner: None,
            maximized: false,
        });
        assert!(!chrome.text.is_empty());
        assert!(!chrome.base_quads.is_empty());
        assert!(chrome.drag_region.w > 100.0);
    }

    #[test]
    fn settings_has_window_and_action_hits() {
        let chrome = build(ChromeInput {
            width: 1280.0,
            height: 800.0,
            scale: 1.0,
            theme: &FARMAN_DARK,
            tabs: &[],
            active: 0,
            settings_open: true,
            profile_menu_open: false,
            palette_open: false,
            palette_query: "",
            palette_selected: 0,
            profiles: &[],
            selected_profile: "powershell",
            font_size: 16.0,
            theme_name: "farman-dark",
            bidi_enabled: true,
            cols: 100,
            rows: 30,
            status: "ready",
            toast: Some("Glass"),
            mouse: (0.0, 0.0),
            config_path: "C:\\FARMAN\\config.toml",
            version: "v0.3.0",
            active_profile: "Windows PowerShell",
            starting: false,
            session_banner: None,
            maximized: false,
        });
        assert!(chrome.overlay_quads.iter().any(|q| q.radius > 12.0));
        assert!(chrome
            .hits
            .iter()
            .any(|h| matches!(h.action, UiAction::CloseOverlay)));
    }

    #[test]
    fn palette_filters_commands_by_all_terms() {
        assert_eq!(palette_matches("").len(), PALETTE_ITEMS.len());
        let themes = palette_matches("theme");
        assert_eq!(themes.len(), 3);
        assert!(themes.iter().all(|item| item.title.contains("Theme")));
        assert!(palette_matches("midnight")
            .iter()
            .any(|item| item.command == PaletteCommand::ThemeMidnight));
        assert!(palette_matches("theme blue")
            .iter()
            .any(|item| item.command == PaletteCommand::ThemeMidnight));
        assert!(palette_matches("no-such-command").is_empty());
    }

    #[test]
    fn palette_builds_action_hits_above_the_panel() {
        let chrome = build(ChromeInput {
            width: 1280.0,
            height: 800.0,
            scale: 1.0,
            theme: &FARMAN_DARK,
            tabs: &[],
            active: 0,
            settings_open: false,
            profile_menu_open: false,
            palette_open: true,
            palette_query: "midnight",
            palette_selected: 0,
            profiles: &[],
            selected_profile: "powershell",
            font_size: 16.0,
            theme_name: "farman-dark",
            bidi_enabled: true,
            cols: 100,
            rows: 30,
            status: "ready",
            toast: None,
            mouse: (640.0, 400.0),
            config_path: "C:\\FARMAN\\config.toml",
            version: "v0.3.0",
            active_profile: "Windows PowerShell",
            starting: false,
            session_banner: None,
            maximized: false,
        });
        assert!(chrome
            .hits
            .iter()
            .any(|hit| matches!(hit.action, UiAction::SetTheme(ref name) if name == "midnight")));
        assert!(chrome
            .text
            .iter()
            .any(|block| block.text == "COMMAND PALETTE"));
    }
}
