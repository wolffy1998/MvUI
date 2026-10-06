//! 只读诊断：rctycn 的 samples 段为什么是空的。
//!
//! 用法：cargo run --release --example samples_probe -- <gamelist.cache> <game>
//!
//! 沿「GameMeta.sampleof → lib.get_idx → RomInfoView.samples」逐段打印，
//! 定位到底哪一环断了。**不修改任何产品代码。**

fn main() {
    let cache = std::env::args()
        .nth(1)
        .expect("gamelist.cache path");
    let game = std::env::args().nth(2).unwrap_or_else(|| "rctycn".into());
    // 2026-10-06：FORMAT_VERSION 升到 5，老缓存（format 4）会被拒绝。
    // 探针改为直接吃 ，这样验证新解析不必先重建 40 秒缓存。
    let audited_flag;
    let lib = if std::path::Path::new(&cache).extension().is_some() {
        match mvui::core::cache::load(std::path::Path::new(&cache), "MAME v0.285 (unknown)") {
            Ok(d) => {
                println!("（读的是缓存，audited={}）", d.audited);
                audited_flag = d.audited;
                d.library
            }
            Err(e) => {
                println!("缓存不可用（{e}），改读 listxml");
                audited_flag = true;
                let f = std::fs::File::open(&cache).expect("open xml");
                let mut sink = |_d: usize| {};
                mvui::core::listxml::parse_from_reader(
                    std::io::BufReader::new(f),
                    false,
                    &mut sink,
                )
                .expect("parse")
            }
        }
    } else {
        let f = std::fs::File::open(&cache).expect("open xml");
        let mut sink = |_d: usize| {};
        println!("（读的是 listxml）");
        audited_flag = true;
        mvui::core::listxml::parse_from_reader(std::io::BufReader::new(f), false, &mut sink)
            .expect("parse")
    };
    println!("audited={} games={}", audited_flag, lib.games.len());

    // 真实样本目录（D://Game//MAME//MAME-0.284//samples）
    let sp = r"D://Game//MAME//MAME-0.284//samples";
    if std::path::Path::new(sp).is_dir() {
        mvui::core::samples::set_sample_dirs(vec![std::path::PathBuf::from(sp)]);
        println!("sample dirs= {}", sp);
    } else {
        println!("!!样本目录不存在: {sp}");
    }

    let Some(gi) = lib.get_idx(&game) else {
        println!("库里没有 {game}");
        return;
    };
    let g = &lib.games[gi];
    println!("\n== {game} ==");
    println!("  sampleof          = {:?}", g.sampleof);
    println!("  GameMeta.samples  = {} 条", g.samples.len());
    println!("  samples 前 5= {:?}", g.samples.iter().take(5).collect::<Vec<_>>());
    println!("  is_bios={} is_device={} is_mechanical={}", g.is_bios, g.is_device, g.is_mechanical);

    // 关键：sampleof 指向的机种在不在库里？
    if g.sampleof.is_empty() {
        println!("\n  !! sampleof 为空 —— 解析层或缓存没拿到这个属性");
    } else {
        match lib.get_idx(&g.sampleof) {
            None => println!("\n  !! get_idx({:?}) = None —— 样本机种不在库里", g.sampleof),
            Some(si) => {
                let s = &lib.games[si];
                println!("\n  get_idx({:?}) = Some({})", g.sampleof, si);
                println!("    样本机种 name={} roms={} is_bios={} is_device={}",
                    s.name, s.roms.len(), s.is_bios, s.is_device);
                for r in s.roms.iter().take(5) {
                    println!("      rom {}available={} nodump={} baddump={}",
                        r.name, r.available, r.is_nodump(), r.is_baddump());
                }
            }
        }
    }

    let v = mvui::core::rominfo::view_of(&lib, &game, audited_flag);
    println!("\n== view_of({game}) ==");
    println!("  roms={} disks={} bios={} devices={} slots={} slot_decls={} samples={}",
        v.roms.len(), v.disks.len(), v.bios.len(), v.devices.len(),
        v.slots.len(), v.slot_decls.len(), v.samples.len());
    for s in &v.samples {
        println!("    sample {} {:?}", s.name, s.state);
    }

    // 全库统计：有多少机种有 sampleof非空 / 有 samples / 能命中
    let mut with_sampleof = 0usize;
    let mut hit = 0usize;
    let mut with_samples_field = 0usize;
    for x in lib.games.iter() {
        if !x.sampleof.is_empty() {
            with_sampleof += 1;
            if lib.get_idx(&x.sampleof).is_some() {
                hit += 1;
            }
        }
        if !x.samples.is_empty() {
            with_samples_field += 1;
        }
    }
    println!("\n== 全库统计 (共 {} 个机种) ==", lib.games.len());
    println!("  sampleof 非空      {with_sampleof}");
    println!("  其中 get_idx 命中  {hit}");
    println!("  样本机种 get_idx 未命中 {}", with_sampleof - hit);
    println!("  GameMeta.samples 非空  {with_samples_field}");

    // 候选：样本包在磁盘上 + 有设备或槽位的机器，用来对照 UI 截图
    println!("\n== 候选（有样本集包+ 有 device/slot）==");
    let dirs = mvui::core::samples::sample_dirs().to_vec();
    let mut n = 0;
    for x in lib.games.iter() {
        if x.sampleof.is_empty() || x.samples.is_empty() { continue; }
        if mvui::core::samples::find_sample_archive(&dirs, &x.sampleof).is_none() { continue; }
        let v = mvui::core::rominfo::view_of(&lib, &x.name, audited_flag);
        if v.slots.is_empty() && v.slot_decls.is_empty() { continue; }
        println!("  {:<16} sampleof={:<14} samples={} devices={} slots={} slot_decls={}",
            x.name, x.sampleof, v.samples.len(), v.devices.len(), v.slots.len(), v.slot_decls.len());
        n += 1;
        if n >= 8 { break; }
    }

    // genpin 到底在不在库里
    match lib.get_idx("genpin") {
        Some(i) => println!("\n  lib.get_idx(\"genpin\") = Some({i}) name={}", lib.games[i].name),
        None => println!("\n  lib.get_idx(\"genpin\") = None"),
    }
}
