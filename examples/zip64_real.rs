// Verify `core::zip64` against the real 6 GB artwork pack.
//
//     cargo run --release --example zip64_real -- <zip> <set> [set...]
//
// `zip::ZipArchive` takes 4 min 42 s on this file and then fails with
// "No CDFH found"; this measures the replacement on the same bytes.

use std::time::Instant;

use mvui::core::zip64;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: zip64_real <zip> <set>...");
    let sets: Vec<String> = args.collect();
    if sets.is_empty() {
        eprintln!("give at least one set name");
        std::process::exit(2);
    }

    let p = std::path::Path::new(&path);
    let t = Instant::now();
    let dir = zip64::read_directory(p).expect("central directory must be readable");
    println!(
        "read_directory      {:8.1} ms   ({} entries)",
        t.elapsed().as_secs_f64() * 1000.0,
        dir.entries.len()
    );

    let mut all_ok = true;
    for set in &sets {
        let name = format!("{set}.png");
        let t = Instant::now();
        match zip64::read_entry(p, &dir, &name) {
            Some(b) => {
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                let decoded = image::load_from_memory(&b)
                    .map(|i| format!("{}x{}", i.width(), i.height()))
                    .unwrap_or_else(|e| format!("DECODE FAILED: {e}"));
                println!("  {name:<24} {ms:8.1} ms  {} bytes  {decoded}", b.len());
                if decoded.starts_with("DECODE FAILED") {
                    all_ok = false;
                }
            }
            None => {
                println!("  {name:<24} MISS");
                all_ok = false;
            }
        }
    }
    if !all_ok {
        std::process::exit(1);
    }
}
