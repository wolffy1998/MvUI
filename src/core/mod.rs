//! The domain layer: everything MvUI does that is not drawing.
//!
//! This was the `mamegui-core` crate. It is a module rather than a separate
//! package now, so the layering is a convention rather than something the build
//! system enforces:
//!
//! * **`core/` must not depend on the UI.** No `egui`, no `eframe`, no
//!   `rfd` here — those belong to the modules beside this one. It is what makes
//!   the logic testable without a window, and it is why the whole of MAME I/O,
//!   the rom audit, the option chain, archive access and the game-list cache
//!   live on this side of the line.
//! * **The UI side may use anything.** `app.rs`, `views.rs` and friends talk to
//!   MAME through `core::mameproc`, read DATs through `core::dat`, and so on.
//!
//! The line is worth keeping: when a change starts needing egui types down
//! here, that is the signal it belongs in the other half.

pub mod archive;
pub mod audit;
pub mod cache;
pub mod dat;
pub mod datindex;
pub mod folders;
pub mod icons;
pub mod launcher;
pub mod library;
pub mod listxml;
pub mod mameproc;
pub mod model;
pub mod options;
pub mod settings;
