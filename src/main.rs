#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// The binary reuses the library's domain layer rather than declaring its own
// copy, so `examples/` and the binary can never drift apart.
use mvui::core;

mod app;
mod background;
mod events;
mod fonts;
mod i18n;
mod icons;
mod views;
mod windows;
mod ui;

use app::MameApp;
use core::settings::GuiSettings;

fn main() -> Result<(), eframe::Error> {
    let (tx, rx) = std::sync::mpsc::channel();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([900.0, 560.0])
            .with_title("MvUI").with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "MvUI",
        options,
        Box::new(move |cc| {
            fonts::install(&cc.egui_ctx);
            style(&cc.egui_ctx);
            Ok(Box::new(MameApp::new(cc, tx, rx)))
        }),
    )
}

/// Global look: floating overlay scroll bars (thin, rounded, fading out when
/// idle). Because floating bars don't allocate layout space, a scrollbar
/// appearing can no longer shift the content width — one less source of the
/// game-list flicker.
fn style(ctx: &egui::Context) {
    ctx.style_mut(|style| {
        let sc = &mut style.spacing.scroll;
        sc.floating = true;
        sc.foreground_color = true;
        sc.bar_width = 12.0;
        sc.floating_width = 5.0;
        sc.bar_inner_margin = 3.0;
        sc.handle_min_length = 24.0;
        sc.dormant_handle_opacity = 0.35;
        sc.active_handle_opacity = 0.55;
        sc.interact_handle_opacity = 0.95;
        sc.dormant_background_opacity = 0.0;
        sc.active_background_opacity = 0.08;
        sc.interact_background_opacity = 0.25;
        // rounded scroll-handle (the painter uses the widget visuals' rounding)
        for w in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            w.rounding = egui::Rounding::same(5.0);
        }
    });
}

/// window icon from the embedded app.ico (same file the exe resource uses)
fn app_icon() -> egui::IconData {
    const RAW: &[u8] = include_bytes!("../assets/images/app.ico");
    let img = image::load_from_memory(RAW).expect("embedded app.ico must decode");
    let rgba = img.to_rgba8();
    egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

/// CFG_PREFIX helper exposed for main-window startup (origin main())
pub fn cfg_prefix() -> std::path::PathBuf {
    GuiSettings::cfg_prefix()
}
