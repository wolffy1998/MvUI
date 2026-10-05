//! 领域层调试日志。
//!
//! core 不允许依赖 UI（分层约定见 `core/mod.rs`），所以拿不到
//! `app::perf_log`——那是 UI 层的东西，反向引用会把分层捅穿。
//! 这里用一个全局 sink 绕开：UI 启动时把 `perf_log` 注册进来，
//! core 打的日志就统一落到 `boot.log`；没注册时静默。
//!
//! 静默这个默认很关键：`cargo test` 跑 core 的 40 项单测时不会
//! 往磁盘写任何东西，测试也就不会互相污染临时目录。
//!
//! 用法：
//! ```ignore
//! dlog!("解析完成: {} 条", n);
//! ```

use std::cell::Cell;
use std::sync::OnceLock;

/// 日志落点。由 UI 层在启动时注册，进程内只允许设置一次。
static SINK: OnceLock<fn(&str)> = OnceLock::new();

thread_local! {
    /// sink 正在执行的标记，防止日志自己把自己再调一遍。
    ///
    /// 这条链子真实存在过：`dlog!` → sink(`app::perf_log`) →
    /// `GuiSettings::cache_dir()` → `cfg_prefix()` → `dlog!` → …… 无限递归，
    /// 栈溢出。**Windows 上它表现为 `0xC0000005` 访问违规**：没有 Rust
    /// panic、没有 stderr 输出、没有事件日志记录，窗口来不及画出来就退了，
    /// 双击 exe 看起来就是"毫无反应"。
    static IN_SINK: Cell<bool> = const { Cell::new(false) };
}

/// 进入 sink；已经在里面则返回 `None`（说明这是一次重入，调用方应丢弃）。
///
/// 用 `Drop` 复位而不是用完手动清：sink 里若 panic，标记不会永久卡住。
struct Reentry;
impl Drop for Reentry {
    fn drop(&mut self) {
        IN_SINK.with(|c| c.set(false));
    }
}
fn enter() -> Option<Reentry> {
    IN_SINK.with(|c| if c.replace(true) { None } else { Some(Reentry) })
}

/// 注册日志落点。已经注册过则返回 false，不覆盖已有 sink。
pub fn set_sink(f: fn(&str)) -> bool {
    SINK.set(f).is_ok()
}

/// 是否已注册落点。未注册时 [`debug`] 是空操作。
///
/// `dlog!` 先问这个再格式化，所以没接日志时连 `format!` 的开销
/// 都省掉了——审计循环里那些逐条调用不会拖慢热路径。
pub fn has_sink() -> bool {
    SINK.get().is_some()
}

/// 写一条调试日志。未注册落点、或这是一次重入调用，都直接返回。
pub fn debug(msg: impl AsRef<str>) {
    let Some(_guard) = enter() else { return };
    if let Some(f) = SINK.get() {
        f(msg.as_ref());
    }
}

/// 领域层调试日志宏。未接落点时整句被跳过。
#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        if $crate::core::log::has_sink() {
            $crate::core::log::debug(format!($($arg)*));
        }
    };
}
