//! All egui drawing: menu tree, dockable tab area (egui_dock), folder dock,
//! status bar with parse/audit progress (origin: mainwindow.ui + QDockWidget tabify).

use crate::app::{MameApp, PlayKind, ListMode, COL_LAST, COLUMN_TITLES};
use crate::icons;
use egui_dock::{DockArea, DockState, NodeIndex};
use crate::core::folders::{self, FolderChild, FolderKind};
use crate::core::launcher::RunMode;
use std::sync::{Arc, Mutex};

/// `done / total` 的百分比，**值域 0..100**。
///
/// 别写成 `done as f32 / total as f32`（那是 0..1）再直接 `{:.0}%` 打印——
/// 审计进度就踩过这个：44000 个单元扫到一半显示的是 "0%"，看着像卡死。
/// `total == 0` 返回 0，调用方自己判断要不要显示。
pub(crate) fn percent(done: usize, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    (done as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
}

/// 解析审计"枚举中"阶段的标签，认出 `core::audit::AuditHandle::
/// set_enumerating` 写的 `enum 2/5 dirs, 13824 units`。
///
/// 返回 `(已扫目录数, 目录总数, 已收单元数)`，任何一段认不出来就是
/// `None` —— 调用方据此退回纯文案。**认不出必须安全失败**：这个标签走
/// 的是一把 `Mutex<String>`，格式万一变了，状态栏该退化成"正在审计"，
/// 而不是把一段原始英文 `enum 2/5 dirs` 甩给用户看。
pub(crate) fn parse_enumerating(cur: &str) -> (Option<usize>, Option<usize>, Option<usize>) {
    // `?` 只能用在返回 Option 的函数里，所以内部先算一个 Option，
    // 再摊成三元组返回。
    let parsed: Option<(usize, usize, usize)> = (|| {
        let rest = cur.strip_prefix("enum ")?;
        let (dirs, rest) = rest.split_once(" dirs")?;
        // 没有 '/' 就是格式不对，整体认不出——不返回"半个结果"。
        let (d, t) = dirs.split_once('/')?;
        let done = d.trim().parse::<usize>().ok()?;
        let total = t.trim().parse::<usize>().ok()?;
        let rest = rest.trim_start().strip_prefix(", ")?;
        let units = rest.strip_suffix(" units")?.trim().parse::<usize>().ok()?;
        Some((done, total, units))
    })();
    // `cur` 不是我们写的那个标签（空串、某个游戏名、格式漂移后的串）
    // 时安静地退化：调用方据此显示纯文案。
    parsed.map_or((None, None, None), |(d, t, u)| (Some(d), Some(t), Some(u)))
}

/// where Help ▸ Documentation points
const HELP_URL: &str = "https://bbs.xqemu.cn/";

/// tabs in the dockable central area
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum MainTab {
    List,
    /// the machine-tree panel; a dock tab like any other, so it can be dragged
    /// around (and other tabs can be docked into it)
    Folders,
    Image(usize),
    Text(usize),
}

/// share of the dock width the folder tree gets by default
const FOLDER_DOCK_FRACTION: f32 = 0.17;

/// size of a folder-tree entry's icon (origin: `res/32x32/folder.png`, which
/// `gamelist.cpp:2830` puts on every tree entry)
const FOLDER_ICON: f32 = 16.0;

/// Floor for the width of the View popups.
///
/// egui sizes a menu to its widest entry (`Style::spacing.menu_width`, 400 px, is
/// only the *maximum* — `menu_popup` passes it as the Area's `default_width`, and
/// the Area then shrinks to `content_ui.min_size()`). Four two-character CJK
/// labels plus a radio circle come to well under 100 px, which produced a popup
/// so narrow that longer entries wrapped one character per line. The background
/// list is worse still: it holds file names. Pinning a minimum makes the popups
/// look deliberate and gives file names somewhere to go.
const MENU_MIN_WIDTH: f32 = 200.0;

use egui_dock::Split;

fn split_right(state: &mut DockState<MainTab>, parent: NodeIndex, fraction: f32, tabs: Vec<MainTab>) -> [NodeIndex; 2] {
    state.split((egui_dock::SurfaceIndex(0), parent), Split::Right, fraction, egui_dock::Node::leaf_with(tabs))
}

fn split_below(state: &mut DockState<MainTab>, parent: NodeIndex, fraction: f32, tabs: Vec<MainTab>) -> [NodeIndex; 2] {
    state.split((egui_dock::SurfaceIndex(0), parent), Split::Below, fraction, egui_dock::Node::leaf_with(tabs))
}

pub fn default_docks_filtered(
    images: &[bool; 7],
    texts: &[bool; crate::core::dat::TEXT_DOCK_COUNT],
    folders: bool,
) -> DockState<MainTab> {
    // The machine tree is a normal dock tab in the left strip (origin: the
    // `QDockWidget` of 1.8.2, which could also be floated and tabified).
    let (mut state, root) = if folders {
        let mut s = DockState::new(vec![MainTab::Folders]);
        let [_, right] = split_right(&mut s, NodeIndex::root(), FOLDER_DOCK_FRACTION, vec![MainTab::List]);
        (s, right)
    } else {
        (DockState::new(vec![MainTab::List]), NodeIndex::root())
    };
    let img_tabs: Vec<MainTab> = (0..7).filter(|&i| images[i]).map(MainTab::Image).collect();
    let txt_tabs: Vec<MainTab> = (0..crate::core::dat::TEXT_DOCK_COUNT)
        .filter(|&i| texts[i])
        .map(MainTab::Text)
        .collect();
    if !img_tabs.is_empty() {
        let [list_node, _] = split_right(&mut state, root, 0.68, img_tabs);
        if !txt_tabs.is_empty() {
            let _ = split_below(&mut state, list_node, 0.70, txt_tabs);
        }
    } else if !txt_tabs.is_empty() {
        let _ = split_below(&mut state, root, 0.70, txt_tabs);
    }
    state
}

/// sort key so two tab sets can be compared without Ord on MainTab
fn tab_key(t: &MainTab) -> (u8, usize) {
    match t {
        MainTab::List => (0, 0),
        MainTab::Image(i) => (1, *i),
        MainTab::Text(i) => (2, *i),
        MainTab::Folders => (3, 0),
    }
}

/// the exact tab set the View-menu visibility checkboxes imply
pub fn expected_tabs(
    images: &[bool; 7],
    texts: &[bool; crate::core::dat::TEXT_DOCK_COUNT],
    folders: bool,
) -> Vec<MainTab> {
    let mut v = vec![MainTab::List];
    if folders {
        v.push(MainTab::Folders);
    }
    v.extend((0..7).filter(|&i| images[i]).map(MainTab::Image));
    v.extend(
        (0..crate::core::dat::TEXT_DOCK_COUNT)
            .filter(|&i| texts[i])
            .map(MainTab::Text),
    );
    v
}

/// Restore the saved dock tree (split ratios + tab positions) from the settings
/// string, but only when it still matches the checkbox state — otherwise a
/// stale layout would resurrect docks the user has switched off.
pub fn restore_docks(
    saved: &str,
    images: &[bool; 7],
    texts: &[bool; crate::core::dat::TEXT_DOCK_COUNT],
    folders: bool,
) -> Option<DockState<MainTab>> {
    if saved.is_empty() {
        return None;
    }
    let state: DockState<MainTab> = serde_json::from_str(saved).ok()?;
    let mut have: Vec<(u8, usize)> = state.iter_all_tabs().map(|(_, t)| tab_key(t)).collect();
    let mut want: Vec<(u8, usize)> = expected_tabs(images, texts, folders)
        .iter()
        .map(tab_key)
        .collect();
    have.sort_unstable();
    want.sort_unstable();
    if have == want {
        Some(state)
    } else {
        None
    }
}

struct DockTabs<'a> {
    app: &'a mut MameApp,
}

/// egui_dock's default look has two artefacts on this layout:
///
/// * a node separator is stroked *on top of* the neighbouring node's tab bar, so
///   the line looks like a stray vertical stroke cutting through the tab (it was
///   clearly visible inside the "Game List"/"Flyer" tabs' left padding);
/// * six image tabs overflow their node, and egui_dock then paints a 7.5px
///   scroll bar pill right under the tab row — that grey rounded bar.
fn dock_style(ctx: &egui::Context, wallpaper: bool, dark: bool) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(&ctx.style());
    // keep the splitter invisible while idle; it still highlights on hover and the
    // grab area (`extra_interact_width`) is untouched
    style.separator.color_idle = egui::Color32::TRANSPARENT;
    // the hover/drag highlight fills the whole 4px grab strip, so a pure black
    // default reads as a heavy bar — soften it
    style.separator.color_hovered = egui::Color32::from_gray(150);
    style.separator.color_dragged = egui::Color32::from_gray(90);
    style.tab_bar.show_scroll_bar_on_overflow = false;
    // slightly narrower tabs, so a full row of image tabs is less likely to
    // overflow in the first place
    style.tab.tab_body.inner_margin = egui::Margin::symmetric(4.0, 2.0);
    // The dock leaves are the one place a translucent fill belongs: this is the
    // layer 1.8.2 made see-through in `setTransparentBg` (QPalette::Base), and
    // it is what lets the wallpaper show through the game list and the info
    // docks. It is set *here* rather than on `visuals.window_fill` because egui
    // derives both this and every menu/popup frame from `window_fill` — making
    // that one translucent turned the whole interface, menus included, to grey
    // mush. Setting `tab_body.bg_fill` here keeps the two independent.
    if wallpaper {
        // 1.8.2 `setTransparentBg`: one brush, `QPalette::Base` →
        // `rgba(0,0,0,128)` dark / `rgba(255,255,255,128)` light
        style.tab.tab_body.bg_fill = if dark {
            egui::Color32::from_black_alpha(128)
        } else {
            egui::Color32::from_white_alpha(128)
        };
    }
    style
}

impl egui_dock::TabViewer for DockTabs<'_> {
    type Tab = MainTab;

    fn title(&mut self, tab: &mut MainTab) -> egui::WidgetText {
        let name = match tab {
            MainTab::List => self.app.tr("Game List"),
            MainTab::Folders => self.app.tr("Folders"),
            MainTab::Image(d) => self.app.tr(crate::core::dat::DOCK_NAMES[*d]),
            // index `DOCK_NAMES` through `text_dock` instead of a private
            // per-panel array: same names, but a malformed saved layout
            // yields a valid dock instead of an out-of-bounds panic
            MainTab::Text(d) => {
                let dock = crate::core::dat::text_dock(*d);
                self.app.tr(crate::core::dat::DOCK_NAMES[dock])
            }
        };
        egui::WidgetText::from(name)
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut MainTab) {
        match tab {
            MainTab::List => self.app.draw_table(ui),
            MainTab::Folders => self.app.draw_folders(ui),
            MainTab::Image(d) => self.app.picture_content(ui, *d),
            MainTab::Text(d) => self.app.documents_content(ui, *d),
        }
    }
}

impl eframe::App for MameApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.stash_ctx(ctx);
        if !self.started {
            self.startup();
        }
        self.pump_events(ctx);
        // apply the persisted theme once per start-up: the visuals are not
        // serialized, so without this the window would come up in egui's
        // default light theme regardless of what `dark_bg` says
        if !self.theme_applied {
            self.theme_applied = true;
            let has_bg = self.background_file.is_some();
            if has_bg {
                if let Some(f) = self.background_file.clone() {
                    let dir = self.bg_dir.clone();
                    self.dark_bg = crate::app::background_is_dark(&dir, &f);
                }
            }
            crate::app::apply_theme_with_bg(ctx, self.dark_bg, has_bg);
            // the zoom lives in egui's options, not in `Style`, so it is not
            // restored by applying the theme — it has to be set once per session
            ctx.set_zoom_factor(self.font_zoom);
        }
        // stamp a selection change once, wherever it came from (click, refilter,
        // restore) — the debounce in `selection_settling` reads it
        if self.current_game != self.published_game {
            self.published_game = self.current_game.clone();
            self.sel_changed_at = Some(std::time::Instant::now());
        }
        if self.needs_refilter {
            self.refilter();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.refresh_all();
        }
        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
            self.search_take_focus = true;
        }
        if self.wants_close {
            self.save_settings();
            self.cleanup_temp_roms(None);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.lib_status != crate::app::LibStatus::Ready
            || self.boot_auditing
            || self.audit_handle.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        // the debounced loads need one more frame once the window has elapsed
        if self.selection_settling() {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }

        crate::windows::draw_windows(self, ctx);
        self.draw_menu(ctx);
        self.draw_toolbar(ctx);
        // the machine tree is a dock tab now (MainTab::Folders) — dragging the
        // splitter next to it resizes it, and its width lives in `dock_layout`
        egui::CentralPanel::default().show(ctx, |ui| {
            // the backdrop goes down first, inside this panel, so the dock area
            // and everything in it are painted over it (see `draw_background`)
            self.draw_background(ui);
            if self.need_mame_pick || self.mame.is_none() {
                self.draw_startup_panel(ui);
                return;
            }
            // whether the dock leaves get the see-through fill, and which of
            // the two veils to use — resolved before `self` is borrowed below
            let wallpaper = self.background_file.is_some();
            let dark = self.dark_bg;
            let mut state = std::mem::replace(
                &mut self.dock_state,
                DockState::new(vec![MainTab::List]),
            );
            {
                let mut viewer = DockTabs { app: self };
                DockArea::new(&mut state)
                    .style(dock_style(ctx, wallpaper, dark))
                    .show_inside(ui, &mut viewer);
            }
            self.dock_state = state;
        });
        self.draw_status(ctx);
        self.draw_toast(ctx);
        self.save_settings_periodic();
    }

    /// the window-manager close button bypasses wants_close
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_settings();
        // the WM close button bypasses wants_close — drop the temp roms here too
        self.cleanup_temp_roms(None);
    }
}

pub fn button(label: impl Into<egui::WidgetText>) -> egui::Button<'static> {
    egui::Button::new(label)
}

impl MameApp {
    pub fn tr(&self, key: &str) -> String {
        crate::i18n::tr(&self.lang, key)
    }

    fn save_settings_periodic(&mut self) {
        self.frame_count += 1;
        // Never write while the MAME binary is still unresolved. The picker can
        // sit open for minutes, and this save runs every 200 frames: back when
        // `save_settings` could not restore a missing `mame_binary`, this timer
        // is what actually rewrote the ini without it, turning a one-off bad
        // read into a permanent "the program forgot my MAME path" bug. The
        // write-side guard in `save_settings` now covers the loss itself; this
        // keeps the timer from saving a half-initialised state at all.
        if self.need_mame_pick || self.mame.is_none() {
            return;
        }
        if self.frame_count % 200 == 0 {
            // 静默版本：这是防崩溃丢设置的兜底定时器，不是用户动作，
            // 每次记一条"保存 N 条"只会稀释 boot.log 里真正值得看的东西
            // （一次审计几十分钟，期间日志里就只剩它了）。
            self.save_settings_quiet();
        }
    }

    // ------------------------------------------------------------------
    // menu tree (origin mainwindow.ui)
    // ------------------------------------------------------------------

    /// The window backdrop, drawn as the **central panel's own frame fill**.
    ///
    /// It used to be painted on `Order::Background` over `ctx.screen_rect()`,
    /// which is wrong twice over. `screen_rect()` is the whole window including
    /// the menu/toolbar/status bars, and the picture is opaque, so it buried the
    /// entire interface — with `Order::Background` nominally the lowest layer,
    /// the panels still lost (verified: alpha 255 hid the menu bar too, alpha
    /// 128 let it bleed through and wash out the text). Docking the picture to
    /// the central panel's frame makes egui own the ordering: the backdrop is
    /// painted first inside that frame and every widget draws on top of it.
    ///
    /// Origin: 1.8.2 `setBgPixmap` put the pixmap on the main window's
    /// background role, then `setTransparentBg` swapped `QPalette::Base` for
    /// `rgba(0,0,0,128)` under the tree/list docks — i.e. a translucent panel
    /// over a window-wide picture. `window_fill` (which `egui_dock` turns into
    /// `TabBodyStyle::bg_fill`, style.rs:704) is that same brush here.
    fn draw_background(&mut self, ui: &mut egui::Ui) {
        let Some(file) = self.background_file.clone() else {
            return;
        };
        let dir = self.bg_dir.clone();
        let ctx = ui.ctx().clone();
        let mut tex = self.bg_tex.take();
        let handle = crate::app::load_background(&dir, &file, &mut tex, &ctx);
        self.bg_tex = tex;
        let Some(tex) = handle else { return };

        let area = ui.max_rect();
        let painter = ui.painter_at(area);
        let id = tex.id();
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        if self.bg_stretch {
            // cover the area, keeping the aspect ratio (origin:
            // `scaled(size(), Qt::KeepAspectRatioByExpanding)`)
            painter.image(id, area, uv, egui::Color32::WHITE);
        } else {
            // repeat at the picture's own size
            let sz = tex.size();
            let (tw, th) = (sz[0].max(1) as f32, sz[1].max(1) as f32);
            let mut y = area.min.y;
            while y < area.max.y {
                let mut x = area.min.x;
                while x < area.max.x {
                    let cell = egui::Rect::from_min_max(
                        egui::pos2(x, y),
                        egui::pos2((x + tw).min(area.max.x), (y + th).min(area.max.y)),
                    );
                    painter.image(id, cell, uv, egui::Color32::WHITE);
                    x += tw;
                }
                y += th;
            }
        }
    }

    fn draw_menu(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button(self.tr("File"), |ui| {
                    self.play_section(ui);
                    ui.separator();
                    self.add_folder_section(ui);
                    let label = self.tr("Remove From This Folder");
                    if ui
                        .add_enabled(self.can_remove_from_folder(), button(label))
                        .clicked()
                    {
                        self.remove_from_folder();
                        ui.close_menu();
                    }
                    ui.separator();
                    let src_label = self.src_properties_label();
                    if ui.add_enabled(self.has_game(), button(src_label)).clicked() {
                        self.open_properties(crate::core::options::OPTLEVEL_SRC);
                        ui.close_menu();
                    }
                    let props = self.tr("Properties");
                    if ui.add_enabled(self.has_game(), button(props)).clicked() {
                        self.open_properties(crate::core::options::OPTLEVEL_CURR);
                        ui.close_menu();
                    }
                    ui.separator();
                    self.audit_submenu(ui);
                    ui.separator();
                    let exit = self.tr("Exit");
                    if ui.button(exit).clicked() {
                        self.wants_close = true;
                    }
                });
                self.settings_menu(ui);
                ui.menu_button(self.tr("View"), |ui| {
                    self.font_submenu(ui);
                    self.background_submenu(ui);
                    ui.separator();
                    ui.menu_button(self.tr("Customize Fields"), |ui| {
                        for i in 1..COL_LAST {
                            let mut v = self.col_visible[i];
                            if ui.checkbox(&mut v, self.tr(COLUMN_TITLES[i])).changed() {
                                self.col_visible[i] = v;
                            }
                        }
                    });
                    self.info_panels_submenu(ui);
                    ui.separator();
                    for (mode, key) in
                        [(ListMode::Grouped, "Grouped"), (ListMode::Details, "Details")]
                    {
                        let label = self.tr(key);
                        if ui.radio(self.list_mode == mode, label).clicked() {
                            self.list_mode = mode;
                            ui.close_menu();
                        }
                    }
                    // grid lines on the game list; on by default
                    let gl = self.tr("Show Grid Lines");
                    ui.checkbox(&mut self.show_grid, gl);
                    let ll = self.tr("Local Language Game List");
                    if ui.checkbox(&mut self.local_game_list, ll).changed() {
                        self.needs_refilter = true;
                    }
                });
                ui.menu_button(self.tr("Help"), |ui| {
                    let d = self.tr("Documentation");
                    if ui.button(d).clicked() {
                        self.open_url(HELP_URL);
                        ui.close_menu();
                    }
                    let a = self.tr("About");
                    if ui.button(a).clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });
    }

    pub fn play_section(&mut self, ui: &mut egui::Ui) {
        let play_label = if self.has_game() {
            format!("{} {}", self.tr("Play"), self.current_game)
        } else {
            self.tr("Play")
        };
        if ui.add_enabled(self.has_game(), button(play_label)).clicked() {
            self.launch(RunMode::Normal, vec![]);
            ui.close_menu();
        }
        ui.menu_button(self.tr("Play With"), |ui| {
            let cl = self.tr("Command Line...");
            if ui.add_enabled(self.has_game(), button(cl)).clicked() {
                self.open_cmd_dialog();
                ui.close_menu();
            }
            ui.separator();
            for kind in [
                PlayKind::Savestate,
                PlayKind::Playback,
                PlayKind::Record,
                PlayKind::Mng,
                PlayKind::Avi,
                PlayKind::Wave,
            ] {
                let label = match kind {
                    PlayKind::Savestate => self.tr("Load Savestate..."),
                    PlayKind::Playback => self.tr("Playback Input..."),
                    PlayKind::Record => self.tr("Record Input..."),
                    PlayKind::Mng => self.tr("Record MNG Output..."),
                    PlayKind::Avi => self.tr("Record AVI Output..."),
                    PlayKind::Wave => self.tr("Record Wave Output..."),
                };
                if ui.add_enabled(self.has_game(), button(label)).clicked() {
                    self.open_play_dialog(kind);
                    ui.close_menu();
                }
            }
        });
        self.delete_cfg_submenu(ui);
    }

    pub fn delete_cfg_submenu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.tr("Delete Cfg"), |ui| {
            let files = self.delete_cfg_candidates();
            for path in &files {
                if ui.button(path.display().to_string()).clicked() {
                    let _ = std::fs::remove_file(path);
                    self.log(format!("deleted {}", path.display()));
                    ui.close_menu();
                }
            }
            if !files.is_empty() {
                ui.separator();
                let ra = self.tr("Remove All");
                if ui.button(ra).clicked() {
                    for p in files {
                        let _ = std::fs::remove_file(&p);
                        self.log(format!("deleted {}", p.display()));
                    }
                    ui.close_menu();
                }
            }
        });
    }

    pub fn add_folder_section(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.tr("Add to Folder"), |ui| {
            for (name, store) in self.ext_folder_data.clone() {
                ui.menu_button(name.clone(), |ui| {
                    let root_label = self.tr("Root Folder [.]");
                    if ui.button(root_label).clicked() {
                        if let Some((_, s)) =
                            self.ext_folder_data.iter_mut().find(|(n, _)| *n == name)
                        {
                            s.add("ROOT_FOLDER", &self.current_game.clone());
                        }
                        self.save_ext_folder(&name);
                        self.needs_refilter = true;
                        ui.close_menu();
                    }
                    ui.separator();
                    for section in store.entries.keys() {
                        let label =
                            section.strip_prefix(folders::EXTFOLDER_MAGIC).unwrap_or(section);
                        if ui.button(label.to_string()).clicked() {
                            if let Some((_, s)) =
                                self.ext_folder_data.iter_mut().find(|(n, _)| *n == name)
                            {
                                s.add(section, &self.current_game.clone());
                            }
                            self.save_ext_folder(&name);
                            self.needs_refilter = true;
                            ui.close_menu();
                        }
                    }
                });
            }
        });
    }

    /// 文件菜单里的 "导出列表" 二级菜单。
    ///
    /// **这里只剩导出项**：单游戏审计 / 审计全部 ROM / 审计全部样本三个按钮
    /// 已按用户要求删掉——「刷新档案」(F5) 本来就做的是 re-audit + re-init，
    /// 重复入口只会让人以为这是两件事。
    pub fn audit_submenu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.tr("Export List"), |ui| {
            for (key, method) in [
                ("Export All Set Issues...", crate::core::audit::AuditMethod::ExportAll),
                ("Export Incomplete Sets Only...", crate::core::audit::AuditMethod::ExportIncomplete),
                ("Export Completely Missing Sets Only...", crate::core::audit::AuditMethod::ExportMissing),
                ("Export All Sets...", crate::core::audit::AuditMethod::ExportComplete),
            ] {
                let text = self.tr(key);
                if ui.button(text).clicked() {
                    self.pick_fixdat_target(method);
                    ui.close_menu();
                }
            }
            ui.separator();
            let hv = self.tr("Export Have List...");
            if ui.button(hv).clicked() {
                self.pick_list_target(true);
                ui.close_menu();
            }
            let ms = self.tr("Export Miss List...");
            if ui.button(ms).clicked() {
                self.pick_list_target(false);
                ui.close_menu();
            }
        });
    }

    /// One "information panels" submenu: the seven picture docks, the five
    /// document docks and the two picture-only options. The old View menu had
    /// them split across "Pictures" and "Documents"; they are the same kind of
    /// thing (a dock you can show or hide), so they live together now.
    fn info_panels_submenu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.tr("Information Panels"), |ui| {
            for i in 0..7 {
                let mut v = self.image_dock_visible[i];
                let name = self.tr(crate::core::dat::DOCK_NAMES[i]);
                if ui.checkbox(&mut v, name).changed() {
                    self.image_dock_visible[i] = v;
                    self.dock_state = default_docks_filtered(
                        &self.image_dock_visible,
                        &self.text_dock_visible,
                        self.show_folder_dock,
                    );
                }
            }
            ui.separator();
            // 文档面板的名字直接取 `DOCK_NAMES`：菜单、tab 标题、存档里的布局
            // 三处用的是同一张表，加面板时不会漏掉某一处（漏掉的表现是
            // "勾上了但 tab 上是另一个名字"）。
            for i in 0..crate::core::dat::TEXT_DOCK_COUNT {
                let mut v = self.text_dock_visible[i];
                let label = self.tr(crate::core::dat::DOCK_NAMES
                    [crate::core::dat::text_dock(i)]);
                if ui.checkbox(&mut v, label).changed() {
                    self.text_dock_visible[i] = v;
                    self.dock_state = default_docks_filtered(
                        &self.image_dock_visible,
                        &self.text_dock_visible,
                        self.show_folder_dock,
                    );
                }
            }
            ui.separator();
            let ea = self.tr("Enforce Aspect Ratio");
            ui.checkbox(&mut self.enforce_aspect, ea);
            let ss = self.tr("Strech Screenshot Larger");
            ui.checkbox(&mut self.stretch_sshot, ss);
        });
    }

    /// View ▸ Icon Font — the interface scale, as one exclusive radio group.
    ///
    /// egui has no per-widget font size, only `TextStyle`s and a global
    /// `Context::set_zoom_factor`. The zoom is the honest choice for a "font"
    /// menu: it scales text, spacing and hit targets together, so enlarged
    /// labels do not end up clipped by unscaled rows.
    fn font_submenu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.menu_button(self.tr("Font"), |ui| {
            // A menu is sized to its widest entry, and four short CJK labels
            // make a very narrow popup. Pin a floor so it lines up with the
            // other submenus instead of hugging the radio circles.
            ui.set_min_width(MENU_MIN_WIDTH);
            // (label key, scale). The steps are the ones that stay legible at
            // both ends on a 1080p screen: 0.8 is still readable CJK, 1.4 is
            // about where rows start needing more room than the labels do.
            for (key, scale) in [
                ("Smaller", 0.8f32),
                ("Default Size", 1.0),
                ("Larger", 1.15),
                ("Largest", 1.4),
            ] {
                let label = self.tr(key);
                let picked = (self.font_zoom - scale).abs() < 0.01;
                if ui.radio(picked, label).clicked() {
                    self.font_zoom = scale;
                    // takes effect at the start of the next pass; the request
                    // makes that happen without waiting for another input event
                    ctx.set_zoom_factor(scale);
                    ctx.request_repaint();
                    self.save_settings();
                    ui.close_menu();
                }
            }
        });
    }

    /// View ▸ Window Background — the light/dark palette pair first, then every
    /// image found in the configured background directory (`.\bkground` by
    /// default) as one exclusive radio group.
    ///
    /// Origin: 1.8.2 kept the palette in its own "Options ▸ GUI Style" group
    /// and the pictures in a separate background group. They answer the same
    /// question — what is behind the panels — so they are one list now, and the
    /// palette leads it because it is the choice that is always available.
    ///
    /// Stretch / Tile are unchanged: they are how a picture is drawn, not which
    /// picture, so they stay pinned below the list.
    fn background_submenu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.menu_button(self.tr("Window Background"), |ui| {
            // rescan on open: the directory is next to mame.exe, so pictures
            // dropped in after start-up show up without a restart
            let fresh = crate::app::scan_backgrounds(&self.bg_dir);
            if fresh != self.bg_choices {
                self.bg_choices = fresh;
            }
            // see `font_submenu` — a background list is mostly file names, and
            // without a floor the popup collapses to the width of "None"
            ui.set_min_width(MENU_MIN_WIDTH);
            // palette first — Light / Dark
            let has_bg = self.background_file.is_some();
            let light = self.tr("Light");
            if ui.radio(!self.dark_bg, light).clicked() {
                self.dark_bg = false;
                crate::app::apply_theme_with_bg(&ctx, false, has_bg);
                ui.close_menu();
            }
            let dark = self.tr("Dark");
            if ui.radio(self.dark_bg, dark).clicked() {
                self.dark_bg = true;
                crate::app::apply_theme_with_bg(&ctx, true, has_bg);
                ui.close_menu();
            }
            ui.separator();
            // then one entry per image in the directory
            let none = self.tr("None");
            let mut picked = self.background_file.clone();
            if ui.radio(picked.is_none(), none).clicked() {
                picked = None;
            }
            for name in self.bg_choices.clone() {
                if ui.radio(picked.as_deref() == Some(name.as_str()), &name).clicked() {
                    picked = Some(name);
                }
            }
            if picked != self.background_file {
                self.background_file = picked;
                match self.background_file.clone() {
                    Some(f) => {
                        // decoding may flip light/dark from the picture's luma
                        let dir = self.bg_dir.clone();
                        let mut tex = self.bg_tex.take();
                        crate::app::load_background(&dir, &f, &mut tex, &ctx);
                        self.bg_tex = tex;
                        let dark = crate::app::background_is_dark(&dir, &f);
                        self.dark_bg = dark;
                        crate::app::apply_theme_with_bg(&ctx, dark, true);
                    }
                    None => {
                        // no picture: the panels go opaque again
                        self.bg_tex = None;
                        let dark = self.dark_bg;
                        crate::app::apply_theme_with_bg(&ctx, dark, false);
                    }
                }
                ctx.request_repaint();
            }
            ui.separator();
            let s = self.tr("Stretch");
            if ui.radio(self.bg_stretch, s).clicked() {
                self.bg_stretch = true;
            }
            let t = self.tr("Tile");
            if ui.radio(!self.bg_stretch, t).clicked() {
                self.bg_stretch = false;
            }
        });
    }

    /// Settings — the top-level menu that owns everything configurable.
    ///
    /// It used to be called "Options" and held two flat entries plus a nested
    /// Language. It is now three peers — Language, Directories, MAME extra
    /// config — with the refresh action last, because "refresh" is an action
    /// rather than a setting and belongs at the end of the list, not in View
    /// where it used to sit.
    fn settings_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.tr("Settings"), |ui| {
            self.language_submenu(ui);

            let d = self.tr("Directories");
            if ui.button(d).clicked() {
                // no pre-seed needed: the dialog reads every field straight out
                // of the GUI settings, and an empty field already means "use the
                // default". It used to be seeded from `rompath`, which is not a
                // path this dialog edits any more.
                self.show_dirs_win = true;
                ui.close_menu();
            }

            // MAME's own per-machine / global options, i.e. everything that ends
            // up in mame.ini rather than in the original GUI ini
            let x = self.tr("MAME Extra Config");
            if ui.button(x).clicked() {
                self.open_properties(crate::core::options::OPTLEVEL_GLOBAL);
                ui.close_menu();
            }

            ui.separator();

            // origin actionRefresh: re-audit + re-init. F5 is bound to the same
            // action in `update`, so the shortcut keeps working from anywhere.
            let r = self.tr("Refresh Database");
            if ui.button(format!("{r}    (F5)")).clicked() {
                self.refresh_all();
                // the localized list is a plain text file the user may have
                // edited since boot; re-read it as part of the same refresh
                self.reload_localized_list();
                ui.close_menu();
            }
        });
    }

    fn language_submenu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.tr("Language"), |ui| {
            for (code, label) in crate::i18n::LANGUAGES {
                if ui
                    .radio(self.lang == *code, egui::RichText::new((*label).to_string()))
                    .clicked()
                {
                    self.lang = code.to_string();
                    self.save_settings();
                    ui.close_menu();
                }
            }
        });
    }

    // ------------------------------------------------------------------
    // toolbar
    // ------------------------------------------------------------------

    fn draw_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // filter popup, left of the search box: the four hide flags
                // used to live in View ▸ Custom Filters
                let any_filter = self.filter_flags != 0;
                let fb = self.tr("Filter");
                let fb = if any_filter {
                    format!("{fb} ({})", self.filter_flags.count_ones())
                } else {
                    fb
                };
                if ui
                    .add(egui::Button::new(fb).selected(any_filter))
                    .on_hover_text(self.tr("Filter the game list"))
                    .clicked()
                {
                    self.show_filter_win = !self.show_filter_win;
                }
                let take_focus = self.search_take_focus;
                let hint = self.tr("Search (Ctrl+F)");
                let resp = ui.add_sized(
                    [280.0, 22.0],
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text(hint)
                        .id(egui::Id::new("search_box")),
                );
                if take_focus {
                    resp.request_focus();
                    self.search_take_focus = false;
                }
                if resp.changed() {
                    self.search_changed();
                }
                if ui.small_button("✕").clicked() {
                    self.search.clear();
                    self.search_changed();
                }
                ui.separator();
                let play = format!("▶ {}", self.tr("Play"));
                if ui.add_enabled(self.has_game(), egui::Button::new(play)).clicked() {
                    self.launch(RunMode::Normal, vec![]);
                }
                // 原来这里还有一项「审计」调`refresh_all()`，与文件菜单的
                // 「刷新档案」完全同源（同一个动作、同一份 F5 快捷键），
                // 用户要求删掉。
            });
        });
    }

    // ------------------------------------------------------------------
    // folder tree dock
    // ------------------------------------------------------------------

    /// Full-row band of a tree entry.
    ///
    /// Tints the whole line — the way the game list's rows do — while the pointer
    /// is on it, and keeps the selection tint while the entry is the current
    /// folder. `SelectableLabel` could not do this: it only ever paints its own,
    /// text-sized rect, so the colour stopped at the end of the name.
    ///
    /// The hover test is done against the pointer rather than through
    /// `Response::hovered()`: the branch arrow is registered after this band and
    /// wins the hover over its own cell, and the tint has to cover that cell too.
    ///
    /// Returns the band and whether the line was clicked.
    fn tree_row_band(ui: &mut egui::Ui, id: egui::Id, panel: egui::Rect) -> (egui::Rect, bool) {
        // the row box is the label's own height; adding the vertical item spacing
        // makes neighbouring bands meet, so the tint reads as one continuous line
        let h = ui.text_style_height(&egui::TextStyle::Button) + ui.spacing().item_spacing.y;
        let top = ui.max_rect().top();
        let band = egui::Rect::from_min_max(
            egui::pos2(panel.left(), top),
            egui::pos2(panel.right(), top + h),
        );
        let hit = ui.interact(band, id, egui::Sense::click());
        // Highlight is hover-only: the line tints while the pointer is on it and
        // goes back to normal when the pointer leaves — whether or not the entry is
        // the current folder. Selection is carried by the label's text colour
        // instead (see `tree_row_label`).
        if ui
            .ctx()
            .input(|i| i.pointer.latest_pos())
            .is_some_and(|p| band.contains(p))
        {
            ui.painter()
                .rect_filled(band, 0.0, ui.visuals().widgets.hovered.bg_fill);
        }
        (band, hit.clicked())
    }

    /// Tree entry label, coloured like the game list's cells (the selection
    /// foreground on the selected line, the normal one otherwise).
    ///
    /// `Sense::empty()` matters: a `Label` defaults to `Sense::hover()` and would
    /// take the hover and the press away from the row band behind it.
    fn tree_row_label(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
        let v = ui.visuals();
        let color = if selected {
            v.selection.stroke.color
        } else {
            v.text_color()
        };
        ui.add(
            egui::Label::new(egui::RichText::new(text).color(color))
                .truncate()
                .selectable(false)
                .sense(egui::Sense { click: false, drag: false, focusable: false }),
        )
    }

    /// One first-level entry: branch column, folder icon, label.
    ///
    /// The branch column is reserved for *every* entry — one that cannot be
    /// expanded keeps it empty — so all icons and labels share one grid, the way
    /// the 1.8.2 `QTreeView` lines up childless entries with the expandable ones
    /// below them.
    ///
    /// Returns `(arrow_clicked, row_clicked, label_dx)`, where `label_dx` is the
    /// distance from the row's left edge to the first character of the label. The
    /// second level is indented by exactly that distance, so a child icon starts
    /// where its parent's text starts. Measuring it beats deriving it from the
    /// style constants, which drift with font and zoom.
    fn folder_row(
        ui: &mut egui::Ui,
        id: egui::Id,
        panel: egui::Rect,
        expandable: bool,
        openness: f32,
        selected: bool,
        text: &str,
    ) -> (bool, bool, f32) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let (_band, row_hit) = Self::tree_row_band(ui, id.with("row"), panel);
            let row_left = ui.max_rect().left();
            // egui gives its collapsing toggler the whole indent width and no gap
            // behind it; mirror that so both kinds of row land on one grid.
            let gap = ui.spacing_mut().item_spacing.x;
            ui.spacing_mut().item_spacing.x = 0.0;
            let (_slot, rect) = ui.allocate_space(Self::folder_branch(ui));
            ui.spacing_mut().item_spacing.x = gap;
            let mut arrow = false;
            if expandable {
                let resp = ui.interact(rect, id.with("arrow"), egui::Sense::click());
                Self::paint_branch_arrow(ui, rect, resp.hovered(), openness);
                arrow = resp.clicked();
            }
            icons::draw_passive(ui, &ctx, icons::FOLDER, FOLDER_ICON);
            let label = Self::tree_row_label(ui, selected, text);
            let dx = label.rect.min.x - row_left;
            // the branch indicator toggles and nothing else, as in the Qt tree:
            // a click on it must not also move the selection
            (arrow, row_hit && !arrow, dx)
        })
        .inner
    }

    /// Flat branch triangle for the folder tree: grey at rest, darker on hover,
    /// turning from ▶ to ▼ like the 1.8.2 tree. egui's own collapsing arrow is a
    /// solid black triangle, which reads too heavy next to the folder icons.
    fn paint_branch_arrow(ui: &egui::Ui, rect: egui::Rect, hovered: bool, openness: f32) {
        let rect = egui::Rect::from_center_size(rect.center(), rect.size() * 0.6);
        let points = if openness > 0.5 {
            vec![rect.left_top(), rect.right_top(), rect.center_bottom()]
        } else {
            vec![rect.left_top(), rect.left_bottom(), rect.right_center()]
        };
        let color = if hovered {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        ui.painter()
            .add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
    }

    /// A second-level entry: folder icon + label and no branch column (nothing
    /// nests below a second-level entry).
    ///
    /// Second-level entries carry the same `res/32x32/folder.png` as the tree
    /// root in the reference layout; the caller indents them so the child icon
    /// starts where the parent label starts.
    fn folder_child_row(
        ui: &mut egui::Ui,
        id: egui::Id,
        panel: egui::Rect,
        selected: bool,
        text: &str,
    ) -> bool {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let (_band, clicked) = Self::tree_row_band(ui, id, panel);
            icons::draw_passive(ui, &ctx, icons::FOLDER, FOLDER_ICON);
            Self::tree_row_label(ui, selected, text);
            clicked
        })
        .inner
    }

    /// Branch column: the full `indent` width, which is what egui's collapsing
    /// header reserves for its toggler.
    fn folder_branch(ui: &egui::Ui) -> egui::Vec2 {
        egui::vec2(ui.spacing().indent, ui.spacing().icon_width)
    }

    /// Draw the second level of one root, indented by `indent` points.
    fn folder_body(ui: &mut egui::Ui, indent: f32, add: impl FnOnce(&mut egui::Ui)) {
        let prev_indent = ui.spacing_mut().indent;
        let prev_vline = ui.visuals().indent_has_left_vline;
        ui.spacing_mut().indent = indent;
        // no guide line down the side of the section: the 1.8.2 tree has none,
        // and the child icons already show the nesting.
        ui.visuals_mut().indent_has_left_vline = false;
        ui.indent("folder_body", add);
        ui.spacing_mut().indent = prev_indent;
        ui.visuals_mut().indent_has_left_vline = prev_vline;
    }

    fn draw_folders(&mut self, ui: &mut egui::Ui) {
        // Never wrap a folder entry onto a second line: when the dock is narrowed
        // the label is elided instead, and entries that no longer fit are simply
        // not shown (origin: the Qt tree view, which elides section text).
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        if self.lib.is_none() {
            self.loading_or_error(ui);
            return;
        }
        let Some(cache) = self.folder_cache.clone() else {
            let p = self.tr("preparing folders…");
            ui.label(p);
            return;
        };
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            // the full-row hover/selection bands span the whole tree, second level
            // included, so they need the panel's own left/right edges
            let panel = ui.max_rect();
            for root in &cache.roots {
                if self.hidden_folders.iter().any(|h| h == &root.label) {
                    continue;
                }
                let is_root_selected = self.folder_matches_root(&root.kind);
                let text = format!("{} ({})", self.tr(&root.label), root.count);
                let kind = root.kind.clone();
                let label = root.label.clone();
                let expandable = !root.children.is_empty();
                let id = ui.make_persistent_id(("root_folder", &root.kind));
                let mut state =
                    egui::containers::collapsing_header::CollapsingState::load_with_default_open(
                        &ctx, id, false,
                    );
                let (arrow, row_hit, child_indent) = Self::folder_row(
                    ui,
                    id,
                    panel,
                    expandable,
                    state.openness(&ctx),
                    is_root_selected,
                    &text,
                );
                if arrow {
                    state.toggle(ui);
                    // `toggle` only flips the in-memory flag; egui's own header
                    // persisted it as a side effect of drawing the body, which we
                    // no longer use, so store it here or the row never opens.
                    state.store(&ctx);
                }
                if row_hit {
                    self.select_root(kind.clone(), &label);
                }
                if state.openness(&ctx) > 0.0 {
                    // keep the flag persisted while the section is open, the way
                    // egui's own `show_body` does — otherwise the row can snap
                    // shut again as soon as the app goes idle
                    state.store(&ctx);
                    Self::folder_body(ui, child_indent, |ui| {
                        for child in &root.children {
                            self.draw_folder_child(ui, &kind, &label, child, panel);
                        }
                    });
                }
            }
            for (name, store) in self.ext_folder_data.clone() {
                let total: usize = store.entries.values().map(|v| v.len()).sum();
                let is_sel = self.current_folder == format!("/{name}");
                let id = ui.make_persistent_id(("ext_folder", &name));
                let mut state =
                    egui::containers::collapsing_header::CollapsingState::load_with_default_open(
                        &ctx, id, false,
                    );
                let text = format!("{name} ({total})");
                let (arrow, row_hit, child_indent) =
                    Self::folder_row(ui, id, panel, true, state.openness(&ctx), is_sel, &text);
                if arrow {
                    state.toggle(ui);
                    state.store(&ctx);
                }
                if row_hit {
                    self.select_ext_root(&name);
                }
                if state.openness(&ctx) > 0.0 {
                    state.store(&ctx);
                    Self::folder_body(ui, child_indent, |ui| {
                        let root_games = store.games_in("ROOT_FOLDER");
                        if !root_games.is_empty() {
                            let rl = self.tr("Root Folder [.]");
                            if Self::folder_child_row(
                                ui,
                                ui.make_persistent_id(("ext_root", &name)),
                                panel,
                                self.current_folder == format!("/{name}")
                                    && self.folder_key.is_none(),
                                &format!("{rl} ({})", root_games.len()),
                            ) {
                                self.select_ext_root(&name);
                            }
                        }
                        for section in store.entries.keys() {
                            let label = section
                                .strip_prefix(folders::EXTFOLDER_MAGIC)
                                .unwrap_or(section);
                            let games = store.games_in(section);
                            if Self::folder_child_row(
                                ui,
                                ui.make_persistent_id(("ext_section", &name, label)),
                                panel,
                                self.current_folder == format!("/{name}/{label}"),
                                &format!("{label} ({})", games.len()),
                            ) {
                                self.select_ext_sub(&name, label);
                            }
                        }
                    });
                }
            }
        });
    }

    fn draw_folder_child(
        &mut self,
        ui: &mut egui::Ui,
        kind: &FolderKind,
        root_label: &str,
        child: &FolderChild,
        panel: egui::Rect,
    ) {
        let selected =
            self.folder_kind == *kind && self.folder_key.as_deref() == Some(child.key.as_str());
        let child_label = match kind {
            FolderKind::Bios => {
                let desc = self
                    .maps
                    .bios_map
                    .iter()
                    .find(|(_, n)| **n == child.key)
                    .map(|(d, _)| d.clone())
                    .unwrap_or_else(|| child.key.clone());
                format!("{desc} ({})", child.count)
            }
            FolderKind::Console => {
                let desc = self
                    .maps
                    .console_map
                    .iter()
                    .find(|(_, n)| **n == child.key)
                    .map(|(d, _)| d.clone())
                    .unwrap_or_else(|| child.key.clone());
                format!("{desc} ({})", child.count)
            }
            _ => {
                // child labels come from `utils->getLongName` (control types,
                // media kinds, dump status) — the 1.8.2 catalogue translates them
                let label = self.tr(&child.label);
                format!("{label} ({})", child.count)
            }
        };
        let id = ui.make_persistent_id(("folder_child", kind, &child.key));
        if Self::folder_child_row(ui, id, panel, selected, &child_label) {
            self.folder_kind = kind.clone();
            self.folder_key = Some(child.key.clone());
            self.current_folder = format!("{root_label}/{}", child.label);
            self.needs_refilter = true;
        }
    }

    // ------------------------------------------------------------------
    // dock content renderers
    // ------------------------------------------------------------------

    pub fn picture_content(&mut self, ui: &mut egui::Ui, dock: usize) {
        let game = self.current_game.clone();
        // A matching entry means the request finished. For a game without art the
        // entry is `(game, None)` and must not be re-requested — otherwise the
        // placeholder would spawn a background load on every single frame.
        let tex = match self.snap_tex.get(&dock) {
            Some((g, t)) if *g == game => t.clone(),
            _ => {
                self.request_preview(dock);
                None
            }
        };
        egui::ScrollArea::both().show(ui, |ui| match tex {
            Some(tex) => {
                let avail = ui.available_size();
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Center), |ui| {
                    ui.add(egui::Image::new(&tex).max_size(avail));
                });
            }
            None => {
                // 没有素材（或还在加载）时显示内嵌的 mame.png 占位图，
                // 而不是画一个写着"No snapshot"的空框。
                let (rect, resp) =
                    ui.allocate_exact_size(ui.available_size(), egui::Sense::click());
                if !crate::icons::draw_placeholder(ui, rect) {
                    // 占位图都解码不出来才退回原来的灰框 + 文字
                    ui.painter().rect_stroke(
                        rect,
                        4.0,
                        egui::Stroke::new(1.0_f32, egui::Color32::GRAY),
                    );
                    let none = self.tr("No snapshot");
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        none,
                        egui::FontId::proportional(15.0),
                        egui::Color32::GRAY,
                    );
                }
                if resp.clicked() {
                    let tabs: Vec<usize> =
                        (0..7).filter(|&i| self.image_dock_visible[i]).collect();
                    if tabs.len() > 1 {
                        let idx = tabs.iter().position(|&t| t == dock).unwrap_or(0);
                        let next = tabs[(idx + 1) % tabs.len()];
                        self.image_dock_tab = next;
                    }
                }
            }
        });
    }

    /// Memoize the Command/History tab parse (see `documents_content`).
    ///
    /// Keyed by (dock, game) — the same key the text cache one level up uses,
    /// so the two caches stay in lockstep. Bounded to a handful of games
    /// (each entry can be several MB of segments) by evicting one non-current
    /// entry per insert: steady state, and no cliff when rotating A→B→A.
    fn cached_lines(
        map: &mut std::collections::HashMap<
            (usize, String),
            std::sync::Arc<Vec<crate::core::dat::DatLine>>,
        >,
        dock: usize,
        game: &str,
        text: &str,
        parse: impl Fn(&str) -> Vec<crate::core::dat::DatLine>,
    ) -> std::sync::Arc<Vec<crate::core::dat::DatLine>> {
        const PARSED_CAP: usize = 8;
        if let Some(hit) = map.get(&(dock, game.to_string())) {
            return hit.clone();
        }
        let parsed = std::sync::Arc::new(parse(text));
        if map.len() >= PARSED_CAP {
            if let Some(k) = map.keys().find(|(_, g)| g != game).cloned() {
                map.remove(&k);
            }
        }
        map.insert((dock, game.to_string()), parsed.clone());
        parsed
    }

    /// 该不该让「审计 Rom」可点。
    ///
    /// 审计要占着库写 `available`，所以两个正在跑的审计都得让位：一个是它自己
    /// （`game_audit`），一个是全库那个（`audit_handle`）——两者同时跑出来的
    /// 结论是交集，谁最后落盘谁赢，用户看到的是"刚审完就又变了"。
    ///
    /// **当前没有菜单入口**（2026-10-05 用户要求：右键与文件菜单里的「审计
    /// Rom」都删掉，全走「刷新档案」F5）。逻辑留着：它仍在
    /// `start_game_audit` 内部做前置判断，且将来要恢复入口时不必重写。
    #[allow(dead_code)]
    pub fn can_audit(&self) -> bool {
        self.game_audit.is_none() && self.audit_handle.is_none()
    }

    /// 只审计当前选中的这一款游戏。
    ///
    /// 范围是它自己 + 依赖的主 ROM 文件 + BIOS + 设备 + 样本 + CHD，见
    /// `core::audit::audit_scope`。**不**重扫全库，所以通常一秒内结束
    /// （`audit_cache` 记着每个包的内容，包没变就只stat 不重开）。
    ///
    /// 旧版 1.8.2 的 `actionAudit` 是把 `mame -verifyroms <game>` 的 stdout
    /// 显示在一个文本框里；这里改成读审计缓存的同一份结论（`audit_game`），
    /// 因此比 `-verifyroms` 快得多，而且拥有/缺失是结构化的、能直接显示状态色。
    ///
    /// **当前没有菜单入口**（同上，`can_audit` 的注释）。整套单游戏审计是
    /// 有价值的实现——`core::audit::find_units_for` 的提速就是为它做的
    /// （5.8s → 0.002s）——所以**不删**，留着备用。
    #[allow(dead_code)]
    pub fn start_game_audit(&mut self) {
        if !self.can_audit() {
            return;
        }
        let game = self.current_game.clone();
        if game.is_empty() {
            return;
        }
        let Some(lib) = self.lib.clone() else { return };
        let handle = Arc::new(crate::core::audit::AuditHandle::new());
        self.game_audit = Some(handle.clone());
        self.game_audit_target = game.clone();
        self.log(format!("auditing rom: {game}"));
        crate::background::run_game_audit(
            lib,
            self.opts
                .clone()
                .unwrap_or_else(|| Arc::new(Mutex::new(crate::core::options::OptionCore::default()))),
            game,
            handle,
            self.events_tx.clone(),
            self.ctx(),
        );
    }

    /// Rom 信息面板（View ▸ 自定义信息栏 ▸ RomInfo）。
    ///
    /// 数据来自**审计缓存**：`gamelist.cache` 里的每条 `RomInfo::available`。
    /// 所以切游戏立刻就有内容，不需要碰磁盘、不需要等 dat 文件。
    ///
    /// 整份视图按游戏名缓存（`rom_views`）：egui 每帧都调这个函数，而
    /// `rominfo::view_of` 要扫库（父集链 + 设备 + 样本）。审计结束时清空
    /// 缓存（`lib_audited` 的写入点都在那儿）。
    pub fn rom_info_content(&mut self, ui: &mut egui::Ui, game: &str) {
        if game.is_empty() || self.lib.is_none() {
            ui.weak(self.tr("Select a game to see its roms."));
            return;
        }
        // 审计正在跑：这一轮的结论马上会变，但显示旧的更糟——用户会以为
        // 刚跑完的审计没生效。所以明说。
        let auditing = self.game_audit.is_some();
        if !self.rom_views.contains_key(game) {
            let view = {
                let Some(lib) = self.lib.clone() else { return };
                let guard = lib.lock().unwrap();
                // 缓存容量：用户快速点过 50 款游戏就该有 50 份视图，每份
                // 几 KB。上限比 dat 缓存小，因为一个游戏一份、且切回来看时
                // 大概率已经审计完了（要最新的可以按 F5 或右键重审）。
                //
                // 超限就**整体清空**，不做逐出记账：一个游戏一份、几 KB，
                // 64 份还超了说明用户在一轮审计前点了 64 款以上——而那轮审计
                // 一结束本来就要清空一次。为这点流量维护 LRU 链表不值得，
                // 而且逐出写错的表现是"面板偶发空白"，很难查。
                const VIEW_CACHE_CAP: usize = 64;
                if self.rom_views.len() >= VIEW_CACHE_CAP {
                    self.rom_views.clear();
                }
                crate::core::rominfo::view_of(&guard, game, self.lib_audited)
            };
            self.rom_views.insert(game.to_string(), view);
        }
        let Some(view) = self.rom_views.get(game).cloned() else { return };
        let note = if auditing {
            Some(self.tr("auditing ROM..."))
        } else {
            Some(self.tr("from audit cache"))
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| crate::rompanel::render(ui, self, &view, note));
    }

    pub fn documents_content(&mut self, ui: &mut egui::Ui, tab: usize) {
        // `tab` is a document-tab index; everything below speaks `DOCK_*`,
        // where the document docks live at 7..12. Converting once here keeps
        // the lookup key, the cache key and the renderer in one index space.
        let dock = crate::core::dat::text_dock(tab);
        let game = self.current_game.clone();
        // Rom 信息面板**不走外部 dat**：它要的是审计结果，而审计结果躺在
        // 游戏库里（`RomInfo::available`，随 `gamelist.cache` 落盘）。所以在
        // `request_dat` 之前就分出去——否则 `dock_file_option(DOCK_ROMINFO)`
        // 返回 `None`，面板会一直等一个永远不会来的文件。
        if dock == crate::core::dat::DOCK_ROMINFO {
            return self.rom_info_content(ui, &game);
        }
        let text = self
            .dat_texts
            .get(&(dock, game.clone()))
            .cloned()
            .flatten();
        if text.is_none() {
            self.request_dat(dock);
        }
        // The Command and History tabs render a parsed `Vec<DatLine>`: the
        // parse is regex work over up to 4000 lines, and egui runs this
        // renderer at display refresh rate — so memoize it per (dock, game)
        // instead of reparsing every frame (see `cached_lines`).
        let mut parsed: std::sync::Arc<Vec<crate::core::dat::DatLine>> = Default::default();
        match dock {
            crate::core::dat::DOCK_COMMAND => {
                if let Some(t) = text.as_deref() {
                    parsed = Self::cached_lines(
                        &mut self.doc_parsed,
                        dock,
                        &game,
                        t,
                        crate::core::dat::convert_command_lines,
                    );
                }
            }
            crate::core::dat::DOCK_HISTORY => {
                if let Some(t) = text.as_deref() {
                    parsed = Self::cached_lines(
                        &mut self.doc_parsed,
                        dock,
                        &game,
                        t,
                        crate::core::dat::convert_history_lines,
                    );
                }
            }
            _ => {}
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            match text {
                Some(_) if dock == crate::core::dat::DOCK_COMMAND => {
                    for line in parsed.iter().take(3000)
                    {
                        // a divider is a whole row, not a segment: origin's
                        // `<br>[\x2500-]{8,}<br>` → `<hr>` broke the line
                        if line.segments.len() == 1
                            && matches!(line.segments[0], crate::core::dat::Segment::Rule)
                        {
                            ui.add_space(3.0);
                            ui.separator();
                            ui.add_space(3.0);
                            continue;
                        }
                        ui.horizontal_wrapped(|ui| {
                            for seg in &line.segments {
                                match seg {
                                    crate::core::dat::Segment::Text(s) => {
                                        ui.monospace(s);
                                    }
                                    crate::core::dat::Segment::Icon(n) => {
                                        // origin: convertCommand emits
                                        // `<img src=":/res/16x16/dir-N.png">` — prefer the
                                        // embedded PNG and fall back to a glyph only when
                                        // the file really is missing
                                        if !icons::notation_icon(ui, n, 16.0) {
                                            let (glyph, color) = icons::notation_glyph(n);
                                            ui.label(egui::RichText::new(glyph)
                                                .monospace()
                                                .strong()
                                                .color(color));
                                        }
                                    }
                                    crate::core::dat::Segment::Rule => {
                                        ui.separator();
                                    }
                                }
                            }
                        });
                    }
                }
                Some(_) if dock == crate::core::dat::DOCK_HISTORY => {
                    // History shares the Command tab's `Segment` shape: the XML
                    // writes sections as `- TECHNICAL -` headers, which
                    // `historyxml::render_text` already turned into a rule with
                    // the title on the following line. Rendering that title as a
                    // heading (rather than body text) is what makes the sections
                    // legible — otherwise the records read as an undifferentiated
                    // wall of prose.
                    let lines = &*parsed;
                    // index of the heading line = the line right after a Rule
                    let heading_after_rule = |i: usize| {
                        i > 0
                            && lines[i - 1].segments.len() == 1
                            && matches!(
                                lines[i - 1].segments[0],
                                crate::core::dat::Segment::Rule
                            )
                    };
                    for (i, line) in lines.iter().enumerate().take(4000) {
                        if line.segments.len() == 1
                            && matches!(line.segments[0], crate::core::dat::Segment::Rule)
                        {
                            ui.add_space(4.0);
                            ui.separator();
                            continue;
                        }
                        let crate::core::dat::Segment::Text(s) = &line.segments[0] else {
                            continue;
                        };
                        if heading_after_rule(i) {
                            ui.add_space(2.0);
                            ui.label(egui::RichText::new(s).strong());
                            ui.add_space(2.0);
                        } else {
                            ui.monospace(s);
                        }
                    }
                }
                Some(text) => {
                    ui.monospace(&text);
                }
                None => {
                    ui.weak("-");
                }
            }
        });
    }

    // ------------------------------------------------------------------
    // status bar (progress + badges)
    // ------------------------------------------------------------------

    fn draw_status(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let count_label = format!("{} {}", self.visible.len(), self.tr("games"));
                ui.label(count_label);
                ui.separator();
                if let Some(m) = self.status_info() {
                    // only the grades + description/year: cloning the whole
                    // GameMeta here cost a few hundred Strings per frame
                    let badges = [
                        ("status", m.badges[0]),
                        ("emulation", m.badges[1]),
                        ("color", m.badges[2]),
                        ("sound", m.badges[3]),
                        ("graphic", m.badges[4]),
                        ("cocktail", m.badges[5]),
                        ("protection", m.badges[6]),
                        ("savestate", m.badges[7]),
                    ];
                    for (name, grade) in badges {
                        if grade == crate::core::model::STATUS_NA {
                            continue;
                        }
                        let text = if name == "savestate" {
                            if grade == 1 {
                                self.tr("supported")
                            } else {
                                self.tr("unsupported")
                            }
                        } else {
                            match grade {
                                1 => self.tr("good"),
                                2 => self.tr("imperfect"),
                                0 => self.tr("preliminary"),
                                _ => self.tr("unknown"),
                            }
                        };
                        let resp = icons::draw_square(ui, icons::status_color(grade), 14.0);
                        // origin: mameopt/status tooltips — tr("status"), tr("emulation"), …
                        resp.on_hover_text(format!("{}: {text}", self.tr(name)));
                    }
                    ui.separator();
                    ui.strong(m.title);
                    ui.weak(format!(
                        "({})",
                        if m.year.is_empty() { "?" } else { &m.year }
                    ));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.running.is_empty() {
                        ui.colored_label(icons::GREEN, "▶ MAME");
                    }
                    let (done, total, cur) = if let Some(h) = &self.audit_handle {
                        h.snapshot()
                    } else if self.boot_auditing {
                        self.audit_stage.clone()
                    } else {
                        (0, 0, String::new())
                    };
                    // Audit progress: text only. The bar that used to sit here
                    // was redundant — `{pct:.0}%` already says the same thing in
                    // the same amount of space. The counts are shown as well:
                    // on a cold first scan the percentage crawls (a 44 k-set
                    // collection spends minutes inside its first single-digit
                    // percents) and an integer percent alone reads as "stuck
                    // at 0%" — the raw counter is what shows it is moving.
                    if total > 0 {
                        let pct = percent(done, total);
                        ui.weak(format!(
                            "{} {pct:.0}% ({done}/{total})",
                            self.tr("Auditing")
                        ));
                    } else if self.boot_auditing || self.audit_handle.is_some() {
                        // 分母还不存在的那一段：正在枚举待扫单元
                        // （`set_total` 排在全量 `read_dir` 之后，见
                        // `core/audit.rs`）。这段过去只打"正在审计"四个
                        // 字，在冷盘上十几秒到几十秒看起来像卡死。
                        // `cur` 形如 `enum 2/5 dirs, 13824 units`。
                        let (dirs_done, dirs_total, units) = parse_enumerating(&cur);
                        // 目录数已知就报"第几个/共几个 + 已收单元"，
                        // 认不出（空串、格式漂移）就退回纯文案。
                        let label = match (dirs_done, dirs_total, units) {
                            (Some(d), Some(t), Some(u)) if t > 0 => format!(
                                "{} {} {d}/{t} · {u}",
                                self.tr("Auditing"),
                                self.tr("scanning"),
                            ),
                            _ => self.tr("Auditing"),
                        };
                        ui.weak(label);
                    }
                    // No cancel affordance: the audit runs in the background and
                    // is not something the user should have to babysit. It is
                    // also not safe to abandon halfway — `AuditDone` is what
                    // persists `audit_cache.bin`, so cancelling meant repeating
                    // the whole first scan next boot. `AuditHandle::cancel` is
                    // kept for the headless examples, which do want a stop.
                    if self.lib_status == crate::app::LibStatus::Loading {
                        let (done, total) = self.lib_progress;
                        if total > 0 {
                            // 阶段二：分母是收输出时数出来的机种总数，真百分比。
                            // 封顶 99% —— 100% 留给"加载完成"，否则解析完还要等
                            // 审计，进度条会先顶到头再纹丝不动
                            let pct = percent(done, total).min(99.0);
                            ui.weak(format!(
                                "{} {pct:.0}% ({done}/{total})",
                                self.tr("Parsing XML")
                            ));
                        } else {
                            // 阶段一：还在收输出，总数要收完才知道，只报台数
                            ui.weak(format!("{} ({done})", self.tr("Reading listxml")));
                        }
                    }
                });
            });
        });
    }

    pub fn loading_or_error(&self, ui: &mut egui::Ui) {
        match self.lib_status {
            crate::app::LibStatus::Idle => {
                ui.label(self.tr("No MAME binary configured."));
            }
            crate::app::LibStatus::Loading => {
                ui.label(self.tr("Loading game list…"));
            }
            crate::app::LibStatus::Error => {
                ui.colored_label(egui::Color32::RED, self.last_error.clone().unwrap_or_default());
            }
            crate::app::LibStatus::Ready => {}
        }
    }

    fn draw_startup_panel(&mut self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                // The logo is drawn through the texture-loading path, not
                // `egui::Image` — the widget showed a red ⚠ here instead of the
                // mark (it paints before the byte loader has resolved the
                // embedded bytes). See `windows::draw_app_logo`.
                ui.add_space(40.0);
                crate::windows::draw_app_logo(ui, 120.0);
                ui.add_space(8.0);
                ui.heading("MvUI");
                ui.add_space(8.0);
                ui.label(self.tr("MAME executable not configured."));
                ui.add_space(12.0);
                let sel = self.tr("Select mame.exe...");
                if ui.button(sel).clicked() {
                    self.open_mame_picker();
                }
                ui.add_space(6.0);
                if let Some(m) = &self.mame {
                    ui.weak(m.path.display().to_string());
                } else if self.picking {
                    ui.weak(self.tr("selecting…"));
                }
            });
        });
    }

    fn draw_toast(&mut self, ctx: &egui::Context) {
        if let Some(err) = self.last_error.clone() {
            egui::Window::new("⚠")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_TOP, [0.0, 60.0])
                .show(ctx, |ui| {
                    ui.label(&err);
                    let ok = self.tr("OK");
                    if ui.button(ok).clicked() {
                        self.last_error = None;
                    }
                });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `percent` 必须返回 0..100，不是 0..1。
    ///
    /// 曾经写成 `(done / total).clamp(0.0, 1.0)` 再 `{:.0}%` 打印，44000 个
    /// 单元扫到一半显示 "0%" —— 界面上看着就是进度条坏了。
    #[test]
    fn percent_is_scaled_to_hundred() {
        assert_eq!(percent(0, 44_000), 0.0);
        assert_eq!(percent(22_000, 44_000), 50.0);
        assert_eq!(percent(44_000, 44_000), 100.0);
        // 没有分母时给 0，让调用方自己决定要不要显示
        assert_eq!(percent(1, 0), 0.0);
        // 计数冲过总数（并发下的常见抖动）不能显示 100% 以上
        assert_eq!(percent(999, 100), 100.0);
    }

    /// `parse_enumerating` 必须认出 `set_enumerating` 写的格式。
    ///
    /// 两侧是对偶的：`core/audit.rs` 改格式而这里没跟上，状态栏就会
    /// 静默退化成"正在审计"（这正是它认不出时的行为，不报错）。
    #[test]
    fn parse_enumerating_reads_the_audit_label() {
        let (d, t, u) = parse_enumerating("enum 2/5 dirs, 13824 units");
        assert_eq!(d, Some(2));
        assert_eq!(t, Some(5));
        assert_eq!(u, Some(13824));
        // 第一个目录、还没收到单元
        assert_eq!(
            parse_enumerating("enum 0/3 dirs, 0 units"),
            (Some(0), Some(3), Some(0))
        );
    }

    /// 认不出就干净地退化成 `None`，**绝不把原始标签甩给用户**。
    ///
    /// 这个标签走的是一把 `Mutex<String>`，格式一旦漂移（比如有人把
    /// "units" 改成 "romsets"），状态栏必须安静地退回纯文案。
    #[test]
    fn parse_enumerating_rejects_anything_else() {
        for s in [
            "",
            "pacman.zip",
            "enum 2/5 dirs",              // 少了单元段
            "enum 2/5 dirsets, 10 units", // 段名变了
            "scan 2/5 dirs, 10 units",    // 前缀变了
            "enum x/y dirs, 10 units",    // 不是数字
        ] {
            assert_eq!(
                parse_enumerating(s),
                (None, None, None),
                "{s:?} 不该被认成枚举进度"
            );
        }
    }

    fn line(t: &str) -> crate::core::dat::DatLine {
        crate::core::dat::DatLine {
            segments: vec![crate::core::dat::Segment::Text(t.to_string())],
        }
    }

    /// The memo must serve the same parse for repeat frames and stay bounded:
    /// a per-frame reparse here is exactly the cost this cache exists to stop.
    #[test]
    fn cached_lines_memoizes_and_stays_bounded() {
        let mut map = std::collections::HashMap::new();
        let first = MameApp::cached_lines(&mut map, 11, "game", "x", |t| vec![line(t)]);
        let again = MameApp::cached_lines(&mut map, 11, "game", "y", |t| vec![line(t)]);
        // second frame: same Arc — the parse is memoized per (dock, game), a
        // changed text only ever arrives through a new game
        assert!(std::sync::Arc::ptr_eq(&first, &again));

        // a different dock is a different entry
        let other = MameApp::cached_lines(&mut map, 7, "game", "y", |_| vec![]);
        assert!(!std::sync::Arc::ptr_eq(&again, &other));

        // eviction keeps the map bounded no matter how many games rotate through
        for i in 0..20 {
            MameApp::cached_lines(&mut map, 11, &format!("g{i}"), "x", |_| vec![]);
        }
        assert!(map.len() <= 8, "map grew to {}", map.len());
    }
}
