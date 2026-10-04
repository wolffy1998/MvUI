//! MvUI — a native Windows front-end for MAME.
//!
//! This is the library face of the crate: it exists so `examples/` and
//! integration tests can reach the domain layer. The application itself is the
//! `mvui` binary, whose entry point is `src/main.rs`.
//!
//! # Layout
//!
//! * [`core`] — the domain layer. MAME I/O, the rom audit, the option chain,
//!   archive access, the game-list cache. It must not depend on the UI; see
//!   [`core`]'s own docs for why that boundary is worth keeping.
//! * everything beside it — the egui front end. Those modules are private to
//!   the binary and intentionally not exposed here.

pub mod core;
