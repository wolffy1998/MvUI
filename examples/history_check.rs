//! Probe the history.xml index without opening a window.
//!
//! Usage:
//!   history_check <path-to-history.xml> [game ...]
//!
//! With no game names it reports the index size and a few sample lookups.

use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: history_check <history.xml> [game ...]");
        std::process::exit(2);
    };
    let path = Path::new(&path);
    let games: Vec<String> = args.collect();

    let t0 = std::time::Instant::now();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {} failed: {e}", path.display());
            std::process::exit(1);
        }
    };
    let read_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let t1 = std::time::Instant::now();
    let idx = mvui::core::historyxml::HistoryXmlIndex::build(&bytes);
    let build_ms = t1.elapsed().as_secs_f64() * 1000.0;

    println!("file       : {} ({} bytes)", path.display(), bytes.len());
    println!("read       : {read_ms:.0} ms");
    println!("build      : {build_ms:.0} ms");
    println!("entries    : {}", idx.entry_count());
    println!("names      : {}", idx.name_count());

    let probes: Vec<String> = if games.is_empty() {
        vec!["88games".into(), "pacman".into(), "karnov".into(), "100mandk".into()]
    } else {
        games
    };
    for g in probes {
        let t2 = std::time::Instant::now();
        let r = idx.range_of(&g);
        let us = t2.elapsed().as_micros();
        match r {
            Some((s, e)) => {
                let payload = String::from_utf8_lossy(&bytes[s..e]);
                let rendered = mvui::core::historyxml::render_text(&payload);
                println!(
                    "  {g:<12} HIT  range={s}..{e} ({} B) {us} us  rendered {} B, {} lines",
                    e - s,
                    rendered.len(),
                    rendered.matches("<br>").count()
                );
            }
            None => println!("  {g:<12} MISS  {us} us"),
        }
    }
}
