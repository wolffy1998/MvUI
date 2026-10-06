//! 领域层调试日志。
//!
//! core 不允许依赖 UI（分层约定见 `core/mod.rs`），所以拿不到
//! `app::perf_log`——那是 UI 层的东西，反向引用会把分层捅穿。
//! 这里用一个全局 sink 绕开：UI 启动时把 `perf_log` 注册进来，
//! core 打的日志就统一落到 `boot.log`；没注册时静默。
//!
//! **只有 debug 构建才产出日志文件。** [`ENABLED`] 是编译期常量，
//! release 下 [`has_sink`] 恒为 false，于是 [`dlog!`] 整句被优化掉
//! ——连 `format!` 都不会执行，`app::perf_log` 也永远不会被调用，
//! `<配置根>/cache/boot.log` 与 `cache` 目录因此都不会被日志创建。
//! 日常开发用 debug 构建即可，release 发行版不落任何日志文件。
//!
//! 没注册 sink 时的静默同样关键：`cargo test` 跑 core 的单测时
//! 不会往磁盘写任何东西，测试也就不会互相污染临时目录。
//!
//! 用法：
//! ```ignore
//! dlog!("解析完成: {} 条", n);
//! ```

use std::cell::Cell;
use std::sync::OnceLock;

/// 日志总闸。`false` = 本次构建不产出任何日志文件。
///
/// 用 `cfg!(debug_assertions)` 而不是自定义 feature：debug 构建本来就是
/// 日常开发用的那个，"要不要日志"跟着构建模式走，用户不需要记第二个
/// 开关；而 `--release` 与 `cargo build` 的差别是所有人都已经知道的事。
///
/// 这是**编译期常量**，所以 `dlog!` 里的 `if` 会被常量折叠掉——不需要
/// 依赖优化器等级，release 下从语义上就不执行。
pub const ENABLED: bool = cfg!(debug_assertions);

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
///
/// release 构建（[`ENABLED`] == false）直接返回 false 且**不注册**：
/// 这样 `has_sink()` 恒为 false，`dlog!` 与 `app::perf_log` 都不会被
/// 触发，`boot.log` 连同它所在的 `cache` 目录都不会被日志创建。
///
/// 判断是运行时分支而不是 `#[cfg]`，是为了让两种构建共用同一份代码：
/// 少一个条件编译块，就少一处"只在 debug 下编得过"的差异。
pub fn set_sink(f: fn(&str)) -> bool {
    if !ENABLED {
        return false;
    }
    SINK.set(f).is_ok()
}

/// 是否真的会写日志。为假时 [`debug`] 与 [`dlog!`] 都是空操作。
///
/// `dlog!` 先问这个再格式化，所以两种关闭方式（没接落点、release 构建）
/// 都**连 `format!` 的开销都省掉了**——校验循环里那些逐条调用不会拖慢
/// 热路径，release 构建里它们也彻底不存在。
pub fn has_sink() -> bool {
    ENABLED && SINK.get().is_some()
}

/// 写一条调试日志。未注册落点、或这是一次重入调用，都直接返回。
pub fn debug(msg: impl AsRef<str>) {
    if !ENABLED {
        return;
    }
    let Some(_guard) = enter() else { return };
    if let Some(f) = SINK.get() {
        f(msg.as_ref());
    }
}

/// 领域层调试日志宏。未接落点（以及 release 构建）时整句被跳过。
#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        if $crate::core::log::has_sink() {
            $crate::core::log::debug(format!($($arg)*));
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// release 构建必须彻底关掉日志，否则发行版会在用户机器上留下
    /// `boot.log`，而用户从没要求过要这个文件。
    ///
    /// 这条断言在 debug 下也成立——它真正守住的是"总闸只能由
    /// `debug_assertions` 决定"，防止有人日后把它改成 `true` 或
    /// 接到某个 feature 上从而在 release 里重新打开。
    #[test]
    fn log_gate_follows_the_build_profile() {
        assert_eq!(
            ENABLED,
            cfg!(debug_assertions),
            "日志总闸必须与构建模式一致：release 不写日志文件"
        );
    }

    /// 没注册落点时 `debug` 是一条空操作——`cargo test` 跑 core 的
    /// 单测时不该往磁盘写任何东西，测试之间也才不会互相污染。
    ///
    /// 这条同时守着 `IN_SINK` 那道重入闸：单测进程里没有落点，
    /// 所以进得去也出得来，不会把标记卡在 true 上。
    #[test]
    fn debug_is_inert_without_a_sink() {
        assert!(
            !has_sink(),
            "单测进程不该注册日志落点，否则测试会写用户的 boot.log"
        );
        debug("这条不该出现在任何地方");
        // 再问一次：确认上面那次调用没有把重入标记卡住
        assert!(!has_sink());
    }

    /// `dlog!` 在没有落点时整句跳过，**包括 `format!` 里的表达式**。
    ///
    /// 断言的是"实参没被求值"，不是"日志没出现"——后者在测试里看不见。
    /// 校验循环里逐单元调用 `dlog!`，一旦这条退化，release 构建就会
    /// 在热路径上白算一遍 `format!`。
    #[test]
    fn dlog_does_not_evaluate_its_arguments() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static EVALS: AtomicUsize = AtomicUsize::new(0);
        fn bump() -> usize {
            EVALS.fetch_add(1, Ordering::Relaxed);
            7
        }
        crate::dlog!("不该被求值 {}", bump());
        assert_eq!(
            EVALS.load(Ordering::Relaxed),
            0,
            "没有落点时 dlog! 连参数都不该求值"
        );
    }

    /// 重入闸：`debug` 在落点执行期间再被调用一次，必须被丢弃。
    ///
    /// 这条链子真实存在过：`dlog!` → sink(`app::perf_log`) →
    /// `GuiSettings::cache_dir()` → `cfg_prefix()` → `dlog!` → ……
    /// 无限递归，栈溢出（Windows 上是 0xC0000005，没有任何报错）。
    /// `settings::cfg_prefix` 里"不许有 `dlog!`"是结构性修复，这道闸
    /// 是兜底——所以它自己必须可靠，包括 sink 里 panic 的情况。
    #[test]
    fn reentry_into_the_sink_is_refused() {
        // 直接测闸门本身，而不是伪造一个落点：`SINK` 是 `OnceLock`，
        // 一旦注册就整个进程共享，测试之间会互相污染。
        let outer = enter().expect("第一次进入应当拿到守卫");
        assert!(
            enter().is_none(),
            "已经在 sink 里了，第二次进入必须被拒绝"
        );
        drop(outer);
        // 守卫析构后必须能重新进入——否则一次 panic 就把这条线程的
        // 日志永久静音了。
        assert!(enter().is_some(), "守卫析构后应当可以再次进入");
    }
}
