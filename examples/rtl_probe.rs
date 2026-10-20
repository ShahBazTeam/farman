//! Standalone probe for the text stack.
//!
//! Run with:
//!   cargo run --example rtl_probe
//!
//! It loads the bundled Persian monospace font, shapes several mixed
//! Persian/English strings with cosmic-text, and prints the resulting visual
//! glyph order. This is intentionally a diagnostic program, not the terminal.

use cosmic_text::{
    fontdb, Attrs, Buffer, Family, FeatureTag, FontFeatures, FontSystem, Metrics, Shaping,
};
use std::sync::Arc;

const FONT: &[u8] = include_bytes!("../assets/fonts/Vazir-Code-Hack.ttf");

fn main() {
    let source = fontdb::Source::Binary(Arc::new(FONT.to_vec()));
    let mut fonts = FontSystem::new_with_fonts([source]);
    let metrics = Metrics::new(18.0, 24.0);
    let mut features = FontFeatures::new();
    features.disable(FeatureTag::STANDARD_LIGATURES);
    features.disable(FeatureTag::CONTEXTUAL_LIGATURES);

    let attrs = Attrs::new()
        .family(Family::Name("Vazir Code Hack"))
        .font_features(features);

    let samples = [
        "خطا در main.rs:42",
        "مسیر فایل: C:\\Users\\User\\Desktop\\Project",
        "نسخه Python: 3.13",
        "سلام دنیا",
        "FARMAN Terminal فرمان",
    ];

    for sample in samples {
        let mut buffer = Buffer::new(&mut fonts, metrics);
        buffer.set_size(Some(1600.0), Some(200.0));
        buffer.set_text(sample, &attrs, Shaping::Advanced, Some(cosmic_text::Align::Left));
        buffer.shape_until_scroll(&mut fonts, false);

        println!("\nINPUT: {sample}");
        for run in buffer.layout_runs() {
            println!(
                "  line={} rtl={} width={:.2} glyphs={}",
                run.line_i,
                run.rtl,
                run.line_w,
                run.glyphs.len()
            );
            for glyph in run.glyphs {
                let cluster = &run.text[glyph.start..glyph.end];
                println!(
                    "    x={:7.2} w={:5.2} level={} cluster={:?} chars={:?}",
                    glyph.x,
                    glyph.w,
                    glyph.level.number(),
                    cluster,
                    cluster.chars().collect::<Vec<_>>()
                );
            }
        }
    }
}
