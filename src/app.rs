//! Application state machine — mirrors MainWindow + Gamelist globals.

use crate::events::{AppEvent, ReadyPayload, SharedLib, SharedOpts};
use crate::core::folders::{FolderKind, FolderMaps};
use crate::core::mameproc::MameBinary;
use crate::core::settings::GuiSettings;
use mvui::dlog;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

pub const COL_DESC: usize = 0;
pub const COL_NAME: usize = 1;
pub const COL_ROM: usize = 2;
pub const COL_MFTR: usize = 3;
pub const COL_SRC: usize = 4;
pub const COL_YEAR: usize = 5;
pub const COL_CLONEOF: usize = 6;
pub const COL_LAST: usize = 7;
pub const COLUMN_TITLES: [&str; COL_LAST] = [
    "Description", "Name", "ROMs", "Manufacturer", "Driver", "Year", "Clone of",
];

/// Origin: the default `column_state` shipped in `res/the original GUI ini` of
/// the original GUI (1.8.2) (Description 242, Name 100, ROMs 36, Manufacturer 142,
/// Driver 67, Year 33, Clone of 58). Kept so a fresh install looks like the
/// original instead of collapsing to egui's generic 100px suggestion.
pub const COL_DEFAULT_WIDTH: [f32; COL_LAST] = [242.0, 100.0, 36.0, 142.0, 67.0, 33.0, 58.0];

/// Lower bound for the drag-to-resize handles.
pub const COL_MIN_WIDTH: [f32; COL_LAST] = [160.0, 70.0, 32.0, 80.0, 50.0, 34.0, 50.0];

pub const F_CLONES: u16 = 0x0001;
pub const F_NONWORKING: u16 = 0x0002;
pub const F_UNAVAILABLE: u16 = 0x0004;
pub const F_MECHANICAL: u16 = 0x4000;

/// `boot.log` 的文件名，落在 `<配置根>/cache/` 底下。
///
/// 只被 debug 构建的写盘函数用到，所以跟着 `#[cfg]` 走——否则 release
/// 下它是一个没人引用的常量，一条 dead_code 警告。
#[cfg(debug_assertions)]
const LOG_FILE_NAME: &str = "boot.log";

/// 超过这个大小就把日志截断（README P3：逐帧日志曾让它无限增长）。
#[cfg(debug_assertions)]
const PERF_LOG_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// 截断后保留的尾部大小。
#[cfg(debug_assertions)]
const PERF_LOG_KEEP_BYTES: u64 = 1024 * 1024;

#[derive(PartialEq, Clone, Copy)]
pub enum LibStatus {
    Idle,
    Loading,
    Ready,
    Error,
}

#[derive(PartialEq, Clone, Copy)]
pub enum ListMode {
    Details,
    Grouped,
}

impl ListMode {
    pub fn key(self) -> &'static str {
        match self {
            ListMode::Details => "Details",
            ListMode::Grouped => "Grouped",
        }
    }
    pub fn from_key(s: &str) -> Self {
        match s {
            "Grouped" => ListMode::Grouped,
            _ => ListMode::Details,
        }
    }
}

/// PlayWith dialog kinds (origin: playoptions dialogs)
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum PlayKind {
    Savestate,
    Playback,
    Record,
    Mng,
    Avi,
    Wave,
}

impl PlayKind {
    pub fn title(self) -> &'static str {
        match self {
            PlayKind::Savestate => "Load Savestate",
            PlayKind::Playback => "Playback Input",
            PlayKind::Record => "Record Input",
            PlayKind::Mng => "Record MNG Output",
            PlayKind::Avi => "Record AVI Output",
            PlayKind::Wave => "Record Wave Output",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            PlayKind::Savestate => "sta",
            PlayKind::Playback | PlayKind::Record => "inp",
            PlayKind::Mng => "mng",
            PlayKind::Avi => "avi",
            PlayKind::Wave => "wav",
        }
    }
}

pub struct MameApp {
    /// the open game-list context menu: (anchor position, row it acts on).
    ///
    /// Built as a plain `Area` at the pointer rather than through
    /// `Response::context_menu`: that one only draws while it is called, and it
    /// insists on being handed the very response that was clicked, which our
    /// build (the menu needs `&mut self`, so it is assembled after the table pass)
    /// cannot provide.
    pub ctx_menu: Option<(egui::Pos2, usize)>,
    pub ctx_handle: Option<egui::Context>,
    pub lang: String,
    pub gui: GuiSettings,
    pub events_tx: Sender<AppEvent>,
    pub events_rx: Receiver<AppEvent>,

    pub mame: Option<MameBinary>,
    pub is_mess: bool,
    pub is_ume: bool,
    pub started: bool,
    /// the light/dark `Visuals` are not serialized, so they are applied once
    /// on the first frame from `dark_bg` (+ whether a background is selected)
    pub theme_applied: bool,
    pub need_mame_pick: bool,
    pub picking: bool,

    pub lib: Option<SharedLib>,
    pub opts: Option<SharedOpts>,
    pub maps: FolderMaps,
    pub folder_cache: Option<Arc<crate::core::folders::FolderCache>>,
    pub lib_status: LibStatus,
    /// `-listxml` 的进度：`(已处理, 总数)`。`total == 0` = 还在收输出。
    pub lib_progress: (usize, usize),

    // folder/filter state (origin: currentFolder/currentGame/hiddenFolders/filterFlags)
    pub folder_kind: FolderKind,
    pub folder_key: Option<String>,
    pub current_folder: String,
    pub hidden_folders: Vec<String>,
    pub ext_folder_data: Vec<(String, crate::core::folders::ExtFolderStore)>,
    pub filter_flags: u16,
    pub search: String,
    pub search_take_focus: bool,
    /// 高级搜索：**搜索要作用于哪些列**（位掩码，下标是 `COL_*`）。
    ///
    /// 默认**所有列都勾选**（`[true; COL_LAST]`）—— 与旧行为一致：搜索框
    /// 一直是"name + description"的全文搜。收窄只发生在用户自己在「高级搜索」
    /// 弹窗里取消勾选之后。
    ///
    /// 存掩码而不是 `Vec<bool>`：`COL_LAST` 是编译期常量 7，位掩码可以直接
    /// `u8` 存进 ini，且"全选 = !0"这个初值不必逐位填。
    pub search_cols: u8,
    /// 「高级搜索」弹窗是否打开（工具栏左侧那个放大镜+清单图标）。
    pub show_advsearch_win: bool,
    /// filtered view. `Rc` so the 46k-entry index can be handed to the table
    /// without a per-frame deep clone (README P2-23)
    pub visible: std::rc::Rc<Vec<usize>>,
    pub needs_refilter: bool,
    pub selected: Option<usize>,
    pub current_game: String,

    // view state (origin: list_mode/sort/columns/zoom/docks)
    pub list_mode: ListMode,
    pub sort_column: usize,
    pub sort_reverse: bool,
    pub col_visible: [bool; COL_LAST],
    /// user-draggable column order (indices into COLUMN_TITLES)
    pub col_order: [usize; COL_LAST],
    /// live column widths, measured from the header cells every frame.
    ///
    /// These have to live in the app state rather than in egui temp data: the
    /// table id contains the column order, so a reorder rebuilds egui_extras'
    /// state — and anything the rebuild reads has to be the width that belongs to
    /// the *column*, not to the slot it used to sit in.
    pub col_widths: [f32; COL_LAST],
    /// in-flight header drag as `(from slot, target slot, has left the source)`.
    ///
    /// It has to be remembered here: a press/release pair spans several frames,
    /// and egui clears `press_origin` on release, so the column the drag started
    /// on is gone by the time we learn where it ended.
    pub header_drag: Option<(usize, usize, bool)>,
    /// pointer x where the current header drag began — the floating header
    /// ghost offsets by `pointer.x - header_drag_x` so it tracks the pointer
    /// the way MvUI/1.8.2 does
    pub header_drag_x: f32,
    /// in-flight **column-width** drag: `(column, pointer x at press, width at
    /// press)`.
    ///
    /// egui_extras' own resize handle is a line that runs from the top of the
    /// *body* to its bottom, so the game rows could be dragged sideways to resize
    /// a column while the header — the only place 1.8.2 lets you do it (a
    /// `QHeaderView` separator) — could not. Resizing is therefore done by hand
    /// here, against the header geometry, and the column list is rebuilt with a
    /// fresh salt so the new width is the one egui_extras lays out.
    pub header_resize: Option<(usize, f32, f32)>,
    /// description width once the user has dragged the Description|Name boundary;
    /// `None` = auto (the column always takes up the leftover space)
    pub col_desc_width: Option<f32>,
    /// bumped on a double-click header reset so the table state (and with it the
    /// restored default column widths) is rebuilt
    pub col_reset_salt: u32,
    /// thin separators between list columns and under each row. Default on —
    /// the striped band alone does not read as a grid once columns are moved
    /// around (user request 2026-10-03).
    pub show_grid: bool,
    pub enforce_aspect: bool,
    pub stretch_sshot: bool,
    pub local_game_list: bool,
    pub dark_bg: bool,
    /// View ▸ Font: the global UI scale. egui has no per-widget "font size"
    /// beyond its text styles, but `Context::set_zoom_factor` scales text,
    /// spacing and hit targets together, which is what a font-size menu has to
    /// mean if the rest of the interface is to stay legible. 1.0 = unscaled.
    pub font_zoom: f32,
    pub image_dock_visible: [bool; 7],
    pub image_dock_tab: usize,
    pub text_dock_visible: [bool; crate::core::dat::TEXT_DOCK_COUNT],
    pub show_folder_dock: bool,
    pub dock_state: egui_dock::DockState<crate::ui::MainTab>,

    pub snap_tex: HashMap<usize, (String, Option<egui::TextureHandle>)>,
    pub snap_requested: HashSet<usize>,
    /// Machine icons (README §6.1, layer ③), keyed by set name. `None` is a
    /// remembered miss: without it a row that has no icon would ask again on
    /// every frame.
    pub game_icons: HashMap<String, Option<egui::TextureHandle>>,
    /// age order of the icon cache, oldest first
    pub icon_lru: VecDeque<String>,
    pub icon_requested: HashSet<String>,
    /// roms unpacked out of a .7z into %TEMP% for a launch, keyed by game so
    /// they can be removed once that game exits (README P2-13)
    pub temp_roms: Vec<(String, std::path::PathBuf)>,
    pub dat_texts: HashMap<(usize, String), Option<String>>,
    pub dat_requested: HashSet<(usize, String)>,
    /// Parsed Command/History tab render trees, keyed by (dock, game). The
    /// parse is regex work over up to 4000 lines and egui renders at refresh
    /// rate, so the result is memoized (see `ui::MameApp::cached_lines`)
    /// instead of being recomputed every frame.
    pub doc_parsed: HashMap<(usize, String), Arc<Vec<crate::core::dat::DatLine>>>,

    // ---- Rom 信息（面板 + 单游戏校验）--------------------------------
    /// 缓存的 Rom 信息视图，键是游戏名。
    ///
    /// egui 在**每帧**调这个渲染器，而 `rominfo::view_of` 要扫库（父集链、
    /// 设备、样本），5 万台游戏的库上一帧 60 次是白烧 CPU。校验结果只在一轮
    /// 校验结束时变一次，所以按游戏名缓存、校验完清空。
    pub rom_views: HashMap<String, crate::core::rominfo::RomInfoView>,
    /// 全库校验过没有（= `gamelist.cache` 的 `verified` 标志）。
    ///
    /// false 时 `rominfo::view_of` 把所有缺失降级成"未校验"——冷启动后校验还
    /// 没跑，此时 `available` 全是默认值，一律显示"缺失"会让用户以为自己的
    /// 盘是空的。
    pub lib_verified: bool,
    // 单游戏校验的三个字段（`game_verify` / `game_verify_target` /
    // `game_verify_result`）与配套的 `GameVerifyDone` 事件已于 2026-06
    // 一并删除。Rom 面板显示的就是 `verify_all` 的结果，要刷新按 F5。

    // dialogs
    pub show_options_win: Option<usize>,
    pub opt_level: usize,
    pub opt_category: String,
    pub opt_edits: HashMap<String, String>,
    pub show_dirs_win: bool,
    pub dirs_buf: String,
    pub show_about: bool,
    /// the game-list filter popup, opened from the toolbar button left of the
    /// search box (the flags themselves are `filter_flags`)
    pub show_filter_win: bool,
    pub show_cmd: bool,
    pub cmd_text: String,
    pub play_dialog: Option<(PlayKind, String)>,
    pub show_verify: bool,
    pub verify_lines: Vec<String>,
    pub verify_handle: Option<Arc<crate::core::verify::VerifyHandle>>,
    pub mounted: HashMap<(String, String), String>,
    pub dirs_target_option: Option<String>,
    /// Working copy of every path in Settings ▸ Directories, keyed by setting
    /// name. Empty string means "use the built-in default", which is what the
    /// dialog shows as a placeholder rather than as a path — that way the user
    /// can always see what the default *is* without having to remember it.
    pub dir_edits: std::collections::BTreeMap<String, String>,
    pub show_csv_win: Option<String>,
    pub exporting_method: Option<crate::core::verify::VerifyMethod>,
    pub export_target: Option<PathBuf>,

    /// §12 of the design doc: preview/DAT/icon loads are debounced against a
    /// settling selection, so dragging through the list does not spawn a load
    /// job for every machine the cursor passes.
    pub sel_changed_at: Option<std::time::Instant>,
    /// last name `sel_changed_at` was stamped for — the change detector lives in
    /// the frame loop, which is the one place every selection path passes through
    pub published_game: String,
    pub progress_open: bool,
    /// verify running as part of first boot (progress via `VerifyProgress`)
    pub boot_verifying: bool,
    /// 冷启动那次校验的进度：`(已扫, 总数, 当前系统)`。
    pub verify_stage: (usize, usize, String),
    pub frame_count: u32,
    /// window background: `None` = the flat theme colour, `Some(file)` = an
    /// image from the `background_directory` (origin: 1.8.2 `background_file` +
    /// the `bgActions` group built out of the directory listing)
    pub background_file: Option<String>,
    /// resolved `background_directory` (origin: `optiontemplate.xml:12`,
    /// default `bkground`, relative to the mame directory). Cached because it
    /// depends on the mame binary, which is only known once it is accepted —
    /// rescanned then, so a picture that appears later is picked up.
    pub bg_dir: PathBuf,
    /// every image file found in `bg_dir`, sorted by name. Rescanned when the
    /// mame binary is accepted and whenever the menu is opened; the menu lists
    /// these as radio items.
    pub bg_choices: Vec<String>,
    /// decoded background, cached by file name — re-decoding a multi-MB PNG on
    /// every frame would be wasteful, and the menu rebuilds the layer often
    pub bg_tex: Option<(String, egui::TextureHandle)>,
    /// tile instead of stretch. The two are mutually exclusive (origin: the
    /// `bgStretchActions` QActionGroup holding Stretch / Tile).
    pub bg_stretch: bool,
    pub running: HashSet<String>,
    pub log_lines: Vec<String>,
    pub last_error: Option<String>,
    pub wants_close: bool,
}

impl MameApp {
    pub fn new(
        cc: &eframe::CreationContext,
        tx: Sender<AppEvent>,
        rx: Receiver<AppEvent>,
    ) -> Self {
        let gui = GuiSettings::load();
        // default language: Simplified Chinese; a saved value outside the
        // three offered languages (e.g. an old "ja_JP") is normalized
        let lang = crate::i18n::normalize(
            gui.get("language").unwrap_or(crate::i18n::DEFAULT_LANG),
        ).to_string();
        let list_mode = ListMode::from_key(gui.get_or("list_mode", "Details"));
        let sort_column = gui
            .get("sort_column")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let sort_reverse = gui.get_bool("sort_reverse");
        let hidden_folders = gui
            .get("hide_folders")
            .map(|s| s.split(';').map(str::to_string).collect())
            .unwrap_or_else(|| vec!["All Games".to_string()]);
        let filter_flags = gui
            .get("folder_flag")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // the saved choice, defaulting to dark as before
        let dark_bg = !gui.get("dark_bg").map(|v| v == "0").unwrap_or(false);
        // View ▸ Font. Clamped to a sane band: egui's zoom multiplies every
        // spacing and row height, so a stored 0.0 or 6.0 would leave an
        // unusable window, and the value is user-editable in the settings file.
        let font_zoom = gui
            .get("font_zoom")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite() && *v >= 0.75 && *v <= 2.0)
            .unwrap_or(1.0);
        // `bg_tile` is gone: tile and stretch are one exclusive choice now, and
        // the old key defaulted to *stretch* (1.8.2 wrote
        // `background_stretch = actionBgTile->isChecked() ? 0 : 1`)
        let bg_stretch = gui.get("bg_stretch").map(|v| v != "0").unwrap_or(true);
        let background_file = gui
            .get("background_file")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let bg_dir = crate::core::paths::background_dir(
            gui.get("background_directory").map(str::trim).filter(|s| !s.is_empty()),
        );
        let bg_choices = scan_backgrounds(&bg_dir);
        let mut col_visible = [true; COL_LAST];
        if let Some(csv) = gui.get("columns_visible") {
            for (i, v) in csv.split(';').enumerate().take(COL_LAST) {
                col_visible[i] = v == "1";
            }
        }
        let mut image_dock_visible = [true; 7];
        if let Some(csv) = gui.get("image_docks") {
            for (i, v) in csv.split(';').enumerate().take(7) {
                image_dock_visible[i] = v == "1";
            }
        }
// 文本面板（含第 6 个 Rom 信息）默认**全开**。用户要过 Rom 信息
        // 默认勾选：它是校验结果的直接视图，面板本身只读缓存、不占后台线程，
        // 没有理由藏着。ini 里存过的开关照旧优先（用户手动关掉就尊重它）。
        let mut text_dock_visible = [true; crate::core::dat::TEXT_DOCK_COUNT];
        if let Some(csv) = gui.get("text_docks") {
            // 必须按 `TEXT_DOCK_COUNT` 截断而不是写死 5：写死的话，用户开过
            // Rom 信息面板 → 存盘 6 段 → 下次启动又被截掉，开关根本存不住。
            for (i, v) in csv.split(';').enumerate().take(crate::core::dat::TEXT_DOCK_COUNT) {
                text_dock_visible[i] = v == "1";
            }
        }

        // column order: stored as "0;1;2;…"; anything invalid falls back
        let mut col_order = [0usize; COL_LAST];
        for (i, v) in col_order.iter_mut().enumerate() {
            *v = i;
        }
        if let Some(csv) = gui.get("column_order") {
            let mut seen = [false; COL_LAST];
            let mut ok = true;
            for (slot, s) in csv.split(';').enumerate() {
                if slot >= COL_LAST {
                    ok = false;
                    break;
                }
                let v: usize = match s.trim().parse::<usize>() {
                    Ok(v) if v < COL_LAST && !seen[v] => v,
                    _ => {
                        ok = false;
                        break;
                    }
                };
                seen[v] = true;
                col_order[slot] = v;
            }
            if !ok {
                for (i, v) in col_order.iter_mut().enumerate() {
                    *v = i;
                }
            }
        }
        let dock_layout = gui.get("dock_layout").unwrap_or("").to_string();
        // description width: empty/absent = auto (fill the leftover space)
        let col_desc_width = gui
            .get("column_desc_width")
            .and_then(|s| s.trim().parse::<f32>().ok())
            .filter(|w| w.is_finite() && *w > 0.0);
        // default on: absent key means "never toggled"
        let show_grid = gui.get("show_grid").map(|v| v != "0").unwrap_or(true);
        let enforce_aspect = gui.get_bool("enforce_aspect");
        let stretch_sshot = gui.get_bool("stretch_screenshot_larger");
        let local_game_list = gui.get_bool("local_game_list");
        let app = Self {
            // the context exists from the start, so background threads spawned
            // before the first frame can already request repaints (ctx() used to
            // expect() a context that was only captured in `update`)
            ctx_menu: None,
            ctx_handle: Some(cc.egui_ctx.clone()),
            lang,
            gui: gui,
            events_tx: tx,
            events_rx: rx,
            mame: None,
            is_mess: false,
            is_ume: false,
            started: false,
            theme_applied: false,
            need_mame_pick: false,
            picking: false,
            lib: None,
            opts: None,
            maps: FolderMaps::default(),
            folder_cache: None,
            lib_status: LibStatus::Idle,
            lib_progress: (0, 0),
            folder_kind: FolderKind::AllArc,
            folder_key: None,
            current_folder: String::new(),
            hidden_folders,
            ext_folder_data: Vec::new(),
            filter_flags,
            search: String::new(),
            search_take_focus: false,
            // 高级搜索默认全选（=旧行为：全文搜 name + description）。
            // 用 `all_search_cols()` 而不是 `u8::MAX`：后者有 8 位，比 7 列
            // 多一位，工具栏按钮上的 `(n/7)` 会显示成 8/7。
            search_cols: crate::views::all_search_cols(),
            show_advsearch_win: false,
            visible: std::rc::Rc::new(Vec::new()),
            needs_refilter: true,
            selected: None,
            current_game: String::new(),
            list_mode,
            sort_column,
            sort_reverse,
            col_visible,
            col_order,
            col_desc_width,
            col_widths: COL_DEFAULT_WIDTH,
            header_drag: None,
            header_drag_x: 0.0,
            header_resize: None,
            col_reset_salt: 0,
            show_grid,
            enforce_aspect,
            stretch_sshot,
            local_game_list,
            dark_bg,
            font_zoom,
            bg_stretch,
            background_file,
            bg_dir,
            bg_choices,
            bg_tex: None,
            image_dock_visible,
            image_dock_tab: 0,
            text_dock_visible,
            show_folder_dock: true,
            dock_state: crate::ui::restore_docks(
                &dock_layout,
                &image_dock_visible,
                &text_dock_visible,
                true,
            )
            .unwrap_or_else(|| {
                crate::ui::default_docks_filtered(&image_dock_visible, &text_dock_visible, true)
            }),
            snap_tex: HashMap::new(),
            snap_requested: HashSet::new(),
            game_icons: HashMap::new(),
            icon_lru: VecDeque::new(),
            icon_requested: HashSet::new(),
            temp_roms: Vec::new(),
            dat_texts: HashMap::new(),
            dat_requested: HashSet::new(),
            doc_parsed: HashMap::new(),
            rom_views: HashMap::new(),
            // 引导流程会按缓存里的 `verified` 标志纠正它（`LibraryReady`）
            lib_verified: false,
            show_options_win: None,
            opt_level: 1,
            opt_category: "Core Video".into(),
            opt_edits: HashMap::new(),
            show_dirs_win: false,
            dirs_buf: String::new(),
            show_about: false,
            show_filter_win: false,
            show_cmd: false,
            cmd_text: String::new(),
            play_dialog: None,
            show_verify: false,
            verify_lines: Vec::new(),
            verify_handle: None,
            exporting_method: None,
            export_target: None,
            mounted: HashMap::new(),
            dirs_target_option: None,
            dir_edits: std::collections::BTreeMap::new(),
            show_csv_win: None,
            sel_changed_at: None,
            published_game: String::new(),
            progress_open: false,
            boot_verifying: false,
            verify_stage: (0, 0, String::new()),
            frame_count: 0,
            running: HashSet::new(),
            log_lines: Vec::new(),
            last_error: None,
            wants_close: false,
        };
        // fonts are installed by `main` before the app is built — installing here
        // as well read every ttc twice and rebuilt the font atlas for nothing
        app
    }

    /// translate + substitute one `{}` placeholder
    pub fn tf(&self, key: &str, arg: impl std::fmt::Display) -> String {
        self.tr(key).replacen("{}", &arg.to_string(), 1)
    }

    pub fn open_properties(&mut self, level: usize) {
        self.show_options_win = Some(level);
        self.opt_level = level;
        self.opt_edits.clear();
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        self.log_lines.push(msg.into());
        if self.log_lines.len() > 2000 {
            self.log_lines.remove(0);
        }
    }

    pub fn poplog(&mut self, msg: impl Into<String>) {
        self.last_error = Some(msg.into());
    }

    /// Remove the temp roms unpacked for a launch. `None` = every game (used on
    /// exit). They live in %TEMP% under their rom name and used to be left behind
    /// for good (README P2-13).
    pub fn cleanup_temp_roms(&mut self, game: Option<&str>) {
        let mut keep: Vec<(String, std::path::PathBuf)> = Vec::new();
        for (g, p) in std::mem::take(&mut self.temp_roms) {
            let wanted = game.map(|want| want == g).unwrap_or(true);
            if wanted {
                let _ = std::fs::remove_file(&p);
            } else {
                keep.push((g, p));
            }
        }
        self.temp_roms = keep;
    }

    /// DAT texts are whole documents (history.dat entries run to hundreds of KB),
    /// and the maps used to grow for the entire session. Past the cap, keep only
    /// the game on screen — anything else is reloaded on demand (README P2-17).
    pub fn trim_dat_cache(&mut self) {
        const DAT_CACHE_CAP: usize = 200;
        if self.dat_texts.len() <= DAT_CACHE_CAP {
            return;
        }
        // Drop the oldest half instead of everything but the current game: the
        // cliff clear was correct but left the whole cache cold after one switch
        // (README P3). The current game's entries are always kept.
        let cur = self.current_game.clone();
        let mut keys: Vec<(usize, String)> = self.dat_texts.keys().cloned().collect();
        keys.sort();
        let half = keys.len() / 2;
        let doomed: Vec<(usize, String)> = keys
            .into_iter()
            .filter(|k| k.1 != cur)
            .take(half)
            .collect();
        for k in doomed {
            self.dat_texts.remove(&k);
            // a dropped entry must also lose its "requested" mark, or the dock
            // would never ask for it again
            self.dat_requested.remove(&k);
        }
    }

    // ---- machine icons (README §6.1 layer ③) ----

    /// Icon of `game`, when it is cached. `None` means either "not loaded yet"
    /// or "known to have no icon" — the caller distinguishes them with
    /// `icon_requested`, exactly like the preview docks do.
    pub fn game_icon(&self, game: &str) -> Option<egui::TextureHandle> {
        self.game_icons.get(game).cloned().flatten()
    }

    /// Debounce window for everything that loads on selection (design doc §12).
    pub const SELECT_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(150);

    /// True while the selection has only just changed: load jobs wait, so a fast
    /// scroll does not queue one per row it passes.
    pub fn selection_settling(&self) -> bool {
        self.sel_changed_at
            .map(|t| t.elapsed() < Self::SELECT_DEBOUNCE)
            .unwrap_or(false)
    }

    /// Is there anything left to ask for? False once an icon is cached *or* the
    /// machine is known to have none — a miss is a result too, and re-asking for
    /// it on every frame would flood the worker with stat() calls.
    pub fn icon_needs_request(&self, game: &str) -> bool {
        !self.game_icons.contains_key(game) && !self.icon_requested.contains(game)
    }

    /// Ask a worker for `game`'s icon, trying `fallbacks` (parent set / host
    /// machine) when the machine has none of its own.
    pub fn request_game_icon(&mut self, game: String, fallbacks: Vec<String>) {
        if game.is_empty() || self.icon_requested.contains(&game) {
            return;
        }
        // debounced like the preview docks: scrolling a 46k list would otherwise
        // queue one icon job per row the cursor crosses
        if self.selection_settling() {
            return;
        }
        let dirs = self.opt_resolved_dirs("icons_directory");
        if dirs.is_empty() {
            return;
        }
        self.icon_requested.insert(game.clone());
        crate::background::load_icon(dirs, game, fallbacks, self.events_tx.clone(), self.ctx());
    }

    pub fn insert_game_icon(&mut self, game: String, tex: Option<egui::TextureHandle>) {
        self.game_icons.insert(game.clone(), tex);
        self.icon_lru.retain(|g| g != &game);
        self.icon_lru.push_back(game);
        // The pack holds one icon per machine — tens of thousands of them — so
        // the cache is bounded and drops the least recently used entries.
        const ICON_CACHE_MAX: usize = 512;
        while self.game_icons.len() > ICON_CACHE_MAX {
            let Some(oldest) = self.icon_lru.pop_front() else { break };
            self.game_icons.remove(&oldest);
        }
    }

    pub fn stash_ctx(&mut self, ctx: &egui::Context) {
        if self.ctx_handle.is_none() {
            self.ctx_handle = Some(ctx.clone());
        }
    }

    pub fn ctx(&self) -> egui::Context {
        self.ctx_handle.as_ref().expect("ctx").clone()
    }

    pub fn save_settings(&mut self) {
        self.write_settings(true)
    }

    /// 周期性保存用（见 `ui::save_settings_periodic`）。
    ///
    /// 写盘的内容与 [`save_settings`] 完全一样，只是不记账：那条定时器
    /// 每 200 帧跑一次，是防崩溃丢设置的兜底而不是用户动作，每次往
    /// boot.log 里塞一条"保存 N 条"只会稀释真正值得看的信息——而且
    /// 一次校验动辄几十分钟，那段时间里它就是日志里的全部内容。
    pub fn save_settings_quiet(&mut self) {
        self.write_settings(false)
    }

    fn write_settings(&mut self, log_it: bool) {
        // `save()` writes the whole map, so a key that is *absent* from it stays
        // absent on disk for good. That is how `mame_binary` was lost: one run
        // started without it in the map (no ini, or an unreadable one), the
        // periodic save then rewrote the file from that map, and every later
        // start read back an ini with no `mame_binary` — fell back to the legacy
        // `mamep.exe`, failed validation and opened the picker again. The user
        // sees a program that "forgets" the MAME path on every launch.
        //
        // `self.mame` is the authority on a *validated* binary, so write it back
        // when the map has dropped it. Doing this here fixes the loop at the
        // write side, which covers every caller of `save_settings`. The rule
        // itself lives in `settings::ensure_mame_binary` so it can be tested
        // without a disk-backed settings table.
        let validated = self.mame.as_ref().map(|m| m.path.to_string_lossy().to_string());
        crate::core::settings::ensure_mame_binary(&mut self.gui.map, validated.as_deref());
        self.gui.set("list_mode", self.list_mode.key());
        self.gui.set_bool("sort_reverse", self.sort_reverse);
        self.gui.set("sort_column", self.sort_column.to_string());
        self.gui.set_bool("show_grid", self.show_grid);
        self.gui.set_bool("enforce_aspect", self.enforce_aspect);
        self.gui.set_bool("stretch_screenshot_larger", self.stretch_sshot);
        self.gui.set_bool("local_game_list", self.local_game_list);
        self.gui.set("hide_folders", self.hidden_folders.join(";"));
        self.gui.set("folder_flag", self.filter_flags.to_string());
        self.gui.set(
            "columns_visible",
            self.col_visible.iter().map(|&v| if v { "1" } else { "0" }.to_string()).collect::<Vec<_>>().join(";"),
        );
        self.gui.set(
            "column_order",
            self.col_order.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(";"),
        );
        // empty = the description re-fills whatever the window leaves over
        self.gui.set(
            "column_desc_width",
            self.col_desc_width.map_or(String::new(), |w| format!("{w:.0}")),
        );
        self.gui.set(
            "image_docks",
            self.image_dock_visible.iter().map(|&v| if v { "1" } else { "0" }.to_string()).collect::<Vec<_>>().join(";"),
        );
        self.gui.set(
            "text_docks",
            self.text_dock_visible.iter().map(|&v| if v { "1" } else { "0" }.to_string()).collect::<Vec<_>>().join(";"),
        );
        // persist the dock tree so dragged layout survives a restart
        if let Ok(json) = serde_json::to_string(&self.dock_state) {
            self.gui.set("dock_layout", json);
        }
        self.gui.set("default_folder", self.current_folder.clone());
        self.gui.set("default_game", self.current_game.clone());
        self.gui.set("language", self.lang.clone());
        // §11/§13: the theme and background mode are GUI state, so they belong
        // in the settings file — they used to be switchable but never persisted,
        // so every session started dark again
        self.gui.set_bool("dark_bg", self.dark_bg);
        self.gui.set("font_zoom", format!("{:.2}", self.font_zoom));
        self.gui.set_bool("bg_stretch", self.bg_stretch);
        self.gui.set(
            "background_file",
            self.background_file.clone().unwrap_or_default(),
        );
        let saved = if log_it {
            self.gui.save()
        } else {
            self.gui.save_quiet()
        };
        if let Err(e) = saved {
            // surfaced instead of silently dropping the settings file
            self.log(format!("settings save failed: {e}"));
        }
    }

    /// origin: MainWindow::init → validateMameBinary + dat load decision.
    /// The file dialog runs on its own thread so the UI never blocks.
    pub fn startup(&mut self) {
        self.started = true;
        // drop whatever a previous session (or a crash) left in the temp-rom dir
        crate::core::archive::clear_temp_rom_dir();
        // 这些曾经是 `eprintln!`，在 release 构建里是死路：`main.rs` 设了
        // `windows_subsystem = "windows"`，进程没有控制台，stderr 永远看不到。
        // 改道 `dlog!`：debug 构建落到 `.mvui/cache/boot.log`，release 下
        // 整句被编译期常量消掉。
        dlog!("启动: 开始校验 MAME 主程序");
        let path = self
            .gui
            .get("mame_binary")
            .unwrap_or("mamep.exe")
            .to_string();
        let version = detect_version(&path);
        dlog!("启动: 探测版本 {:?} 路径 {}", version, path);
        if self.try_accept_mame(&path, &version) {
            dlog!("启动: MAME 可用（{}），开始引导", version);
            self.boot();
        } else {
            dlog!("启动: MAME 不可用，打开文件选择框");
            self.open_mame_picker();
        }
        dlog!("启动: 校验结束");
    }

    fn try_accept_mame(&mut self, path: &str, version: &str) -> bool {
        let self_exe = std::env::current_exe().ok();
        let invalid = path.is_empty()
            || version.is_empty()
            || self_exe
                .as_ref()
                .map(|e| {
                    e.canonicalize().ok()
                        == PathBuf::from(path).canonicalize().ok()
                })
                .unwrap_or(false);
        if invalid {
            return false;
        }
        self.gui.set("mame_binary", path.to_string());
        if let Err(e) = self.gui.save() {
            // surfaced instead of silently dropping the settings file
            self.log(format!("settings save failed: {e}"));
        }
        self.mame = Some(MameBinary {
            path: PathBuf::from(path),
            version: version.to_string(),
        });
        // the background directory is anchored to the exe dir, but rescan it
        // here anyway: this is the first moment the GUI settings are known good
        let dir = self.content_background_dir();
        if dir != self.bg_dir {
            self.bg_dir = dir;
            // a picture configured for the old dir is no longer reachable
            if self
                .background_file
                .as_deref()
                .is_some_and(|f| !self.bg_dir.join(f).is_file())
            {
                self.background_file = None;
                self.bg_tex = None;
            }
        }
        self.bg_choices = scan_backgrounds(&self.bg_dir);
        let base = PathBuf::from(path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        self.is_mess = base.contains("mess");
        self.is_ume = base.contains("ume");
        self.log(format!("mame: {path} ({version})"));
        true
    }

    pub fn open_mame_picker(&mut self) {
        if self.picking {
            return;
        }
        self.picking = true;
        self.need_mame_pick = true;
        let tx = self.events_tx.clone();
        let ctx = self.ctx();
        // the dialog runs on its own thread — capture the translated title now
        let title = self.tr("MAME executable:");
        let start = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_default();
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title(title)
                .set_directory(&start)
                .add_filter("Executable files (*.exe)", &["exe"])
                .add_filter("All Files (*)", &["*"])
                .pick_file();
            let path = picked
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let version = if path.is_empty() { String::new() } else { detect_version(&path) };
            let _ = tx.send(AppEvent::MameVersionChecked { path, version });
            ctx.request_repaint();
        });
    }

    pub fn boot(&mut self) {
        dlog!("引导: 启动后台线程（读缓存 → listxml → showconfig → 校验）");
        self.need_mame_pick = false;
        self.lib_status = LibStatus::Loading;
        self.progress_open = true;
        let mame = self.mame.clone().unwrap();
        crate::background::boot_library(mame, self.events_tx.clone(), self.ctx());
    }

    /// resolve a ';'-list dir option against the mame.exe directory
    pub fn opt_resolved_dirs(&self, key: &str) -> String {
        let Some(opts) = self.opts.as_ref() else { return String::new() };
        let Ok(core) = opts.try_lock() else { return String::new() };
        let Some(o) = core.opts.get(key) else { return String::new() };
        core.resolve_dir_list(&o.globalvalue)
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(";")
    }

    // ------------------------------------------------------------------
    // content paths (artwork, DATs, backgrounds, folder lists)
    //
    // These deliberately do NOT go through `opt_resolved_dirs`: that resolves
    // against the mame directory, which is right for rompath and wrong for
    // everything MvUI itself supplies. See `core::paths` for the rule.
    // ------------------------------------------------------------------

    /// The user's setting for a content path, GUI settings first.
    ///
    /// Every one of these keys is `guivisible="1"` in the template, which is what
    /// makes 1.8.2 persist it into the original GUI ini rather than mame.ini. The GUI
    /// map is therefore the authoritative copy; the option chain is only
    /// consulted as a fallback for installs whose mame.ini already carries a
    /// value from the old layout.
    fn content_setting(&self, key: &str) -> Option<String> {
        if let Some(v) = self.gui.get(key).map(str::trim).filter(|s| !s.is_empty()) {
            return Some(v.to_string());
        }
        self.opts
            .as_ref()
            .and_then(|o| o.try_lock().ok())
            .and_then(|o| o.opts.get(key).map(|p| p.globalvalue.clone()))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    }

    /// Directories to search for one image dock, `;`-separated.
    pub fn content_image_dirs(&self, dock: usize) -> String {
        let key = crate::core::dat::dock_directory_option(dock);
        let configured = self.content_setting(key);
        crate::core::paths::resolve_list(configured.as_deref(), default_image_subdir(dock))
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(";")
    }

    /// Absolute path of one document dock's `.dat`.
    pub fn content_dat_file(&self, dock: usize) -> String {
        let Some(key) = crate::core::dat::dock_file_option(dock) else {
            return String::new();
        };
        let configured = self.content_setting(key);
        let fallback = crate::core::paths::DAT_FILES
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, f)| *f)
            .unwrap_or("history.xml");
        match configured {
            Some(v) => crate::core::paths::resolve(Some(&v), fallback),
            None => crate::core::paths::exe_dir()
                .join(crate::core::paths::DAT_SUBDIR)
                .join(fallback),
        }
        .to_string_lossy()
        .to_string()
    }

    /// The external folder lists directory (`<exe>/folders` by default).
    pub fn content_folders_dir(&self) -> PathBuf {
        let configured = self.content_setting("folder_directory");
        crate::core::paths::folders_dir(configured.as_deref())
    }

    /// The localized game list (`<exe>/mame_cn.lst` by default).
    pub fn content_localized_list(&self) -> PathBuf {
        let configured = self.content_setting("localized_list_file");
        crate::core::paths::localized_list(configured.as_deref())
    }

    /// Where the wallpaper images live.
    pub fn content_background_dir(&self) -> PathBuf {
        let configured = self.content_setting("background_directory");
        crate::core::paths::background_dir(configured.as_deref())
    }

    /// origin: ensure_chain — cumulative option chain load to the current level
    pub fn ensure_chain(&mut self) {
        let Some(opts) = self.opts.clone() else { return };
        let Some(lib) = self.lib.clone() else { return };
        let meta = match self.current_meta() {
            Some(m) => m,
            None => return,
        };
        let gui: std::collections::HashMap<String, String> =
            self.gui.map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        let mut core = opts.lock().unwrap();
        let libg = lib.lock().unwrap();
        core.chain_load(&meta, &libg, &gui, self.opt_level);
    }

    /// raw command execution for the CmdUI dialog (origin -noreadconfig path)
    pub fn launch_raw(&mut self, args: Vec<String>) {
        let Some(mame) = self.mame.clone() else { return };
        self.log(format!("launch: {} {:?}", mame.path.display(), args));
        match mame.spawn_run(&args) {
            Ok(child) => {
                let game = self.current_game.clone();
                self.running.insert(game.clone());
                let tx = self.events_tx.clone();
                let ctx = self.ctx();
                std::thread::spawn(move || {
                    let mut child = child;
                    let code = child.wait().ok().and_then(|s| s.code());
                    let _ = tx.send(AppEvent::MameExited { game, code });
                    ctx.request_repaint();
                });
            }
            Err(e) => self.poplog(e.to_string()),
        }
    }

    pub fn refresh_all(&mut self) {
        // origin actionRefresh: romVerifyor->verify() (internal verify + re-init)
        if self.lib.is_none() {
            return;
        }
        self.start_internal_verify();
    }

    pub fn start_internal_verify(&mut self) {
        if self.verify_handle.is_some() {
            return;
        }
        let Some(lib) = self.lib.clone() else { return };
        let handle = Arc::new(crate::core::verify::VerifyHandle::new());
        self.verify_handle = Some(handle.clone());
        let opts = self.opts.clone().unwrap_or_else(|| Arc::new(Mutex::new(crate::core::options::OptionCore::default())));
        let is_mess = self.is_mess;
        crate::background::run_verify(
            lib,
            opts,
            handle,
            self.events_tx.clone(),
            self.ctx(),
            is_mess,
        );
    }

    pub fn export_fixdat(&mut self, method: crate::core::verify::VerifyMethod) {
        let Some(lib) = self.lib.clone() else { return };
        let target = self.export_target.clone().unwrap_or_else(|| {
            std::env::temp_dir().join("mvui_fixdat.dat")
        });
        let guard = lib.lock().unwrap();
        match crate::core::verify::export_fixdat(&guard, method, &target) {
            Ok(n) => self.log(format!("fixdat written: {n} sets -> {}", target.display())),
            Err(e) => self.poplog(format!("fixdat export failed: {e}")),
        }
    }

    pub fn export_game_list(&mut self, have: bool) {
        let Some(lib) = self.lib.clone() else { return };
        let target = self.export_target.clone().unwrap_or_else(|| {
            std::env::temp_dir().join(if have { "have.txt" } else { "miss.txt" })
        });
        let guard = lib.lock().unwrap();
        let mut names: Vec<String> = guard
            .games
            .iter()
            .filter(|g| {
                !g.is_device
                    && g.devices.is_empty()
                    && (if have { g.available == 1 } else { g.available != 1 })
            })
            .map(|g| g.name.clone())
            .collect();
        names.sort();
        let _ = std::fs::write(&target, names.join("\n"));
        self.log(format!("list exported: {} games -> {}", names.len(), target.display()));
    }

    pub fn pump_events(&mut self, ctx: &egui::Context) {
        let mut any = false;
        while let Ok(ev) = self.events_rx.try_recv() {
            any = true;
            match ev {
                AppEvent::MameVersionChecked { path, version } => {
                    // The picker has answered (picked *or* cancelled): clear the
                    // flag here, otherwise cancelling the dialog — or picking
                    // something that is not a MAME — left the "Select mame.exe…"
                    // button dead for the rest of the session (README N5).
                    self.picking = false;
                    self.need_mame_pick = true;
                    if path.is_empty() {
                        // Cancelled, i.e. chose nothing. That is not an error and
                        // must not raise a toast: the start-up panel already says
                        // "未配置 MAME 主程序。" in plain sight, so a modal telling
                        // the user the same thing — with an OK button whose only
                        // job is to dismiss it — is pure noise. Stay on the panel
                        // and let them pick again (or not).
                    } else if self.try_accept_mame(&path, &version) {
                        self.need_mame_pick = false;
                        self.boot();
                    } else {
                        // A file *was* chosen and it is not a usable MAME: that
                        // is a real failure worth reporting.
                        self.poplog(self.tr("Could not find valid MAME/MESS."));
                    }
                }
                AppEvent::LibProgress { done, total } => {
                    self.lib_progress = (done, total);
                }
                AppEvent::VerifyProgress { done, total, system } => {
                    self.verify_stage = (done, total, system);
                }
                AppEvent::LibraryReady(res) => {
                    dlog!("事件: LibraryReady 到达");
                    match res {
                        Ok(payload) => {
                            let ReadyPayload { lib, folders, from_cache, verified } = payload;
                            let machine_count = {
                                let mut guard = lib.lock().unwrap();
                                // the localized list is applied here, not during the
                                // verify: it is a plain text file the user edits by
                                // hand, so re-reading it must not require re-verifying
                                // 40 000 roms
                                self.apply_localized_list(&mut guard);
                                self.maps = FolderMaps::build(&guard);
                                guard.len()
                            };
                            // the library handle used to be dropped on the floor
                            // here: self.lib stayed None, so refilter() cleared the
                            // list ("0 games"), draw_folders() fell through to an
                            // empty Ready branch, and every lookup returned None
                            self.lib = Some(lib);
                            self.folder_cache = Some(folders);
                            self.boot_verifying = !from_cache;
                            // 校验前发布的那一次 `verified == false`：Rom 信息
                            // 面板这时把所有条目显示成"未校验"而不是"缺失"
                            self.lib_verified = verified;
                            self.rom_views.clear();
                            self.lib_status = LibStatus::Ready;
                            self.progress_open = false;
                            self.load_ext_folders();
                            self.needs_refilter = true;
                            if let Some(g) = self.gui.get("default_game") {
                                self.current_game = g.to_string();
                            }
                            self.log("game list ready".to_string());
                            dlog!(
                                "事件: LibraryReady 处理完毕（{} 台机种, 来自缓存={}）",
                                machine_count,
                                from_cache
                            );
                            dlog!("事件: 库校验标志 verified={verified}");
                        }
                        Err(e) => {
                            self.lib_status = LibStatus::Error;
                            self.poplog(e);
                            self.progress_open = false;
                        }
                    }
                }
                AppEvent::OptionsReady(res) => {
                    if let Ok(opts) = &res {
                        // GUI paths from pGuiSettings (origin loadIni GUI-overlap)
                        let gui_keys: Vec<(String, String)> = self
                            .gui
                            .map
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        let mut guard = opts.lock().unwrap();
                        for (k, v) in gui_keys {
                            if let Some(o) = guard.opts.get_mut(&k) {
                                if o.guivisible {
                                    o.globalvalue = v.clone();
                                    o.currvalue = v;
                                }
                            }
                        }
                        self.dirs_buf = guard
                            .opts
                            .get("rompath")
                            .map(|o| o.currvalue.clone())
                            .unwrap_or_default();
                    }
                    self.opts = res.ok();
                }
                AppEvent::FoldersReady(cache) => {
                    self.folder_cache = Some(cache);
                    if let Some(lib) = self.lib.clone() {
                        if let Ok(guard) = lib.try_lock() {
                            self.maps = FolderMaps::build(&guard);
                        }
                    }
                    self.needs_refilter = true;
                }
                AppEvent::VerifyStarted(h) => {
                    // boot verify: keep the handle so the status bar can read its
                    // counter and label the run "Verifying"
                    self.verify_handle = Some(h);
                }
                AppEvent::VerifyDone(res) => {
                    self.verify_handle = None;
                    self.boot_verifying = false;
                    // 全库校验结束：每条 rom 的 available 都变了，而面板缓存的正是那些
                    // 旧结论。清掉，下一帧按当前游标重算。同时把 `lib_verified`
                    // 置真：之前面板把一切缺失都降级成"未校验"显示，现在该让它说真话了。
                    self.lib_verified = true;
                    self.rom_views.clear();
                    match res {
                        Ok(m) => self.log(m),
                        Err(m) => self.log(format!("verify: {m}")),
                    }
                    if let Some((method, target)) = self.exporting_method.zip(self.export_target.clone()) {
                        self.export_target = Some(target);
                        self.export_fixdat(method);
                        self.exporting_method = None;
                        self.export_target = None;
                    }
                    self.needs_refilter = true;
                }
                AppEvent::SnapReady { dock, game, width, height, rgba } => {
                    // the reply means the request is no longer in flight: drop the
                    // record, otherwise coming back to this game later (A→B→A)
                    // would find the key still present and never load it again
                    self.snap_requested.remove(&crate::views::dock_key(dock, &game));
                    // a late reply for a game the user has already left must not
                    // overwrite the current preview with stale art
                    if game == self.current_game {
                        if width == 0 {
                            // remember "this game has no art" so the placeholder
                            // does not re-request every frame
                            self.snap_tex.insert(dock, (game, None));
                        } else {
                            let tex = ctx.load_texture(
                                format!("snap{dock}-{game}"),
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [width as usize, height as usize],
                                    &rgba,
                                ),
                                egui::TextureOptions::LINEAR,
                            );
                            self.snap_tex.insert(dock, (game, Some(tex)));
                        }
                    }
                }
                AppEvent::IconReady { game, width, height, rgba } => {
                    // the reply means the request is no longer in flight
                    self.icon_requested.remove(&game);
                    let tex = if width == 0 {
                        // remember "this machine has no icon", so the status
                        // square is not chased by a request every frame
                        None
                    } else {
                        Some(ctx.load_texture(
                            format!("icon-{game}"),
                            egui::ColorImage::from_rgba_unmultiplied(
                                [width as usize, height as usize],
                                &rgba,
                            ),
                            egui::TextureOptions::LINEAR,
                        ))
                    };
                    self.insert_game_icon(game, tex);
                }
                AppEvent::DatReady { dock, game, text } => {
                    self.dat_requested.remove(&(dock, game.clone()));
                    if game == self.current_game {
                        self.dat_texts.insert((dock, game), text);
                        self.trim_dat_cache();
                    }
                }
                AppEvent::VerifyLine(line) => {
                    self.verify_lines.push(line);
                }
                AppEvent::VerifyOutputDone => {}
                AppEvent::MameExited { game, code } => {
                    self.running.remove(&game);
                    self.cleanup_temp_roms(Some(&game));
                    self.log(format!("mame exited ({game}): {code:?}"));
                }
                AppEvent::Log(m) => self.log(m),
            }
        }
        if any {
            ctx.request_repaint();
        }
    }

    // ---- ext folders (origin: parseExtFolders/initExtFolders) ----

    /// Fill `lc_desc` / `lc_mftr` from `mame_cn.lst`.
    ///
    /// 1.8.2 read these from the binary `.mmo` file next to mame.exe; the port
    /// never implemented that, so the Localized Game List switch did nothing at
    /// all (`lc_desc` was always empty). The replacement is a tab-separated
    /// text list in the MvUI directory — see `core::lst`.
    ///
    /// A missing list is not an error: it just leaves every game untranslated,
    /// which is the same as having the switch off.
    pub fn apply_localized_list(&mut self, lib: &mut crate::core::library::GameLibrary) {
        let path = self.content_localized_list();
        if !path.is_file() {
            return;
        }
        let map = crate::core::lst::load(&path);
        if map.is_empty() {
            return;
        }
        let mut hits = 0usize;
        for g in &mut lib.games {
            if let Some((desc, _mftr)) = map.get(&g.name) {
                // Only the description is localized. The list's second column is
                // not a manufacturer translation — in every shipped file it
                // repeats the description — so writing it into `lc_mftr` put the
                // Chinese title in the Manufacturer column.
                g.lc_desc = desc.clone();
                hits += 1;
            }
        }
        self.log(format!(
            "localized list: {} entries from {} ({} matched)",
            map.len(),
            path.display(),
            hits
        ));
    }

    /// Re-read `mame_cn.lst` into the live library and refilter.
    pub fn reload_localized_list(&mut self) {
        let Some(lib) = self.lib.clone() else { return };
        if let Ok(mut guard) = lib.lock() {
            // clear first so a removed entry reverts to the MAME description
            for g in &mut guard.games {
                g.lc_desc.clear();
            }
            self.apply_localized_list(&mut guard);
        }
        self.needs_refilter = true;
    }

    pub fn load_ext_folders(&mut self) {
        self.ext_folder_data.clear();
        // `<exe>/folders` unless the user pointed it elsewhere; the directory is
        // created here because an absent one means the tree has no root at all
        // and 1.8.2 wrote Favorites.ini into it unconditionally.
        let dir = self.content_folders_dir();
        let folder_dir = dir.to_string_lossy().to_string();
        let mut names: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|x| x == "ini").unwrap_or(false) {
                    if let Some(stem) = p.file_stem() {
                        names.push(stem.to_string_lossy().to_string());
                    }
                }
            }
        }
        // auto-create Favorites (origin exact bytes)
        let has_fav = names.iter().any(|n| n == "Favorites");
        if !has_fav {
            let fav = dir.join("Favorites.ini");
            if !fav.exists() {
                let _ = std::fs::write(&fav, crate::core::folders::FAVORITES_INI_BYTES);
                names.push("Favorites".into());
            }
        }
        names.sort();
        names.dedup();
        for name in &names {
            if let Some(text) = read_ext_folder_text(&folder_dir, name) {
                let store = crate::core::folders::ExtFolderStore {
                    name: name.clone(),
                    entries: crate::core::folders::parse_ext_folders(&text),
                    writable: true,
                };
                self.ext_folder_data.push((name.clone(), store));
            }
        }
    }

    pub fn save_ext_folder(&mut self, name: &str) {
        let dir = self.content_folders_dir();
        if let Some((_, store)) = self.ext_folder_data.iter_mut().find(|(n, _)| n == name) {
            let text = crate::core::folders::save_ext_folders(&store.entries);
            let _ = std::fs::write(dir.join(format!("{name}.ini")), text);
        }
    }
}

/// Default subdirectory of one image dock, used when the option is unset.
fn default_image_subdir(dock: usize) -> &'static str {
    crate::core::paths::IMAGE_DIRS
        .get(dock)
        .map(|(_, rel)| *rel)
        .unwrap_or("pcb")
}

/// Every usable background image, by file name, sorted. Origin: 1.8.2 built the
/// background menu out of a `QDir` listing filtered to `*.png` / `*.jpg`, with
/// one checkable action per file. An empty directory simply means no entries —
/// the menu then only offers the light/dark pair.
///
/// Both PNG and JPEG are accepted, as in 1.8.2. Restricting this to PNG (as an
/// earlier pass here did, reading the requirement too literally) silently hides
/// every JPEG wallpaper: the files are still in `bkground`, they just never
/// appear in the menu, and the only visible symptom is that "the directory has
/// pictures in it but only some of them show up".
pub fn scan_backgrounds(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            let is_image = lower.ends_with(".png")
                || lower.ends_with(".jpg")
                || lower.ends_with(".jpeg");
            (is_image && e.path().is_file()).then_some(name)
        })
        .collect();
    v.sort_by_key(|n| n.to_ascii_lowercase());
    v
}

/// Average luma of a background image, `true` (dark UI) when it cannot be read.
///
/// Origin: 1.8.2 scaled the wallpaper to 1x1 and compared `qGray` against 128
/// to decide whether the UI should be dark or light. Same rule, but the sample
/// is taken from a *thumbnail* rather than the full bitmap: a 5888x3312
/// wallpaper is 19.5 Mpx, and walking every pixel costs hundreds of
/// milliseconds on the UI thread — which the old Qt code never paid because
/// `QImage::scaled(1,1)` downsampled as part of the scale.
pub fn background_is_dark(dir: &Path, file: &str) -> bool {
    let Some(bytes) = std::fs::read(dir.join(file)).ok() else {
        return true;
    };
    let Some(img) = image::load_from_memory(&bytes).ok() else {
        return true;
    };
    // `thumbnail` box-filters — same "average the whole picture" semantics as
    // `QImage::scaled(1,1)`, without materialising 19.5 Mpx of RGBA first.
    let img = fit_within(&img, 64, 64).to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return true;
    }
    let mut sum = 0u64;
    for p in img.pixels() {
        let p = p.0;
        sum += (299 * p[0] as u64 + 587 * p[1] as u64 + 114 * p[2] as u64) / 1000;
    }
    sum / ((w * h) as u64) < 128
}

/// Scale `img` to fit inside `max_w` x `max_h`, keeping the aspect ratio.
/// Never enlarges: origin: 1.8.2 only ever scaled *down* to the window.
fn fit_within(img: &image::DynamicImage, max_w: u32, max_h: u32) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w <= max_w && h <= max_h {
        return img.clone();
    }
    let scale = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
}

/// Load `file` from `dir` as a texture, reusing the cached one.
///
/// The handle is cached by file name: the menu can rebuild this layer every
/// frame, and re-decoding a multi-megapixel PNG each time would be wasteful.
///
/// Bitmaps are downscaled to at most [`BG_MAX_EDGE`] on the long side first.
/// Origin: 1.8.2 did the same before handing the pixmap to Qt
/// (`bkgroundImg.scaled(size(), Qt::KeepAspectRatioByExpanding, ...)` in
/// `setBgPixmap`): a raw 19.5 Mpx wallpaper is ~78 MB of VRAM, and its mip
/// chain is pure waste for a backdrop only ever drawn at window size. 4096
/// still covers a 4K window 1:1.
const BG_MAX_EDGE: u32 = 4096;

pub fn load_background(
    dir: &Path,
    file: &str,
    tex: &mut Option<(String, egui::TextureHandle)>,
    ctx: &egui::Context,
) -> Option<egui::TextureHandle> {
    if let Some((name, handle)) = tex {
        if name == file {
            return Some(handle.clone());
        }
    }
    let path = dir.join(file);
    let bytes = std::fs::read(&path).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    // cap the texture: see BG_MAX_EDGE
    let img = fit_within(&img, BG_MAX_EDGE, BG_MAX_EDGE);
    let img = img.to_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let handle = ctx.load_texture(
        file,
        egui::ColorImage::from_rgba_unmultiplied([w, h], &img),
        // repeat wrap, otherwise "Tile" shows one stretched copy
        egui::TextureOptions::LINEAR_REPEAT,
    );
    *tex = Some((file.to_string(), handle.clone()));
    Some(handle)
}

/// Apply the light/dark theme to the context.
///
/// `dark_bg` used to be a plain flag with nothing reading it: switching the
/// theme in the menu changed no colours at all. This is the missing half — the
/// base `Visuals` (panel fills, text, borders) is what makes the difference
/// readable, so it is what we set.
///
/// `transparent` says a wallpaper is selected. It deliberately does almost
/// nothing to the fills: the picture is drawn by `ui::draw_background` as the
/// central panel's own backdrop, so it already sits *under* the dock area, and
/// every panel keeps its opaque brush on top. That matches what 1.8.2 achieves
/// with `setBgPixmap` (pixmap on the main window's background role) plus
/// `setTransparentBg` (`QPalette::Base` → `rgba(0,0,0,128)`), and it is why
/// the reference build stays readable over a busy wallpaper: the text brush is
/// fully opaque, and so is everything it is drawn on.
pub fn apply_theme_with_bg(ctx: &egui::Context, dark: bool, transparent: bool) {
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    // keep the tuned scroll-bar/selection tweaks from `main.rs::style`
    v.selection.bg_fill = egui::Color32::from_rgb(0, 120, 215);
    v.widgets.hovered.bg_fill = v.widgets.noninteractive.weak_bg_fill;
    if transparent {
        let veil = if dark {
            egui::Color32::from_black_alpha(128)
        } else {
            egui::Color32::from_white_alpha(128)
        };
        // `window_fill` stays **opaque** on purpose. egui derives three things
        // from it: every dock leaf's `TabBodyStyle::bg_fill` (style.rs:704),
        // every menu frame (`Frame::menu`, frame.rs:123) and every popup. Only
        // the first one should be see-through — 1.8.2's `setTransparentBg`
        // swapped exactly one brush (`QPalette::Base` → `rgba(0,0,0,128)`) and
        // left the menu bar and toolbars solid. Making `window_fill` translucent
        // to get the dock right therefore washed out the menus too; the veil now
        // goes into `ui::dock_style`'s `tab_body.bg_fill` instead, which is the
        // one slot only the dock leaves read.
        //
        // Striped rows are drawn by the game list on top of the leaf, so they
        // take the same veil — an opaque stripe over a translucent panel reads
        // as a bright band floating above the picture.
        v.faint_bg_color = veil;
        // 1.8.2 kept the text fully opaque over the wallpaper
        // (`QDockWidget, QStatusBar QLabel { color: white }` in the stylesheet
        // `setBgPixmap` builds); egui does the same, but make sure nothing
        // else has left a faded override behind.
        v.override_text_color = None;
        //
        // Two earlier attempts, both wrong, for the record:
        //  * only `extreme_bg_color` translucent — on the mistaken belief it was
        //    the dock backdrop. It is egui's void/fallback colour, so nothing
        //    changed and the picture stayed invisible.
        //  * this veil *plus* painting the picture on `Order::Background` over
        //    `ctx.screen_rect()` — the picture then covered the whole window
        //    including the menu bar and toolbar and buried the entire
        //    interface. The picture is now drawn inside the central panel by
        //    `ui::draw_background`.
        //  * putting the veil on `window_fill` — right colour, wrong consumer:
        //    the menu and popup frames read the same slot, so they went
        //    translucent too.
    }
    ctx.set_visuals(v);
}

/// 启动/性能日志的落点：写 `<配置根>/cache/boot.log`。
///
/// **只有 debug 构建会落盘。** release 下这是一个真正的空操作——
/// 不 stat、不建目录、不开文件句柄，所以发行版既不会留下
/// `boot.log`，也不会因为日志去创建 `cache` 目录（那个目录本来
/// 只由缓存自己按需创建）。总闸是 [`crate::core::log::ENABLED`]，
/// 编译期常量，因此 `format!` 在调用点之外也不会被求值。
///
/// 上限：一次长会话配上逐帧日志会让它无限增长（README P3）。超过
/// [`PERF_LOG_MAX_BYTES`] 时截断，只留最后 [`PERF_LOG_KEEP_BYTES`]。
///
/// 这是 core 的 `dlog!` 落的同一个地方——注册在 `main` 里，见
/// [`crate::core::log::set_sink`]。
pub fn perf_log(msg: &str) {
    if !crate::core::log::ENABLED {
        return;
    }
    write_boot_log(msg);
}

#[cfg(debug_assertions)]
fn write_boot_log(msg: &str) {
    use std::io::Write;
    let p = GuiSettings::cache_dir().join(LOG_FILE_NAME);
    if let Ok(meta) = std::fs::metadata(&p) {
        if meta.len() > PERF_LOG_MAX_BYTES {
            if let Ok(bytes) = std::fs::read(&p) {
                let from = bytes.len().saturating_sub(PERF_LOG_KEEP_BYTES as usize);
                let tail = &bytes[from..];
                let cut = tail
                    .iter()
                    .position(|&b| b == b'\n')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let _ = std::fs::write(&p, &tail[cut..]);
            }
        }
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
        let _ = writeln!(f, "[{:?}] {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0), msg);
    }
}

/// release 构建下没有落点函数体可编，整个写盘逻辑（包括那两个
/// 字节数上限）都不参与编译。
#[cfg(not(debug_assertions))]
fn write_boot_log(_msg: &str) {}

pub fn detect_version(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    match MameBinary::detect(Path::new(path)) {
        Ok(m) if !m.version.is_empty() => m.version,
        _ => String::new(),
    }
}

/// read `folders/<name>.ini` from the first folder dir that has it
/// (decoded through the shared UTF-8-or-locale reader)
fn read_ext_folder_text(folder_dir: &str, name: &str) -> Option<String> {
    for d in folder_dir.split(';') {
        let p = PathBuf::from(d.trim()).join(format!("{name}.ini"));
        if p.is_file() {
            return crate::core::options::read_text_file(&p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `perf_log` 必须跟着构建模式走：debug 落盘，release 什么都不做。
    ///
    /// 这条在 release 下跑时是**真正有牙齿**的那条——`perf_log` 连
    /// `cache_dir()` 都不会调，所以既不 stat 也不建目录，发行版不会在
    /// 用户机器上凭空造出 `.mvui/cache/boot.log`（README P3 那个 4 MB
    /// 上限之所以存在，就是因为它一度每帧都在长）。
    ///
    /// debug 下断言的是同一件事的另一头：调用安全返回、不 panic。
    /// 真正的"落盘"没法在这里断言——`cfg_prefix()` 是 `OnceLock`
    /// 全局，写用户目录既不可测也不该做。
    #[test]
    fn perf_log_follows_the_build_profile() {
        assert_eq!(
            crate::core::log::ENABLED,
            cfg!(debug_assertions),
            "日志总闸必须与构建模式一致"
        );
        // 两种构建下都必须安全返回（release 下是一次真正的空操作）
        perf_log("单测: 这条在 release 下什么都不该做");
    }

    /// 日志落点的文件名不能被"顺手改掉"——`settings::cache_dir()` 下面
    /// 只有这一个日志文件，改名会让用户找不到它，也和 README 里写的
    /// 排查步骤对不上。
    #[cfg(debug_assertions)]
    #[test]
    fn log_file_name_is_stable() {
        assert_eq!(LOG_FILE_NAME, "boot.log");
    }

    /// 截断上限必须还在：一次长会话配上逐帧日志会把它喂到无上限增长。
    #[cfg(debug_assertions)]
    #[test]
    fn log_stays_capped() {
        assert!(PERF_LOG_MAX_BYTES <= 16 * 1024 * 1024);
        assert!(PERF_LOG_KEEP_BYTES < PERF_LOG_MAX_BYTES);
    }
}
