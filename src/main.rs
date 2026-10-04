#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 二进制直接复用库里的领域层，而不是自己再声明一份副本，这样
// `examples/` 和主程序永远不会各走各的。
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

    // 把领域层的日志落点接到 `perf_log`。
    //
    // core 不许反向依赖 UI，所以它不能自己调 `app::perf_log`；反过来，
    // 不注册这个 sink，core 里那些 `dlog!` 就全是空操作，boot.log 里
    // 只会有 UI 侧的记录。必须在启动**早期**注册，连解析和审计之前
    // 的过程才记得到。
    let _ = core::log::set_sink(app::perf_log);

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
            // egui 0.29 默认**不带**任何图片加载器（`Loaders::default`
            // 把 `image` 留空——只填了 bytes 和 texture 两级）。于是
            // 任何把编码后的字节交给 egui 的东西——`egui::include_image!`
            // 产出的正是这种——都解不出来：`Context::try_load_image`
            // 返回 `NoImageLoaders`，控件就静静地什么都不画。这就是
            // 启动标志、About 标志和目录对话框标题栏标志全都看不见，
            // 而**窗口**图标却正常的原因——上面的 `app_icon` 是手工
            // 用 `image` crate 解码的，从不走这条链。
            //
            // 这个调用是幂等的，而且只增加 `file`/`bytes`/`http` 的
            // 处理能力，所以启动时调一次就能覆盖之后所有的 `Image`。
            egui_extras::install_image_loaders(&cc.egui_ctx);
            style(&cc.egui_ctx);
            Ok(Box::new(MameApp::new(cc, tx, rx)))
        }),
    )
}

/// 全局观感：浮动式叠加滚动条（细、圆角、空闲时淡出）。
///
/// 因为浮动滚动条不占布局空间，滚动条出现时就不会再挤动内容宽度——
/// 游戏列表闪烁的来源又少了一个。
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

/// 窗口图标，来自内嵌的 app.ico（和 exe 资源用的是同一个文件）。
fn app_icon() -> egui::IconData {
    const RAW: &[u8] = include_bytes!("../assets/icons/app.ico");
    let img = image::load_from_memory(RAW).expect("embedded app.ico must decode");
    let rgba = img.to_rgba8();
    egui::IconData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    }
}

/// CFG_PREFIX 助手，暴露给主窗口启动流程用（origin: main()）。
pub fn cfg_prefix() -> std::path::PathBuf {
    GuiSettings::cfg_prefix()
}
