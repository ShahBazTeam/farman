//! GPU renderer: colored quads plus cosmic-text/glyphon glyph rendering.
//!
//! Terminal cells are painted as quads at their visual (bidi-mapped) position,
//! then the shaped terminal paragraph is drawn on top. UI chrome is drawn in a
//! second pass so overlays can cover terminal output without terminal glyphs
//! bleeding through.

use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};
use cosmic_text::{Attrs, Buffer, Family, Metrics, Shaping, Weight, Wrap};
use glyphon::{
    Color as GlyphonColor, Resolution, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};
use std::mem;
use wgpu::{
    AddressMode, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry,
    BindingType, BlendState, BufferBindingType, BufferDescriptor, BufferUsages, ColorTargetState,
    ColorWrites, Device, Extent3d, FilterMode, FragmentState, LoadOp, MultisampleState, Operations,
    Origin3d, PipelineLayoutDescriptor, PrimitiveState, Queue, RenderPass,
    RenderPassColorAttachment, RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor,
    SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    StoreOp, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension, VertexAttribute, VertexBufferLayout, VertexFormat,
    VertexState, VertexStepMode,
};

use crate::snapshot::{CursorShapeView, Snapshot};
use crate::text::{BidiPolicy, RowVisual, ScreenText, TextEngine, UI_FAMILY};
use crate::types::{Rgb, Rgba, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
pub struct Quad {
    pub rect: Rect,
    /// Bottom color. With `top_color` this creates a vertical glass gradient.
    pub color: Rgba,
    pub top_color: Rgba,
    pub radius: f32,
    /// Anti-alias / soft-edge width in physical pixels. Large values turn a
    /// rounded rectangle into a gentle soft-edged card/shadow.
    pub softness: f32,
    /// 1.0 for a radial light field, 0.0 for a solid/gradient rounded rect.
    pub radial: f32,
    /// Adds a subtle per-pixel frosted grain to glass fills.
    pub frost: f32,
}

impl Quad {
    pub fn new(rect: Rect, color: Rgba) -> Self {
        Self {
            rect,
            color,
            top_color: color,
            radius: 0.0,
            softness: 0.75,
            radial: 0.0,
            frost: 0.0,
        }
    }

    pub fn rounded(rect: Rect, color: Rgba, radius: f32) -> Self {
        Self {
            rect,
            color,
            top_color: color,
            radius,
            softness: 0.75,
            radial: 0.0,
            frost: 0.0,
        }
    }

    pub fn glass(rect: Rect, top: Rgba, bottom: Rgba, radius: f32) -> Self {
        Self {
            rect,
            color: bottom,
            top_color: top,
            radius,
            softness: 0.75,
            radial: 0.0,
            frost: 0.0,
        }
    }

    /// A soft radial light field. The rect is the glow's bounding box and the
    /// center is its midpoint; the alpha fades quadratically to the edges.
    pub fn glow(rect: Rect, color: Rgba, _softness: f32) -> Self {
        Self {
            rect,
            color,
            top_color: color,
            radius: 0.0,
            softness: 0.75,
            radial: 1.0,
            frost: 0.0,
        }
    }

    /// A rounded rectangle with a soft, blurred-looking edge.
    pub fn soft(rect: Rect, color: Rgba, radius: f32, softness: f32) -> Self {
        Self {
            rect,
            color,
            top_color: color,
            radius,
            softness,
            radial: 0.0,
            frost: 0.0,
        }
    }

    /// Give a translucent glass fill its fine frosted grain.
    pub fn frosted(mut self) -> Self {
        self.frost = 1.0;
        self
    }
}

#[derive(Debug, Clone)]
pub struct TextBlock {
    pub text: String,
    pub rect: Rect,
    pub size: f32,
    pub color: Rgb,
    /// Extra width used for alignment. Defaults to `rect.w`.
    pub width: Option<f32>,
    pub align: TextAlign,
    pub weight: Weight,
    pub mono: bool,
}

impl TextBlock {
    pub fn new(text: impl Into<String>, rect: Rect, size: f32, color: Rgb) -> Self {
        Self {
            text: text.into(),
            rect,
            size,
            color,
            width: None,
            align: TextAlign::Left,
            weight: Weight::NORMAL,
            mono: false,
        }
    }

    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn weight(mut self, weight: Weight) -> Self {
        self.weight = weight;
        self
    }

    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub background: Rgb,
    pub terminal_bg: Rgb,
    pub terminal_fg: Rgb,
    pub cursor_color: Rgb,
    pub selection_color: Rgb,
    pub base_quads: Vec<Quad>,
    pub overlay_quads: Vec<Quad>,
    pub text: Vec<TextBlock>,
    pub images: Vec<Rect>,
    pub terminal: Option<Snapshot>,
    pub terminal_rect: Rect,
    pub bidi_policy: BidiPolicy,
}

impl Default for Scene {
    fn default() -> Self {
        Self {
            background: Rgb::new(8, 13, 24),
            terminal_bg: Rgb::new(8, 13, 24),
            terminal_fg: Rgb::new(215, 226, 240),
            cursor_color: Rgb::new(53, 214, 232),
            selection_color: Rgb::new(37, 75, 102),
            base_quads: Vec::new(),
            overlay_quads: Vec::new(),
            text: Vec::new(),
            images: Vec::new(),
            terminal: None,
            terminal_rect: Rect::default(),
            bidi_policy: BidiPolicy::Auto,
        }
    }
}

pub struct GpuRenderer {
    engine: TextEngine,
    buffer: Option<Buffer>,
    screen: Option<ScreenText>,
    visuals: Vec<RowVisual>,
    last_revision: u64,
    last_session_id: u64,
    last_cols: usize,
    last_policy: BidiPolicy,
    last_terminal_size: (u32, u32),
    target_format: TextureFormat,
    width: u32,
    height: u32,
    quads: QuadPipeline,
    images: ImagePipeline,
    atlas: TextAtlas,
    viewport: Viewport,
    terminal_text: TextRenderer,
    ui_text: TextRenderer,
    _cache: glyphon::Cache,
    font_size: f32,
    line_height: f32,
    family: String,
}

impl GpuRenderer {
    pub fn new(
        device: &Device,
        queue: &Queue,
        target_format: TextureFormat,
        width: u32,
        height: u32,
        font_size_px: f32,
        line_height: f32,
        family: &str,
    ) -> Result<Self> {
        let engine = TextEngine::new(font_size_px, line_height, Some(family));
        let cache = glyphon::Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, target_format);
        let terminal_text = TextRenderer::new(
            &mut atlas,
            device,
            MultisampleState::default(),
            None,
        );
        let ui_text = TextRenderer::new(&mut atlas, device, MultisampleState::default(), None);
        let quads = QuadPipeline::new(device, target_format);
        let images = ImagePipeline::new(device, queue, target_format);
        Ok(Self {
            engine,
            buffer: None,
            screen: None,
            visuals: Vec::new(),
            last_revision: 0,
            last_session_id: 0,
            last_cols: 0,
            last_policy: BidiPolicy::Auto,
            last_terminal_size: (0, 0),
            target_format,
            width,
            height,
            quads,
            images,
            atlas,
            viewport,
            terminal_text,
            ui_text,
            _cache: cache,
            font_size: font_size_px,
            line_height,
            family: family.to_owned(),
        })
    }

    pub fn cell_metrics(&self) -> (f32, f32) {
        (self.engine.metrics.cell_width, self.engine.metrics.cell_height)
    }

    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    pub fn line_height(&self) -> f32 {
        self.line_height
    }

    pub fn family(&self) -> &str {
        &self.family
    }

    /// Visual cell map for the most recently shaped terminal frame. Mouse
    /// hit-testing uses this to select the logical cell under a visual column.
    pub fn visual_row(&self, row: usize) -> Option<&RowVisual> {
        self.visuals.get(row)
    }

    pub fn shaped_revision(&self) -> u64 {
        self.last_revision
    }

    pub fn set_font(
        &mut self,
        device: &Device,
        queue: &Queue,
        font_size_px: f32,
        line_height: f32,
        family: &str,
    ) {
        if (font_size_px - self.font_size).abs() < 0.01
            && (line_height - self.line_height).abs() < 0.001
            && family == self.family
        {
            return;
        }
        self.engine = TextEngine::new(font_size_px, line_height, Some(family));
        self.font_size = font_size_px;
        self.line_height = line_height;
        self.family = family.to_owned();
        self.buffer = None;
        self.screen = None;
        self.visuals.clear();
        self.last_revision = 0;
        self.invalidate_atlas(device, queue);
    }

    fn invalidate_atlas(&mut self, device: &Device, queue: &Queue) {
        // Glyph rasterization depends on font size/family. Recreate the atlas
        // and its renderers instead of allowing stale cache entries to draw.
        let cache = glyphon::Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, self.target_format);
        self.terminal_text =
            TextRenderer::new(&mut atlas, device, MultisampleState::default(), None);
        self.ui_text = TextRenderer::new(&mut atlas, device, MultisampleState::default(), None);
        self.atlas = atlas;
        self.viewport = viewport;
        self._cache = cache;
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    pub fn render(
        &mut self,
        device: &Device,
        queue: &Queue,
        view: &TextureView,
        scene: &Scene,
    ) -> Result<()> {
        self.viewport.update(
            queue,
            Resolution {
                width: self.width,
                height: self.height,
            },
        );

        // ---- terminal paragraph (logical text → visual order map) ----
        let mut terminal_area_buffer: Option<&Buffer> = None;
        if let Some(snapshot) = scene.terminal.as_ref() {
            let size = (scene.terminal_rect.w.max(1.0) as u32, scene.terminal_rect.h.max(1.0) as u32);
            let needs_rebuild = self.last_revision != snapshot.generation
                || self.last_session_id != snapshot.session_id
                || self.last_cols != snapshot.cols
                || self.last_policy != scene.bidi_policy
                || self.last_terminal_size != size
                || self.buffer.is_none();
            if needs_rebuild {
                let screen = ScreenText::build(snapshot, scene.bidi_policy);
                let mut buffer = if let Some(existing) = self.buffer.take() {
                    existing
                } else {
                    Buffer::new(
                        &mut self.engine.font_system,
                        Metrics::new(
                            self.engine.metrics.font_size,
                            self.engine.metrics.cell_height,
                        ),
                    )
                };
                // Keep the buffer metrics in sync if the engine changed.
                if (buffer.metrics().font_size - self.engine.metrics.font_size).abs() > 0.01 {
                    buffer = Buffer::new(
                        &mut self.engine.font_system,
                        Metrics::new(
                            self.engine.metrics.font_size,
                            self.engine.metrics.cell_height,
                        ),
                    );
                }
                let visuals = self.engine.shape_screen(
                    &mut buffer,
                    &screen,
                    scene.terminal_rect.w.max(1.0),
                    scene.terminal_rect.h.max(1.0),
                );
                self.visuals = visuals;
                self.screen = Some(screen);
                self.buffer = Some(buffer);
                self.last_revision = snapshot.generation;
                self.last_session_id = snapshot.session_id;
                self.last_cols = snapshot.cols;
                self.last_policy = scene.bidi_policy;
                self.last_terminal_size = size;
            }
            terminal_area_buffer = self.buffer.as_ref();
        } else {
            self.last_revision = 0;
        }

        // ---- UI text buffers ----
        let mut ui_buffers: Vec<Buffer> = Vec::with_capacity(scene.text.len());
        for block in &scene.text {
            if block.text.is_empty() {
                ui_buffers.push(Buffer::new(
                    &mut self.engine.font_system,
                    Metrics::new(block.size, (block.size * 1.35).ceil()),
                ));
                continue;
            }
            let mut buffer = Buffer::new(
                &mut self.engine.font_system,
                Metrics::new(block.size, (block.size * 1.35).ceil()),
            );
            let width = block.width.unwrap_or(block.rect.w).max(1.0);
            buffer.set_size(Some(width), Some(block.rect.h.max(block.size * 2.0)));
            buffer.set_wrap(Wrap::None);
            let align = match block.align {
                TextAlign::Left => cosmic_text::Align::Left,
                TextAlign::Center => cosmic_text::Align::Center,
                TextAlign::Right => cosmic_text::Align::Right,
            };
            let family = if block.mono {
                Family::Name("Vazir Code Hack")
            } else {
                Family::Name(UI_FAMILY)
            };
            let attrs = Attrs::new()
                .family(family)
                .weight(block.weight)
                .color(cosmic_text::Color::rgb(
                    block.color.r,
                    block.color.g,
                    block.color.b,
                ));
            buffer.set_text(&block.text, &attrs, Shaping::Advanced, Some(align));
            buffer.shape_until_scroll(&mut self.engine.font_system, false);
            ui_buffers.push(buffer);
        }

        // ---- prepare terminal glyphs ----
        let engine = &mut self.engine;
        let terminal_bounds = TextBounds {
            left: scene.terminal_rect.x.max(0.0) as i32,
            top: scene.terminal_rect.y.max(0.0) as i32,
            right: scene.terminal_rect.right().ceil() as i32,
            bottom: scene.terminal_rect.bottom().ceil() as i32,
        };
        let terminal_areas: Vec<TextArea<'_>> = terminal_area_buffer
            .map(|buffer| {
                vec![TextArea {
                    buffer,
                    left: scene.terminal_rect.x,
                    top: scene.terminal_rect.y,
                    scale: 1.0,
                    bounds: terminal_bounds,
                    default_color: GlyphonColor::rgb(
                        scene.terminal_fg.r,
                        scene.terminal_fg.g,
                        scene.terminal_fg.b,
                    ),
                    custom_glyphs: &[],
                }]
            })
            .unwrap_or_default();
        self.terminal_text
            .prepare(
                device,
                queue,
                &mut engine.font_system,
                &mut self.atlas,
                &self.viewport,
                terminal_areas,
                &mut engine.swash_cache,
            )
            .map_err(|err| anyhow!("terminal glyph prepare failed: {err}"))?;

        // ---- prepare UI text ----
        let ui_areas: Vec<TextArea<'_>> = ui_buffers
            .iter()
            .zip(scene.text.iter())
            .map(|(buffer, block)| TextArea {
                buffer,
                left: block.rect.x,
                top: block.rect.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: block.rect.x.max(0.0) as i32,
                    top: block.rect.y.max(0.0) as i32,
                    right: block.rect.right().ceil() as i32,
                    bottom: block.rect.bottom().ceil() as i32,
                },
                default_color: GlyphonColor::rgb(
                    block.color.r,
                    block.color.g,
                    block.color.b,
                ),
                custom_glyphs: &[],
            })
            .collect();
        self.ui_text
            .prepare(
                device,
                queue,
                &mut engine.font_system,
                &mut self.atlas,
                &self.viewport,
                ui_areas,
                &mut engine.swash_cache,
            )
            .map_err(|err| anyhow!("UI glyph prepare failed: {err}"))?;

        let mut base_quads = scene.base_quads.clone();
        self.append_terminal_quads(scene, &mut base_quads);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("FARMAN frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("FARMAN base pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color {
                            r: scene.background.to_linear(1.0)[0] as f64,
                            g: scene.background.to_linear(1.0)[1] as f64,
                            b: scene.background.to_linear(1.0)[2] as f64,
                            a: 1.0,
                        }),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.quads.draw(
                device,
                queue,
                &mut pass,
                &base_quads,
                self.width,
                self.height,
            );
            self.images.draw(
                device,
                queue,
                &mut pass,
                &scene.images,
                self.width,
                self.height,
            );
            self.terminal_text
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(|err| anyhow!("terminal glyph render failed: {err}"))?;
        }
        {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("FARMAN overlay pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !scene.overlay_quads.is_empty() {
                self.quads.draw(
                    device,
                    queue,
                    &mut pass,
                    &scene.overlay_quads,
                    self.width,
                    self.height,
                );
            }
            self.ui_text
                .render(&self.atlas, &self.viewport, &mut pass)
                .map_err(|err| anyhow!("UI glyph render failed: {err}"))?;
        }
        queue.submit(Some(encoder.finish()));
        self.atlas.trim();
        Ok(())
    }

    fn append_terminal_quads(&self, scene: &Scene, out: &mut Vec<Quad>) {
        let (Some(snapshot), Some(_screen)) = (scene.terminal.as_ref(), self.screen.as_ref()) else {
            return;
        };
        let Some(cursor) = snapshot.cursor else {
            // Even without a cursor, paint non-default cell backgrounds.
            self.append_cell_backgrounds(scene, snapshot, out);
            return;
        };
        self.append_cell_backgrounds(scene, snapshot, out);
        let Some(row) = self.visuals.get(cursor.row) else {
            return;
        };
        let Some((x, _)) = row.cell_rect(cursor.col) else {
            return;
        };
        let cell_h = self.engine.metrics.cell_height;
        let cell_w = self.engine.metrics.cell_width;
        let (caret_x, _) = row.caret_x(cursor.col, cell_w);
        let y = scene.terminal_rect.y + cursor.row as f32 * cell_h;
        match cursor.shape {
            CursorShapeView::Hidden => {}
            CursorShapeView::Block => {
                out.push(Quad::rounded(
                    Rect::new(scene.terminal_rect.x + x, y, cell_w, cell_h),
                    Rgba::with_alpha(scene.cursor_color, 0.55),
                    3.0,
                ));
            }
            CursorShapeView::Beam => {
                let w = (cell_w * 0.16).clamp(1.0, 3.0);
                out.push(Quad::rounded(
                    Rect::new(scene.terminal_rect.x + caret_x - w, y, w, cell_h),
                    Rgba::opaque(scene.cursor_color),
                    w * 0.5,
                ));
            }
            CursorShapeView::Underline => {
                let h = (cell_h * 0.08).clamp(1.0, 3.0);
                out.push(Quad::new(
                    Rect::new(
                        scene.terminal_rect.x + x,
                        y + cell_h - h,
                        cell_w,
                        h,
                    ),
                    Rgba::opaque(scene.cursor_color),
                ));
            }
            CursorShapeView::HollowBlock => {
                let t = 1.0;
                out.push(Quad::new(
                    Rect::new(scene.terminal_rect.x + x, y, cell_w, t),
                    Rgba::opaque(scene.cursor_color),
                ));
                out.push(Quad::new(
                    Rect::new(scene.terminal_rect.x + x, y + cell_h - t, cell_w, t),
                    Rgba::opaque(scene.cursor_color),
                ));
                out.push(Quad::new(
                    Rect::new(scene.terminal_rect.x + x, y, t, cell_h),
                    Rgba::opaque(scene.cursor_color),
                ));
                out.push(Quad::new(
                    Rect::new(scene.terminal_rect.x + x + cell_w - t, y, t, cell_h),
                    Rgba::opaque(scene.cursor_color),
                ));
            }
        }
    }

    fn append_cell_backgrounds(&self, scene: &Scene, snapshot: &Snapshot, out: &mut Vec<Quad>) {
        let cell_h = self.engine.metrics.cell_height;
        for (row_index, row_cells) in snapshot.rows.iter().enumerate() {
            let Some(visual) = self.visuals.get(row_index) else {
                continue;
            };
            for (col, cell) in row_cells.cells.iter().enumerate() {
                if col >= visual.cols || cell.flags.wide_spacer || cell.flags.leading_wide_spacer {
                    continue;
                }
                let mut bg = cell.bg;
                let mut fg = cell.fg;
                if cell.flags.inverse {
                    mem::swap(&mut bg, &mut fg);
                }
                let selected = snapshot.is_selected(row_index, col);
                if selected {
                    bg = bg.blend(scene.selection_color, 0.40);
                }
                if !selected && bg == scene.terminal_bg {
                    continue;
                }
                let Some((x, w)) = visual.cell_rect(col) else {
                    continue;
                };
                let y = scene.terminal_rect.y + row_index as f32 * cell_h;
                out.push(Quad::new(
                    Rect::new(scene.terminal_rect.x + x, y, w, cell_h),
                    Rgba::opaque(bg),
                ));
                if cell.flags.underline {
                    let h = (cell_h * 0.06).clamp(1.0, 2.0);
                    out.push(Quad::new(
                        Rect::new(
                            scene.terminal_rect.x + x,
                            y + cell_h - h - 1.0,
                            w,
                            h,
                        ),
                        Rgba::opaque(fg),
                    ));
                }
                if cell.flags.strike {
                    let h = (cell_h * 0.05).clamp(1.0, 2.0);
                    out.push(Quad::new(
                        Rect::new(
                            scene.terminal_rect.x + x,
                            y + cell_h * 0.52,
                            w,
                            h,
                        ),
                        Rgba::opaque(fg),
                    ));
                }
            }
        }
    }
}



#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    color: [f32; 4],
    local: [f32; 2],
    half_size: [f32; 2],
    radius: f32,
    softness: f32,
    radial: f32,
    frost: f32,
}

const VERTEX_ATTRIBUTES: [VertexAttribute; 8] = [
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    VertexAttribute {
        format: VertexFormat::Float32x4,
        offset: 8,
        shader_location: 1,
    },
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 24,
        shader_location: 2,
    },
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 32,
        shader_location: 3,
    },
    VertexAttribute {
        format: VertexFormat::Float32,
        offset: 40,
        shader_location: 4,
    },
    VertexAttribute {
        format: VertexFormat::Float32,
        offset: 44,
        shader_location: 5,
    },
    VertexAttribute {
        format: VertexFormat::Float32,
        offset: 48,
        shader_location: 6,
    },
    VertexAttribute {
        format: VertexFormat::Float32,
        offset: 52,
        shader_location: 7,
    },
];

const QUAD_SHADER: &str = r#"
struct Globals {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) local: vec2<f32>,
    @location(3) half_size: vec2<f32>,
    @location(4) radius: f32,
    @location(5) softness: f32,
    @location(6) radial: f32,
    @location(7) frost: f32,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    @location(3) radius: f32,
    @location(4) softness: f32,
    @location(5) radial: f32,
    @location(6) frost: f32,
};

fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r, r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    let ndc = vec2<f32>(
        input.pos.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - input.pos.y / globals.viewport.y * 2.0,
    );
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.color = input.color;
    out.local = input.local;
    out.half_size = input.half_size;
    out.radius = input.radius;
    out.softness = input.softness;
    out.radial = input.radial;
    out.frost = input.frost;
    return out;
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    var coverage: f32;
    if (input.radial > 0.5) {
        let uv = input.local / max(input.half_size, vec2<f32>(1.0));
        let t = clamp(1.0 - length(uv), 0.0, 1.0);
        coverage = t * t * (3.0 - 2.0 * t);
    } else {
        let d = sd_round_box(input.local, input.half_size, input.radius);
        let feather = max(input.softness, 0.75);
        coverage = clamp(0.5 - d / feather, 0.0, 1.0);
    }
    if coverage <= 0.001 {
        discard;
    }
    var rgb = input.color.rgb;
    if (input.frost > 0.5) {
        // Tiny linear-light perturbation: at the dark end of an sRGB target a
        // large linear delta would look like visible static, so keep it below
        // roughly one 8-bit code point after conversion.
        let grain = (hash21(input.position.xy) - 0.5) * 0.0005;
        rgb = clamp(rgb + vec3<f32>(grain), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(rgb, input.color.a * coverage);
}
"#;

struct QuadPipeline {
    pipeline: RenderPipeline,
    bind_group: BindGroup,
    _bind_group_layout: BindGroupLayout,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    vertex_capacity: u64,
    vertex_count: u32,
}

impl QuadPipeline {
    fn new(device: &Device, format: TextureFormat) -> Self {
        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("FARMAN quad shader"),
            source: ShaderSource::Wgsl(QUAD_SHADER.into()),
        });
        let bind_group_layout =
            device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("FARMAN quad globals layout"),
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let uniform = device.create_buffer(&BufferDescriptor {
            label: Some("FARMAN quad globals"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FARMAN quad globals group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("FARMAN quad pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("FARMAN quad pipeline"),
            layout: Some(&layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: mem::size_of::<Vertex>() as u64,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES,
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertex_capacity = 6 * 4096 * mem::size_of::<Vertex>() as u64;
        let vertices = device.create_buffer(&BufferDescriptor {
            label: Some("FARMAN quad vertices"),
            size: vertex_capacity,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bind_group,
            _bind_group_layout: bind_group_layout,
            uniform,
            vertices,
            vertex_capacity,
            vertex_count: 0,
        }
    }

    fn draw(
        &mut self,
        device: &Device,
        queue: &Queue,
        pass: &mut RenderPass<'_>,
        quads: &[Quad],
        width: u32,
        height: u32,
    ) {
        if quads.is_empty() {
            return;
        }
        let mut vertices = Vec::with_capacity(quads.len() * 6);
        for quad in quads {
            let x0 = quad.rect.x;
            let y0 = quad.rect.y;
            let x1 = quad.rect.right();
            let y1 = quad.rect.bottom();
            let cx = (x0 + x1) * 0.5;
            let cy = (y0 + y1) * 0.5;
            let half_size = [(x1 - x0).abs() * 0.5, (y1 - y0).abs() * 0.5];
            let radius = quad.radius.max(0.0).min(half_size[0].min(half_size[1]));
            let softness = quad.softness.max(0.75);
            let top = quad.top_color.to_linear();
            let bottom = quad.color.to_linear();
            let radial = quad.radial;
            let frost = quad.frost;
            let mut push = |x: f32, y: f32, color: [f32; 4]| {
                vertices.push(Vertex {
                    pos: [x, y],
                    color,
                    local: [x - cx, y - cy],
                    half_size,
                    radius,
                    softness,
                    radial,
                    frost,
                });
            };
            push(x0, y0, top);
            push(x1, y0, top);
            push(x1, y1, bottom);
            push(x0, y0, top);
            push(x1, y1, bottom);
            push(x0, y1, bottom);
        }
        let bytes: &[u8] = bytemuck::cast_slice(&vertices);
        if bytes.len() as u64 > self.vertex_capacity {
            self.vertex_capacity = (bytes.len() as u64).next_power_of_two();
            self.vertices = device.create_buffer(&BufferDescriptor {
                label: Some("FARMAN quad vertices"),
                size: self.vertex_capacity,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.vertices, 0, bytes);
        let globals = [width as f32, height as f32, 0.0, 0.0];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&globals));
        self.vertex_count = vertices.len() as u32;
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
    }
}

const MARK_PNG: &[u8] = include_bytes!("../assets/farman-mark.png");

const IMAGE_SHADER: &str = r#"
struct Globals {
    viewport: vec2<f32>,
    _pad: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var mark: texture_2d<f32>;
@group(0) @binding(2) var mark_sampler: sampler;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(input: VertexIn) -> VertexOut {
    var out: VertexOut;
    let ndc = vec2<f32>(
        input.pos.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - input.pos.y / globals.viewport.y * 2.0,
    );
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = input.uv;
    return out;
}

@fragment
fn fs_main(input: VertexOut) -> @location(0) vec4<f32> {
    return textureSample(mark, mark_sampler, input.uv);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ImageVertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

const IMAGE_VERTEX_ATTRIBUTES: [VertexAttribute; 2] = [
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 0,
        shader_location: 0,
    },
    VertexAttribute {
        format: VertexFormat::Float32x2,
        offset: 8,
        shader_location: 1,
    },
];

struct ImagePipeline {
    pipeline: RenderPipeline,
    bind_group: BindGroup,
    _bind_group_layout: BindGroupLayout,
    uniform: wgpu::Buffer,
    vertices: wgpu::Buffer,
    vertex_capacity: u64,
    vertex_count: u32,
}

impl ImagePipeline {
    fn new(device: &Device, queue: &Queue, format: TextureFormat) -> Self {
        let image = image::load_from_memory(MARK_PNG)
            .expect("embedded FARMAN mark must decode")
            .to_rgba8();
        let (width, height) = image.dimensions();
        let row_bytes = width as usize * 4;
        let padded_row_bytes = row_bytes
            .div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
        let mut pixels = vec![0u8; padded_row_bytes * height as usize];
        for row in 0..height as usize {
            let source = &image.as_raw()[row * row_bytes..(row + 1) * row_bytes];
            pixels[row * padded_row_bytes..row * padded_row_bytes + row_bytes]
                .copy_from_slice(source);
        }
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("FARMAN mark texture"),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &pixels,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row_bytes as u32),
                rows_per_image: Some(height),
            },
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&TextureViewDescriptor::default());
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("FARMAN mark sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let shader = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("FARMAN image shader"),
            source: ShaderSource::Wgsl(IMAGE_SHADER.into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("FARMAN image layout"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let uniform = device.create_buffer(&BufferDescriptor {
            label: Some("FARMAN image globals"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FARMAN image group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("FARMAN image pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("FARMAN image pipeline"),
            layout: Some(&layout),
            vertex: VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(VertexBufferLayout {
                    array_stride: mem::size_of::<ImageVertex>() as u64,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &IMAGE_VERTEX_ATTRIBUTES,
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let vertex_capacity = 6 * 64 * mem::size_of::<ImageVertex>() as u64;
        let vertices = device.create_buffer(&BufferDescriptor {
            label: Some("FARMAN image vertices"),
            size: vertex_capacity,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bind_group,
            _bind_group_layout: bind_group_layout,
            uniform,
            vertices,
            vertex_capacity,
            vertex_count: 0,
        }
    }

    fn draw(
        &mut self,
        device: &Device,
        queue: &Queue,
        pass: &mut RenderPass<'_>,
        images: &[Rect],
        width: u32,
        height: u32,
    ) {
        if images.is_empty() {
            return;
        }
        let mut vertices = Vec::with_capacity(images.len() * 6);
        for rect in images {
            let x0 = rect.x;
            let y0 = rect.y;
            let x1 = rect.right();
            let y1 = rect.bottom();
            let mut push = |x: f32, y: f32, u: f32, v: f32| {
                vertices.push(ImageVertex {
                    pos: [x, y],
                    uv: [u, v],
                });
            };
            push(x0, y0, 0.0, 0.0);
            push(x1, y0, 1.0, 0.0);
            push(x1, y1, 1.0, 1.0);
            push(x0, y0, 0.0, 0.0);
            push(x1, y1, 1.0, 1.0);
            push(x0, y1, 0.0, 1.0);
        }
        let bytes: &[u8] = bytemuck::cast_slice(&vertices);
        if bytes.len() as u64 > self.vertex_capacity {
            self.vertex_capacity = (bytes.len() as u64).next_power_of_two();
            self.vertices = device.create_buffer(&BufferDescriptor {
                label: Some("FARMAN image vertices"),
                size: self.vertex_capacity,
                usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.vertices, 0, bytes);
        let globals = [width as f32, height as f32, 0.0, 0.0];
        queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&globals));
        self.vertex_count = vertices.len() as u32;
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
    }
}

/// Small helper for building scenes consistently.
pub fn text_block_bounds(rect: Rect) -> TextBounds {
    TextBounds {
        left: rect.x.max(0.0) as i32,
        top: rect.y.max(0.0) as i32,
        right: rect.right().ceil() as i32,
        bottom: rect.bottom().ceil() as i32,
    }
}

/// Verify that a scene's terminal glyph cache is populated. Used by the
/// headless renderer's self-test.
pub fn has_terminal_text(renderer: &GpuRenderer) -> bool {
    renderer
        .screen
        .as_ref()
        .is_some_and(|screen| !screen.text.is_empty())
}

#[allow(dead_code)]
fn _assert_send() {
    fn is_send<T: Send>() {}
    // GpuRenderer is intentionally not required to be Send: it lives on the
    // winit event-loop thread. This function merely documents the constraint.
    let _ = is_send::<Scene>;
}

