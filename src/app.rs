//! Application lifecycle, window, tabs, input dispatch and scene assembly.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::event::Event as TermEvent;
use alacritty_terminal::index::Side;
use anyhow::{anyhow, Context, Result};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{
    ElementState, Ime, KeyEvent, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, PhysicalKey};
use winit::window::{CursorIcon, Fullscreen, Icon, ResizeDirection, Window};

#[cfg(windows)]
use winit::platform::windows::WindowAttributesExtWindows;

use crate::config::{self, AppPaths, Config, LoadedConfig};
use crate::input::{self, KeyChord, MouseButton, MouseReport};
use crate::platform;
use crate::profile::{self, Profile};
use crate::renderer::{GpuRenderer, Scene};
use crate::session::Session;
use crate::snapshot::Snapshot;
use crate::text::BidiPolicy;
use crate::theme::Theme;
use crate::types::Rect;
use crate::ui::{self, ChromeInput, TabInfo, UiAction};

#[derive(Debug, Clone)]
pub enum AppEvent {
    SessionWake(u64),
}

struct Tab {
    id: u64,
    profile_id: String,
    profile_name: String,
    title: String,
    session: Option<Session>,
    error: Option<String>,
    starting: bool,
    exit_banner: Option<String>,
    started_at: Instant,
}

impl Tab {
    fn display_title(&self) -> String {
        if !self.title.is_empty() {
            truncate_title(&self.title, 36)
        } else {
            self.profile_name.clone()
        }
    }

    fn is_exited(&self) -> bool {
        self.exit_banner.is_some() || self.session.as_ref().is_some_and(|s| s.exit.is_some())
    }
}

#[derive(Debug, Clone, Copy)]
struct SelectionDrag {
    row: usize,
    col: usize,
}

struct GpuState {
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: GpuRenderer,
    adapter_name: String,
}

impl GpuState {
    fn new(
        window: Arc<Window>,
        event_loop: &ActiveEventLoop,
        font_size_px: f32,
        line_height: f32,
        family: &str,
    ) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance
            .create_surface(window.clone())
            .context("create wgpu surface")?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        }))
        .map_err(|err| anyhow!("no compatible GPU adapter: {err}"))?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("FARMAN device"),
            ..Default::default()
        }))
        .context("request wgpu device")?;

        let caps = surface.get_capabilities(&adapter);
        if caps.formats.is_empty() {
            return Err(anyhow!("surface reports no supported formats"));
        }
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        let renderer = GpuRenderer::new(
            &device,
            &queue,
            format,
            width,
            height,
            font_size_px,
            line_height,
            family,
        )?;
        tracing::info!(
            adapter = %info.name,
            backend = ?info.backend,
            format = ?format,
            "wgpu initialized"
        );
        Ok(Self {
            instance,
            surface,
            device,
            queue,
            config,
            renderer,
            adapter_name: info.name,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        let width = size.width.max(1);
        let height = size.height.max(1);
        if width == self.config.width && height == self.config.height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.renderer.resize(width, height);
    }

    fn recreate_surface(&mut self, window: Arc<Window>) -> Result<()> {
        self.surface = self
            .instance
            .create_surface(window)
            .context("recreate wgpu surface")?;
        self.surface.configure(&self.device, &self.config);
        Ok(())
    }

    fn render(&mut self, window: &Window, scene: &Scene) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated
            | wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                self.surface.configure(&self.device, &self.config);
                window.request_redraw();
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                return Err(anyhow!("GPU surface lost"));
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(anyhow!("surface validation error"));
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.render(&self.device, &self.queue, &view, scene)?;
        self.queue.present(frame);
        Ok(())
    }
}

pub struct App {
    paths: AppPaths,
    config: Config,
    config_path: PathBuf,
    profiles: Vec<Profile>,
    tabs: Vec<Tab>,
    active: usize,
    next_session_id: u64,
    theme: &'static Theme,
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    proxy: EventLoopProxy<AppEvent>,
    icon: Option<Icon>,
    modifiers: ModifiersState,
    mouse: (f32, f32),
    hover_hit: Option<usize>,
    settings_open: bool,
    profile_menu_open: bool,
    palette_open: bool,
    palette_query: String,
    palette_selected: usize,
    selected_profile: String,
    toast: Option<(String, Instant)>,
    status: String,
    chrome_hits: Vec<ui::HitRegion>,
    terminal_rect: Rect,
    chrome_drag_region: Rect,
    last_title_click: Option<(Instant, f32, f32)>,
    exit_requested: bool,
    grid: (usize, usize),
    cell_size: (f32, f32),
    selection_drag: Option<SelectionDrag>,
    mouse_down: Option<WinitMouseButton>,
    clipboard: Option<arboard::Clipboard>,
    last_scale: f64,
    notices: Vec<String>,
    fatal: Option<String>,
    fonts_applied: bool,
    /// Bumped when theme/config changes require terminal glyph colors to be
    /// re-shaped even though the underlying terminal generation is unchanged.
    view_epoch: u64,
}

impl App {
    pub fn new(loaded: LoadedConfig, paths: AppPaths, proxy: EventLoopProxy<AppEvent>) -> Self {
        let mut config = loaded.config;
        let mut notices = loaded.notices;
        notices.extend(config.normalize());
        let theme = Theme::by_name_or_default(&config.theme);
        let profiles = profile::discover(&config);
        let selected_profile = if profiles.iter().any(|p| p.id == config.default_profile) {
            config.default_profile.clone()
        } else {
            profiles.first().map(|p| p.id.clone()).unwrap_or_default()
        };
        let status = "Ready".to_owned();
        let toast = notices.first().cloned().map(|n| {
            (
                n,
                Instant::now() + Duration::from_secs(8),
            )
        });
        let icon = load_icon();
        Self {
            paths,
            config,
            config_path: loaded.path,
            profiles,
            tabs: Vec::new(),
            active: 0,
            next_session_id: 1,
            theme,
            window: None,
            gpu: None,
            proxy,
            icon,
            modifiers: ModifiersState::empty(),
            mouse: (0.0, 0.0),
            hover_hit: None,
            settings_open: false,
            profile_menu_open: false,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            selected_profile,
            toast,
            status,
            chrome_hits: Vec::new(),
            terminal_rect: Rect::default(),
            chrome_drag_region: Rect::default(),
            last_title_click: None,
            exit_requested: false,
            grid: (0, 0),
            cell_size: (0.0, 0.0),
            selection_drag: None,
            mouse_down: None,
            clipboard: None,
            last_scale: 1.0,
            notices,
            fatal: None,
            fonts_applied: false,
            view_epoch: 0,
        }
    }

    fn active_session(&self) -> Option<&Session> {
        self.tabs
            .get(self.active)
            .and_then(|tab| tab.session.as_ref())
    }

    fn active_session_mut(&mut self) -> Option<&mut Session> {
        self.tabs
            .get_mut(self.active)
            .and_then(|tab| tab.session.as_mut())
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn toggle_maximize(&self) {
        if let Some(window) = &self.window {
            window.set_maximized(!window.is_maximized());
        }
    }

    fn request_exit(&mut self) {
        self.exit_requested = true;
        self.request_redraw();
    }

    fn close_all_sessions(&mut self) {
        for tab in &mut self.tabs {
            if let Some(session) = tab.session.as_mut() {
                session.close();
            }
        }
    }

    fn set_toast(&mut self, message: impl Into<String>) {
        let message = message.into();
        tracing::info!(message = %message, "status");
        self.status = message.clone();
        self.toast = Some((message, Instant::now() + Duration::from_secs(5)));
        self.request_redraw();
    }

    fn save_config(&mut self) {
        if let Err(err) = config::save_config(&self.config_path, &self.config) {
            self.set_toast(format!("Could not save configuration: {err}"));
            tracing::error!(error = %err, "save configuration");
        }
    }

    fn apply_theme(&mut self, name: &str) {
        if let Some(theme) = Theme::by_name(name) {
            self.theme = theme;
            self.config.theme = name.to_owned();
            self.view_epoch = self.view_epoch.wrapping_add(1);
            if let Some(window) = self.window.as_deref() {
                platform::apply_borderless_chrome(
                    window,
                    (
                        theme.header_bg.r,
                        theme.header_bg.g,
                        theme.header_bg.b,
                    ),
                );
            }
        }
    }

    fn apply_font(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        let font_px = (self.config.font.size * scale).clamp(8.0, 96.0);
        gpu.renderer.set_font(
            &gpu.device,
            &gpu.queue,
            font_px,
            self.config.font.line_height,
            &self.config.font.family,
        );
        self.fonts_applied = true;
        self.update_grid();
        self.request_redraw();
    }

    fn update_grid(&mut self) {
        let (Some(window), Some(gpu)) = (self.window.as_ref(), self.gpu.as_ref()) else {
            return;
        };
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let rect = ui::terminal_rect(size.width as f32, size.height as f32, scale);
        let (cell_w, cell_h) = gpu.renderer.cell_metrics();
        if cell_w <= 0.0 || cell_h <= 0.0 {
            return;
        }
        let cols = ((rect.w / cell_w).floor() as usize).clamp(2, 500);
        let rows = ((rect.h / cell_h).floor() as usize).clamp(2, 500);
        self.terminal_rect = rect;
        self.cell_size = (cell_w, cell_h);
        if self.grid != (cols, rows) {
            self.grid = (cols, rows);
            let cell_w_px = cell_w.round().clamp(1.0, u16::MAX as f32) as u16;
            let cell_h_px = cell_h.round().clamp(1.0, u16::MAX as f32) as u16;
            for tab in &mut self.tabs {
                if let Some(session) = tab.session.as_mut() {
                    session.resize(cols as u16, rows as u16, cell_w_px, cell_h_px);
                }
            }
            tracing::info!(cols, rows, class = "grid", "grid resized");
            self.request_redraw();
        }
    }

    fn spawn_tab(&mut self, profile_id: Option<String>) {
        if self.gpu.is_none() {
            return;
        }
        if self.grid.0 < 2 || self.grid.1 < 2 {
            return;
        }
        let profile_id = profile_id.unwrap_or_else(|| self.selected_profile.clone());
        let Some(profile) = self
            .profiles
            .iter()
            .find(|p| p.id == profile_id)
            .or_else(|| self.profiles.first())
            .cloned()
        else {
            self.set_toast("No shell profiles are available");
            return;
        };
        let cell_w_px = self.cell_size.0.round().clamp(1.0, u16::MAX as f32) as u16;
        let cell_h_px = self.cell_size.1.round().clamp(1.0, u16::MAX as f32) as u16;
        let id = self.next_session_id;
        self.next_session_id += 1;
        let options = profile.spawn_options(
            id,
            self.grid.0 as u16,
            self.grid.1 as u16,
            cell_w_px,
            cell_h_px,
            &self.config,
        );
        let proxy = self.proxy.clone();
        let wake = Arc::new(move |session_id: u64| {
            let _ = proxy.send_event(AppEvent::SessionWake(session_id));
        });
        let session = Session::spawn(options, wake);
        let (session, error) = match session {
            Ok(session) => (Some(session), None),
            Err(err) => {
                tracing::error!(profile = %profile.name, error = %err, "failed to spawn shell");
                (None, Some(format!("Could not start {}: {err}", profile.name)))
            }
        };
        if let Some(err) = &error {
            self.set_toast(err.clone());
        }
        self.tabs.push(Tab {
            id,
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            title: String::new(),
            session,
            error,
            starting: true,
            exit_banner: None,
            started_at: Instant::now(),
        });
        self.active = self.tabs.len() - 1;
        self.selected_profile = profile.id;
        self.settings_open = false;
        self.profile_menu_open = false;
        self.request_redraw();
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let mut tab = self.tabs.remove(index);
        if let Some(session) = tab.session.as_mut() {
            session.close();
        }
        if self.tabs.is_empty() {
            self.active = 0;
        } else if index < self.active {
            self.active -= 1;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        self.selection_drag = None;
        self.request_redraw();
    }

    fn cycle_tab(&mut self, backwards: bool) {
        if self.tabs.len() < 2 {
            return;
        }
        if backwards {
            self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
        } else {
            self.active = (self.active + 1) % self.tabs.len();
        }
        self.request_redraw();
    }

    fn toggle_palette(&mut self) {
        self.palette_open = !self.palette_open;
        self.settings_open = false;
        self.profile_menu_open = false;
        self.palette_query.clear();
        self.palette_selected = 0;
        self.request_redraw();
    }

    fn palette_key(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(winit::keyboard::NamedKey::Escape) => {
                self.palette_open = false;
                self.palette_query.clear();
                self.palette_selected = 0;
            }
            Key::Named(winit::keyboard::NamedKey::ArrowUp) => {
                let count = ui::palette_matches(&self.palette_query).len();
                if count > 0 {
                    self.palette_selected = (self.palette_selected + count - 1) % count;
                }
            }
            Key::Named(winit::keyboard::NamedKey::ArrowDown) => {
                let count = ui::palette_matches(&self.palette_query).len();
                if count > 0 {
                    self.palette_selected = (self.palette_selected + 1) % count;
                }
            }
            Key::Named(winit::keyboard::NamedKey::Enter) => {
                self.run_palette();
                return;
            }
            Key::Named(winit::keyboard::NamedKey::Backspace) => {
                self.palette_query.pop();
                self.palette_selected = 0;
            }
            Key::Character(text)
                if !self.modifiers.control_key() && !self.modifiers.alt_key() =>
            {
                let mut added = false;
                for ch in text.chars() {
                    if !ch.is_control() && self.palette_query.chars().count() < 64 {
                        self.palette_query.push(ch);
                        added = true;
                    }
                }
                if added {
                    self.palette_selected = 0;
                }
            }
            _ => {}
        }
        self.request_redraw();
    }

    fn run_palette(&mut self) {
        let action = ui::palette_matches(&self.palette_query)
            .get(self.palette_selected)
            .map(|item| item.command.action());
        let Some(action) = action else {
            return;
        };
        self.palette_open = false;
        self.palette_query.clear();
        self.palette_selected = 0;
        self.handle_ui_action(action);
    }

    fn handle_ui_action(&mut self, action: UiAction) {
        if !matches!(action, UiAction::TogglePalette | UiAction::Noop) {
            self.palette_open = false;
        }
        match action {
            UiAction::NewTab(profile) => {
                self.spawn_tab(profile);
            }
            UiAction::CloseTab(index) => self.close_tab(index),
            UiAction::ActivateTab(index) => {
                if index < self.tabs.len() {
                    self.active = index;
                    self.selection_drag = None;
                    self.request_redraw();
                }
            }
            UiAction::ToggleSettings => {
                self.settings_open = !self.settings_open;
                self.profile_menu_open = false;
                self.request_redraw();
            }
            UiAction::CloseOverlay => {
                self.settings_open = false;
                self.profile_menu_open = false;
                self.palette_open = false;
                self.palette_query.clear();
                self.palette_selected = 0;
                self.request_redraw();
            }
            UiAction::FontDelta(delta) => {
                let next = (self.config.font.size + delta as f32).clamp(8.0, 48.0);
                if (next - self.config.font.size).abs() > f32::EPSILON {
                    self.config.font.size = next;
                    self.apply_font();
                    self.save_config();
                }
            }
            UiAction::SetTheme(name) => {
                self.apply_theme(&name);
                self.save_config();
                self.request_redraw();
            }
            UiAction::CycleDefaultProfile => {
                if !self.profiles.is_empty() {
                    let index = self
                        .profiles
                        .iter()
                        .position(|p| p.id == self.config.default_profile)
                        .unwrap_or(0);
                    let next = &self.profiles[(index + 1) % self.profiles.len()];
                    self.config.default_profile = next.id.clone();
                    self.selected_profile = next.id.clone();
                    self.save_config();
                    self.request_redraw();
                }
            }
            UiAction::ToggleBidi => {
                self.config.bidi.enabled = !self.config.bidi.enabled;
                self.save_config();
                self.request_redraw();
            }
            UiAction::ReloadConfig => {
                let loaded = config::load_or_create(&self.paths);
                self.config = loaded.config;
                self.config.normalize();
                self.notices = loaded.notices;
                self.theme = Theme::by_name_or_default(&self.config.theme);
                self.profiles = profile::discover(&self.config);
                self.selected_profile = self.config.default_profile.clone();
                self.view_epoch = self.view_epoch.wrapping_add(1);
                self.apply_font();
                self.set_toast("Configuration reloaded");
                self.request_redraw();
            }
            UiAction::ToggleProfileMenu => {
                self.profile_menu_open = !self.profile_menu_open;
                self.settings_open = false;
                self.request_redraw();
            }
            UiAction::TogglePalette => self.toggle_palette(),
            UiAction::MinimizeWindow => {
                if let Some(window) = &self.window {
                    window.set_minimized(true);
                }
            }
            UiAction::ToggleMaximize => self.toggle_maximize(),
            UiAction::CloseWindow => self.request_exit(),
            UiAction::Noop => {}
        }
    }

    fn poll_sessions(&mut self) -> bool {
        let theme = self.theme;
        let mut events: Vec<(usize, Vec<TermEvent>)> = Vec::new();
        let mut changed = false;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            if let Some(session) = tab.session.as_mut() {
                if session.poll(theme) {
                    tab.starting = false;
                    changed = true;
                }
                if tab.title != session.title {
                    tab.title = session.title.clone();
                    changed = true;
                }
                let pending = session.take_events();
                if !pending.is_empty() {
                    events.push((index, pending));
                }
                if session.exit.is_some() && tab.exit_banner.is_none() {
                    let info = session.exit.as_ref().unwrap();
                    let code = info
                        .code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unknown".to_owned());
                    tab.exit_banner = Some(if info.success {
                        "Shell exited".to_owned()
                    } else {
                        format!("Shell exited (code {code})")
                    });
                    tab.starting = false;
                    changed = true;
                }
            }
        }
        for (index, pending) in events {
            for event in pending {
                match event {
                    TermEvent::ClipboardStore(_, text) => {
                        if text.len() <= 1_048_576 {
                            self.set_clipboard_text(text);
                        }
                    }
                    TermEvent::ClipboardLoad(_, formatter) => {
                        let text = self
                            .clipboard
                            .as_mut()
                            .and_then(|c| c.get_text().ok())
                            .unwrap_or_default();
                        let response = formatter(&text);
                        if let Some(tab) = self.tabs.get_mut(index) {
                            if let Some(session) = tab.session.as_ref() {
                                session.write(response.into_bytes());
                            }
                        }
                    }
                    TermEvent::Bell => {}
                    _ => {}
                }
            }
        }
        if changed {
            self.request_redraw();
        }
        changed
    }

    fn set_clipboard_text(&mut self, text: String) {
        let result = self.ensure_clipboard().map(|clipboard| clipboard.set_text(text));
        match result {
            Some(Ok(())) => self.set_toast("Copied to clipboard"),
            Some(Err(err)) => self.set_toast(format!("Clipboard error: {err}")),
            None => self.set_toast("Clipboard is unavailable"),
        }
    }

    fn ensure_clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    fn copy_selection(&mut self) {
        let Some(session) = self.active_session() else {
            self.set_toast("No active session");
            return;
        };
        match session.selection_text() {
            Some(text) if !text.is_empty() => {
                let chars = text.chars().count();
                self.set_clipboard_text(text);
                self.status = format!("Copied {chars} characters");
            }
            _ => self.set_toast("Select text first"),
        }
    }

    fn paste_clipboard(&mut self) {
        let Some(text) = self
            .clipboard
            .as_mut()
            .and_then(|c| c.get_text().ok())
        else {
            // Reinitialize once; clipboard may have been unavailable at start.
            self.clipboard = arboard::Clipboard::new().ok();
            let Some(text) = self.clipboard.as_mut().and_then(|c| c.get_text().ok()) else {
                self.set_toast("Clipboard is empty or unavailable");
                return;
            };
            self.send_paste(text);
            return;
        };
        self.send_paste(text);
    }

    fn send_paste(&mut self, text: String) {
        if text.len() > 4 * 1024 * 1024 {
            self.set_toast("Paste rejected: clipboard is larger than 4 MiB");
            return;
        }
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(session) = tab.session.as_mut() else {
            self.set_toast("No active session");
            return;
        };
        if session.exit.is_some() {
            self.set_toast("The shell has exited");
            return;
        }
        let bracketed = session.bracketed_paste();
        let mut normalized = text.replace("\r\n", "\n").replace('\n', "\r");
        normalized = normalized.replace('\0', "");
        if bracketed {
            normalized = normalized.replace("\x1b[201~", "");
            let mut bytes = Vec::with_capacity(normalized.len() + 12);
            bytes.extend_from_slice(b"\x1b[200~");
            bytes.extend_from_slice(normalized.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            session.write(bytes);
        } else {
            session.write(normalized.into_bytes());
        }
        self.request_redraw();
    }

    fn handle_shortcut(&mut self, event: &KeyEvent, mods: ModifiersState) -> bool {
        let bindings = self.config.keybindings.clone();
        let matches = |text: &str| {
            KeyChord::parse(text)
                .map(|chord| chord.matches(event, mods))
                .unwrap_or(false)
        };

        if matches(&bindings.new_tab) {
            self.spawn_tab(None);
            return true;
        }
        if matches(&bindings.close_tab) {
            self.close_tab(self.active);
            return true;
        }
        if matches(&bindings.copy) {
            self.copy_selection();
            return true;
        }
        if matches(&bindings.paste) {
            self.paste_clipboard();
            return true;
        }
        if matches(&bindings.settings) {
            self.settings_open = !self.settings_open;
            self.profile_menu_open = false;
            self.request_redraw();
            return true;
        }
        if matches(&bindings.next_tab) {
            self.cycle_tab(mods.shift_key());
            return true;
        }
        if matches(&bindings.font_increase) {
            self.handle_ui_action(UiAction::FontDelta(1));
            return true;
        }
        if matches(&bindings.font_decrease) {
            self.handle_ui_action(UiAction::FontDelta(-1));
            return true;
        }

        // Local scrollback. Shift+PageUp/PageDown is the conventional terminal
        // binding and must not be forwarded to full-screen applications.
        if mods.shift_key() && !mods.control_key() && !mods.alt_key() {
            match event.logical_key {
                Key::Named(winit::keyboard::NamedKey::PageUp) => {
                    if let Some(session) = self.active_session_mut() {
                        session.scroll_page(true);
                    }
                    self.request_redraw();
                    return true;
                }
                Key::Named(winit::keyboard::NamedKey::PageDown) => {
                    if let Some(session) = self.active_session_mut() {
                        session.scroll_page(false);
                    }
                    self.request_redraw();
                    return true;
                }
                _ => {}
            }
        }
        if mods.control_key() && mods.shift_key() {
            match event.logical_key {
                Key::Named(winit::keyboard::NamedKey::ArrowUp) => {
                    if let Some(session) = self.active_session_mut() {
                        session.scroll(1);
                    }
                    self.request_redraw();
                    return true;
                }
                Key::Named(winit::keyboard::NamedKey::ArrowDown) => {
                    if let Some(session) = self.active_session_mut() {
                        session.scroll(-1);
                    }
                    self.request_redraw();
                    return true;
                }
                _ => {}
            }
        }
        // Portable extras that are intentionally hardware-key based.
        if mods.control_key() && !mods.alt_key() {
            if let Key::Character(text) = &event.logical_key {
                match text.chars().next() {
                    Some('+') | Some('=') => {
                        self.handle_ui_action(UiAction::FontDelta(1));
                        return true;
                    }
                    Some('-') => {
                        self.handle_ui_action(UiAction::FontDelta(-1));
                        return true;
                    }
                    Some('0') => {
                        self.config.font.size = 16.0;
                        self.apply_font();
                        self.save_config();
                        return true;
                    }
                    Some('1'..='9') => {
                        let index = text.chars().next().unwrap().to_digit(10).unwrap() as usize - 1;
                        if index < self.tabs.len() {
                            self.active = index;
                            self.request_redraw();
                        }
                        return true;
                    }
                    _ => {}
                }
            }
        }
        if event.logical_key == Key::Named(winit::keyboard::NamedKey::F11) {
            if let Some(window) = &self.window {
                let fullscreen = window.fullscreen().is_some();
                window.set_fullscreen(if fullscreen {
                    None
                } else {
                    Some(Fullscreen::Borderless(None))
                });
            }
            return true;
        }
        if mods.control_key() && mods.shift_key() {
            match &event.logical_key {
                Key::Character(text) if text.as_ref() == "C" || text.as_ref() == "c" => {
                    self.copy_selection();
                    return true;
                }
                Key::Character(text) if text.as_ref() == "V" || text.as_ref() == "v" => {
                    self.paste_clipboard();
                    return true;
                }
                _ => {}
            }
        }
        if mods.control_key() && event.logical_key == Key::Named(winit::keyboard::NamedKey::Insert) {
            self.copy_selection();
            return true;
        }
        if mods.shift_key() && event.logical_key == Key::Named(winit::keyboard::NamedKey::Insert) {
            self.paste_clipboard();
            return true;
        }
        false
    }

    fn keyboard_input(&mut self, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        if self.modifiers.control_key() && !self.modifiers.alt_key() {
            let logical_k = matches!(
                &event.logical_key,
                Key::Character(text) if text.as_ref().eq_ignore_ascii_case("k")
            );
            if logical_k || event.physical_key == PhysicalKey::Code(KeyCode::KeyK) {
                self.toggle_palette();
                return;
            }
        }
        if self.palette_open {
            self.palette_key(&event);
            return;
        }
        if self.settings_open || self.profile_menu_open {
            if event.logical_key == Key::Named(winit::keyboard::NamedKey::Escape) {
                self.settings_open = false;
                self.profile_menu_open = false;
                self.request_redraw();
                return;
            }
            // Application shortcuts remain available; typing does not go to
            // the shell while a modal panel is open.
            let _ = self.handle_shortcut(&event, self.modifiers);
            return;
        }
        if self.handle_shortcut(&event, self.modifiers) {
            return;
        }
        let app_cursor = self
            .active_session()
            .map(|s| s.application_cursor())
            .unwrap_or(false);
        if let Some(bytes) = input::encode_key(&event, self.modifiers, app_cursor) {
            if let Some(session) = self.active_session() {
                if session.is_running() {
                    session.write(bytes);
                    self.request_redraw();
                } else {
                    self.set_toast(    "The shell has exited - Ctrl+Shift+T for a new tab");
                }
            }
        }
    }

    fn ime_commit(&mut self, text: &str) {
        if self.palette_open {
            if self.palette_query.chars().count() < 64 {
                self.palette_query.push_str(text);
                self.palette_selected = 0;
                self.request_redraw();
            }
            return;
        }
        if self.settings_open || self.profile_menu_open {
            return;
        }
        if let Some(session) = self.active_session() {
            if session.is_running() {
                session.write(text.as_bytes());
                self.request_redraw();
            }
        }
    }

    fn active_snapshot(&self) -> Option<Snapshot> {
        self.active_session()
            .map(|session| session.snapshot(self.theme))
    }

    fn terminal_cell_at(&self, x: f32, y: f32, snapshot: &Snapshot) -> Option<(usize, usize, Side)> {
        let gpu = self.gpu.as_ref()?;
        if !self.terminal_rect.contains(x, y) {
            return None;
        }
        let (cell_w, cell_h) = gpu.renderer.cell_metrics();
        let local_x = x - self.terminal_rect.x;
        let local_y = y - self.terminal_rect.y;
        let visual_col = (local_x / cell_w).floor().max(0.0) as usize;
        let row = (local_y / cell_h).floor().max(0.0) as usize;
        if row >= snapshot.rows.len() || visual_col >= snapshot.cols {
            return None;
        }
        let visual = gpu.renderer.visual_row(row);
        let col = visual
            .map(|v| v.logical_at_visual(visual_col))
            .unwrap_or(visual_col);
        let fraction = (local_x / cell_w).fract();
        let side = if fraction > 0.5 { Side::Right } else { Side::Left };
        Some((row, col, side))
    }

    fn begin_selection(&mut self, x: f32, y: f32) {
        let Some(snapshot) = self.active_snapshot() else {
            return;
        };
        let Some((row, col, side)) = self.terminal_cell_at(x, y, &snapshot) else {
            return;
        };
        if let Some(session) = self.active_session_mut() {
            session.selection_start(row, col, side);
        }
        self.selection_drag = Some(SelectionDrag { row, col });
        self.request_redraw();
    }

    fn update_selection(&mut self, x: f32, y: f32) {
        let Some(snapshot) = self.active_snapshot() else {
            return;
        };
        let Some((row, col, side)) = self.terminal_cell_at(x, y, &snapshot) else {
            return;
        };
        if let Some(session) = self.active_session_mut() {
            session.selection_update(row, col, side);
        }
        self.selection_drag = Some(SelectionDrag { row, col });
        self.request_redraw();
    }

    fn finish_selection(&mut self) {
        self.selection_drag = None;
        if let Some(session) = self.active_session_mut() {
            if !session.has_selection() {
                session.selection_clear();
            }
        }
        self.request_redraw();
    }

    fn mouse_report(&mut self, button: MouseButton, pressed: bool, motion: bool) {
        let Some(snapshot) = self.active_snapshot() else {
            return;
        };
        let Some(cell) = self.terminal_cell_at(self.mouse.0, self.mouse.1, &snapshot) else {
            return;
        };
        let report = MouseReport {
            button,
            pressed,
            motion,
            col: cell.1,
            row: cell.0,
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
            ctrl: self.modifiers.control_key(),
            sgr: snapshot.sgr_mouse,
            utf8: snapshot.utf8_mouse,
        };
        let bytes = input::encode_mouse(report);
        if let Some(session) = self.active_session() {
            session.write(bytes);
        }
    }

    fn resize_direction(&self) -> Option<ResizeDirection> {
        let window = self.window.as_ref()?;
        if window.fullscreen().is_some() || window.is_maximized() {
            return None;
        }
        let size = window.inner_size();
        let edge = 6.0 * window.scale_factor() as f32;
        let (x, y) = self.mouse;
        let w = size.width as f32;
        let h = size.height as f32;
        if w <= 1.0 || h <= 1.0 {
            return None;
        }
        let left = x <= edge;
        let right = x >= w - edge;
        let top = y <= edge;
        let bottom = y >= h - edge;
        match (top, bottom, left, right) {
            (true, _, true, _) => Some(ResizeDirection::NorthWest),
            (true, _, _, true) => Some(ResizeDirection::NorthEast),
            (_, true, true, _) => Some(ResizeDirection::SouthWest),
            (_, true, _, true) => Some(ResizeDirection::SouthEast),
            (true, _, _, _) => Some(ResizeDirection::North),
            (_, true, _, _) => Some(ResizeDirection::South),
            (_, _, true, _) => Some(ResizeDirection::West),
            (_, _, _, true) => Some(ResizeDirection::East),
            _ => None,
        }
    }

    fn update_cursor(&self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let cursor = if let Some(direction) = self.resize_direction() {
            CursorIcon::from(direction)
        } else if ui::hit_test(&self.chrome_hits, self.mouse.0, self.mouse.1)
            .as_ref()
            .is_some_and(|action| {
                !matches!(action, UiAction::CloseOverlay | UiAction::Noop)
            })
        {
            CursorIcon::Pointer
        } else if self.terminal_rect.contains(self.mouse.0, self.mouse.1) {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };
        window.set_cursor(cursor);
    }

    fn titlebar_press(&mut self) {
        let now = Instant::now();
        let (x, y) = self.mouse;
        let double = self.last_title_click.is_some_and(|(then, last_x, last_y)| {
            now.duration_since(then) < Duration::from_millis(420)
                && (x - last_x).abs() < 6.0
                && (y - last_y).abs() < 6.0
        });
        if double {
            self.last_title_click = None;
            self.toggle_maximize();
            return;
        }
        self.last_title_click = Some((now, x, y));
        if let Some(window) = self.window.as_ref() {
            let _ = window.drag_window();
        }
    }

    fn mouse_input(&mut self, state: ElementState, button: WinitMouseButton) {
        let in_terminal = self.terminal_rect.contains(self.mouse.0, self.mouse.1);
        match (state, button) {
            (ElementState::Pressed, WinitMouseButton::Left) => {
                if let Some(direction) = self.resize_direction() {
                    if let Some(window) = self.window.as_ref() {
                        let _ = window.drag_resize_window(direction);
                    }
                    return;
                }
                if let Some(action) = ui::hit_test(&self.chrome_hits, self.mouse.0, self.mouse.1) {
                    self.handle_ui_action(action);
                    return;
                }
                if self.settings_open || self.profile_menu_open || self.palette_open {
                    self.settings_open = false;
                    self.profile_menu_open = false;
                    self.palette_open = false;
                    self.palette_query.clear();
                    self.palette_selected = 0;
                    self.request_redraw();
                    return;
                }
                if self.chrome_drag_region.contains(self.mouse.0, self.mouse.1) {
                    self.titlebar_press();
                    return;
                }
                if in_terminal {
                    let snapshot = self.active_snapshot();
                    let report_to_app = snapshot
                        .as_ref()
                        .is_some_and(|s| s.mouse_reporting && !self.modifiers.shift_key());
                    if report_to_app {
                        self.mouse_down = Some(WinitMouseButton::Left);
                        self.mouse_report(MouseButton::Left, true, false);
                    } else {
                        self.begin_selection(self.mouse.0, self.mouse.1);
                    }
                }
            }
            (ElementState::Released, WinitMouseButton::Left) => {
                if self.mouse_down == Some(WinitMouseButton::Left) {
                    self.mouse_down = None;
                    self.mouse_report(MouseButton::Left, false, false);
                } else {
                    self.finish_selection();
                }
            }
            (ElementState::Pressed, WinitMouseButton::Middle) => {
                if let Some(UiAction::ActivateTab(index)) =
                    ui::hit_test(&self.chrome_hits, self.mouse.0, self.mouse.1)
                {
                    self.close_tab(index);
                }
            }
            (ElementState::Pressed, WinitMouseButton::Right) => {
                if in_terminal {
                    let snapshot = self.active_snapshot();
                    if snapshot
                        .as_ref()
                        .is_some_and(|s| s.mouse_reporting && !self.modifiers.shift_key())
                    {
                        self.mouse_down = Some(WinitMouseButton::Right);
                        self.mouse_report(MouseButton::Right, true, false);
                    } else {
                        self.paste_clipboard();
                    }
                }
            }
            (ElementState::Released, WinitMouseButton::Right) => {
                if self.mouse_down == Some(WinitMouseButton::Right) {
                    self.mouse_down = None;
                    self.mouse_report(MouseButton::Right, false, false);
                }
            }
            _ => {}
        }
    }

    fn mouse_moved(&mut self, x: f32, y: f32) {
        self.mouse = (x, y);
        if self.selection_drag.is_some() {
            self.update_selection(x, y);
            self.update_cursor();
            return;
        }
        if self.mouse_down.is_some() {
            let snapshot = self.active_snapshot();
            if snapshot.as_ref().is_some_and(|s| s.mouse_motion) {
                self.mouse_report(MouseButton::None, true, true);
            }
        }
        let new_hit = self
            .chrome_hits
            .iter()
            .enumerate()
            .rev()
            .find(|(_, hit)| hit.rect.contains(x, y))
            .map(|(i, _)| i);
        if new_hit != self.hover_hit {
            self.hover_hit = new_hit;
            self.request_redraw();
        }
        self.update_cursor();
    }

    fn mouse_wheel(&mut self, delta: MouseScrollDelta) {
        if !self.terminal_rect.contains(self.mouse.0, self.mouse.1) {
            return;
        }
        if self.settings_open || self.profile_menu_open || self.palette_open {
            return;
        }
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => y.round() as i32 * 3,
            MouseScrollDelta::PixelDelta(pos) => {
                let cell_h = self.cell_size.1.max(1.0);
                (pos.y as f32 / cell_h).round() as i32
            }
        };
        if lines == 0 {
            return;
        }
        let snapshot = self.active_snapshot();
        let report_to_app = snapshot
            .as_ref()
            .is_some_and(|s| s.mouse_reporting && !self.modifiers.shift_key());
        if report_to_app {
            // Wheel buttons use codes 64/65 rather than button 3.
            let Some(snapshot) = snapshot else { return };
            let Some(cell) = self.terminal_cell_at(self.mouse.0, self.mouse.1, &snapshot) else {
                return;
            };
            let mut bytes = Vec::new();
            let button = if lines > 0 { 64 } else { 65 };
            if snapshot.sgr_mouse {
                bytes.extend_from_slice(
                    format!(
                        "\x1b[<{};{};{}M",
                        button,
                        cell.1 + 1,
                        cell.0 + 1
                    )
                    .as_bytes(),
                );
            } else {
                bytes.extend_from_slice(b"\x1b[M");
                bytes.push((32 + button) as u8);
                bytes.push((32 + cell.1 + 1).min(255) as u8);
                bytes.push((32 + cell.0 + 1).min(255) as u8);
            }
            if let Some(session) = self.active_session() {
                session.write(bytes);
            }
        } else if let Some(session) = self.active_session_mut() {
            session.scroll(lines);
            self.request_redraw();
        }
    }

    fn on_resize(&mut self, size: PhysicalSize<u32>) {
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(size);
        }
        self.update_grid();
        self.request_redraw();
    }

    fn on_scale_changed(&mut self, scale: f64) {
        self.last_scale = scale;
        let size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(PhysicalSize::new(1, 1));
        if let Some(gpu) = self.gpu.as_mut() {
            gpu.resize(size);
        }
        self.apply_font();
    }

    fn build_and_render(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        if self.gpu.is_none() {
            return;
        }
        if window.inner_size().width == 0 || window.inner_size().height == 0 {
            return;
        }
        self.poll_sessions();
        self.update_grid();
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        let scale = window.scale_factor() as f32;
        let mut tab_titles: Vec<String> = Vec::with_capacity(self.tabs.len());
        for tab in &self.tabs {
            tab_titles.push(tab.display_title());
        }
        let tab_infos: Vec<TabInfo<'_>> = self
            .tabs
            .iter()
            .zip(tab_titles.iter())
            .map(|(tab, title)| TabInfo {
                title,
                starting: tab.starting,
                exited: tab.is_exited(),
            })
            .collect();
        let active = self.active.min(self.tabs.len().saturating_sub(1));
        let active_profile = self
            .tabs
            .get(active)
            .map(|t| t.profile_name.as_str())
            .unwrap_or("Shell");
        let starting = self.tabs.get(active).is_some_and(|t| t.starting && t.session.is_some());
        let banner = self
            .tabs
            .get(active)
            .and_then(|t| t.exit_banner.clone().or_else(|| t.error.clone()));
        let banner_ref = banner.as_deref();
        let toast_ref = self
            .toast
            .as_ref()
            .filter(|(_, expiry)| Instant::now() < *expiry)
            .map(|(text, _)| text.as_str());
        let config_path = self.config_path.to_string_lossy().into_owned();
        let status = self.status.clone();
        let theme_name = self.config.theme.clone();
        let maximized = window.is_maximized();
        let chrome = ui::build(ChromeInput {
            width: window.inner_size().width as f32,
            height: window.inner_size().height as f32,
            scale,
            theme: self.theme,
            tabs: &tab_infos,
            active,
            settings_open: self.settings_open,
            profile_menu_open: self.profile_menu_open,
            palette_open: self.palette_open,
            palette_query: &self.palette_query,
            palette_selected: self.palette_selected,
            profiles: &self.profiles,
            selected_profile: &self.selected_profile,
            font_size: self.config.font.size,
            theme_name: &theme_name,
            bidi_enabled: self.config.bidi.enabled,
            cols: self.grid.0,
            rows: self.grid.1,
            status: &status,
            toast: toast_ref,
            mouse: self.mouse,
            config_path: &config_path,
            version: concat!("v", env!("CARGO_PKG_VERSION")),
            active_profile,
            starting,
            session_banner: banner_ref,
            maximized,
        });
        self.chrome_hits = chrome.hits;
        self.terminal_rect = chrome.terminal_rect;
        self.chrome_drag_region = chrome.drag_region;
        let snapshot = self
            .tabs
            .get(active)
            .and_then(|t| t.session.as_ref())
            .map(|s| {
                let mut snapshot = s.snapshot(self.theme);
                snapshot.generation = snapshot.generation.wrapping_add(self.view_epoch);
                snapshot
            });
        let scene = Scene {
            background: self.theme.background,
            terminal_bg: self.theme.background,
            terminal_fg: self.theme.foreground,
            cursor_color: self.theme.cursor,
            selection_color: self.theme.selection,
            base_quads: chrome.base_quads,
            overlay_quads: chrome.overlay_quads,
            text: chrome.text,
            terminal: snapshot,
            terminal_rect: chrome.terminal_rect,
            bidi_policy: BidiPolicy::from_config(&self.config.bidi),
        };
        if let Err(err) = gpu.render(window, &scene) {
            tracing::error!(error = %err, "render failed");
            match gpu.recreate_surface(window.clone()) {
                Ok(()) => window.request_redraw(),
                Err(recreate) => {
                    self.fatal = Some(format!("{err}; could not recover: {recreate}"));
                    tracing::error!(error = %recreate, "surface recovery failed");
                    self.set_toast("Rendering device lost. Please restart FARMAN.");
                }
            }
        }
        self.update_window_title();
    }

    fn update_window_title(&self) {
        let Some(window) = &self.window else {
            return;
        };
        let title = self
            .tabs
            .get(self.active)
            .map(|tab| tab.display_title())
            .unwrap_or_else(|| "No session".to_owned());
        window.set_title(&format!("FARMAN Terminal - {title}"));
    }

    fn ensure_initial_tab(&mut self) {
        if !self.tabs.is_empty() {
            return;
        }
        self.spawn_tab(None);
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title("FARMAN Terminal")
            .with_inner_size(LogicalSize::new(
                self.config.window.width as f64,
                self.config.window.height as f64,
            ))
            .with_min_inner_size(LogicalSize::new(820.0, 560.0))
            .with_window_icon(self.icon.take())
            .with_decorations(false)
            .with_maximized(self.config.window.maximized);
        #[cfg(windows)]
        {
            attrs = attrs.with_undecorated_shadow(true);
        }
        let window = match event_loop.create_window(attrs) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                tracing::error!(error = %err, "could not create window");
                self.fatal = Some(format!("Could not create the FARMAN window: {err}"));
                event_loop.exit();
                return;
            }
        };
        platform::fit_window_to_work_area(&window);
        window.set_ime_allowed(true);
        platform::apply_borderless_chrome(
            &window,
            (
                self.theme.header_bg.r,
                self.theme.header_bg.g,
                self.theme.header_bg.b,
            ),
        );
        let scale = window.scale_factor() as f32;
        let font_px = (self.config.font.size * scale).clamp(8.0, 96.0);
        match GpuState::new(
            window.clone(),
            event_loop,
            font_px,
            self.config.font.line_height,
            &self.config.font.family,
        ) {
            Ok(gpu) => {
                self.gpu = Some(gpu);
                self.window = Some(window.clone());
                self.last_scale = window.scale_factor();
                self.update_grid();
                self.ensure_initial_tab();
                self.update_window_title();
                window.focus_window();
                window.request_redraw();
                tracing::info!(
                    version = env!("CARGO_PKG_VERSION"),
                    config = %self.config_path.display(),
                    "FARMAN Terminal started"
                );
            }
            Err(err) => {
                tracing::error!(error = %err, "GPU initialization failed");
                self.fatal = Some(format!("FARMAN could not initialize GPU rendering:\n{err}"));
                platform::message_box("FARMAN Terminal", &self.fatal.clone().unwrap(), true);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if self.gpu.is_none() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                self.close_all_sessions();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => self.on_resize(size),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.on_scale_changed(scale_factor)
            }
            WindowEvent::RedrawRequested => self.build_and_render(),
            WindowEvent::KeyboardInput { event, .. } => self.keyboard_input(event),
            WindowEvent::Ime(Ime::Commit(text)) => self.ime_commit(&text),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_moved(position.x as f32, position.y as f32);
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse = (-1.0, -1.0);
                self.hover_hit = None;
                self.update_cursor();
                self.request_redraw();
            }
            WindowEvent::MouseInput {
                state,
                button,
                ..
            } => self.mouse_input(state, button),
            WindowEvent::MouseWheel { delta, .. } => self.mouse_wheel(delta),
            WindowEvent::Focused(focused) => {
                if let Some(window) = &self.window {
                    window.set_ime_allowed(focused);
                }
                self.request_redraw();
            }
            WindowEvent::Occluded(_) => {}
            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::SessionWake(_) => {
                self.poll_sessions();
                self.request_redraw();
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.exit_requested {
            self.close_all_sessions();
            event_loop.exit();
            return;
        }
        if let Some((_, expiry)) = &self.toast {
            if Instant::now() >= *expiry {
                self.toast = None;
                self.request_redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(*expiry));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
        // Starting sessions need a watchdog only for the visual hint. All data
        // delivery is event-driven, so no polling loop is required.
        if self.tabs.iter().any(|t| t.starting && t.started_at.elapsed() > Duration::from_secs(30)) {
            for tab in &mut self.tabs {
                if tab.starting && tab.started_at.elapsed() > Duration::from_secs(30) {
                    tab.starting = false;
                }
            }
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        for tab in &mut self.tabs {
            if let Some(session) = tab.session.as_mut() {
                session.close();
            }
        }
        tracing::info!("FARMAN Terminal exiting");
    }
}

fn truncate_title(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

fn load_icon() -> Option<Icon> {
    let bytes = include_bytes!("../assets/farman-icon.png");
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let image = image::imageops::resize(&image, 256, 256, image::imageops::FilterType::Lanczos3);
    Icon::from_rgba(image.into_raw(), 256, 256).ok()
}

