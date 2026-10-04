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

use std::sync::OnceLock;

/// 日志落点。由 UI 层在启动时注册，进程内只允许设置一次。
static SINK: OnceLock<fn(&str)> = OnceLock::new();

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

/// 写一条调试日志。未注册落点时直接返回。
pub fn debug(msg: impl AsRef<str>) {
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
