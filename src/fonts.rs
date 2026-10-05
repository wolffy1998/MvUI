//! Font loading: system CJK fonts with graceful fallback (design doc §8).

use mvui::dlog;
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
            Ok(_) => dlog!("字体: {} 不是可用的 sfnt 字体，跳过", p),
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
        None => dlog!(
            "字体: 没找到可用的中文字体，界面会出现方框（tofu）——\
             试过的路径: {}",
            CJK_CANDIDATES.join(", ")
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
        None => dlog!(
            "字体: 没找到可用的等宽字体——试过的路径: {}",
            MONO_CANDIDATES.join(", ")
        ),
    }
    ctx.set_fonts(fonts);
}
