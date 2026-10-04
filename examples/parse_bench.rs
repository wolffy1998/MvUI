//! Benchmark: MvUI's listxml parse chain WITHOUT the ROM audit, phase by
//! phase, plus a bincode serialization buffering comparison.
//!
//! Usage: cargo run --release --example parse_bench -- <path-to-listxml.xml>

use std::io::{BufReader, Write};
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("usage: parse_bench <listxml.xml>");
    let mut total = 0.0f64;

    let t = Instant::now();
    let bytes = std::fs::read(&path).expect("read listxml file");
    let io = t.elapsed().as_secs_f64();
    total += io;
    println!("1. 磁盘读取 320MB          {:>8.2}s", io);

    let t = Instant::now();
    let mut lib =
        mvui::core::listxml::parse_from_reader(BufReader::new(&bytes[..]), false, &mut |_| {})
            .expect("parse");
    let parse = t.elapsed().as_secs_f64();
    total += parse;
    println!("2. XML 解析               {:>8.2}s  ({} machines)", parse, lib.games.len());

    let t = Instant::now();
    lib.rebuild_indexes();
    let idx = t.elapsed().as_secs_f64();
    total += idx;
    println!("3. rebuild_indexes        {:>8.2}s", idx);

    let t = Instant::now();
    lib.complete_data();
    let cd = t.elapsed().as_secs_f64();
    total += cd;
    println!("4. complete_data          {:>8.2}s", cd);

    let t = Instant::now();
    let cache = mvui::core::folders::compute_folder_cache(&lib, false);
    let fc = t.elapsed().as_secs_f64();
    total += fc;
    println!("5. compute_folder_cache   {:>8.2}s  ({} roots)", fc, cache.roots.len());
    drop(cache);

    // ---- bincode 写盘对比：生产路径 cache.rs 是无缓冲 serialize_into ----
    let tmp1 = std::env::temp_dir().join("mvui-bench-unbuf.bin");
    let t = Instant::now();
    {
        let f = std::fs::File::create(&tmp1).unwrap();
        bincode::serialize_into(&mut &f, &lib).unwrap();
        f.sync_all().ok();
    }
    let unbuf = t.elapsed().as_secs_f64();
    let size = std::fs::metadata(&tmp1).map(|m| m.len()).unwrap_or(0);
    println!("6a. bincode 无缓冲写+fsync {:>8.2}s  (cache {:.0} MB)  ← 生产路径写法", unbuf, size as f64 / 1e6);

    let tmp2 = std::env::temp_dir().join("mvui-bench-buf.bin");
    let t = Instant::now();
    {
        let f = std::fs::File::create(&tmp2).unwrap();
        let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
        bincode::serialize_into(&mut w, &lib).unwrap();
        w.flush().unwrap();
    }
    let buf = t.elapsed().as_secs_f64();
    let size2 = std::fs::metadata(&tmp2).map(|m| m.len()).unwrap_or(0);
    println!("6b. bincode BufWriter(1MB) {:>8.2}s  (cache {:.0} MB)", buf, size2 as f64 / 1e6);

    let t = Instant::now();
    let raw = std::fs::read(&tmp2).unwrap();
    let lib2: mvui::core::library::GameLibrary = bincode::deserialize_from(&raw[..]).unwrap();
    let ld = t.elapsed().as_secs_f64();
    println!("7. cache 全读+反序列化      {:>8.2}s  ({} games)", ld, lib2.games.len());
    let _ = std::fs::remove_file(&tmp1);
    let _ = std::fs::remove_file(&tmp2);
    drop(lib2);

    println!("\n冷启动解析链 (1..5 合计):    {:>8.2}s", total);
}
