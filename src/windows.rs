//! Dialog windows (origin: optionsUI/csvCfgUI/dirsUI/playOptionsUI/cmdUI/aboutUI).

use crate::app::{
    MameApp, PlayKind, UiFontFamily, UiFontPrefs, F_CLONES, F_MECHANICAL, F_NONWORKING,
    F_UNAVAILABLE, COL_LAST, COLUMN_TITLES,
};
use crate::core::options::{
    OptKind, OptionCore, OPTLEVEL_BIOS, OPTLEVEL_CLONEOF, OPTLEVEL_CURR, OPTLEVEL_GLOBAL,
    OPTLEVEL_HORIZONT, OPTLEVEL_LAST, OPTLEVEL_SRC, OPTLEVEL_VERTICAL, LEVEL_NAMES,
};

/// A window frame that does not let the wallpaper through.
///
/// `apply_theme_with_bg` makes `window_fill` half-transparent while a
/// background image is set, which is right for the dock panels — the picture is
/// supposed to show through them. It is wrong for a dialog: a settings window
/// you cannot read the labels in is worse than one that hides the wallpaper, so
/// every `egui::Window` gets an explicitly opaque fill.
pub fn opaque_frame(ctx: &egui::Context) -> egui::Frame {
    let mut f = egui::Frame::window(&ctx.style());
    f.fill = f.fill.gamma_multiply(0.0);
    f.fill = if ctx.style().visuals.dark_mode {
        egui::Color32::from_rgb(17, 24, 39)
    } else {
        egui::Color32::from_rgb(255, 255, 255)
    };
    f.stroke.color = egui::Color32::from_rgb(203, 213, 225);
    f.rounding = egui::Rounding::same(12.0);
    f
}

/// The MvUI mark, used on the start-up panel, in About, and in the title bar of
/// the directories dialog.
///
/// egui's `include_image!` expands to a fresh `include_bytes!` **at every call
/// site**, so the three uses below would have embedded the same PNG three times
/// and uploaded three identical textures. One accessor, one copy.
pub fn app_logo() -> egui::ImageSource<'static> {
    egui::include_image!("../assets/images/logo.png")
}

/// Draw the app logo at `size` px square, centred in `ui`.
///
/// Goes through [`egui::ImageSource::load`] instead of `egui::Image::new`, for
/// the same reason the dirs-dialog title bar does (see its comment): the
/// `Image` widget renders egui's ⚠ fallback glyph when its source does not
/// resolve, and `include_image!` hands over a *bytes* source that only decodes
/// if the byte loader is registered and ready at that moment. On the start-up
/// panel — which paints on the very first frames, before anything else has
/// warmed the loader — it did not resolve, and the user saw a small red ⚠
/// where the logo belongs, with no way to tell it was a load failure.
///
/// `TexturePoll::Ready` is the only case that draws; `Pending` (the loader has
/// not finished) just skips this frame and asks for another. Returning whether
/// anything was drawn lets the caller add the spacing rather than leave a gap.
pub fn draw_app_logo(ui: &mut egui::Ui, size: f32) -> bool {
    let Ok(egui::load::TexturePoll::Ready { texture }) = app_logo().load(
        ui.ctx(),
        egui::TextureOptions::LINEAR,
        egui::SizeHint::Scale(egui::emath::OrderedFloat(size)),
    ) else {
        // not decoded yet — keep animating so the next frame retries
        ui.ctx().request_repaint();
        return false;
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter()
        .image(texture.id, rect, crate::icons::full_uv(), egui::Color32::WHITE);
    true
}

/// Height of the band `egui::Window` reserves for its title bar, which is where
/// [`paint_title_logo`] puts the mark. Computed the same way `Window` does it.
pub fn title_bar_height(ctx: &egui::Context) -> f32 {
    let style = ctx.style();
    ctx.fonts(|f| f.row_height(&style.text_styles[&egui::TextStyle::Heading]))
        + style.spacing.window_margin.top
        + style.spacing.window_margin.bottom
}

/// Paint the MvUI mark into the window's own title bar, left of the title text.
///
/// egui 0.29's `Window` has no `title_bar(|ui| …)` hook (that arrived in 0.30),
/// and the mark cannot be added as an ordinary widget: the title bar is drawn by
/// `Window` itself *above* the closure's rectangle, while `Ui::painter` is
/// clipped to that rectangle, so a shape drawn up there is discarded without
/// warning. Cloning the painter and replacing the clip rect with the title bar's
/// own bounds lifts that restriction.
///
/// Painting from inside the `show()` closure is deliberate and is what makes it
/// visible: `Window` reserves the title-bar background as a placeholder *before*
/// running the closure (`window.rs:523`) and fills it in afterwards
/// (`window.rs:592`). Because `Painter::set` replaces a shape in place, that
/// background keeps its early index and every shape added from here lands on top.
///
/// `bar_h` should come from [`title_bar_height`] with the same `ctx`.
pub fn paint_title_logo(ui: &egui::Ui, ctx: &egui::Context, bar_h: f32) {
    let style = ctx.style();
    let content = ui.max_rect();
    let bar = egui::Rect::from_min_max(
        egui::pos2(ui.clip_rect().min.x, content.min.y - bar_h),
        egui::pos2(ui.clip_rect().max.x, content.min.y),
    );
    let size = (bar_h - 8.0).clamp(12.0, 20.0);
    let mark = egui::Rect::from_center_size(
        egui::pos2(
            bar.min.x + style.spacing.window_margin.left + size * 0.5,
            bar.center().y,
        ),
        egui::vec2(size, size),
    );
    // `Image::paint_at` needs a `Ui`, and the `Ui` we have is clipped away up
    // here, so go straight to the painter and address the texture by hand.
    // `SizedTexture` carries the id and the source size but no UV rect, so the
    // whole texture is addressed directly.
    let texture = match app_logo().load(
        ctx,
        egui::TextureOptions::LINEAR,
        egui::SizeHint::Scale(egui::emath::OrderedFloat(size)),
    ) {
        Ok(egui::load::TexturePoll::Ready { texture }) => texture,
        _ => {
            // Not decoded yet. **Ask for another frame**: `load` kicks the
            // decode off and answers `Pending`, so this is the normal state on
            // the first frame that touches the logo. Without the repaint
            // request nothing else would ever redraw this window, the texture
            // would finish decoding into an atlas nobody repaints, and the mark
            // would stay missing until the user happened to click something.
            // (`draw_app_logo` does the same, for the same reason.)
            ctx.request_repaint();
            return;
        }
    };
    let mut p = ui.painter().clone();
    p.set_clip_rect(bar);
    p.image(texture.id, mark, crate::icons::full_uv(), egui::Color32::WHITE);
}

pub fn draw_windows(app: &mut MameApp, ctx: &egui::Context) {
    draw_options(app, ctx);
    draw_dirs(app, ctx);
    draw_play(app, ctx);
    draw_cmd(app, ctx);
    draw_about(app, ctx);
    draw_verify(app, ctx);
    // `draw_rom_verify` 已随单游戏校验一起删除（2026-06）。
    draw_filter(app, ctx);
    draw_advanced_search(app, ctx);
    draw_font_windows(app, ctx);
}

fn draw_font_windows(app: &mut MameApp, ctx: &egui::Context) {
    let lang = app.lang.clone();
    let mut changed = false;
    changed |= draw_font_window(
        ctx,
        &lang,
        "Game List Font",
        &mut app.show_list_font_win,
        &mut app.list_font,
        UiFontPrefs { family: UiFontFamily::Proportional, size: 14.0, bold: false },
    );
    changed |= draw_font_window(
        ctx,
        &lang,
        "Info Panel Font",
        &mut app.show_info_font_win,
        &mut app.info_font,
        UiFontPrefs { family: UiFontFamily::Monospace, size: 14.0, bold: false },
    );
    changed |= draw_font_window(
        ctx,
        &lang,
        "Category Font",
        &mut app.show_folder_font_win,
        &mut app.folder_font,
        UiFontPrefs { family: UiFontFamily::Proportional, size: 14.0, bold: false },
    );
    if changed {
        app.save_settings();
        ctx.request_repaint();
    }
}

fn draw_font_window(
    ctx: &egui::Context,
    lang: &str,
    title_key: &str,
    show: &mut bool,
    prefs: &mut UiFontPrefs,
    default: UiFontPrefs,
) -> bool {
    if !*show {
        return false;
    }
    let mut changed = false;
    egui::Window::new(crate::i18n::tr(lang, title_key))
        .open(show)
        .collapsible(false)
        .resizable(false)
        .default_width(280.0)
        // centered on the window: settings dialogs are modal in spirit, and a
        // fixed anchor also keeps them from being dragged off-screen
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.label(crate::i18n::tr(lang, "Font Family"));
                ui.horizontal(|ui| {
                    changed |= ui
                        .radio_value(
                            &mut prefs.family,
                            UiFontFamily::Proportional,
                            crate::i18n::tr(lang, "Proportional"),
                        )
                        .changed();
                    changed |= ui
                        .radio_value(
                            &mut prefs.family,
                            UiFontFamily::Monospace,
                            crate::i18n::tr(lang, "Monospace"),
                        )
                        .changed();
                });
                ui.separator();
                changed |= ui
                    .add(egui::Slider::new(&mut prefs.size, 9.0..=28.0).text(crate::i18n::tr(lang, "Size")))
                    .changed();
                changed |= ui.checkbox(&mut prefs.bold, crate::i18n::tr(lang, "Bold")).changed();
                ui.separator();
                ui.label(prefs.rich_text(crate::i18n::tr(lang, "Preview Text")));
                if ui.button(crate::i18n::tr(lang, "Reset")).clicked() {
                    *prefs = default;
                    changed = true;
                }
            });
        });
    changed
}

/// Filter popup, opened from the toolbar button left of the search box. These
/// four flags used to sit in View ▸ Custom Filters; the menu entry is gone, the
/// filtering itself is unchanged.
/// 高级搜索弹窗：勾选**搜索要作用在哪几列**。
///
/// 用户 2026-06 要求：「高级搜索就是可以筛选按游戏列表某列搜索，默认是所有
/// 列都勾选可以搜索。」所以：
///
/// - 默认 `search_cols = u8::MAX`（7 列全勾），也就是旧行为 —— 搜索框一直是
///   `name + description` 的全文搜。
/// - 一列都不勾时**不过滤**（等于"搜索框里什么都没有"，比"什么都搜不到"
///   合理，否则用户会以为搜索坏了）。这一点由
///   [`MameApp::row_matches_query`] 里的"没有任何列命中就 false"配合 ——
///   全不勾时它对每一行都返回 false，所以要在这里显式拦一下。
/// - 勾选变化立刻 `needs_refilter`：否则用户改完勾选要等到下一次敲键盘
///   才看到结果。
fn draw_advanced_search(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_advsearch_win;
    egui::Window::new(app.tr("Advanced search"))
        .open(&mut show)
        .resizable(false)
        .collapsible(false)
        .default_width(260.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            ui.weak(app.tr("Search only in the ticked columns"));
            ui.separator();
            for i in 0..COL_LAST {
                let bit = 1u8 << i;
                let mut on = app.search_cols & bit != 0;
                if ui
                    .checkbox(&mut on, app.tr(COLUMN_TITLES[i]))
                    .changed()
                {
                    if on {
                        app.search_cols |= bit;
                    } else {
                        app.search_cols &= !bit;
                    }
                    app.needs_refilter = true;
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(app.tr("Select all")).clicked() {
                    app.search_cols = crate::views::all_search_cols();
                    app.needs_refilter = true;
                }
                if ui.button(app.tr("Clear")).clicked() {
                    app.search_cols = 0;
                    app.needs_refilter = true;
                }
            });
        });
    app.show_advsearch_win = show;
}

fn draw_filter(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_filter_win;
    egui::Window::new(app.tr("Filter"))
        .open(&mut show)
        .resizable(false)
        .collapsible(false)
        .default_width(240.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            for (flag, key) in [
                (F_CLONES, "Hide Clones"),
                (F_NONWORKING, "Hide Non-Working"),
                (F_UNAVAILABLE, "Hide Unavailable"),
                (F_MECHANICAL, "Hide Mechanical"),
            ] {
                let mut on = app.filter_flags & flag != 0;
                if ui.checkbox(&mut on, app.tr(key)).changed() {
                    if on {
                        app.filter_flags |= flag;
                    } else {
                        app.filter_flags &= !flag;
                    }
                    app.needs_refilter = true;
                }
            }
        });
    app.show_filter_win = show;
}

// ---------------------------------------------------------------------
// options (origin: OptionsUI tabs + category lists + OptionDelegate).
// Levels: Global/Horizont/Vertical/Source/Bios/Game. The old GUI page
// moved to Settings ▸ Directories; the Cloneof page is gone, but clone
// inis still take part in chain inheritance — they are just no longer
// editable here.
// ---------------------------------------------------------------------

fn draw_options(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_options_win.is_some();
    let level = app.show_options_win.unwrap_or(OPTLEVEL_GLOBAL);
    egui::Window::new(app.tr("Options"))
        .open(&mut show)
        .resizable(true)
        .default_width(860.0)
        .default_height(560.0)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            app.ensure_chain();
            // level tabs — Global / Horizont / Vertical / Source / Bios / Game.
            // Both orientation pages are always present: they are two distinct
            // files in the ini chain (see OPTLEVEL_HORIZONT/VERTICAL), and the
            // one that is off the current game's chain still shows its own
            // stored values (snapshot reads the level field, not currvalue).
            ui.horizontal(|ui| {
                for lvl in [
                    OPTLEVEL_GLOBAL,
                    OPTLEVEL_HORIZONT,
                    OPTLEVEL_VERTICAL,
                    OPTLEVEL_SRC,
                    OPTLEVEL_BIOS,
                    OPTLEVEL_CURR,
                ] {
                    let name = LEVEL_NAMES[lvl.min(OPTLEVEL_LAST - 1)];
                    if ui
                        .selectable_label(app.opt_level == lvl, app.tr(name))
                        .clicked()
                    {
                        app.opt_level = lvl;
                        app.opt_edits.clear();
                    }
                }
            });
            ui.separator();
            let Some(opts) = app.opts.clone() else {
                ui.weak(format!("({})", app.tr("option template not loaded")));
                return;
            };
            // one snapshot per frame carries display/parent/changed, so the
            // render loop never touches the options mutex per row
            let snap = build_snapshot(&opts, app);
            // wider, less fiddly scroll bars (the thin default ones were hard
            // to grab on a 4K display)
            ui.spacing_mut().scroll.bar_width = 10.0;
            ui.spacing_mut().scroll.bar_outer_margin = 4.0;
            // fixed-width category sidebar + rows taking the rest — the old
            // equal `ui.columns(2)` split starved the rows area and left the
            // category list half the dialog wide
            egui::SidePanel::left("opt_categories")
                .resizable(false)
                .default_width(160.0)
                .frame(egui::Frame::none())
                .show_inside(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for c in CORE_CATEGORIES_VEC {
                            // `opt_category` keeps the English key; only the display
                            // is translated, so the snapshot filter still matches
                            if ui.selectable_label(app.opt_category == c, app.tr(c)).clicked() {
                                app.opt_category = c.to_string();
                            }
                        }
                    });
                });
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("opt_rows")
                        .num_columns(3)
                        .striped(true)
                        .min_col_width(110.0)
                        .show(ui, |ui| {
                            for item in &snap {
                                match item {
                                    SnapRow::Title(t) => {
                                        ui.end_row();
                                        ui.colored_label(
                                            egui::Color32::from_rgb(0, 60, 160),
                                            egui::RichText::new(app.tr(t).to_uppercase()).strong(),
                                        );
                                        ui.weak("");
                                        ui.weak("");
                                        ui.end_row();
                                    }
                                    SnapRow::Opt(d) => {
                                        // origin: OptionUtils::addModelItem — en_US shows
                                        // the template guiname capitalised, every other
                                        // language translates `lower(getLongName(name))`
                                        // (which is why the catalogue keys are lowercase)
                                        let shown = if app.lang == "en_US" {
                                            crate::core::options::capitalize_str(&d.gui_name)
                                        } else {
                                            crate::core::options::capitalize_str(
                                                &app.tr(&d.gui_name.to_lowercase()),
                                            )
                                        };
                                        let mut label = egui::RichText::new(shown);
                                        if d.changed {
                                            label = label.strong();
                                        }
                                        ui.label(label);
                                        edit_control(app, ui, d, &opts);
                                        // current value + reset; the minimum width
                                        // keeps the column from drifting row by row
                                        ui.horizontal(|ui| {
                                            ui.set_min_width(120.0);
                                            ui.weak(
                                                egui::RichText::new(app.tr(&d.display)).small(),
                                            );
                                            if d.changed && ui.small_button("↺").clicked() {
                                                app.opt_edits
                                                    .insert(d.name.clone(), d.parent_display.clone());
                                                apply_edit(app, &opts, &d.name);
                                            }
                                        });
                                        ui.end_row();
                                    }
                                }
                            }
                        });
                });
            ui.separator();
            ui.horizontal(|ui| {
                ui.weak(app.tf(
                    "editing level: {} — edits are written to the matching ini on change",
                    app.tr(LEVEL_NAMES[app.opt_level.min(OPTLEVEL_LAST - 1)]),
                ));
            });
        });
    // sync open flag
    app.show_options_win = if show { Some(level) } else { None };
}

#[derive(Clone, Debug)]
struct SnapOpt {
    name: String,
    gui_name: String,
    kind: u8, // 0 bool 1 int 2 float 3 str-combo 4 str-edit 5 file 6 dir 7 dirs 8 csv 9 plain
    /// display form of the **level field** (globalvalue/horzvalue/…), not
    /// currvalue — the off-chain orientation page must show its own stored
    /// value, which currvalue (the on-chain effective value) would hide
    display: String,
    /// display form of the level this one overrides; the ↺ button writes this
    /// back, and `changed` is just `display != parent_display`
    parent_display: String,
    changed: bool,
    choices: Vec<(String, String)>,
    min: f64,
    max: f64,
}

#[derive(Clone, Debug)]
enum SnapRow {
    Title(String),
    Opt(SnapOpt),
}

const CORE_CATEGORIES_VEC: [&str; 7] = [
    "Core Video", "OSD Video", "Screen", "Audio", "Control", "Vector", "Misc",
];

fn kind_u8(k: OptKind) -> u8 {
    match k {
        OptKind::Bool => 0,
        OptKind::Int => 1,
        OptKind::Float => 2,
        OptKind::Str => 3,
        OptKind::StrEditable => 4,
        OptKind::File | OptKind::DatFile | OptKind::CfgFile | OptKind::ExeFile => 5,
        OptKind::Dir => 6,
        OptKind::Dirs => 7,
        OptKind::Csv => 8,
        OptKind::Unknown => 9,
    }
}

fn build_snapshot(opts: &std::sync::Mutex<OptionCore>, app: &MameApp) -> Vec<SnapRow> {
    let guard = match opts.try_lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let level = app.opt_level;
    let mut rows = Vec::new();
    for (cat_key, names) in &guard.opt_cat_map {
        let seg = cat_key.split('_').nth(1).unwrap_or("");
        // sidebar match (origin: optSubCat == tr(key.split('_')[1]))
        if seg != app.opt_category {
            continue;
        }
        rows.push(SnapRow::Title(cat_key.split('_').last().unwrap_or("").to_string()));
        for name in names {
            let Some(o) = guard.opts.get(name) else { continue };
            // visibility per level (origin updateModel filter); the orientation
            // pages mirror the source page — options hidden at driver level are
            // hidden there too (user rule)
            let visible = match level {
                OPTLEVEL_GLOBAL => o.globalvisible,
                OPTLEVEL_HORIZONT | OPTLEVEL_VERTICAL => o.srcvisible,
                OPTLEVEL_SRC => o.srcvisible,
                OPTLEVEL_BIOS => o.biosvisible,
                _ => o.gamevisible,
            };
            if !visible {
                continue;
            }
            // level field + the level it overrides, in one place — former
            // `is_changed`/`parent_value` re-locked the mutex per row per frame
            let (lvl_val, par_val) = match level {
                OPTLEVEL_GLOBAL => (&o.globalvalue, &o.defvalue),
                OPTLEVEL_HORIZONT => (&o.horzvalue, &o.globalvalue),
                OPTLEVEL_VERTICAL => (&o.vertvalue, &o.globalvalue),
                OPTLEVEL_SRC => (&o.srcvalue, &o.globalvalue),
                OPTLEVEL_BIOS => (&o.biosvalue, &o.srcvalue),
                OPTLEVEL_CLONEOF => (&o.cloneofvalue, &o.biosvalue),
                _ => (&o.currvalue, &o.cloneofvalue),
            };
            let display = guard.get_long_value(name, lvl_val);
            let parent_display = guard.get_long_value(name, par_val);
            let choices: Vec<(String, String)> = o
                .values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    (
                        v.clone(),
                        o.guivalues.get(i).cloned().unwrap_or_else(|| v.clone()),
                    )
                })
                .collect();
            rows.push(SnapRow::Opt(SnapOpt {
                name: name.clone(),
                gui_name: if o.guiname.is_empty() {
                    crate::core::options::capitalize_str(name)
                } else {
                    o.guiname.clone()
                },
                kind: kind_u8(o.kind.unwrap_or(OptKind::Unknown)),
                changed: display != parent_display,
                display,
                parent_display,
                choices,
                min: o.min.parse().unwrap_or(0.0),
                max: o.max.parse().unwrap_or(100.0),
            }));
        }
    }
    rows
}

fn edit_control(app: &mut MameApp, ui: &mut egui::Ui, d: &SnapOpt, opts: &std::sync::Mutex<OptionCore>) {
    // fixed-width control column so the value column starts at one x for
    // every row (the grid otherwise sizes each cell to its own content)
    ui.set_min_width(200.0);
    let mut val = app.opt_edits.get(&d.name).cloned().unwrap_or_else(|| d.display.clone());
    match d.kind {
        0 => {
            let mut on = val == "true";
            if ui.checkbox(&mut on, "").changed() {
                app.opt_edits.insert(d.name.clone(), if on { "true".into() } else { "false".into() });
                apply_edit(app, opts, &d.name);
            }
        }
        1 | 2 => {
            let is_float = d.kind == 2;
            let mut v = val.parse::<f64>().unwrap_or(d.min);
            let resp = ui.add(
                egui::Slider::new(&mut v, d.min..=d.max)
                    .show_value(true)
                    .text(""),
            );
            if resp.changed() {
                let s = if is_float { format!("{v:.2}") } else { format!("{}", v as i64) };
                app.opt_edits.insert(d.name.clone(), s);
            }
            // save once per drag, not once per frame: apply_edit rewrites the
            // whole ini file, and a drag fires `changed` on every repaint
            if (resp.drag_stopped() || resp.lost_focus()) && app.opt_edits.contains_key(&d.name) {
                apply_edit(app, opts, &d.name);
            }
        }
        3 if !d.choices.is_empty() => {
            // Every value is shown, not hidden behind a drop-down.
            //
            // The template's widest enumeration is `bgfx_backend` with 7
            // entries on the win port (`snapview`, `video` have 6, most have
            // 5 or fewer), so laying them out inline costs a few rows and
            // saves a click on every single edit. A combo also had a second
            // problem: it covered the rows below it and inherited the
            // translucent window fill, so the open list was hard to read
            // over a wallpaper.
            //
            // The canonical value stays untranslated; the shown guivalue is what
            // the 1.8.2 catalogue translates (e.g. Auto / None).
            let picked = d
                .choices
                .iter()
                .find(|(_, gui)| *gui == val)
                .map(|(canon, _)| canon.clone())
                .unwrap_or_else(|| val.clone());
            let mut next: Option<String> = None;
            if d.choices.len() > 6 {
                // wide enumerations wrap into columns so one option does not
                // push the rest of the table off the dialog
                let per_col = (d.choices.len() + 1) / 2;
                ui.horizontal(|ui| {
                    for chunk in d.choices.chunks(per_col) {
                        ui.vertical(|ui| {
                            for (_canon, gui) in chunk {
                                if ui
                                    .selectable_label(picked == *gui, app.tr(gui))
                                    .clicked()
                                {
                                    next = Some(gui.clone());
                                }
                            }
                        });
                    }
                });
            } else {
                for (_canon, gui) in &d.choices {
                    if ui.selectable_label(picked == *gui, app.tr(gui)).clicked() {
                        next = Some(gui.clone());
                    }
                }
            }
            if let Some(gui) = next {
                app.opt_edits.insert(d.name.clone(), gui);
                apply_edit(app, opts, &d.name);
            }
        }
        // 3 with empty choices falls through here: showconfig-only strings
        // (snapname, joystick_map, …) have no template value list.
        3 | 4 | 9 => {
            if ui
                .add_sized([240.0, 18.0], egui::TextEdit::singleline(&mut val))
                .lost_focus()
            {
                app.opt_edits.insert(d.name.clone(), val);
                apply_edit(app, opts, &d.name);
            }
        }
        5 => {
            ui.horizontal(|ui| {
                if ui
                    .add_sized([200.0, 18.0], egui::TextEdit::singleline(&mut val))
                    .lost_focus()
                {
                    app.opt_edits.insert(d.name.clone(), val.clone());
                    apply_edit(app, opts, &d.name);
                }
                if ui.small_button("...").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                        app.opt_edits.insert(d.name.clone(), p.to_string_lossy().to_string());
                        apply_edit(app, opts, &d.name);
                    }
                }
            });
        }
        6 => {
            ui.horizontal(|ui| {
                if ui
                    .add_sized([200.0, 18.0], egui::TextEdit::singleline(&mut val))
                    .lost_focus()
                {
                    app.opt_edits.insert(d.name.clone(), val.clone());
                    apply_edit(app, opts, &d.name);
                }
                if ui.small_button("...").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        app.opt_edits.insert(d.name.clone(), p.to_string_lossy().to_string());
                        apply_edit(app, opts, &d.name);
                    }
                }
            });
        }
        7 => {
            ui.horizontal(|ui| {
                if ui
                    .add_sized([200.0, 18.0], egui::TextEdit::singleline(&mut val))
                    .lost_focus()
                {
                    app.opt_edits.insert(d.name.clone(), val.clone());
                    apply_edit(app, opts, &d.name);
                }
                if ui.small_button(app.tr("Edit")).clicked() {
                    app.dirs_buf = val.clone();
                    app.show_dirs_win = true;
                    app.dirs_target_option = Some(d.name.clone());
                }
            });
        }
        _ => {
            // csv → checkbox dialog on demand
            ui.horizontal(|ui| {
                ui.add_sized([180.0, 18.0], egui::TextEdit::singleline(&mut val));
                if ui.small_button("...").clicked() {
                    app.show_csv_win = Some(d.name.clone());
                }
            });
        }
    }
}

/// origin: setModelData cascade — write the level value and propagate down
/// until the first locally-divergent level, then saveIniFile(level).
fn apply_edit(app: &mut MameApp, opts: &std::sync::Mutex<OptionCore>, name: &str) {
    let Some(lib) = app.lib.clone() else { return };
    let Some(new_disp) = app.opt_edits.get(name).cloned() else { return };
    let level = app.opt_level;
    let meta = app.current_meta().unwrap_or_default();
    let mut guard = opts.lock().unwrap();
    let short = guard.get_short_value(name, &new_disp);
    let Some(o) = guard.opts.get_mut(name) else { return };
    // write level field
    match level {
        OPTLEVEL_GLOBAL => o.globalvalue = short.clone(),
        OPTLEVEL_HORIZONT => o.horzvalue = short.clone(),
        OPTLEVEL_VERTICAL => o.vertvalue = short.clone(),
        OPTLEVEL_SRC => o.srcvalue = short.clone(),
        OPTLEVEL_BIOS => o.biosvalue = short.clone(),
        _ => {}
    }
    // currvalue is the *effective* value the game reads; only the level the
    // game's native orientation actually puts in its ini chain may touch it
    // (origin: parse_standard_inis loads horizont OR vertical, never both).
    // Editing the off-chain page must stay a pure file edit.
    let on_chain = match level {
        OPTLEVEL_HORIZONT => meta.is_horz,
        OPTLEVEL_VERTICAL => !meta.is_horz,
        _ => true,
    };
    if on_chain {
        o.currvalue = short;
    }
    // save ini for the level.
    //
    // The path comes from `OptionCore::ini_file_for`, i.e. the very function
    // `chainLoadOptions` reads the level back through: writing anywhere else
    // (this used to be `<mame>/source/<sourcefile with ".c" trimmed>.ini`)
    // means the level reads back empty on the next session — and for a modern
    // `pacman.cpp` the old string surgery even produced `pacmanpp.ini`
    // (origin: mame-0.168 emuopts.cpp::parse_standard_inis).
    let ini_path = {
        let libg = lib.lock().unwrap();
        guard.ini_file_for(level, &meta, &libg)
    };
    let default_ini = {
        let libg = lib.lock().unwrap();
        libg.default_ini.clone()
    };
    if let Err(e) = guard.save_ini_file(level, &ini_path, &default_ini) {
        app.log(format!("could not write {}: {e}", ini_path.display()));
    }
}

// ---------------------------------------------------------------------
// dirs dialog (origin: DirsUI — multi-path list editor)
// ---------------------------------------------------------------------

/// One editable path in Settings ▸ Directories.
///
/// Everything is owned rather than borrowed: the whole table is built before the
/// `Window` closure runs, because the closure needs `&mut app` for the edit map
/// and `app.tr` for the labels, and those two cannot be live at the same time.
struct DirRow {
    key: &'static str,
    /// already translated
    label: String,
    /// the value the field starts out holding. It used to be a greyed-out
    /// placeholder shown only while the field was empty, which made every row
    /// look blank on open and needed a "reset" button to get the value back;
    /// it is now written into the field for real. Sourced from the template's
    /// `default=` attributes (see `options::template_default`) so the XML stays
    /// the one place a default is written — the dialog only reformats
    /// `dats\history.xml` into the Windows-style `.\dats\history.xml` shown
    /// here. Empty only for MAME itself, which has no default to offer.
    default_value: String,
    is_dir: bool,
}

/// The dialog table: `(translated group heading, rows)`.
fn dir_rows(app: &MameApp) -> Vec<(String, Vec<DirRow>)> {
    use crate::core::{dat, paths};
    let tr = |s: &str| app.tr(s).to_string();
    // the template's default, shown the way the resolver reads it back: the
    // Windows-style `.\` prefix plus `\` segments. `template_default` is the
    // single source — the same value chain_load seeds and the reader resolves.
    let rel = |key: &str| {
        let d = crate::core::options::template_default(key);
        if d.is_empty() {
            d
        } else {
            format!(".\\{}", d.replace('/', "\\"))
        }
    };

    let mut out: Vec<(String, Vec<DirRow>)> = Vec::new();

    // The MAME executable. Not a content path — it is the one absolute location
    // in this dialog, and the only one MvUI genuinely cannot guess, because it
    // is chosen at first start rather than derived from anything on disk.
    out.push((
        tr("MAME"),
        vec![DirRow {
            key: "mame_binary",
            label: tr("MAME program"),
            default_value: app
                .gui
                .get("mame_binary")
                .map(|p| p.to_string())
                .unwrap_or_default(),
            is_dir: false,
        }],
    ));

    // Artwork — one row per image dock, each defaulting to `.\<its own dir>`
    let artwork: Vec<DirRow> = paths::IMAGE_DIRS
        .iter()
        .enumerate()
        .map(|(i, (key, _))| DirRow {
            key,
            label: tr(dat::DOCK_NAMES.get(i).copied().unwrap_or("Image")),
            default_value: rel(key),
            is_dir: true,
        })
        .collect();
    out.push((tr("Artwork"), artwork));

    // Documents — one row per .dat, each naming the concrete file it reads. The
    // `dats\` segment is part of the default, not decoration: `paths::dat_file`
    // resolves an unset option to `<exe>/dats/<name>`, so showing `.\command.dat`
    // here would name a path nothing ever reads.
    let documents: Vec<DirRow> = paths::DAT_FILES
        .iter()
        .map(|(key, _)| {
            let name = dat::DOCK_NAMES
                .iter()
                .find(|n| dat::dock_file_option(docks_index(n)) == Some(*key))
                .copied()
                .unwrap_or("History");
            DirRow {
                key,
                label: tr(name),
                default_value: rel(key),
                is_dir: false,
            }
        })
        .collect();
    out.push((tr("Documents"), documents));

    // Everything else MvUI supplies itself
    out.push((
        tr("Other"),
        vec![
            DirRow {
                key: "localized_list_file",
                label: tr("Localized game list"),
                default_value: rel("localized_list_file"),
                is_dir: false,
            },
            DirRow {
                key: "background_directory",
                label: tr("Background images"),
                default_value: rel("background_directory"),
                is_dir: true,
            },
            DirRow {
                key: "folder_directory",
                label: tr("Folder lists"),
                default_value: rel("folder_directory"),
                is_dir: true,
            },
        ],
    ));

    out
}

fn docks_index(name: &str) -> usize {
    crate::core::dat::DOCK_NAMES
        .iter()
        .position(|n| *n == name)
        .unwrap_or(0)
}

fn draw_dirs(app: &mut MameApp, ctx: &egui::Context) {
    let mut open = app.show_dirs_win;
    let mut apply = false;
    let mut browse: Option<(&'static str, bool)> = None;

    // snapshot: the table is static apart from translations, and the edit map
    // is the working copy the closure mutates
    let table = dir_rows(app);
    let mut edits = std::mem::take(&mut app.dir_edits);
    for row in table.iter().flat_map(|(_, r)| r.iter()) {
        edits.entry(row.key.to_string()).or_insert_with(|| {
            // the configured value if there is one, otherwise the default —
            // filled in rather than hinted, so the field is never blank
            let cur = app.gui.get(row.key).unwrap_or_default().trim().to_string();
            if cur.is_empty() {
                row.default_value.clone()
            } else {
                cur
            }
        });
    }
    let note = app
        .tr("Relative paths are resolved against the program directory.")
        .to_string();
    let exe_note = crate::core::paths::exe_dir().to_string_lossy().to_string();
    let browse_label = app.tr("Browse...").to_string();
    let ok_label = app.tr("OK").to_string();
    let title = app.tr("Directories").to_string();
    // egui 0.29's `Window` has no `title_bar(|ui| …)` hook, so the mark is
    // painted into the window's own title bar by hand — see
    // `paint_title_logo` for why the closure's own painter cannot be used.
    let bar_h = title_bar_height(ctx);

    // the response is not needed: the title-bar mark is painted from inside the
    // closure (see below), and every value it carried is read back out of
    // `edits` / `open` afterwards
    let _ = egui::Window::new(title)
        .open(&mut open)
        .resizable(true)
        .default_width(680.0)
        // no collapse triangle in the title bar: this dialog has one job and a
        // full-height list, and a minimisable frame only invites hiding it
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        // The title is translated, but the window's position must not move when
        // the language does — `Window::new` derives its `Area` id from the title
        // text, so switching language would otherwise reset the placement.
        .id(egui::Id::new("mvui_dirs"))
        .show(ctx, |ui| {
            // The MvUI mark, at the left of the title bar.
            paint_title_logo(ui, ctx, bar_h);
            ui.label(note);
            ui.label(
                egui::RichText::new(exe_note.clone()).small().weak(),
            );
            ui.add_space(4.0);
            ui.spacing_mut().scroll.bar_width = 10.0;
            ui.spacing_mut().scroll.bar_outer_margin = 4.0;
            egui::ScrollArea::vertical().max_height(460.0).show(ui, |ui| {
                for (group, rows) in &table {
                    egui::CollapsingHeader::new(group.clone())
                        .default_open(true)
                        .show(ui, |ui| {
                            for row in rows {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [140.0, 18.0],
                                        egui::Label::new(row.label.clone()),
                                    );
                                    let value = edits.get_mut(row.key).expect("seeded above");
                                    ui.add_sized(
                                        [280.0, 18.0],
                                        egui::TextEdit::singleline(value)
                                            .desired_width(280.0),
                                    );
                                    if ui.small_button(browse_label.clone()).clicked() {
                                        browse = Some((row.key, row.is_dir));
                                    }
                                });
                            }
                        });
                }
            });
            ui.separator();
            // One action, right-aligned. "Cancel" is the title bar's X, which is
            // where a cancel belongs; a second button restating it only invites
            // the question of which one is authoritative.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(ok_label).clicked() {
                    apply = true;
                }
            });
        });

    // The title bar's X is the cancel action: it discards the working copy and
    // writes nothing, exactly as the removed Cancel button did.
    let cancelled = !open;

    if let Some((key, is_dir)) = browse {
        let cur = edits.get(key).cloned().unwrap_or_default();
        let mut dlg = rfd::FileDialog::new();
        let start_dir = if cur.is_empty() {
            crate::core::paths::exe_dir()
        } else {
            let p = std::path::Path::new(&cur);
            if p.is_absolute() {
                if p.is_dir() {
                    p.to_path_buf()
                } else {
                    p.parent()
                        .map(|d| d.to_path_buf())
                        .unwrap_or_else(crate::core::paths::exe_dir)
                }
            } else {
                // a relative value names something under the exe dir; start the
                // browser there rather than at the process cwd
                let abs = crate::core::paths::exe_dir().join(p);
                if abs.is_dir() {
                    abs
                } else {
                    abs.parent()
                        .map(|d| d.to_path_buf())
                        .unwrap_or_else(crate::core::paths::exe_dir)
                }
            }
        };
        dlg = dlg.set_directory(&start_dir);
        // The MAME row picks the executable itself, so the filter is what stops
        // the user from browsing to a folder and wondering why nothing loads.
        dlg = if key == "mame_binary" {
            dlg.add_filter("MAME", &["exe"])
        } else {
            dlg
        };
        let picked = if is_dir { dlg.pick_folder() } else { dlg.pick_file() };
        if let Some(p) = picked {
            edits.insert(key.to_string(), p.to_string_lossy().to_string());
        }
    }

    if apply {
        // an empty field means "use the default", so the key is *removed*
        // rather than stored as an empty string — that keeps a single code path
        // (`core::paths`) deciding what the default is
        let empty: Vec<String> = edits
            .iter()
            .filter(|(_, v)| v.trim().is_empty())
            .map(|(k, _)| k.clone())
            .collect();
        for k in empty {
            app.gui.remove(&k);
            edits.remove(&k);
        }
        // A picked MAME path is stored as given, but a *cleared* one must not
        // silently leave the app pointing at nothing: fall back to the chain's
        // current value so an empty field can never brick the emulator path.
        if !edits.contains_key("mame_binary") {
            if let Some(cur) = app
                .opts
                .as_ref()
                .and_then(|o| o.try_lock().ok())
                .and_then(|o| o.opts.get("mame_binary").map(|p| p.currvalue.clone()))
                .filter(|s| !s.trim().is_empty())
            {
                edits.insert("mame_binary".to_string(), cur);
            }
        }
        for (k, v) in &edits {
            app.gui.set(k, v.clone());
        }
        let _ = app.gui.save();

        // keep the option chain in step; the content loaders read
        // `content_setting`, which falls back to it for values that only ever
        // lived in mame.ini
        if let Some(opts) = app.opts.clone() {
            let mut guard = opts.lock().unwrap();
            for (k, v) in &edits {
                if let Some(o) = guard.opts.get_mut(k) {
                    o.globalvalue = v.clone();
                    o.currvalue = v.clone();
                }
            }
        }

        // re-read everything that depends on these paths
        let dir = app.content_background_dir();
        if dir != app.bg_dir {
            app.bg_dir = dir.clone();
            if app
                .background_file
                .as_deref()
                .is_some_and(|f| !dir.join(f).is_file())
            {
                app.background_file = None;
                app.bg_tex = None;
            }
        }
        app.bg_choices = crate::app::scan_backgrounds(&app.bg_dir);
        app.load_ext_folders();
        app.reload_localized_list();
        // rompath feeds the verify, so a change there invalidates the results
        app.refresh_all();
    }

    // keep the working copy only while the dialog is open
    app.dir_edits = if app.show_dirs_win && !apply && !cancelled {
        edits
    } else {
        Default::default()
    };
    app.show_dirs_win = app.show_dirs_win && !apply && !cancelled;
}

// ---------------------------------------------------------------------
// play dialogs (origin: playOptionsUI init/runMame slots)
// ---------------------------------------------------------------------

fn draw_play(app: &mut MameApp, ctx: &egui::Context) {
    if app.play_dialog.is_none() {
        return;
    }
    let (kind, _) = app.play_dialog.clone().unwrap();
    let mut keep = true;
    let close_flag = std::cell::Cell::new(false);
    let run_flag = std::cell::Cell::new(false);
    egui::Window::new(app.tr(kind.title()))
        .open(&mut keep)
        .resizable(false)
        .collapsible(false)
        .default_width(480.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            let mut file = app
                .play_dialog
                .as_ref()
                .map(|(_, f)| f.clone())
                .unwrap_or_default();
            ui.horizontal(|ui| {
                if ui
                    .add_sized([330.0, 20.0], egui::TextEdit::singleline(&mut file))
                    .changed()
                {
                    if let Some((_, f)) = app.play_dialog.as_mut() {
                        *f = file.clone();
                    }
                }
                let ext = kind.ext();
                if ui.small_button("...").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter(&format!("*.{ext}"), &[ext])
                        .add_filter("All Files (*)", &["*"])
                        .save_file()
                    {
                        if let Some((_, f)) = app.play_dialog.as_mut() {
                            *f = p.to_string_lossy().to_string();
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(app.tr("OK")).clicked() {
                    run_flag.set(true);
                    close_flag.set(true);
                }
                if ui.button(app.tr("Cancel")).clicked() {
                    close_flag.set(true);
                }
            });
        });
    let run = run_flag.get();
    if close_flag.get() {
        let taken = app.play_dialog.take();
        if run {
            if let Some((kind, file)) = taken {
                let args = play_args_for(app, kind, &file);
                app.launch(crate::core::launcher::RunMode::Normal, args);
            }
        }
    }
}

fn play_args_for(app: &MameApp, kind: PlayKind, file: &str) -> Vec<String> {
    let opt = |key: &str| {
        app.opts
            .as_ref()
            .and_then(|o| o.try_lock().ok())
            .and_then(|o| o.opts.get(key).map(|p| p.currvalue.clone()))
            .unwrap_or_default()
    };
    let first = |s: &str| s.split(';').next().unwrap_or("").to_string();
    match kind {
        PlayKind::Savestate => vec!["-state".into(), file.into()],
        PlayKind::Playback => {
            let d = first(&opt("input_directory"));
            vec![
                "-input_directory".into(),
                d,
                "-playback".into(),
                std::path::Path::new(file)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                "-nvram_directory".into(),
                std::env::temp_dir().to_string_lossy().to_string(),
            ]
        }
        PlayKind::Record => {
            let d = first(&opt("input_directory"));
            vec![
                "-input_directory".into(),
                d,
                "-record".into(),
                std::path::Path::new(file)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ]
        }
        PlayKind::Mng => {
            let d = first(&opt("snapshot_directory"));
            vec![
                "-snapshot_directory".into(),
                d,
                "-mngwrite".into(),
                std::path::Path::new(file)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ]
        }
        PlayKind::Avi => {
            let d = first(&opt("snapshot_directory"));
            vec![
                "-snapshot_directory".into(),
                d,
                "-aviwrite".into(),
                std::path::Path::new(file)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
            ]
        }
        PlayKind::Wave => vec!["-wavwrite".into(), file.into()],
    }
}

// ---------------------------------------------------------------------
// cmd dialog (origin: CmdUI)
// ---------------------------------------------------------------------

fn draw_cmd(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_cmd;
    let result = std::cell::Cell::new(0u8);
    egui::Window::new(app.tr("Command Line"))
        .open(&mut show)
        .resizable(true)
        .collapsible(false)
        .default_width(720.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            // fill the dialog and give the command room to breathe; the fixed
            // 700×60 box left dead space at every other window width
            ui.add_sized(
                [ui.available_width(), 160.0],
                egui::TextEdit::multiline(&mut app.cmd_text),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui.button(app.tr("OK")).clicked() {
                    result.set(1);
                }
                if ui.button(app.tr("Cancel")).clicked() {
                    result.set(2);
                }
                ui.weak(app.tr("runs with -noreadconfig"));
            });
        });
    app.show_cmd = show && result.get() != 2;
    let run = result.get() == 1;
    if run {
        let text = app.cmd_text.clone();
        let mame = app.mame.clone();
        if let Some(m) = mame {
            let path = m.path.to_string_lossy().to_string();
            let rest = text.strip_prefix(&path).unwrap_or(&text).trim().to_string();
            let mut args: Vec<String> = vec!["-noreadconfig".into()];
            args.extend(rest.split_whitespace().map(str::to_string));
            app.launch_raw(args);
        }
    }
}

// ---------------------------------------------------------------------
// about / verify windows
// ---------------------------------------------------------------------

fn draw_about(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_about;
    egui::Window::new(app.tr("About"))
        .open(&mut show)
        .resizable(false)
        .collapsible(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                // same reason as the start-up panel: `egui::Image` renders the
                // ⚠ fallback when the embedded bytes have not resolved yet
                draw_app_logo(ui, 96.0);
            });
            ui.heading(format!("MvUI v{}", env!("CARGO_PKG_VERSION")));
            ui.label(app.tr("MvUI — a Rust + egui frontend for MAME"));
            ui.weak(app.tf(
                "mame: {}",
                app.mame
                    .as_ref()
                    .map(|m| m.version.clone())
                    .unwrap_or_else(|| app.tr("not detected")),
            ));
        });
    app.show_about = show;
}

fn draw_verify(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_verify;
    egui::Window::new(app.tr("Checking..."))
        .open(&mut show)
        .resizable(true)
        .collapsible(false)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            ui.spacing_mut().scroll.bar_width = 10.0;
            ui.spacing_mut().scroll.bar_outer_margin = 4.0;
            // height wraps the log so far (up to a cap), instead of a fixed
            // 320px box that starts mostly empty — width stays pinned so the
            // monospace lines do not set the window size
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(300.0)
                .show(ui, |ui| {
                    for l in app.verify_lines.clone() {
                        ui.monospace(l);
                    }
                });
        });
    app.show_verify = show;
}

