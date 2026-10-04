//! Micro-bench: split list_archive's per-zip cost into open vs central-directory.
//! Usage: cargo run --release --example open_probe -- <rom-dir> [sample]

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
    println!("sampled {} zips", sample.len());

    // pass 1: open only
    let t = Instant::now();
    for p in &sample {
        std::fs::File::open(p).unwrap();
    }
    let open1 = t.elapsed().as_secs_f64();
    println!("pass1 File::open          {:>7.2}ms/zip", open1 * 1000.0 / sample.len() as f64);

    // pass 2: open + ZipArchive::new (CD parse), second pass = OS cache warm
    let t = Instant::now();
    for p in &sample {
        let f = std::fs::File::open(p).unwrap();
        let z = zip::ZipArchive::new(f).unwrap();
        let _ = z.len();
    }
    let cd1 = t.elapsed().as_secs_f64();
    println!("pass2 open + 中央目录      {:>7.2}ms/zip", cd1 * 1000.0 / sample.len() as f64);

    let t = Instant::now();
    for p in &sample {
        let f = std::fs::File::open(p).unwrap();
        let z = zip::ZipArchive::new(f).unwrap();
        let _ = z.len();
    }
    let cd2 = t.elapsed().as_secs_f64();
    println!("pass3 同上（OS 缓存热）    {:>7.2}ms/zip", cd2 * 1000.0 / sample.len() as f64);
}
