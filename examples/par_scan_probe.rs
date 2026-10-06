//! Micro-bench: is the verify IO-bound in a way that multithreading helps?
//! Same `list_archive` workload, sequential (1 thread) vs N threads, over two
//! disjoint samples of the real rompath.
//!
//! Usage: cargo run --release --example par_scan_probe -- <rom-dir> [per-sample]
//!
//! **The answer on a spinning disk is "no", and that is why `verify.rs` is
//! sequential.** Measured here: ~52 zips/s with one thread vs ~37 zips/s with
//! four — four readers make the head seek between them and every unit pays for
//! it. Run this again before reintroducing parallelism, and run it against the
//! storage you actually care about: on an SSD the answer can flip, and this
//! probe is the cheap way to check rather than guessing.
//!
//! (It used to depend on `rayon`; that dependency was dropped from the build
//! when the verify went sequential, so the threads here are plain `std::thread`
//! and the probe stays runnable.)

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let dir = std::env::args().nth(1).expect("rom dir");
    let per: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let threads: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);

    let mut zips: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .expect("read dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|x| x.eq_ignore_ascii_case("zip"))
                .unwrap_or(false)
        })
        .collect();
    zips.sort();
    if zips.len() < per * 2 {
        panic!("need at least {} zips", per * 2);
    }
    let head = &zips[..per]; // 字母序前段
    let tail = &zips[zips.len() - per..]; // 字母序后段（不相交）

    let t = Instant::now();
    for p in head {
        let _ = mvui::core::archive::list_archive(p);
    }
    let seq = t.elapsed().as_secs_f64();
    println!(
        "sequential (1 线程)  {:>4} zips  {:>6.1}s  = {:>6.0} zips/s",
        head.len(),
        seq,
        head.len() as f64 / seq
    );

    // Interleaved by index so each worker gets a spread across the directory
    // rather than one contiguous run — that is what makes the seek pattern
    // realistic (and, on a spinning disk, what makes it lose).
    let next = Arc::new(AtomicUsize::new(0));
    let t = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..threads {
            let next = next.clone();
            s.spawn(move || loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= tail.len() {
                    break;
                }
                let _ = mvui::core::archive::list_archive(&tail[i]);
            });
        }
    });
    let par = t.elapsed().as_secs_f64();
    println!(
        "threads ({threads} 线程)    {:>4} zips  {:>6.1}s  = {:>6.0} zips/s",
        tail.len(),
        par,
        tail.len() as f64 / par
    );
    println!(
        "并行加速比: {:.2}x  (<1.00 = 并行更慢)",
        (tail.len() as f64 / par) / (head.len() as f64 / seq)
    );
}
