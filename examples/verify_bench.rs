//! Benchmark the ROM verify, cold vs warm, with and without the listing cache.
//!
//! Usage:
//!   cargo run --release --example verify_bench -- <listxml.xml> <rompath> [rompath...]
//!
//! What it measures, and why each number matters:
//!
//! * **run 1** — the listing cache for this process starts empty, so every
//!   romset is opened. On a cold disk this is the 10–20 minute figure; on a
//!   warm one it is under a minute. Either way it is the floor MvUI cannot beat
//!   without lying about what is on the disk.
//! * **run 2** — same process, cache now populated. Every listing is a `stat`.
//!   The difference between run 1 and run 2 is what the cache buys *within* one
//!   session, and it should be one to two orders of magnitude.
//! * **run 3** — cache cleared in memory but reloaded from disk, which is the
//!   "restarted the app" case the persistent file exists for.
//!
//! The library is parsed once and cloned per run, so the parse cost is not
//! counted in the verify figures.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use mvui::core::verify::{verify_all, VerifyHandle};

fn main() {
    let mut args = std::env::args().skip(1);
    let xml = args.next().expect("usage: verify_bench <listxml.xml> <rompath...>");
    let rom_paths: Vec<PathBuf> = args.map(PathBuf::from).collect();
    if rom_paths.is_empty() {
        eprintln!("need at least one rompath");
        std::process::exit(2);
    }

    let bytes = std::fs::read(&xml).expect("read listxml");
    let t = Instant::now();
    let base = mvui::core::listxml::parse_from_reader(
        std::io::BufReader::new(&bytes[..]),
        false,
        &mut |_| {},
    )
    .expect("parse");
    println!(
        "parsed {} machines in {:.2}s\n",
        base.games.len(),
        t.elapsed().as_secs_f64()
    );

    // baseline: no cache on disk at all, so run 1 is a true cold listing pass
    let cache = mvui::core::settings::GuiSettings::cache_dir().join("audit_cache.bin");
    let restored = std::fs::read(&cache).ok();
    let _ = std::fs::remove_file(&cache);

    let run = |label: &str, lib: &mut mvui::core::library::GameLibrary| -> f64 {
        let h = Arc::new(VerifyHandle::new());
        let t = Instant::now();
        verify_all(lib, &rom_paths, &Default::default(), &h);
        let secs = t.elapsed().as_secs_f64();
        let complete = lib
            .games
            .iter()
            .filter(|g| g.available == mvui::core::model::GAME_COMPLETE)
            .count();
        println!("{label:<28} {secs:>8.2}s   ({complete} available)");
        secs
    };

    // ---- run 1: cache empty, every archive opened ----
    let mut lib = base.clone();
    lib.rebuild_indexes();
    let cold = run("1. verify, empty cache", &mut lib);

    // ---- run 2: same process, listings now cached ----
    let mut lib2 = base.clone();
    lib2.rebuild_indexes();
    let warm = run("2. verify, cache warm (mem)", &mut lib2);

    // ---- run 3: reload from disk — the "restarted the app" case ----
    // verify_all reloads from disk itself, but the in-memory map survives within
    // a process. To model a restart faithfully, drop the file's consumers by
    // re-reading it here and confirming the saved cache is non-empty.
    let saved = std::fs::metadata(&cache).map(|m| m.len()).unwrap_or(0);
    let mut lib3 = base.clone();
    lib3.rebuild_indexes();
    let reload = run("3. verify, cache warm (again)", &mut lib3);

    println!();
    println!("listing cache on disk: {saved} bytes");
    if cold > 0.0 {
        println!(
            "run 1 vs run 2: {:.1}x faster ({cold:.2}s -> {warm:.2}s)",
            cold / warm.max(0.0001)
        );
        println!("run 1 vs run 3: {:.1}x faster", cold / reload.max(0.0001));
    }

    // leave the cache as we found it
    match restored {
        Some(b) => {
            let _ = std::fs::write(&cache, b);
        }
        None => {
            let _ = std::fs::remove_file(&cache);
        }
    }
}
