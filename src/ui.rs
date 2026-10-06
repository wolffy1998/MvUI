//! All egui drawing: menu tree, dockable tab area (egui_dock), folder dock,
//! status bar with parse/verify progress (origin: mainwindow.ui + QDockWidget tabify).

use crate::app::{MameApp, ListMode, COL_LAST, COLUMN_TITLES};
use crate::icons;
use egui_dock::{DockArea, DockState, NodeIndex};
use crate::core::folders::{self, FolderChild, FolderKind};
use crate::core::launcher::RunMode;

/// `done / total` 的百分比，**值域 0..100**。
///
/// 别写成 `done as f32 / total as f32`（那是 0..1）再直接 `{:.0}%` 打印——
/// 校验进度就踩过这个：44000 个单元扫到一半显示的是 "0%"，看着像卡死。
/// `total == 0` 返回 0，调用方自己判断要不要显示。
pub(crate) fn percent(done: usize, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    (done as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
}

/// 解析校验"枚举中"阶段的标签，认出 `core::verify::VerifyHandle::
/// set_enumerating` 写的 `enum 2/5 dirs, 13824 units`。
///
/// 返回 `(已扫目录数, 目录总数, 已收单元数)`，任何一段认不出来就是
/// `None` —— 调用方据此退回纯文案。**认不出必须安全失败**：这个标签走
/// 的是一把 `Mutex<String>`，格式万一变了，状态栏该退化成"正在校验"，
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
    // The dock sits inside the central panel, whose own fill is the veil
    // (`apply_theme_with_bg` puts `panel_fill` = veil while a wallpaper is
    // active). The dock must therefore stay *transparent* here: giving the
    // tab body or the tab bar its own veil would stack two translucencies
    // and darken the picture twice.
    if wallpaper {
        style.tab.tab_body.bg_fill = egui::Color32::TRANSPARENT;
        style.tab_bar.bg_fill = egui::Color32::TRANSPARENT;
        // the active tab still needs to stand out against the see-through
        // tab row, so it alone gets a more solid veil
        style.tab.active.bg_fill = if dark {
            egui::Color32::from_black_alpha(200)
        } else {
            egui::Color32::from_white_alpha(216)
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
        // 信息栏字体也管 dock 标签标题（用户 2026-10-07：信息栏字体 = 文档
        // 显示的字体、各种窗口的标题、以及里面显示的字体）
        egui::WidgetText::from(self.app.info_font.rich_text(name))
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
            || self.boot_verifying
            || self.verify_handle.is_some()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        // the debounced loads need one more frame once the window has elapsed
        if self.selection_settling() {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
        }

        crate::windows::draw_windows(self, ctx);
        // the backdrop first, on the lowest layer, so every panel (menu bar,
        // toolbar, status bar, central) paints its veil over it — see
        // `draw_background`
        self.draw_background(ctx);
        self.draw_menu(ctx);
        self.draw_toolbar(ctx);
        // the machine tree is a dock tab now (MainTab::Folders) — dragging the
        // splitter next to it resizes it, and its width lives in `dock_layout`
        egui::CentralPanel::default().show(ctx, |ui| {
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

/// 一个**只有图标、没有文字**的工具栏按钮。
///
/// 为什么要自己画而不是 `egui::Button::image()`：那条路走
/// `egui::load::Icon::Name`，依赖 egui 的图片加载器已经装好。**本项目只在
/// 部分路径装了 `install_image_loaders`**，没装的那条路上
/// `egui::load` 会**静默什么都不画** —— 按钮变成一个空白小块，看不出是
/// 搜索还是清除。走 [`icons::put`] 用的是自己的纹理表，任何情况下都一致。
///
/// 做法：`allocate_ui_with_layout` 拿到按钮大小的方块居中画图标，再用
/// `interact` 拿到点击。**按钮本身的边框/悬停底色照旧画**，否则它就不是
/// 按钮了（用户要的是"带图标的按钮"，不是裸图标）。
fn icon_button(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    icon: &str,
    tooltip: impl Into<String>,
    enabled: bool,
) -> egui::Response {
    let size = egui::vec2(24.0, 22.0);
    let sense = if enabled {
        egui::Sense::click()
    } else {
        // 仍然给 hover：用户要能看到 tip 才知道这个按钮是干什么的，
        // 只是点了没反应。
        egui::Sense::hover()
    };
    let (rect, mut resp) = ui.allocate_exact_size(size, sense);
    // **必须显式改 `Response.enabled` 字段**（egui 0.29 里它是字段不是方法）——
    // 只把 `Sense` 换成 `hover` 只是"不接收点击"，`enabled` 仍为 true，
    // 于是 `add_enabled(...)` 那类上层包装照样当它可点。画成灰色却还能点 =
    // 最坏的一种按钮（用户点了没反应，只会觉得程序卡了）。
    resp.enabled = enabled;
    if ui.is_rect_visible(rect) {
        // 与普通按钮同一套视觉（悬停/按下变色），只是内容换成图标。
        // egui 0.29 的入口是 `visuals().widgets.{inactive,hovered,active}`，
        // 圆角字段叫 `rounding`（不叫 `corner_radius`）。
        //
        // **先把三个分支的视觉都取出来再画** —— 直接持有 `&ui.visuals()` 的
        // 引用会让 `ui` 被不可变借用，而下面 `icons::put(ui, …)` 要可变借用，
        // 编译器会报E0502。所以先克隆（`WidgetVisuals: Clone`，四个字段都是
        // 廉价值），借用随之结束。
        let (v, stroke) = {
            let w = &ui.visuals().widgets;
            let v = if !enabled {
                &w.inactive
            } else if resp.is_pointer_button_down_on() {
                &w.active
            } else if resp.hovered() {
                &w.hovered
            } else {
                &w.inactive
            };
            (v.clone(), v.fg_stroke)
        };
        // 按钮底色用 `weak_bg_fill`（可透明），描边用 `bg_stroke`。
        // egui 0.29 的 `Painter::rect` 还是 4 参（**没有** 0.31 才有的
        // `StrokeKind`），描边宽度在 `Stroke` 里。
        if v.weak_bg_fill != egui::Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, v.rounding, v.weak_bg_fill);
        }
        ui.painter()
            .rect(rect, v.rounding, v.weak_bg_fill, v.bg_stroke);
        let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
        if !crate::icons::put(ui, ctx, icon, icon_rect) {
            // 纹理未解码（第一帧）或图标名写错：画一个描边方块。
            // **不能什么都不画** —— 空白按钮用户根本猜不出是搜索还是清除。
            ui.painter().rect_stroke(icon_rect, 2.0, stroke);
        }
    }
    resp.on_hover_text(tooltip.into())
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
            // （一次校验几十分钟，期间日志里就只剩它了）。
            self.save_settings_quiet();
        }
    }

    // ------------------------------------------------------------------
    // menu tree (origin mainwindow.ui)
    // ------------------------------------------------------------------

    /// The window backdrop, painted over the **whole window** on the
    /// `Order::Background` layer — below the menu bar, the toolbar, the
    /// status bar and the central panel alike.
    ///
    /// Every panel keeps its own brush on top (`panel_fill`), which
    /// `apply_theme_with_bg` turns into a translucent veil while a wallpaper
    /// is active, so the picture shows through the bars while their text
    /// stays fully opaque and readable. That is the 1.8.2 shape exactly:
    /// `setBgPixmap` put the pixmap on the *window's* background role, and
    /// `setTransparentBg` swapped the panel brushes for translucent ones.
    ///
    /// Why this works now when an earlier attempt failed: the failure was
    /// never the `Order::Background` layer itself — it was that the veil of
    /// that experiment sat on `window_fill`, the one brush the menu and
    /// popup frames also read, so the whole interface washed out. With the
    /// veil on `panel_fill` (and `window_fill` left opaque) the floating
    /// menus stay solid while every panel goes see-through.
    fn draw_background(&mut self, ctx: &egui::Context) {
        let Some(file) = self.background_file.clone() else {
            return;
        };
        let dir = self.bg_dir.clone();
        let mut tex = self.bg_tex.take();
        let handle = crate::app::load_background(&dir, &file, &mut tex, ctx);
        self.bg_tex = tex;
        let Some(tex) = handle else { return };

        let area = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
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
                    if ui.add_enabled(self.can_remove_from_folder(), button(label)).clicked() {
                        self.remove_from_folder();
                        ui.close_menu();
                    }
                    ui.separator();
                    self.verify_submenu(ui);
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
                        // first entry, ahead of the column list: whether the game
                        // list loads the per-machine icons at all. Off by default
                        // (user request 2026-10-07) — off also skips the icon file
                        // lookups while scrolling, and the driver-status square is
                        // drawn either way.
                        let si = self.tr("Show Icons");
                        ui.checkbox(&mut self.show_list_icons, si);
                        ui.separator();
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
        // plain "运行" — no game name suffix (user request 2026-10-07). The
        // PlayWith submenu (savestate/playback/record/command line) and the
        // Delete Cfg submenu were removed on the same request; double-click
        // still runs the selection.
        if ui.add_enabled(self.has_game(), button(self.tr("Play"))).clicked() {
            self.launch(RunMode::Normal, vec![]);
            ui.close_menu();
        }
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
    /// **这里只剩导出项**：单游戏校验 / 校验全部 ROM / 校验全部样本三个按钮
    /// 已按用户要求删掉——「刷新档案」(F5) 本来就做的是 re-verify + re-init，
    /// 重复入口只会让人以为这是两件事。
    pub fn verify_submenu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.tr("Export List"), |ui| {
            for (key, method) in [
                ("Export All Set Issues...", crate::core::verify::VerifyMethod::ExportAll),
                ("Export Incomplete Sets Only...", crate::core::verify::VerifyMethod::ExportIncomplete),
                ("Export Completely Missing Sets Only...", crate::core::verify::VerifyMethod::ExportMissing),
                ("Export All Sets...", crate::core::verify::VerifyMethod::ExportComplete),
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

    fn font_submenu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.tr("Font"), |ui| {
            ui.set_min_width(MENU_MIN_WIDTH);
            for size in [8.0f32, 16.0, 32.0, 64.0] {
                let label = format!("{}x{}", size as i32, size as i32);
                if ui.radio((self.list_icon_size - size).abs() < 0.1, label).clicked() {
                    self.list_icon_size = size;
                    self.save_settings();
                    ui.close_menu();
                }
            }
            ui.separator();
            if ui.button(self.tr("Game List Font")).clicked() {
                self.show_list_font_win = true;
                ui.close_menu();
            }
            if ui.button(self.tr("Info Panel Font")).clicked() {
                self.show_info_font_win = true;
                ui.close_menu();
            }
            if ui.button(self.tr("Category Font")).clicked() {
                self.show_folder_font_win = true;
                ui.close_menu();
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
                        let dir = self.bg_dir.clone();
                        let mut tex = self.bg_tex.take();
                        crate::app::load_background(&dir, &f, &mut tex, &ctx);
                        self.bg_tex = tex;
                        crate::app::apply_theme_with_bg(&ctx, self.dark_bg, true);
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

            // origin actionRefresh: re-verify + re-init. F5 is bound to the same
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

    /// 顶部工具栏：**高级搜索 · 过滤 · [搜索框] · 搜索 · 清除**
    ///
    /// 布局按用户 2026-06 的要求：搜索框**左侧**是高级搜索与过滤两个弹窗
    /// 入口，**右侧**是「搜索」与「清除」两个带图标的按钮，**没有运行按钮**
    /// （运行走 `has_game()` 的双击或 F5；工具栏那个 `▶ Play` 已按要求删除）。
    ///
    /// 「清除」原来是 `ui.small_button("✕")` 那个纯文字叉，现在换成图标
    /// 按钮，与搜索按钮成对。图标走 [`icons::draw_passive`]，它不抢 hover
    /// （`Sense` 全false），所以不会把右键菜单吃掉。
    fn draw_toolbar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // ---- 左：高级搜索（按列筛选）----
                //
                // **按钮上不显示 `(3/7)` 这样的分数**（用户 2026-10-06 要求）。
                // 分数要表达的是"当前搜哪几列"，而这件事按钮本身表达不了：
                // 它长得和平常一模一样，用户读到的是"高级搜索 (3/7)"，第一反应
                // 是"3/7 是什么意思"。要知道搜了几列，得先知道总列数——而总列数
                // 会随用户自己拖动列的显示/隐藏而变，这个分数在不同机器上还不
                // 一样（COL_LAST 是编译期常量，但 `col_visible` 是用户态的）。
                //
                // 分数真正有用的地方是弹窗里那一排勾选框，勾选状态一眼可见。
                // 工具栏只保留"非全选就高亮"这一个信号：按钮被按下 = 搜索范围
                // 被收窄过，全选时不高亮。
                let cols_on = self.search_cols.count_ones() as usize;
                if ui
                    .add(
                        egui::Button::new(self.tr("Advanced search"))
                            .selected(cols_on != COL_LAST),
                    )
                    .on_hover_text(self.tr("Choose which columns the search looks at"))
                    .clicked()
                {
                    self.show_advsearch_win = !self.show_advsearch_win;
                }

                // ---- 左：过滤（四个 hide 标志）----
                let any_filter = self.filter_flags != 0;
                let fb = self.tr("Filter");
                let fb = if any_filter {
                    format!("{fb} ({})", self.filter_flags.count_ones())
                } else {
                    fb.to_string()
                };
                if ui
                    .add(egui::Button::new(fb).selected(any_filter))
                    .on_hover_text(self.tr("Filter the game list"))
                    .clicked()
                {
                    self.show_filter_win = !self.show_filter_win;
                }

                ui.separator();

                // ---- 中：搜索框 ----
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
                // 回车即搜：文本框的 `changed()` 已经逐字符过滤了，回车只是把
                // 焦点状态收一收（键盘用户按完回车不会留着焦点高亮）。
                if resp.changed() || resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.search_changed();
                }

                // ---- 右：搜索 / 清除（带图标）----
                ui.add_space(4.0);
                if icon_button(ui, ctx, "16x16/system-search.png", self.tr("Search"), true).clicked() {
                    self.search_changed();
                }
                // 「清除」在没有搜索词时**画灰但仍可点**（点了等于再搜一次
                // 全部，语义上就是"回到无搜索"，不該禁掉——禁掉会让用户
                // 以为这个按钮坏了）。灰态靠 `enabled` 传下去。
                let has_text = !self.search.is_empty();
                if icon_button(ui, ctx, "16x16/clear.png", self.tr("Clear the search"), has_text)
                    .clicked()
                {
                    self.search.clear();
                    self.search_changed();
                }
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
    fn tree_row_band(
        ui: &mut egui::Ui,
        id: egui::Id,
        panel: egui::Rect,
        font: crate::app::UiFontPrefs,
    ) -> (egui::Rect, bool) {
        // the row box is the label's own height; adding the vertical item spacing
        // makes neighbouring bands meet, so the tint reads as one continuous line.
        // `size * 1.4` is the painted label height (row-height factor, taller for
        // CJK) — a band sized to the nominal font size left a gap under every
        // row and the hover tint looked one pixel short.
        let h = font.size * 1.4 + 4.0 + ui.spacing().item_spacing.y;
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
    fn tree_row_label(
        ui: &mut egui::Ui,
        selected: bool,
        text: &str,
        font: crate::app::UiFontPrefs,
    ) -> egui::Response {
        let v = ui.visuals();
        let color = if selected {
            v.selection.stroke.color
        } else {
            v.text_color()
        };
        ui.add(
            egui::Label::new(font.rich_text(text).color(color))
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
        font: crate::app::UiFontPrefs,
    ) -> (bool, bool, f32) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let (_band, row_hit) = Self::tree_row_band(ui, id.with("row"), panel, font);
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
            let label = Self::tree_row_label(ui, selected, text, font);
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
        font: crate::app::UiFontPrefs,
    ) -> bool {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let (_band, clicked) = Self::tree_row_band(ui, id, panel, font);
            icons::draw_passive(ui, &ctx, icons::FOLDER, FOLDER_ICON);
            Self::tree_row_label(ui, selected, text, font);
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
                    self.folder_font,
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
                let (arrow, row_hit, child_indent) = Self::folder_row(
                    ui,
                    id,
                    panel,
                    true,
                    state.openness(&ctx),
                    is_sel,
                    &text,
                    self.folder_font,
                );
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
                                self.folder_font,
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
                                self.folder_font,
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
        if Self::folder_child_row(ui, id, panel, selected, &child_label, self.folder_font) {
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
    // 单游戏校验（`can_verify` / `start_game_verify`）已于 2026-10-06 删除。
    // Rom 面板显示的就是 `verify_all` 的结果，要刷新按 F5。

    /// Rom 信息面板（View ▸ 自定义信息栏 ▸ RomInfo）。
    ///
    /// 数据来自**校验缓存**：`gamelist.cache` 里的每条 `RomInfo::available`。
    /// 所以切游戏立刻就有内容，不需要碰磁盘、不需要等 dat 文件。
    ///
    /// 整份视图按游戏名缓存（`rom_views`）：egui 每帧都调这个函数，而
    /// `rominfo::view_of` 要扫库（父集链 + 设备 + 样本）。校验结束时清空
    /// 缓存（`lib_verified` 的写入点都在那儿）。
    pub fn rom_info_content(&mut self, ui: &mut egui::Ui, game: &str) {
        if game.is_empty() || self.lib.is_none() {
            ui.weak(self.tr("Select a game to see its roms."));
            return;
        }
        if !self.rom_views.contains_key(game) {
            let view = {
                let Some(lib) = self.lib.clone() else { return };
                let guard = lib.lock().unwrap();
                // 缓存容量：用户快速点过 50 款游戏就该有 50 份视图，每份
                // 几 KB。上限比 dat 缓存小，因为一个游戏一份、且切回来看时
                // 大概率已经校验完了（要最新的可以按 F5 或右键重审）。
                //
                // 超限就**整体清空**，不做逐出记账：一个游戏一份、几 KB，
                // 64 份还超了说明用户在一轮校验前点了 64 款以上——而那轮校验
                // 一结束本来就要清空一次。为这点流量维护 LRU 链表不值得，
                // 而且逐出写错的表现是"面板偶发空白"，很难查。
                const VIEW_CACHE_CAP: usize = 64;
                if self.rom_views.len() >= VIEW_CACHE_CAP {
                    self.rom_views.clear();
                }
                crate::core::rominfo::view_of(&guard, game, self.lib_verified)
            };
            self.rom_views.insert(game.to_string(), view);
        }
        let Some(view) = self.rom_views.get(game).cloned() else { return };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 信息栏字体作用于整个 Rom 面板。rompanel 的列宽测量
                // （`text_w`/`measure_grid`）和渲染（`cell`/`state_with_icon`）
                // 都用 `FontSelection::default()` → `Style::font_id`，所以
                // 在入口设一次 `override_font_id`，两边同时跟着变，列宽不会
                // 因为字号变化而错位。
                ui.style_mut().override_font_id = Some(self.info_font.font_id());
                crate::rompanel::render(ui, self, &view)
            });
    }

    pub fn documents_content(&mut self, ui: &mut egui::Ui, tab: usize) {
        // `tab` is a document-tab index; everything below speaks `DOCK_*`,
        // where the document docks live at 7..12. Converting once here keeps
        // the lookup key, the cache key and the renderer in one index space.
        let dock = crate::core::dat::text_dock(tab);
        let game = self.current_game.clone();
        // 信息栏字体（View ▸ Font ▸ Info Panel Font）：文档正文用它排版；
        // 下面 Rom 信息面板走 `override_font_id`（见 `rom_info_content`）。
        let info_font = self.info_font;
        // Rom 信息面板**不走外部 dat**：它要的是校验结果，而校验结果躺在
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
                                        ui.label(info_font.rich_text(s));
                                    }
                                    crate::core::dat::Segment::Icon(n) => {
                                        // origin: convertCommand emits
                                        // `<img src=":/res/16x16/dir-N.png">` — prefer the
                                        // embedded PNG and fall back to a glyph only when
                                        // the file really is missing. Buttons always take
                                        // the glyph (see `notation_file`): the letter is
                                        // the payload, and matching the icon height keeps
                                        // mixed rows on one visual line.
                                        if !icons::notation_icon(ui, n, 16.0) {
                                            let (glyph, color) = icons::notation_glyph(n);
                                            ui.label(egui::RichText::new(glyph)
                                                .monospace()
                                                .strong()
                                                .size(16.0)
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
                            ui.label(info_font.rich_text(s).strong());
                            ui.add_space(2.0);
                        } else {
                            ui.label(info_font.rich_text(s));
                        }
                    }
                }
                Some(text) => {
                    ui.label(info_font.rich_text(&text));
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
                    let (done, total, cur) = if let Some(h) = &self.verify_handle {
                        h.snapshot()
                    } else if self.boot_verifying {
                        self.verify_stage.clone()
                    } else {
                        (0, 0, String::new())
                    };
                    // Verify progress: text only. The bar that used to sit here
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
                            self.tr("Verifying")
                        ));
                    } else if self.boot_verifying || self.verify_handle.is_some() {
                        // 枚举待扫单元的那一段：分母还不存在（`set_total` 排在全量`read_dir`
                        // 之后，见 `core/verify.rs`）。`cur` 形如
                        // `enum 2/5 dirs, 13824 units`。
                        //
                        // **文案就是"正在校验"**（用户 2026-06 定的）：枚举是
                        // 校验的第一阶段，不是另一件事，所以不另起一个名字。
                        // 之前这里显示"正在枚举"，而进度条一格不动，看着像
                        // 卡在别的什么地方。
                        //
                        // **一定要带上已读取的数量** —— 这阶段没有百分比，
                        // 数字才是"它在动"的证据。
                        let (dirs_done, dirs_total, units) = parse_enumerating(&cur);
                        let label = match (dirs_done, dirs_total, units) {
                            // 目录数已知：`正在校验 2/5 个目录 · 13824 个包`
                            (Some(d), Some(t), Some(u)) if t > 0 => format!(
                                "{} · {}",
                                self.tr("scanning {d}/{t}")
                                    .replace("{d}", &d.to_string())
                                    .replace("{t}", &t.to_string()),
                                self.tr("{} archives").replace("{}", &u.to_string()),
                            ),
                            // 只认出单元数（目录总数为 0 的退化情况）
                            (_, _, Some(u)) => {
                                self.tr("{} archives").replace("{}", &u.to_string())
                            }
                            // 认不出（空串、格式漂移）：宁可退回纯文案，
                            // 也不把原始英文标签甩给用户
                            _ => self.tr("scanning"),
                        };
                        ui.weak(label);
                    }
                    // No cancel affordance: the verify runs in the background and
                    // is not something the user should have to babysit. It is
                    // also not safe to abandon halfway — `VerifyDone` is what
                    // persists `audit_cache.bin`, so cancelling meant repeating
                    // the whole first scan next boot. `VerifyHandle::cancel` is
                    // kept for the headless examples, which do want a stop.
                    if self.lib_status == crate::app::LibStatus::Loading {
                        let (done, total) = self.lib_progress;
                        if total > 0 {
                            // 阶段二：分母是收输出时数出来的机种总数，真百分比。
                            // 封顶 99% —— 100% 留给"加载完成"，否则解析完还要等
                            // 校验，进度条会先顶到头再纹丝不动
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
    /// 两侧是对偶的：`core/verify.rs` 改格式而这里没跟上，状态栏就会
    /// 静默退化成"正在校验"（这正是它认不出时的行为，不报错）。
    #[test]
    fn parse_enumerating_reads_the_verify_label() {
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

#[cfg(test)]
mod toolbar_icon_tests {
    use super::*;

    /// **工具栏那几个图标名必须真的在资产表里。**
    ///
    /// 图标名写错是**运行期静默失败**：`icons::put` 查不到纹理就返回 false，
    /// 我画的那个描边方块顶上，编译期一个错都不报。而工具栏只有 22px 高，
    /// 截图标跟没截几乎看不出区别 —— 只能靠这条测试。
    ///
    /// 这条也解释了为什么 `lib.rs` 要 `pub mod icons`：探针和测试都要查这张表。
    #[test]
    fn the_toolbar_icons_exist_in_the_asset_table() {
        for name in [
            "16x16/system-search.png", // 搜索
            "16x16/clear.png",         // 清除
            "16x16/advanced.png",      // 高级搜索
            "16x16/status_missing.png",// 未拥有（Rom 面板用，同表）
        ] {
            assert!(
                crate::icons::ICONS.iter().any(|(k, _)| *k == name),
                "assets 里没有 {name} —— 图标会在运行期静默不显示"
            );
        }
    }

    /// **高级搜索按钮上不许再拼`(x/y)` 分数**（用户 2026-10-06 要求）。
    ///
    /// 分数回答不了用户的问题：按钮长得和平常一模一样，读到"高级搜索
    /// (3/7)"第一反应是"3/7 是什么意思"，而要知道分母是7 又得先知道游戏
    /// 列表有几列 —— 那个数还会随用户自己拖列而变。同样的信息在弹窗里
    /// 是一排勾选框，一眼可见，不需要在按钮上再压缩成一个分数。
    ///
    /// 这条钉住"标签就是纯文案"，并且提醒：**收窄状态改用按钮高亮表达**
    /// （`selected(cols_on != COL_LAST)`），别又把分数加回来。
    #[test]
    fn the_advanced_search_button_carries_no_fraction() {
        let src = include_str!("ui.rs");
        let body = src
            .split("fn draw_toolbar(")
            .nth(1)
            .expect("找不到 draw_toolbar");
        // 只看工具栏函数本体
        let body: String = {
            let mut out = String::new();
            for (i, line) in body.lines().enumerate() {
                if i > 0 && line == "}" {
                    break;
                }
                out.push_str(line);
                out.push('\n');
            }
            out
        };
        let code: String = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("{} ({}/{})") && !code.contains("({}/{})"),
            "高级搜索按钮不许再拼分数：\n{code}"
        );
        // 但"非全选要高亮"这个信号必须还在 —— 那是分数唯一的替代品
        assert!(
            code.contains("selected(cols_on != COL_LAST)"),
            "收窄状态改用按钮高亮表达，别把高亮一起删了：\n{code}"
        );
    }

    /// 图标按钮的 enabled 状态**真的关掉了点击**，不只是少收一次点击。
    ///
    /// `Sense::hover()` 只是"不接收点击"，`Response.enabled` 仍是 true，
    /// 上层`add_enabled(...)` 之类照样当它可点 —— 画成灰色却还能点是最坏的
    /// 一种按钮（用户点了没反应，只会觉得程序卡了）。
    ///
    /// 顺带钉住一个egui 的既有行为：**disabled 就不响应 hover**，所以
    /// 「清除」按钮在无搜索词时既点不动、也不出 tooltip。这里给出的是
    /// `Sense::click()`（与 egui 的 `Button` 一致），不是自创的"灰但有 tip"。
    #[test]
    fn a_disabled_icon_button_really_cannot_be_clicked() {
        let ctx = egui::Context::default();
        // `Context::run` 的返回值（FullOutput）必须用掉，否则一条 warning
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let off = icon_button(ui, ctx, "16x16/clear.png", "tip", false);
                assert!(!off.enabled(), "disabled 时 enabled 必须是 false");
                let on = icon_button(ui, ctx, "16x16/clear.png", "tip", true);
                assert!(on.enabled(), "enabled 时必须是 true");
            });
        });
    }
}
}
