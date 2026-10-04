//! Micro-bench: list_archive cost on 7z variants (solid / non-solid /
//! compressed header) vs a real MAME game zip. Also checks CRC presence —
//! the audit matcher skips entries without a CRC.
//!
//! Usage: cargo run --release --example sevenz_probe -- <dir-with-7zs> <control-zip>

use mvui::core::archive::list_archive;
use std::time::Instant;

fn bench(path: &std::path::Path, label: &str, iters: usize) {
    // 首次调用（冷，含磁盘寻道与头部解码）
    let t = Instant::now();
    let entries = match list_archive(path) {
        Ok(e) => e,
        Err(e) => {
            println!("{:<26} FAILED: {}", label, e);
            return;
        }
    };
    let cold = t.elapsed().as_secs_f64();
    let crcs = entries.iter().filter(|e| e.crc.is_some()).count();

    let mut total = 0.0f64;
    for _ in 0..iters {
        let t = Instant::now();
        let _ = list_archive(path).unwrap();
        total += t.elapsed().as_secs_f64();
    }
    println!(
        "{:<26} {:>3} entries  crc {}/{}  cold {:>6.2}ms  warm {:>6.2}ms/包",
        label,
        entries.len(),
        crcs,
        entries.len(),
        cold * 1000.0,
        total * 1000.0 / iters as f64
    );
}

fn main() {
    let dir = std::env::args().nth(1).expect("dir with 7z files");
    let control = std::env::args().nth(2);

    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("7z")).unwrap_or(false))
        .collect();
    paths.sort();
    for p in &paths {
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        bench(p, &name, 50);
    }
    if let Some(c) = control {
        bench(std::path::Path::new(&c), "control: 真实游戏 zip", 50);
    }
}
