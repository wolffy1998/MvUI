//! Background -> UI event plumbing.

use std::sync::{Arc, Mutex};

use crate::core::folders::FolderCache;
use crate::core::library::GameLibrary;
use crate::core::options::OptionCore;

pub type SharedLib = Arc<Mutex<GameLibrary>>;
pub type SharedOpts = Arc<Mutex<OptionCore>>;

/// What `LibraryReady` carries.
///
/// `opts` used to be a field as well, but the cache path filled it with a dummy
/// `OptionCore::default()` that nobody read — options always arrive through
/// `OptionsReady`. Dropped so the struct cannot mislead a reader.
pub struct ReadyPayload {
    pub lib: SharedLib,
    pub folders: Arc<FolderCache>,
    pub from_cache: bool,
    /// 库里的可用性数据**是否已经过校验**（`gamelist.cache` 的 `verified`
    /// 标志）。
    ///
    /// Rom 信息面板靠它区分"缺失"和"还没查"：校验前发布的那一次是 `false`
    /// （界面刚出来、校验还在跑），此时每条 rom 的 `available` 都还是默认值
    /// `false`。拿它当"缺失"显示，用户会看到一屏红色，而实际上校验正要把它
    /// 变成绿色。
    pub verified: bool,
}

pub enum AppEvent {
    MameVersionChecked { path: String, version: String },
    /// `-listxml` 的进度。
    ///
    /// `total == 0` 表示还在收子进程输出——机种总数要收完整份才知道，此时只
    /// 报台数；`total > 0` 是解析阶段，分母就是收输出时数出来的真总数。
    LibProgress { done: usize, total: usize },
    /// 校验进度。`system` 是当前正在扫的机种/系统名，状态栏会显示它。
    VerifyProgress { done: usize, total: usize, system: String },
    LibraryReady(Result<ReadyPayload, String>),
    /// the boot chain's verify handle, so the UI can report "Verifying nn%" from
    /// the first tick (the handle owns the counter the progress thread reads)
    VerifyStarted(Arc<crate::core::verify::VerifyHandle>),
    OptionsReady(Result<SharedOpts, String>),
    /// folder tree rebuilt after an verify changed availability
    FoldersReady(Arc<FolderCache>),
    VerifyDone(Result<String, String>),
    // `GameVerifyDone` 已于 2026-10-06 删除（单游戏校验整条路走不通了）。
    SnapReady { dock: usize, game: String, width: u32, height: u32, rgba: Vec<u8> },
    /// A machine icon arrived (or was found missing: `width == 0`).
    /// `game` is the row it belongs to even when the bytes came from a parent
    /// set — the inheritance is resolved by the loader (README §6.1③).
    IconReady { game: String, width: u32, height: u32, rgba: Vec<u8> },
    DatReady { dock: usize, game: String, text: Option<String> },
    /// `-verifyroms` / `-verifysamples` 的逐行输出与结束标记。
    ///
    /// **当前没有菜单入口**（同 `GameVerifyDone`）。来自 1.8.2 那条原样搬来的
    /// 输出泵，保留以便接回 MAME 原生校验。
    #[allow(dead_code)]
    VerifyLine(String),
    /// `-verifyroms` 输出泵的结束标记。
    ///
    /// **与上面的 `VerifyDone(Result<..>)` 是两回事**：那个是「全库校验结束」，
    /// 这个是「MAME 自己的校验输出读完」。原先两者都叫 `AuditDone`，靠后定义
    /// 那个把前面的**整个覆盖**了（E0428），全库校验结束的事件因此消失。
    #[allow(dead_code)]
    VerifyOutputDone,
    MameExited { game: String, code: Option<i32> },
    Log(String),
}
