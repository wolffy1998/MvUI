//! Probe: time MvUI's ROM verify against the real rompath, phase by phase.
//!
//! Usage: cargo run --release --example verify_probe -- <listxml.xml> <rompath>;<rompath>...

use std::io::BufReader;
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("listxml path");
    let rompaths: Vec<std::path::PathBuf> = std::env::args()
        .nth(2)
        .unwrap_or_default()
        .split(';')
        .map(std::path::PathBuf::from)
        .collect();

    let t = Instant::now();
    let f = std::fs::File::open(&path).expect("open listxml");
    let mut lib = mvui::core::listxml::parse_from_reader(BufReader::new(f), false, &mut |_| {})
        .expect("parse");
    lib.rebuild_indexes();
    lib.complete_data();
    eprintln!("[probe] lib ready in {:.1}s ({} games)", t.elapsed().as_secs_f64(), lib.games.len());

    // ---- 单元收集（与 verify.rs 117-143 相同的口径）----
    let t = Instant::now();
    let mut n_dir = 0usize;
    let mut n_zip = 0usize;
    let mut n_7z = 0usize;
    let mut units: Vec<(std::path::PathBuf, usize)> = Vec::new();
    for dir in &rompaths {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                if let Some(gi) = lib.get_idx(&name) {
                    units.push((p, gi));
                    n_dir += 1;
                }
            } else if mvui::core::archive::is_zip(&p) {
                let name = p.file_stem().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                if let Some(gi) = lib.get_idx(&name) {
                    units.push((p, gi));
                    n_zip += 1;
                }
            } else if mvui::core::archive::is_7z(&p) {
                let name = p.file_stem().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                if let Some(gi) = lib.get_idx(&name) {
                    units.push((p, gi));
                    n_7z += 1;
                }
            }
        }
    }
    eprintln!(
        "[probe] 单元收集 {:.2}s：{} 个单元（dir {} / zip {} / 7z {}），其他文件 {} 个",
        t.elapsed().as_secs_f64(),
        units.len(),
        n_dir,
        n_zip,
        n_7z,
        0
    );

    // ---- list_archive 微基准：每类抽前 N 个测单包成本 ----
    for (label, want, n) in [("zip", true, 400usize), ("7z", false, 100usize)] {
        let mut sampled = 0usize;
        let t = Instant::now();
        for (p, _) in &units {
            let hit = if want { mvui::core::archive::is_zip(p) } else { mvui::core::archive::is_7z(p) };
            if !hit {
                continue;
            }
            let _ = mvui::core::archive::list_archive(p);
            sampled += 1;
            if sampled >= n {
                break;
            }
        }
        let el = t.elapsed().as_secs_f64();
        if sampled > 0 {
            eprintln!(
                "[probe] list_archive({label}) 采样 {} 个：{:.3}s，平均 {:.2}ms/包",
                sampled,
                el,
                el * 1000.0 / sampled as f64
            );
        } else {
            eprintln!("[probe] list_archive({label}) 无样本");
        }
    }

    // ---- 完整审计 + 看门狗每 10s 打进度 ----
    let handle = mvui::core::verify::VerifyHandle::new();
    let h2 = handle.clone();
    let wd = std::thread::spawn(move || {
        let start = Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let (done, total, cur) = h2.snapshot();
            eprintln!(
                "[watchdog {:>6.1}s] {done}/{total} {}",
                start.elapsed().as_secs_f64(),
                cur
            );
            if h2.is_finished() {
                break;
            }
        }
    });

    let t = Instant::now();
    mvui::core::verify::verify_all(&mut lib, &rompaths, &std::collections::HashMap::new(), &handle);
    let verify = t.elapsed().as_secs_f64();
    let complete = lib.games.iter().filter(|g| g.available == 1).count();
    eprintln!("[probe] verify_all 总耗时 {:.2}s，审计后可用 {} 套", verify, complete);
    handle.finish();
    let _ = wd.join();
}
