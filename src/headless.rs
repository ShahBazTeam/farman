//! Offscreen GPU render probe.
//!
//! `farman --render-test <png>` builds a scene with mixed Persian/English
//! terminal rows, renders it through the real wgpu + glyphon pipeline into an
//! sRGB texture, reads the pixels back and writes a PNG. This is how the RTL
//! path is verified without relying on a screenshot of the window.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use image::{ImageBuffer, Rgba as ImageRgba};
use unicode_width::UnicodeWidthChar;
use wgpu::{
    BufferDescriptor, BufferUsages, CommandEncoderDescriptor, DeviceDescriptor, Extent3d,
    Instance, InstanceDescriptor, MapMode, PollType, RequestAdapterOptions, TexelCopyBufferInfo,
    TexelCopyBufferLayout, TexelCopyTextureInfo, TextureDescriptor, TextureDimension,
    TextureFormat, TextureUsages,
};

use crate::renderer::{GpuRenderer, Scene};
use crate::snapshot::{CellView, CursorShapeView, CursorView, Snapshot};
use crate::text::{BidiPolicy, TERMINAL_FAMILY, UI_FAMILY};
use crate::theme::FARMAN_DARK;
use crate::types::{Rgb, Rect};

pub fn run(path: &Path, width: u32, height: u32) -> Result<()> {
    pollster::block_on(render(path, width, height))
}

async fn render(path: &Path, width: u32, height: u32) -> Result<()> {
    let instance = Instance::new(InstanceDescriptor::new_without_display_handle());
    let adapter = instance
        .request_adapter(&RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .map_err(|err| anyhow!("no GPU adapter available for headless render: {err}"))?;
    let info = adapter.get_info();
    let (device, queue) = adapter
        .request_device(&DeviceDescriptor {
            label: Some("FARMAN headless device"),
            ..Default::default()
        })
        .await
        .context("headless request_device")?;
    let format = TextureFormat::Rgba8UnormSrgb;
    let mut renderer = GpuRenderer::new(
        &device,
        &queue,
        format,
        width,
        height,
        18.0,
        1.35,
        TERMINAL_FAMILY,
    )?;
    let (cell_w, cell_h) = renderer.cell_metrics();
    let terminal_rect = Rect::new(24.0, 76.0, width as f32 - 48.0, height as f32 - 120.0);
    let cols = ((terminal_rect.w / cell_w).floor() as usize).max(20);
    let rows = ((terminal_rect.h / cell_h).floor() as usize).max(10);

    let mut snapshot = Snapshot::empty(cols, rows);
    snapshot.generation = 1;
    snapshot.session_id = 9001;
    snapshot.cursor = Some(CursorView {
        row: 7,
        col: 9,
        shape: CursorShapeView::Block,
    });

    let cyan = FARMAN_DARK.accent;
    let green = FARMAN_DARK.success;
    let yellow = FARMAN_DARK.warning;
    let violet = FARMAN_DARK.accent_violet;
    let muted = FARMAN_DARK.dim_foreground;
    let lines: &[(&str, Rgb)] = &[
        ("FARMAN Terminal — GPU/RTL render probe", FARMAN_DARK.bright_foreground),
        ("", FARMAN_DARK.foreground),
        ("خطا در main.rs:42", FARMAN_DARK.foreground),
        ("مسیر فایل: C:\\Users\\User\\Desktop\\Project", cyan),
        ("نسخه Python: 3.13", green),
        ("سلام دنیا — FARMAN فرمان", violet),
        ("PS D:\\farman> python app.py", FARMAN_DARK.foreground),
        ("سلام", FARMAN_DARK.foreground),
        ("کد خطا: 0x80070005 — access denied", yellow),
        ("✓ ANSI true color   ✓ Unicode bidi   ✓ ConPTY", muted),
        ("مسیر: D:\\پروژه‌ها\\فارسی\\main.rs", FARMAN_DARK.foreground),
        ("FARMAN · فرمان · terminal", cyan),
    ];
    for (row, (line, color)) in lines.iter().enumerate().take(rows) {
        fill_row(&mut snapshot.rows[row], line, *color, FARMAN_DARK.background);
    }

    // A simple UI header proves the same glyph atlas is used for chrome text.
    let mut scene = Scene {
        background: FARMAN_DARK.background,
        terminal_bg: FARMAN_DARK.background,
        terminal_fg: FARMAN_DARK.foreground,
        cursor_color: FARMAN_DARK.cursor,
        selection_color: FARMAN_DARK.selection,
        terminal: Some(snapshot),
        terminal_rect,
        bidi_policy: BidiPolicy::Auto,
        ..Default::default()
    };
    scene.base_quads.push(crate::renderer::Quad::new(
        Rect::new(0.0, 0.0, width as f32, 56.0),
        crate::types::Rgba::opaque(FARMAN_DARK.header_bg),
    ));
    scene.base_quads.push(crate::renderer::Quad::new(
        Rect::new(24.0, 56.0, width as f32 - 48.0, 1.0),
        crate::types::Rgba::opaque(FARMAN_DARK.border),
    ));
    scene.text.push(
        crate::renderer::TextBlock::new(
            "FARMAN  /  offscreen RTL verification · فرمان",
            Rect::new(24.0, 14.0, 900.0, 30.0),
            16.0,
            FARMAN_DARK.foreground,
        )
        .weight(cosmic_text::Weight::MEDIUM),
    );
    scene.text.push(
        crate::renderer::TextBlock::new(
            "bidi=auto  font=Vazir Code Hack",
            Rect::new(width as f32 - 424.0, 18.0, 400.0, 24.0),
            11.0,
            FARMAN_DARK.muted,
        )
        .align(crate::renderer::TextAlign::Right),
    );
    scene.text.push(
        crate::renderer::TextBlock::new(
            "GPU текст shaping — UBA — UTF-8",
            Rect::new(24.0, height as f32 - 30.0, width as f32 - 48.0, 20.0),
            10.0,
            FARMAN_DARK.dim_foreground,
        )
        .align(crate::renderer::TextAlign::Center),
    );

    let texture = device.create_texture(&TextureDescriptor {
        label: Some("FARMAN headless target"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.render(&device, &queue, &view, &scene)?;

    let unpadded = width * 4;
    let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("FARMAN headless readback"),
        size: (padded * height) as u64,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("FARMAN headless copy"),
    });
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    let _ = device.poll(PollType::Wait {
        submission_index: None,
        timeout: None,
    });
    rx.recv()
        .map_err(|_| anyhow!("GPU readback channel closed"))?
        .map_err(|err| anyhow!("GPU readback failed: {err}"))?;
    let data = slice
        .get_mapped_range()
        .map_err(|err| anyhow!("map readback buffer: {err}"))?;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        rgba.extend_from_slice(&data[start..start + unpadded as usize]);
    }
    drop(data);
    buffer.unmap();

    let image: ImageBuffer<ImageRgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, rgba).ok_or_else(|| anyhow!("invalid readback image"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    image
        .save(path)
        .with_context(|| format!("save {}", path.display()))?;

    let non_bg = image
        .pixels()
        .filter(|p| {
            p[0] != FARMAN_DARK.background.r
                || p[1] != FARMAN_DARK.background.g
                || p[2] != FARMAN_DARK.background.b
        })
        .count();
    let persian_pixels = image
        .pixels()
        .filter(|p| p[0] as u32 + p[1] as u32 + p[2] as u32 > 200)
        .count();
    if non_bg < 10_000 {
        return Err(anyhow!(
            "render probe produced too few non-background pixels ({non_bg})"
        ));
    }
    println!(
        "render-test OK: {} ({}x{}), adapter={}, non-bg-pixels={}, bright-pixels={}, cell={:.2}x{:.2}, grid={}x{}",
        path.display(),
        width,
        height,
        info.name,
        non_bg,
        persian_pixels,
        cell_w,
        cell_h,
        cols,
        rows
    );
    println!("fonts: terminal={TERMINAL_FAMILY}, ui={UI_FAMILY}; bidi=auto; scale=1.0");
    Ok(())
}

fn fill_row(
    row: &mut crate::snapshot::RowView,
    text: &str,
    fg: Rgb,
    bg: Rgb,
) {
    let mut col = 0usize;
    for ch in text.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width == 0 {
            if col > 0 {
                if let Some(cell) = row.cells.get_mut(col - 1) {
                    cell.zerowidth.push(ch);
                }
            }
            continue;
        }
        let span = width.min(2);
        if col + span > row.cells.len() {
            break;
        }
        {
            let cell: &mut CellView = &mut row.cells[col];
            cell.ch = ch;
            cell.fg = fg;
            cell.bg = bg;
            cell.flags.wide = span == 2;
        }
        if span == 2 {
            let spacer: &mut CellView = &mut row.cells[col + 1];
            spacer.ch = ' ';
            spacer.fg = fg;
            spacer.bg = bg;
            spacer.flags.wide_spacer = true;
        }
        col += span;
    }
}

