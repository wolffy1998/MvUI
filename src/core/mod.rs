//! 领域层：MvUI 里所有"不画界面"的部分。
//!
//! 这一层曾经是独立的 `the legacy core crate` crate，现在是模块，所以分层
//! 是**约定**而不是构建系统强制的（改名前的老注释里提到的
//! `crates/the legacy core crate/` 早已不存在）：
//!
//! * **`core/` 不得依赖 UI。** 这里不允许出现 `egui`、`eframe`、`rfd`
//!   这些类型——它们属于旁边的模块。正因为守住了这条线，MAME 进程
//!   调用、rom 审计、选项链、归档访问、游戏库缓存才能在**没有窗口**
//!   的情况下被测试。
//! * **UI 侧可以用任何东西。** `app.rs`、`views.rs` 们通过
//!   `core::mameproc` 调 MAME、通过 `core::dat` 读 DAT，不受限制。
//!
//! 这条线值得守住：一旦某个改动开始需要把 egui 类型传进这一层，
//! 那就是信号——它该待在另外半边。
//!
//! 日志走 [`log`] 模块：core 不能直接调 UI 的 `perf_log`（那会反向
//! 依赖 UI），只能往那个 sink 里写，由 UI 决定最终落到哪。

pub mod archive;
pub mod audit;
pub mod audit_cache;
pub mod cache;
pub mod dat;
pub mod datindex;
pub mod folders;
pub mod historyxml;
pub mod icons;
pub mod launcher;
pub mod library;
pub mod listxml;
pub mod log;
pub mod lst;
pub mod mameproc;
pub mod model;
pub mod options;
pub mod paths;
pub mod settings;
pub mod zip64;
