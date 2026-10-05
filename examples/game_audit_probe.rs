//! Probe: where does a **single-game** audit's wall clock actually go?
//!
//! `audit_game` looks cheap — it only opens the few archives that belong to one
//! game — but the surrounding steps are sized for the *whole library*:
//!
//!   * `audit_cache::load()`  reads and bincode-decodes the entire archive
//!     listing cache (every zip in the rompath, with every entry's name/size/
//!     crc) just so a handful of `list_cached` calls can hit it.
//!   * the unit enumeration `read_dir`s every rompath in full.
//!   * `audit_cache::prune` `stat`s every key in the cache.
//!   * `save_library` re-serialises all 49 676 games.
//!
//! This probe times each phase separately against the real rompath, so the
//! optimisation is aimed at the phase that actually dominates instead of the
//! one that looks expensive.
//!
//! Usage:
//!   cargo run --release --example game_audit_probe -- <gamelist.cache> <rompath>;<rompath>... <game>

use std::time::Instant;

fn secs(t: Instant) -> f64 {
    t.elapsed().as_secs_f64()
}

fn main() {
    let cache = std::env::args().nth(1).expect("gamelist.cache path");
    let rompaths: Vec<std::path::PathBuf> = std::env::args()
        .nth(2)
        .unwrap_or_default()
        .split(';')
        .map(std::path::PathBuf::from)
        .collect();
    let game = std::env::args().nth(3).unwrap_or_else(|| "gtmrusa".into());

    // ---- phase 0: the library itself ----
    let t = Instant::now();
    let data = mvui::core::cache::load(std::path::Path::new(&cache), "MAME v0.285 (unknown)").expect("load library");
    let mut lib = data.library;
    eprintln!("[0] library load           {:>7.3}s  ({} games)", secs(t), lib.len());

    // ---- phase 1: the scope, i.e. which games this audit touches ----
    let t = Instant::now();
    let scope = mvui::core::audit::audit_scope(&lib, &game).expect("scope");
    eprintln!(
        "[1] audit_scope            {:>7.3}s  ({} games in scope: {:?})",
        secs(t),
        scope.len(),
        scope.all()
    );

    // ---- phase 2: audit_cache::load (the whole archive listing cache) ----
    let t = Instant::now();
    mvui::core::audit_cache::load();
    eprintln!(
        "[2] audit_cache::load      {:>7.3}s  ({} cached archives)",
        secs(t),
        mvui::core::audit_cache::len()
    );

    // ---- phase 3: unit enumeration ----
    // 两种写法对着跑，好让优化前后的数字留在同一张表里比：
    //   old = 逐 rompath 做 `read_dir`，4.4 万个条目逐个 `is_dir()`
    //   new = `find_units_for`，按名字直接 `stat` 三种候选路径
    let gis = scope.all();
    let t = Instant::now();
    let wanted: std::collections::HashSet<String> =
        gis.iter().map(|&gi| lib.games[gi].name.to_lowercase()).collect();
    let mut old_units: Vec<(std::path::PathBuf, usize)> = Vec::new();
    for dir in &rompaths {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let stem = if p.is_dir() {
                p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
            } else {
                p.file_stem().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default()
            };
            if !wanted.contains(&stem) {
                continue;
            }
            if let Some(gi) = lib.get_idx(&stem) {
                if !old_units.iter().any(|(up, _)| up == &p) {
                    old_units.push((p, gi));
                }
            }
        }
    }
    eprintln!(
        "[3] read_dir enumeration   {:>7.3}s  ({} units matched)   <- OLD",
        secs(t),
        old_units.len()
    );

    let t = Instant::now();
    let new_units = mvui::core::audit::find_units_for(&lib, &gis, &rompaths);
    eprintln!(
        "    find_units_for         {:>7.3}s  ({} units matched)   <- NEW",
        secs(t),
        new_units.len()
    );
    // 命中集合必须完全一致，否则就不是等价替换而是改了行为
    let mut a: Vec<_> = old_units.iter().map(|(p, _)| p.clone()).collect();
    let mut b: Vec<_> = new_units.iter().map(|(p, _)| p.clone()).collect();
    a.sort();
    b.sort();
    assert_eq!(a, b, "新写法找到的归档集合与旧枚举不一致");

    // ---- phase 4: the actual archive scan (the part that is genuinely needed) ----
    let t = Instant::now();
    let handle = mvui::core::audit::AuditHandle::new();
    let n = mvui::core::audit::audit_game(&mut lib, &game, &rompaths, &handle);
    eprintln!("[4] audit_game (total)     {:>7.3}s  ({n} archives)", secs(t));

    // ---- phase 5: audit_cache::prune — `exists()` on every cached key ----
    let t = Instant::now();
    mvui::core::audit_cache::prune(&[], 60000);
    eprintln!("[5] audit_cache::prune     {:>7.3}s", secs(t));

    // ---- phase 6: save_library (re-serialise all 49 676 games) ----
    let t = Instant::now();
    let out = std::env::temp_dir().join("mvui-probe-gamelist.cache");
    let r = mvui::core::cache::save_library(&out, "MAME v0.285 (unknown)", &lib, true);
    eprintln!("[6] save_library           {:>7.3}s  ({:?})", secs(t), r.is_ok());
    let _ = std::fs::remove_file(&out);

    // ---- phase 7: audit_cache::save ----
    let t = Instant::now();
    mvui::core::audit_cache::save();
    eprintln!("[7] audit_cache::save      {:>7.3}s", secs(t));
}
