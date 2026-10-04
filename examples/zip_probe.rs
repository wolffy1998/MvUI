//! Micro-bench: where does list_archive's per-zip cost go?
//!
//! (a) ZipArchive::new only      — central directory parse
//! (b) + by_index_raw loop       — proposed fix (CD-only, no local header reads)
//! (c) + by_index loop           — current production code (archive.rs::list_archive)
//!
//! Usage: cargo run --release --example zip_probe -- <rom-dir> [sample]

use std::fs::File;
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).expect("rom dir");
    let sample_n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(300);

    let mut zips: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("zip")).unwrap_or(false))
        .collect();
    zips.sort();
    let step = (zips.len() / sample_n).max(1);
    let sample: Vec<_> = zips.into_iter().step_by(step).take(sample_n).collect();
    println!("sampled {} zips from {}", sample.len(), dir);

    let t = Instant::now();
    let mut entries = 0usize;
    for p in &sample {
        let f = File::open(p).unwrap();
        let z = zip::ZipArchive::new(f).unwrap();
        entries += z.len();
    }
    let a = t.elapsed().as_secs_f64();
    println!(
        "(a) ZipArchive::new        {:>7.1}ms/zip  (avg {} entries/zip)",
        a * 1000.0 / sample.len() as f64,
        entries / sample.len().max(1)
    );

    let t = Instant::now();
    for p in &sample {
        let f = File::open(p).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        for i in 0..z.len() {
            let _ = z.by_index_raw(i).unwrap();
        }
    }
    let b = t.elapsed().as_secs_f64();
    println!("(b) + by_index_raw loop    {:>7.1}ms/zip  ← 建议改法", b * 1000.0 / sample.len() as f64);

    let t = Instant::now();
    for p in &sample {
        let f = File::open(p).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        for i in 0..z.len() {
            let _ = z.by_index(i).unwrap();
        }
    }
    let c = t.elapsed().as_secs_f64();
    println!("(c) + by_index loop        {:>7.1}ms/zip  ← 现行 list_archive", c * 1000.0 / sample.len() as f64);
}
