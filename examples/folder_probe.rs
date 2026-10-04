//! Probe: parse a listxml dump and print MvUI's own folder-cache tallies,
//! plus the status-bar (visible-list) count for the AllArc root.
//!
//! Usage: cargo run --release --example folder_probe -- <path-to-listxml.xml>

use std::io::BufReader;

fn main() {
    let path = std::env::args().nth(1).expect("usage: folder_probe <listxml.xml>");
    let f = std::fs::File::open(&path).expect("open listxml");
    let t = std::time::Instant::now();
    let mut lib = mvui::core::listxml::parse_from_reader(BufReader::new(f), false, &mut |_| {})
        .expect("parse");
    lib.rebuild_indexes();
    lib.complete_data();
    println!("parsed in {:.1}s", t.elapsed().as_secs_f64());
    println!("lib.games total: {}", lib.games.len());
    println!(
        "devices: {}  bios: {}  consoles(softwarelists): {}",
        lib.games.iter().filter(|g| g.is_device).count(),
        lib.games.iter().filter(|g| g.is_bios).count(),
        lib.games.iter().filter(|g| g.is_console()).count(),
    );
    let cache = mvui::core::folders::compute_folder_cache(&lib, false);
    for r in &cache.roots {
        println!("root {:<16} {}", r.label, r.count);
    }
    // 状态栏口径：AllArc + 无过滤 + refilter 的 is_device 剔除
    let visible: usize = lib
        .games
        .iter()
        .filter(|g| !g.is_device && !g.is_bios && !g.is_ext_rom && !g.is_console())
        .count();
    println!("status-bar口径（全部街机 visible）: {visible}");
}
