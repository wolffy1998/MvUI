//! Dialog windows (origin: optionsUI/csvCfgUI/dirsUI/playOptionsUI/cmdUI/aboutUI).

use crate::app::{
    MameApp, PlayKind, F_CLONES, F_MECHANICAL, F_NONWORKING, F_UNAVAILABLE,
};
use crate::core::options::{
    OptKind, OptionCore, GUI_CATEGORIES, OPTLEVEL_BIOS, OPTLEVEL_CLONEOF,
    OPTLEVEL_GUI, OPTLEVEL_GLOBAL, OPTLEVEL_SRC, LEVEL_NAMES,
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
        egui::Color32::from_rgb(32, 32, 32)
    } else {
        egui::Color32::from_rgb(252, 252, 252)
    };
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
    ui.painter().image(
        texture.id,
        rect,
        egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(texture.size.x as f32, texture.size.y as f32),
        ),
        egui::Color32::WHITE,
    );
    true
}

pub fn draw_windows(app: &mut MameApp, ctx: &egui::Context) {
    draw_options(app, ctx);
    draw_dirs(app, ctx);
    draw_play(app, ctx);
    draw_cmd(app, ctx);
    draw_about(app, ctx);
    draw_verify(app, ctx);
    draw_filter(app, ctx);
}

/// Filter popup, opened from the toolbar button left of the search box. These
/// four flags used to sit in View ▸ Custom Filters; the menu entry is gone, the
/// filtering itself is unchanged.
fn draw_filter(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_filter_win;
    egui::Window::new(app.tr("Filter"))
        .open(&mut show)
        .resizable(false)
        .default_width(240.0)
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
// options (origin: OptionsUI 6 tabs + category lists + OptionDelegate)
// ---------------------------------------------------------------------

fn draw_options(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_options_win.is_some();
    let level = app.show_options_win.unwrap_or(OPTLEVEL_GLOBAL);
    egui::Window::new(app.tf("Options - {}", app.tr(LEVEL_NAMES[level.min(5)])))
        .open(&mut show)
        .resizable(true)
        .default_width(860.0)
        .default_height(560.0)
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            app.ensure_chain();
            // level tabs
            ui.horizontal(|ui| {
                for (i, name) in LEVEL_NAMES.iter().enumerate() {
                    if ui.selectable_label(app.opt_level == i, app.tr(*name)).clicked() {
                        app.opt_level = i;
                        app.opt_edits.clear();
                    }
                }
            });
            ui.separator();
            let Some(opts) = app.opts.clone() else {
                ui.weak(format!("({})", app.tr("option template not loaded")));
                return;
            };
            // build a snapshot to render without holding the lock while editing
            let snap = build_snapshot(&opts, app);
            // left category list + right rows
            ui.columns(2, |cols| {
                let list = &mut cols[0];
                egui::ScrollArea::vertical().show(list, |ui| {
                    let cats: Vec<&str> = if app.opt_level == OPTLEVEL_GUI {
                        GUI_CATEGORIES.iter().map(|s| s.as_ref()).collect()
                    } else {
                        CORE_CATEGORIES_VEC.iter().map(|s| s.as_ref()).collect()
                    };
                    for c in cats {
                        // `opt_category` keeps the English key; only the display
                        // is translated, so the snapshot filter still matches
                        if ui.selectable_label(app.opt_category == c, app.tr(c)).clicked() {
                            app.opt_category = c.to_string();
                        }
                    }
                });
                let rows = &mut cols[1];
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(rows, |ui| {
                    egui::Grid::new("opt_rows")
                        .num_columns(3)
                        .striped(true)
                        .min_col_width(110.0)
                        .show(ui, |ui| {
                            let mut current_title = String::new();
                            for item in &snap {
                                match item {
                                    SnapRow::Title(t) => {
                                        current_title = t.clone();
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
                                        if app.opt_level == OPTLEVEL_GUI && !d.guivisible {
                                            continue;
                                        }
                                        if !current_category_matches(&current_title, &app.opt_category, app.opt_level) {
                                            continue;
                                        }
                                        let changed = is_changed(&opts, app.opt_level, d);
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
                                        if changed {
                                            label = label.strong();
                                        }
                                        ui.label(label);
                                        edit_control(app, ui, d, &opts);
                                        // current value + reset
                                        ui.horizontal(|ui| {
                                            ui.weak(egui::RichText::new(app.tr(&d.display)).small());
                                            if changed && ui.small_button("↺").clicked() {
                                                let parent_val = parent_value(&opts, app.opt_level, d);
                                                app.opt_edits.insert(d.name.clone(), parent_val);
                                                apply_edit(app, &opts, &d.name);
                                            }
                                        });
                                        ui.end_row();
                                    }
                                }
                            }
                        });
                });
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.weak(app.tf(
                    "editing level: {} — edits are written to the matching ini on change",
                    app.tr(LEVEL_NAMES[app.opt_level.min(5)]),
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
    value: String,
    display: String,
    defvalue: String,
    globalvalue: String,
    srcvalue: String,
    biosvalue: String,
    cloneofvalue: String,
    choices: Vec<(String, String)>,
    min: f64,
    max: f64,
    guivisible: bool,
}

#[derive(Clone, Debug)]
enum SnapRow {
    Title(String),
    Opt(SnapOpt),
}

const CORE_CATEGORIES_VEC: [&str; 7] = [
    "Core Video", "OSD Video", "Screen", "Audio", "Control", "Vector", "Misc",
];

fn current_category_matches(title: &str, category: &str, level: usize) -> bool {
    // category list membership was applied when building the snapshot
    let _ = (title, category, level);
    true
}

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
        let in_cat = if level == OPTLEVEL_GUI {
            matches!(
                cat_key.split('_').nth(1).unwrap_or(""),
                "GUI Paths" | "MAME Paths" | "MESS Paths"
            ) && seg == gui_category_key(&app.opt_category)
        } else {
            seg == app.opt_category
        };
        if !in_cat {
            continue;
        }
        rows.push(SnapRow::Title(cat_key.split('_').last().unwrap_or("").to_string()));
        for name in names {
            let Some(o) = guard.opts.get(name) else { continue };
            // visibility per level (origin updateModel filter)
            let visible = match level {
                OPTLEVEL_GUI => o.guivisible,
                OPTLEVEL_GLOBAL => o.globalvisible,
                OPTLEVEL_SRC => o.srcvisible,
                OPTLEVEL_BIOS => o.biosvisible,
                OPTLEVEL_CLONEOF => o.cloneofvisible,
                _ => o.gamevisible,
            };
            if !visible {
                continue;
            }
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
                value: o.currvalue.clone(),
                display: guard.get_long_value(name, &o.currvalue),
                defvalue: o.defvalue.clone(),
                globalvalue: o.globalvalue.clone(),
                srcvalue: o.srcvalue.clone(),
                biosvalue: o.biosvalue.clone(),
                cloneofvalue: o.cloneofvalue.clone(),
                choices,
                min: o.min.parse().unwrap_or(0.0),
                max: o.max.parse().unwrap_or(100.0),
                guivisible: o.guivisible,
            }));
        }
    }
    rows
}

fn gui_category_key(label: &str) -> &'static str {
    match label {
        "GUI Paths" => "GUI Paths",
        "MAME Paths" => "MAME Paths",
        "MESS Paths" => "MESS Paths",
        _ => "GUI Paths",
    }
}

fn is_changed(core: &std::sync::Mutex<OptionCore>, level: usize, d: &SnapOpt) -> bool {
    let guard = match core.try_lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let comp = match level {
        OPTLEVEL_GUI | OPTLEVEL_GLOBAL => &d.defvalue,
        OPTLEVEL_SRC => &d.globalvalue,
        OPTLEVEL_BIOS => &d.srcvalue,
        OPTLEVEL_CLONEOF => &d.biosvalue,
        _ => &d.cloneofvalue,
    };
    guard.get_long_value(&d.name, comp) != d.display
}

fn parent_value(core: &std::sync::Mutex<OptionCore>, level: usize, d: &SnapOpt) -> String {
    let guard = match core.try_lock() {
        Ok(g) => g,
        Err(_) => return d.value.clone(),
    };
    let comp = match level {
        OPTLEVEL_GUI | OPTLEVEL_GLOBAL => &d.defvalue,
        OPTLEVEL_SRC => &d.globalvalue,
        OPTLEVEL_BIOS => &d.srcvalue,
        OPTLEVEL_CLONEOF => &d.biosvalue,
        _ => &d.cloneofvalue,
    };
    guard.get_short_value(&d.name, &guard.get_long_value(&d.name, comp))
}

fn edit_control(app: &mut MameApp, ui: &mut egui::Ui, d: &SnapOpt, opts: &std::sync::Mutex<OptionCore>) {
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
                apply_edit(app, opts, &d.name);
            }
        }
        3 => {
            // Every value is shown, not hidden behind a drop-down.
            //
            // The template's widest enumeration is `scale_effect` with 18
            // entries and the widest common one (`snapview`, `video`) has 5-9,
            // so laying them out inline costs a few rows and saves a click on
            // every single edit. A combo also had a second problem: it covered
            // the rows below it and inherited the translucent window fill, so
            // the open list was hard to read over a wallpaper.
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
        4 | 9 => {
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
        OPTLEVEL_GUI | OPTLEVEL_GLOBAL => o.globalvalue = short.clone(),
        OPTLEVEL_SRC => o.srcvalue = short.clone(),
        OPTLEVEL_BIOS => o.biosvalue = short.clone(),
        OPTLEVEL_CLONEOF => o.cloneofvalue = short.clone(),
        _ => {}
    }
    o.currvalue = short;
    // GUI keys persist into pGuiSettings (origin saveSettings)
    if level == OPTLEVEL_GUI {
        app.gui.set(name, o.currvalue.clone());
        let _ = app.gui.save();
        return;
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
    /// it is now written into the field for real, so the dialog shows the paths
    /// MvUI actually uses — `.\snap`, `.\dats\command.dat`, `.\mame_cn.lst` — and
    /// the program directory is stated once in the header instead of on all 14
    /// rows. Empty only for MAME itself, which has no default to offer.
    default_value: String,
    is_dir: bool,
}

/// The dialog table: `(translated group heading, rows)`.
fn dir_rows(app: &MameApp) -> Vec<(String, Vec<DirRow>)> {
    use crate::core::{dat, paths};
    let tr = |s: &str| app.tr(s).to_string();
    // a relative path as the user types it: the Windows-style `.\` prefix plus
    // the segments, joined the same way the resolver will join them
    let rel = |parts: &[&str]| format!(".\\{}", parts.join("\\"));

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
        .map(|(i, (key, dir))| DirRow {
            key,
            label: tr(dat::DOCK_NAMES.get(i).copied().unwrap_or("Image")),
            default_value: rel(&[dir]),
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
        .map(|(key, file)| {
            let name = dat::DOCK_NAMES
                .iter()
                .find(|n| dat::dock_file_option(docks_index(n)) == Some(*key))
                .copied()
                .unwrap_or("History");
            DirRow {
                key,
                label: tr(name),
                default_value: rel(&[paths::DAT_SUBDIR, file]),
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
                default_value: rel(&[paths::LST_FILE]),
                is_dir: false,
            },
            DirRow {
                key: "background_directory",
                label: tr("Background images"),
                default_value: rel(&[paths::BG_SUBDIR]),
                is_dir: true,
            },
            DirRow {
                key: "folder_directory",
                label: tr("Folder lists"),
                default_value: rel(&[paths::FOLDERS_SUBDIR]),
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
    // egui 0.29's `Window` has no `title_bar(|ui| …)` hook (that arrived in
    // 0.30), so the mark is painted into the window's own title bar by hand —
    // see below for why the closure's own painter cannot be used directly.
    let style = ctx.style();
    let bar_h = ctx.fonts(|f| f.row_height(&style.text_styles[&egui::TextStyle::Heading]))
        + style.spacing.window_margin.top
        + style.spacing.window_margin.bottom;

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
        .frame(opaque_frame(ctx))
        // The title is translated, but the window's position must not move when
        // the language does — `Window::new` derives its `Area` id from the title
        // text, so switching language would otherwise reset the placement.
        .id(egui::Id::new("mamepgui_dirs"))
        .show(ctx, |ui| {
            // The MvUI mark, at the left of the title bar. It cannot be added as
            // a widget: the title bar is drawn by `Window` itself *above* this
            // closure's rectangle, and `Ui::painter` is clipped to that
            // rectangle, so a shape drawn up there is discarded without warning.
            // Cloning the painter and replacing the clip rect with the title
            // bar's own bounds lifts that restriction.
            //
            // Painting here rather than after `show()` returns is deliberate and
            // is what makes it visible: `Window` reserves the frame background
            // (`Shape::Noop`, `frame.rs:247`) and the title-bar background
            // (`window.rs:523`) as placeholders *before* running this closure and
            // fills them in afterwards (`frame.rs:339`, `window.rs:592`). Because
            // `Painter::set` replaces a shape in place, those backgrounds keep
            // their early indices and every shape added from here lands on top.
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
            // `Image::paint_at` needs a `Ui`, and the `Ui` we have is clipped
            // away up here, so go straight to the painter and address the texture
            // by hand. `SizedTexture` carries the id and the source size but no UV
            // rect, so the whole texture is addressed directly.
            if let Ok(egui::load::TexturePoll::Ready { texture }) = app_logo().load(
                ctx,
                egui::TextureOptions::LINEAR,
                egui::SizeHint::Scale(egui::emath::OrderedFloat(size)),
            ) {
                let mut p = ui.painter().clone();
                p.set_clip_rect(bar);
                p.image(
                    texture.id,
                    mark,
                    egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(texture.size.x as f32, texture.size.y as f32),
                    ),
                    egui::Color32::WHITE,
                );
            }
            ui.label(note);
            ui.label(
                egui::RichText::new(exe_note.clone()).small().weak(),
            );
            ui.add_space(4.0);
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
        // rompath feeds the audit, so a change there invalidates the results
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
        .default_width(480.0)
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
        .default_width(720.0)
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            ui.add_sized([700.0, 60.0], egui::TextEdit::multiline(&mut app.cmd_text));
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
        .default_width(520.0)
        .default_height(320.0)
        .frame(opaque_frame(ctx))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for l in app.verify_lines.clone() {
                    ui.monospace(l);
                }
            });
        });
    app.show_verify = show;
}
