//! 只读诊断：某台机种的 samples 段为什么是那个状态。
//!
//! 用法：
//!   cargo run --release --example samples_probe -- <gamelist.cache|listxml> [game]
//!   cargo run --release --example samples_probe -- <gamelist.cache|listxml> [game] <samplepath>
//!
//! **走的是引导同一条路**（2026-06 起）：先 `set_sample_dirs` +
//! `scan_sample_sets`（`verify_all` 开头做的事），再 `view_of`。手工绕过
//! 扫描直接问磁盘的话，验的就不是产品代码走的那条路了 —— 2026-10-06那个
//! 「探针 18/18 Good 而程序全灰」就是手工 `set_sample_dirs` 造成的。
//!
//! 不修改任何产品代码。

fn main() {
    let arg1 = std::env::args().nth(1).expect("用法: samples_probe <cache|listxml> [game] [samplepath]");
    let game = std::env::args().nth(2).unwrap_or_else(|| "rctycn".into());
    // 样本目录：第三个参数给真实目录，不给就用 MAME 0.284 的标准位置
    let sp = std::env::args().nth(3).unwrap_or_else(|| {
        "D:\\Game\\MAME\\MAME-0.284\\samples".to_string()
    });

    let verified_flag;
    let lib = if std::path::Path::new(&arg1).extension().is_some() {
        match mvui::core::cache::load(std::path::Path::new(&arg1), "MAME v0.285 (unknown)") {
            Ok(d) => {
                println!("（读的是缓存，verified={}）", d.verified);
                verified_flag = d.verified;
                d.library
            }
            Err(e) => {
                println!("缓存不可用（{e}），改读 listxml");
                verified_flag = true;
                parse_xml(&arg1)
            }
        }
    } else {
        verified_flag = true;
        println!("（读的是 listxml）");
        parse_xml(&arg1)
    };
    println!("verified={} games={}", verified_flag, lib.games.len());

    // ---- 引导同一条路：发布目录 → 扫一次 ----
    let spath = std::path::PathBuf::from(&sp);
    if spath.is_dir() {
        mvui::core::samples::set_sample_dirs(vec![spath.clone()]);
        println!("samplepath = {sp}");
    } else {
        println!("!! 样本目录不存在: {sp}（会扫到 0 个）");
        mvui::core::samples::set_sample_dirs(vec![spath.clone()]);
    }
    let dirs = mvui::core::samples::sample_dirs();
    let n = mvui::core::samples::scan_sample_sets(&dirs);
    let sets = mvui::core::samples::sample_sets();
    println!("扫到样本集 {n} 个，前 10: {:?}", &sets[..sets.len().min(10)]);

    let Some(gi) = lib.get_idx(&game) else {
        println!("库里没有 {game}");
        return;
    };
    let g = &lib.games[gi];
    println!("\n== {game} ==");
    println!("  sampleof         = {:?}", g.sampleof);
    println!("  GameMeta.samples = {} 条", g.samples.len());
    println!("  samples 前 5= {:?}", g.samples.iter().take(5).collect::<Vec<_>>());

    let v = mvui::core::rominfo::view_of(&lib, &game, verified_flag);
    println!("\n== view_of({game}) ==");
    println!(
        "  roms={} disks={} bios={} devices={} samples={}",
        v.roms.len(),
        v.disks.len(),
        v.bios.len(),
        v.devices.len(),
        v.samples.len()
    );
    for s in &v.samples {
        println!("    sample {} {:?}", s.name, s.state);
    }

    // ---- 全库统计 ----
    let mut with_sampleof = 0usize;
    let mut with_samples_field = 0usize;
    let mut owned = 0usize;
    let mut self_ref = 0usize;
    for x in lib.games.iter() {
        if !x.sampleof.is_empty() {
            with_sampleof += 1;
            if x.sampleof.eq_ignore_ascii_case(&x.name) {
                self_ref += 1;
            }
            if mvui::core::samples::verify_game_sample(x, verified_flag).is_some() {
                owned += 1;
            }
        }
        if !x.samples.is_empty() {
            with_samples_field += 1;
        }
    }
    println!("\n== 全库统计（共 {} 个机种）==", lib.games.len());
    println!("  sampleof 非空{with_sampleof}（其中自引用 {self_ref}）");
    let rows = with_sampleof - self_ref;
    println!("  会产出行（排除自引用后）  {rows}");
    println!("  其中当前报拥有            {owned}");
    println!("  GameMeta.samples 非空     {with_samples_field}");
    println!(
        "  盘上样本集                {n}{}",
        if with_sampleof > 0 {
            format!("（覆盖率 {:.0}%）", 100.0 * owned as f64 / (with_sampleof - self_ref).max(1) as f64)
        } else {
            String::new()
        }
    );
}

fn parse_xml(path: &str) -> mvui::core::library::GameLibrary {
    let f = std::fs::File::open(path).expect("open xml");
    let mut sink = |_d: usize| {};
    mvui::core::listxml::parse_from_reader(std::io::BufReader::new(f), false, &mut sink)
        .expect("parse")
}
