//! Font loading: system CJK fonts with graceful fallback (design doc §8).

use egui::{FontData, FontDefinitions, FontFamily};

const CJK_CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\simhei.ttf",
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\MSJH.TTC",
    "C:\\Windows\\Fonts\\meiryo.ttc",
];

const MONO_CANDIDATES: &[&str] = &[
    "C:\\Windows\\Fonts\\consola.ttf",
    "C:\\Windows\\Fonts\\lucon.ttf",
];

const CJK_FONT_KEY: &str = "cjk-main";
const MONO_FONT_KEY: &str = "mono-main";

/// Cheap sanity check for an sfnt container (`ttf`/`ttc`/`otf`).
///
/// A truncated file — or a non-font dropped at one of the candidate paths — used
/// to abort the whole chain: the read succeeded, egui then failed to parse it,
/// and every glyph rendered as tofu with nothing written to the log. Checking
/// the signature lets `install` fall through to the next candidate instead.
fn looks_like_font(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0x00, 0x01, 0x00, 0x00]) | Some(b"true") | Some(b"ttcf") | Some(b"OTTO")
    )
}

fn first_usable(paths: &[&str]) -> Option<Vec<u8>> {
    for p in paths {
        match std::fs::read(p) {
            Ok(bytes) if looks_like_font(&bytes) => return Some(bytes),
            Ok(_) => crate::app::perf_log(&format!("fonts: {p} is not a usable font")),
            Err(_) => {}
        }
    }
    None
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    match first_usable(CJK_CANDIDATES) {
        Some(bytes) => {
            fonts
                .font_data
                .insert(CJK_FONT_KEY.to_owned(), FontData::from_owned(bytes));
            if let Some(fam) = fonts.families.get_mut(&FontFamily::Proportional) {
                fam.insert(0, CJK_FONT_KEY.to_owned());
            }
            if let Some(fam) = fonts.families.get_mut(&FontFamily::Monospace) {
                fam.push(CJK_FONT_KEY.to_owned());
            }
        }
        // nothing usable: log it, because the default UI language is Chinese and
        // the symptom (every glyph a box) is otherwise impossible to diagnose
        None => crate::app::perf_log(
            "fonts: no usable CJK font found — missing glyphs expected (tofu)",
        ),
    }
    match first_usable(MONO_CANDIDATES) {
        Some(bytes) => {
            fonts
                .font_data
                .insert(MONO_FONT_KEY.to_owned(), FontData::from_owned(bytes));
            if let Some(fam) = fonts.families.get_mut(&FontFamily::Monospace) {
                fam.insert(0, MONO_FONT_KEY.to_owned());
            }
        }
        None => crate::app::perf_log("fonts: no usable monospace font found"),
    }
    ctx.set_fonts(fonts);
}
