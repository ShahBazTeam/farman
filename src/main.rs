#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Diagnostic helpers (text-probe/render-test), test-only accessors and a few
// extension points for forthcoming UI work are intentionally kept in the
// binary. Their use is covered by the test suite and CLI probes.
#![allow(dead_code)]

mod app;
mod config;
mod headless;
mod input;
mod platform;
mod profile;
mod renderer;
mod session;
mod snapshot;
mod text;
mod theme;
mod types;
mod ui;

use std::path::PathBuf;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use crate::app::{App, AppEvent};
use crate::config::AppPaths;
use crate::text::{debug_shape, TextEngine, TERMINAL_FAMILY};

fn main() {
    if let Err(err) = run() {
        tracing::error!(error = %err, "fatal error");
        platform::message_box("FARMAN Terminal", &format!("FARMAN could not start:\n{err}"), true);
        eprintln!("FARMAN could not start: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("FARMAN Terminal v{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "--text-probe") {
        return text_probe();
    }
    if args.first().is_some_and(|a| a == "--render-test") {
        let path = args
            .get(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/rtl-render.png"));
        println!("FARMAN GPU render probe → {}", path.display());
        return headless::run(&path, 1280, 800);
    }

    let paths = AppPaths::discover();
    paths.ensure().context("create application data directories")?;
    let _guard = init_logging(&paths)?;

    std::panic::set_hook(Box::new(|info| {
        tracing::error!(panic = %info, "panic");
    }));

    let loaded = config::load_or_create(&paths);
    for notice in &loaded.notices {
        tracing::warn!(notice = %notice, "configuration notice");
    }

    let event_loop = winit::event_loop::EventLoop::<AppEvent>::with_user_event()
        .build()
        .context("build winit event loop")?;
    let proxy = event_loop.create_proxy();
    let mut app = App::new(loaded, paths, proxy);
    event_loop
        .run_app(&mut app)
        .context("run winit application")?;
    Ok(())
}

fn init_logging(paths: &AppPaths) -> Result<tracing_appender::non_blocking::WorkerGuard> {
    let appender = tracing_appender::rolling::daily(&paths.log_dir, "farman.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);
    let filter = EnvFilter::try_from_env("FARMAN_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        log_dir = %paths.log_dir.display(),
        "logger initialized"
    );
    Ok(guard)
}

fn text_probe() -> Result<()> {
    let mut engine = TextEngine::new(18.0, 1.35, Some(TERMINAL_FAMILY));
    println!(
        "FARMAN text probe: cell={:.3}x{:.3}, family={}",
        engine.metrics.cell_width,
        engine.metrics.cell_height,
        engine.family()
    );
    let samples = [
        "خطا در main.rs:42",
        "مسیر فایل: C:\\Users\\User\\Desktop\\Project",
        "نسخه Python: 3.13",
        "سلام دنیا",
        "FARMAN Terminal فرمان",
    ];
    for sample in samples {
        let (rtl, glyphs, width) = debug_shape(&mut engine, sample, 1600.0);
        println!("\nINPUT: {sample}\n  rtl={rtl} width={width:.2} glyphs={}", glyphs.len());
        for glyph in glyphs {
            println!(
                "  x={:8.2} w={:6.2} level={} cluster={:?}",
                glyph.x, glyph.width, glyph.level, glyph.cluster
            );
        }
    }
    Ok(())
}

fn print_help() {
    println!(
        "FARMAN Terminal v{}\n\
         A Windows terminal with first-class Persian/Arabic text support.\n\n\
         Usage:\n\
           farman                     Launch the terminal (real ConPTY shell)\n\
           farman --text-probe        Print cosmic-text/HarfRust shaping diagnostics\n\
           farman --render-test [png] Render a mixed RTL scene offscreen via wgpu\n\
           farman --version           Print version\n\
           farman --help              Show this help\n\n\
         Configuration: %APPDATA%\\FARMAN\\config.toml\n\
         Logs:          %LOCALAPPDATA%\\FARMAN\\logs",
        env!("CARGO_PKG_VERSION")
    );
}
