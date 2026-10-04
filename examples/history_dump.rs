//! Dump one history.xml record as the document panel would render it.
//!
//! Usage: history_dump <history.xml> <game>

use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(game)) = (args.next(), args.next()) else {
        eprintln!("usage: history_dump <history.xml> <game>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(&path).expect("read");
    let idx = mvui::core::historyxml::HistoryXmlIndex::build(&bytes);
    let Some((s, e)) = idx.range_of(&game) else {
        println!("MISS {game}");
        return;
    };
    let payload = String::from_utf8_lossy(&bytes[s..e]);
    println!("=== RAW PAYLOAD ({} B) ===", e - s);
    println!("{payload}");
    println!("=== RENDERED ===");
    println!("{}", mvui::core::historyxml::render_text(&payload));
    let _ = Path::new(&path);
}
