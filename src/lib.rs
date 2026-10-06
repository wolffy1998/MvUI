//! MvUI — a native Windows front-end for MAME.
//!
//! This is the library face of the crate: it exists so `examples/` and
//! integration tests can reach the domain layer. The application itself is the
//! `mvui` binary, whose entry point is `src/main.rs`.
//!
//! # Layout
//!
//! * [`core`] — the domain layer. MAME I/O, the rom verify, the option chain,
//!   archive access, the game-list cache. It must not depend on the UI; see
//!   [`core`]'s own docs for why that boundary is worth keeping.
//! * everything beside it — the egui front end. Those modules are private to
//!   the binary and intentionally not exposed here.

pub mod core;

// 图标资产表（`build.rs` 扫 `assets/icons/**`生成的 `ICONS`）。
//
// **必须从 lib 导出**，否则 `examples/` 里的探针查不了"某个图标名到底在不在
// 资产表里" —— 而这个名字写错是**运行期静默失败**（图标不画，不报错），
// 编译期一个错都不报。没有可访问的表就只能靠肉眼看截图。
pub mod icons;
