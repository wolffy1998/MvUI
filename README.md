# MvUI

A native Windows front-end for [MAME](https://www.mamedev.org/), written in Rust
with [egui](https://github.com/emilk/egui). It replaces the Qt/C++ **mamepgui
1.8.2** GUI: same feature set, same configuration semantics, modern rendering.

<!-- docs/ANALYSIS-mamepgui-1.8.2.md is the analysis of the original codebase and
     is the behaviour reference. docs/DESIGN.md is the redesign document.
     docs/WORKSPACE.md documents the Rust workspace itself. -->

## Status

Rewritten feature-for-feature against the 1.8.2 reference. `cargo check
--workspace` is warning-free and `cargo test --workspace` passes 27 tests
(21 core + 6 app).

**Not yet ported:** IPS/patch and M1 support (deliberately deferred — see
`[package.metadata.notes]` in the app's `Cargo.toml`).

## Build

```sh
cd mamegui-rs
cargo build --release      # → mamegui-rs/target/release/mvui.exe
```

Requires a Rust toolchain. On Windows the executable icon additionally needs
MinGW's `windres` on `PATH` (`%USERPROFILE%\scoop\apps\mingw\current\bin`);
without it the build still succeeds and only prints a `cargo:warning`.

## Layout

```
├── README.md              this file: overview + build
├── docs/
│   ├── ANALYSIS-…1.8.2.md  how the original Qt/C++ GUI works
│   ├── DESIGN.md           the redesign document (design intent, §-referenced)
│   ├── WORKSPACE.md        the Rust workspace in detail
│   └── OPTIMIZATION.md     change log
├── icon-designs/          app icon sources
├── mamegui-rs/            the Rust workspace
│   ├── crates/mamegui-core/   pure logic, no UI: MAME I/O, rom audit, options
│   ├── crates/mamegui-app/    egui/eframe front-end (binary: `mvui`)
│   └── assets/                icons, optiontemplate.xml, backgrounds
└── .workbuddy/            development notes and screenshot tooling
```

`mamegui-core` holds everything that is not drawing: reading `listxml`, the rom
audit, the option chain, archive access, the game-list cache. `mamegui-app` only
draws and dispatches. That split is what makes the logic testable without a
window.

## Design notes

* **The original is the specification.** Where behaviour is unclear, the answer
  is in the 1.8.2 sources (`audit.cpp`, `gamelist.cpp`, `prototype.cpp`,
  `mainwindow.cpp`, `utils.cpp`), not in intuition. Ported code carries an
  `origin:` comment naming the upstream function and line.
* **Configuration is layered**, matching MAME: `mame.ini` (global) →
  `ini/source/<set>.ini` → BIOS → clone-of, plus a GUI override set. The
  override only applies to options marked `guivisible="1"`.
* **Blocking work never runs on the UI thread.** The game list, audit, preview
  and icon loads all go through background jobs that post events back.
* **The game list is virtualised** — `egui_extras`' table only builds visible
  rows, so ~40 000 machines load without a frame-time cliff.

## Security

Audited and hardened:

* **Zip Slip** — archive entry names and `.dat` rom names are external input.
  `core::archive::sanitized_join` refuses any path that would land outside the
  extraction directory (traversal, absolute, drive-relative and UNC forms), and
  the extraction is skipped rather than redirected. Covered by 5 tests.
* **Allocation DoS** — archive headers declare uncompressed sizes. Icon
  extraction no longer pre-allocates from that value (`MAX_ICON_BYTES`), so a
  crafted `icons.zip` cannot abort the process before a byte is read.
* MAME is launched through `Command::arg` (never a shell), so no argument can be
  interpreted as a command.

## Licence

The original mamepgui 1.8.2 is GPL-2.0; this rewrite follows suit.
