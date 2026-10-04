//! Diagnostic: exercise the boot cache path headlessly.
//!
//! Usage: cargo run --release --example cache_check -- [<mame.exe>]
//!
//! Reproduces exactly what `background::boot_run` does with the cached library
//! — `cache::load` against the version the current `detect()` reports — without
//! creating a window. Useful for telling "the GUI never started" apart from
//! "the cache load is broken", which the boot trace alone cannot distinguish.

use std::path::Path;

fn main() {
    let mame = std::env::args()
        .nth(1)
        .unwrap_or_else(|| r"D:\Game\MAME\MAME-0.284\mame.exe".to_string());
    println!("mame: {mame}");

    let version = match mvui::core::mameproc::MameBinary::detect(Path::new(&mame)) {
        Ok(m) => m.version,
        Err(e) => {
            println!("detect FAILED: {e}");
            return;
        }
    };
    println!("detected version: {version:?}");

    let path = mvui::core::settings::GuiSettings::cache_dir().join("gamelist.cache");
    println!("cache path:       {}", path.display());
    match std::fs::metadata(&path) {
        Ok(m) => println!("cache size:       {} bytes", m.len()),
        Err(e) => println!("cache stat:       {e}"),
    }

    let t = std::time::Instant::now();
    match mvui::core::cache::load(&path, &version) {
        Ok(data) => {
            println!(
                "load OK in {:.2?}: audited={} games={} cached_version={:?}",
                t.elapsed(),
                data.audited,
                data.library.games.len(),
                data.mame_version
            );
        }
        Err(e) => println!("load ERR in {:.2?}: {e}", t.elapsed()),
    }
}
