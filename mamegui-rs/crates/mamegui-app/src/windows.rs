//! Dialog windows (origin: optionsUI/csvCfgUI/dirsUI/playOptionsUI/cmdUI/aboutUI).

use crate::app::{
    MameApp, PlayKind, F_CLONES, F_MECHANICAL, F_NONWORKING, F_UNAVAILABLE,
};
use mamegui_core::options::{
    OptKind, OptionCore, GUI_CATEGORIES, OPTLEVEL_BIOS, OPTLEVEL_CLONEOF,
    OPTLEVEL_GUI, OPTLEVEL_GLOBAL, OPTLEVEL_SRC, LEVEL_NAMES,
};

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
                                            mamegui_core::options::capitalize_str(&d.gui_name)
                                        } else {
                                            mamegui_core::options::capitalize_str(
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
                    mamegui_core::options::capitalize_str(name)
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
            let combo = egui::ComboBox::from_id_salt(format!("opt-{}", d.name))
                .selected_text(app.tr(&val));
            combo.show_ui(ui, |ui| {
                for (canon, gui) in &d.choices {
                    // the canonical value stays untranslated; the shown guivalue
                    // is what the 1.8.2 catalogue translates (e.g. Auto / None)
                    if ui.selectable_label(val == *gui, app.tr(gui)).clicked() {
                        app.opt_edits.insert(d.name.clone(), gui.clone());
                        apply_edit(app, opts, &d.name);
                    }
                    let _ = canon;
                }
            });
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

fn draw_dirs(app: &mut MameApp, ctx: &egui::Context) {
    let mut show = app.show_dirs_win;
    let mut apply = false;
    egui::Window::new(app.tr("Directories"))
        .open(&mut show)
        .resizable(true)
        .default_width(520.0)
        .show(ctx, |ui| {
            let mut items: Vec<String> = app
                .dirs_buf
                .split(';')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let mut changed = false;
            let mut remove: Option<usize> = None;
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for (i, item) in items.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized([360.0, 18.0], egui::TextEdit::singleline(item))
                            .changed()
                        {
                            changed = true;
                        }
                        if ui.small_button("...").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                *item = p.to_string_lossy().to_string();
                                changed = true;
                            }
                        }
                        if ui.small_button("−").clicked() {
                            remove = Some(i);
                        }
                    });
                }
            });
            if let Some(r) = remove {
                items.remove(r);
                changed = true;
            }
            ui.horizontal(|ui| {
                if ui.button(app.tr("Append directory")).clicked() {
                    items.push(String::new());
                    changed = true;
                }
            });
            if changed {
                app.dirs_buf = items.join(";");
            }
            ui.separator();
            if ui.button(app.tr("OK")).clicked() {
                apply = true;
            }
        });
    app.show_dirs_win = show;
    if apply {
        if let (Some(opts), Some(name)) = (&app.opts, app.dirs_target_option.clone()) {
            let mut guard = opts.lock().unwrap();
            if let Some(o) = guard.opts.get_mut(&name) {
                o.globalvalue = app.dirs_buf.clone();
                o.currvalue = app.dirs_buf.clone();
            }
            let path = guard.mame_ini_path.join(if guard.mess_like { "mess.ini" } else { "mame.ini" });
            let default_ini = app
                .lib
                .as_ref()
                .and_then(|l| l.try_lock().ok())
                .map(|l| l.default_ini.clone())
                .unwrap_or_default();
            let _ = guard.save_ini_file(OPTLEVEL_GLOBAL, &path, &default_ini);
        } else {
            app.gui.set("rompath", app.dirs_buf.clone());
            let _ = app.gui.save();
        }
    }
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
                app.launch(mamegui_core::launcher::RunMode::Normal, args);
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
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add(
                    egui::Image::new(egui::include_image!("../../../assets/logo.png"))
                        .max_size(egui::vec2(96.0, 96.0)),
                );
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
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for l in app.verify_lines.clone() {
                    ui.monospace(l);
                }
            });
        });
    app.show_verify = show;
}
