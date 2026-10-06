//! 探针：实测**真实代码路径**下 Rom 面板每一列的 x 起点，以及图标与文字
//! 的垂直中心差。
//!
//! 两个问题，各有一个探针：
//!
//! 1. **crc / 区域列没左对齐**（2026-10-06 截图）。根因：`icons::put` 走
//!    `Ui::put` → `allocate_new_ui` → `advance_after_rects`，**参与布局**，
//!    在 `state_with_icon` 死占状态列之后又推了一次游标，推的量随状态词
//!    宽变化（「拥有」比「未拥有」窄）→ 同一屏里每行的 crc 起点都不同。
//! 2. **图标与文字上下没对齐**。根因：图标顶对齐 16px 格子的顶部，而
//!    文字行高约 19px，中心差 1.5px。
//!
//! **两个都是"推理不算数"的类型**，所以这里让 egui 自己算：建一个带真
//! 字体的 `Context`，拿到真 `Ui`，把每列的 `rect.left()` 打出来。
//!
//! 用法：`cargo run --example col_probe`
//!
//! 不用 `egui::__run_test_ui` —— 它会 `set_fonts(FontDefinitions::empty())`
//! 省 CPU，那样量出来的不是真实字宽。

use egui::{Align, Direction, Layout, Rect, RichText, Vec2};

/// 复刻 `rompanel::cell`：死占 `w`，在占好的 rect 里画字，返回 rect。
fn cell(ui: &mut egui::Ui, w: f32, text: &str) -> Rect {
    let style = ui.style().clone();
    let mut job = egui::text::LayoutJob::default();
    RichText::new(text)
        .monospace()
        .append_to(&mut job, &style, egui::FontSelection::default(), Align::Min);
    job.wrap.max_width = w;
    let galley = ui.fonts(|f| f.layout_job(job));
    let (_id, rect) = ui.allocate_space(Vec2::new(w, galley.size().y));
    ui.painter().galley(
        egui::pos2(rect.left(), rect.top()),
        galley,
        ui.visuals().text_color(),
    );
    rect
}

/// 复刻 `rompanel::state_with_icon` 的**几何部分**（不含纹理查表）。
///
/// `paint_icon` 让调用方决定"画图标"这一步做不做布局 —— 同一个函数于是能
/// 分别量 `ui.put` 与 `painter` 两种实现，看出差别。
fn state_cell(
    ui: &mut egui::Ui,
    w: f32,
    word: &str,
    paint_icon: &mut dyn FnMut(&mut egui::Ui, Rect),
) -> (Rect, f32) {
    let style = ui.style().clone();
    let mut job = egui::text::LayoutJob::default();
    RichText::new(word).append_to(&mut job, &style, egui::FontSelection::default(), Align::Min);
    job.wrap.max_width = f32::MAX;
    let galley = ui.fonts(|f| f.layout_job(job));
    let word_w = galley.size().x;
    let text_h = galley.size().y;
    let row_h = text_h.max(16.0);
    let (_id, rect) = ui.allocate_space(Vec2::new(w, row_h));
    ui.painter().galley(rect.min, galley, ui.visuals().text_color());
    // 与 rompanel::state_with_icon 同一公式：对齐**文字中线**，不是格子中线
    let icon_y = rect.min.y + text_h / 2.0 - 8.0;
    let icon_rect = Rect::from_min_size(
        egui::pos2(rect.min.x + word_w + 4.0, icon_y),
        Vec2::splat(16.0),
    );
    paint_icon(ui, icon_rect);
    // 图标中心与文字中线的垂直差（现在应为 0）
    let dy = (icon_y + 8.0 - (rect.min.y + text_h / 2.0)).abs();
    (rect, dy)
}

fn tight(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let layout = Layout {
        main_dir: Direction::LeftToRight,
        main_wrap: false,
        main_align: Align::Min,
        main_justify: false,
        cross_align: Align::Min,
        cross_justify: false,
    };
    ui.scope_builder(egui::UiBuilder::new().layout(layout), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.spacing_mut().item_spacing.y = 0.0;
        add(ui);
    });
}

const WORDS: [&str; 4] = ["拥有", "未拥有", "坏 dump", "未 dump"];

fn measure(mode: &str) {
    let ctx = egui::Context::default();
    // 真字体：量出来的宽度才与运行期一致
    let px = egui::ColorImage::new([1, 1], egui::Color32::WHITE);
    let texture = ctx.load_texture("probe", px, egui::TextureOptions::NEAREST);
    let mut rows: Vec<(String, f32, f32, f32)> = Vec::new();

    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let (name_w, state_w, crc_w, region_w) = (110.0_f32, 44.0_f32, 94.0_f32, 108.0_f32);
            for word in WORDS {
                let use_put = mode == "put";
                let mut painter_icon = |ui: &mut egui::Ui, r: Rect| {
                    if use_put {
                        // 旧实现：`ui.put`（参与布局）
                        ui.put(r, egui::Image::new(&texture).fit_to_exact_size(r.size()));
                    } else {
                        // 新实现：纯 painter（不碰游标）
                        ui.painter().rect_filled(r, 0.0, egui::Color32::GREEN);
                    }
                };
                let (mut crc_x, mut region_x, mut dy) = (0.0_f32, 0.0_f32, 0.0_f32);
                tight(ui, |ui| {
                    let _ = cell(ui, name_w, "kn21-1.bin");
                    ui.add_space(12.0);
                    let (_s, d) = state_cell(ui, state_w, word, &mut painter_icon);
                    dy = d;
                    ui.add_space(12.0);
                    crc_x = cell(ui, crc_w, "crc(0dff3b12)").left();
                    ui.add_space(12.0);
                    region_x = cell(ui, region_w, "maincpu").left();
                });
                rows.push((word.to_string(), crc_x, region_x, dy));
            }
        });
    });

    println!("=== 画图标方式：{mode} ===");
    println!(
        "    {:12} {:>8} {:>8} {:>12}",
        "状态词", "crc", "region", "文字中心差"
    );
    for (w, c, r, dy) in &rows {
        println!("    {w:12} {c:8.1} {r:8.1} {dy:12.1}");
    }
    let crcs: Vec<f32> = rows.iter().map(|r| r.1).collect();
    let same = crcs.iter().all(|v| (v - crcs[0]).abs() < 0.01);
    println!(
        "    => crc 起点：{}（{}）",
        if same { "四行一致 ✓" } else { "四行各不相同 ✗" },
        crcs.iter()
            .map(|v| format!("{v:.0}"))
            .collect::<Vec<_>>()
            .join("/")
    );
    let dys: Vec<f32> = rows.iter().map(|r| r.3).collect();
    println!(
        "    => 文字中心差：{:.2}px（应0）",
        dys.iter().fold(0.0_f32, |a, b| a.max(*b))
    );
}

fn main() {
    measure("put");
    println!();
    measure("painter");
}
