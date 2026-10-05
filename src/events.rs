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
    /// 库里的可用性数据**是否已经过审计**（`gamelist.cache` 的 `audited`
    /// 标志）。
    ///
    /// Rom 信息面板靠它区分"缺失"和"还没查"：审计前发布的那一次是 `false`
    /// （界面刚出来、审计还在跑），此时每条 rom 的 `available` 都还是默认值
    /// `false`。拿它当"缺失"显示，用户会看到一屏红色，而实际上审计正要把它
    /// 变成绿色。
    pub audited: bool,
}

pub enum AppEvent {
    MameVersionChecked { path: String, version: String },
    /// `-listxml` 的进度。
    ///
    /// `total == 0` 表示还在收子进程输出——机种总数要收完整份才知道，此时只
    /// 报台数；`total > 0` 是解析阶段，分母就是收输出时数出来的真总数。
    LibProgress { done: usize, total: usize },
    /// 审计进度。`system` 是当前正在扫的机种/系统名，状态栏会显示它。
    AuditProgress { done: usize, total: usize, system: String },
    LibraryReady(Result<ReadyPayload, String>),
    /// the boot chain's audit handle, so the UI can report "Auditing nn%" from
    /// the first tick (the handle owns the counter the progress thread reads)
    AuditStarted(Arc<crate::core::audit::AuditHandle>),
    OptionsReady(Result<SharedOpts, String>),
    /// folder tree rebuilt after an audit changed availability
    FoldersReady(Arc<FolderCache>),
    AuditDone(Result<String, String>),
    /// 单游戏审计跑完（右键/菜单「审计 ROM」）。
    ///
    /// 视图是**那一瞬间的快照**，不是"回头去库里读"——审计在后台线程上
    /// 改了共享的库，而用户可能在这期间点了别的游戏。带着快照回来，弹窗
    /// 讲的一定是它自己审的那个游戏。
    GameAuditDone {
        game: String,
        result: Result<crate::core::rominfo::RomInfoView, String>,
    },
    SnapReady { dock: usize, game: String, width: u32, height: u32, rgba: Vec<u8> },
    /// A machine icon arrived (or was found missing: `width == 0`).
    /// `game` is the row it belongs to even when the bytes came from a parent
    /// set — the inheritance is resolved by the loader (README §6.1③).
    IconReady { game: String, width: u32, height: u32, rgba: Vec<u8> },
    DatReady { dock: usize, game: String, text: Option<String> },
    VerifyLine(String),
    VerifyDone,
    MameExited { game: String, code: Option<i32> },
    Log(String),
}
