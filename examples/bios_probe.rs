//! 诊断 BIOS 段：BIOS 文件有没有从 Rom 段里独立出去。
//!
//! 直接喂真实 `-listxml`（绕开缓存 —— 缓存要重建 40 秒，而且 2026-10-06
//! 之前那份是 format 4，已经被 `FORMAT_VERSION=5` 拒绝）。
//!
//! 三问：
//! 1. 全库有多少机种带 `bios_sets`（对照真实 xml 的 3655）
//! 2. **带 `bios=` 的 rom 有没有漏进 Rom 段**（用户截图里的
//!    `pgm_p02s.u20` / `pgm_p01s.u20` 就在 Rom 段里）
//! 3. BIOS 段列出来的文件 == 带 `bios=` 的 rom 吗（`kovplus` 应显示 V1+V2）

use std::collections::HashSet;

fn main() {
    let path = std::env::args().nth(1).expect("path to -listxml output");
    let f = std::fs::File::open(&path).expect("open xml");
    let mut sink = |_done: usize| {};
    let lib = mvui::core::listxml::parse_from_reader(std::io::BufReader::new(f), false, &mut sink)
        .expect("parse");
    println!("解析出 {} 个机种", lib.len());

    let with_sets = lib.games.iter().filter(|g| !g.bios_sets.is_empty()).count();
    let with_bios_roms = lib
        .games
        .iter()
        .filter(|g| g.roms.iter().any(|r| !r.bios.is_empty()))
        .count();
    println!("[1] 带 bios_sets 的机种: {with_sets}  （真实 xml 统计 = 3655）");
    println!("    带 bios= 的 rom 的机种: {with_bios_roms}");

    // 问 2/3：逐个机种核对「Rom 段里有没有带 bios 的条目」
    let mut leaked: Vec<(String, Vec<String>)> = Vec::new();
    let mut not_rendered: Vec<String> = Vec::new();
    let mut count_mismatch: Vec<(String, usize, usize)> = Vec::new();
    let mut with_bios_attr = 0usize;

    for g in lib.games.iter() {
        if g.is_device || g.is_mechanical {
            continue;
        }
        let tagged: Vec<&str> = g
            .roms
            .iter()
            .filter(|r| !r.bios.is_empty())
            .map(|r| r.name.as_str())
            .collect();
        if tagged.is_empty() {
            continue;
        }
        with_bios_attr += 1;
        let v = mvui::core::rominfo::view_of(&lib, &g.name, true);

        // Rom 段里有没有这些名字
        let rom_names: HashSet<&str> = v.roms.iter().map(|r| r.name.as_str()).collect();
        let bad: Vec<String> = tagged
            .iter()
            .filter(|n| rom_names.contains(**n))
            .map(|n| n.to_string())
            .collect();
        if !bad.is_empty() {
            leaked.push((g.name.clone(), bad));
        }
        // BIOS 段是不是每个都渲染了
        let missing: Vec<&str> = tagged
            .iter()
            .filter(|t| {
                !v.bios
                    .iter()
                    .any(|b| b.roms.iter().any(|r| r.name == **t))
            })
            .copied()
            .collect();
        if !missing.is_empty() {
            not_rendered.push(format!("{} 缺 {:?}", g.name, missing));
        }
        let shown: usize = v.bios.iter().map(|b| b.roms.len()).sum();
        if shown != tagged.len() {
            count_mismatch.push((g.name.clone(), tagged.len(), shown));
        }
    }

    println!();
    println!("[2] Rom 段里混进了带 bios= 条目的机种: {}", leaked.len());
    for (n, b) in leaked.iter().take(8) {
        println!("    {n}: {b:?}");
    }
    println!();
    println!("[3] BIOS 段漏渲染了某个带 bios= 的文件: {}", not_rendered.len());
    for n in not_rendered.iter().take(8) {
        println!("    {n}");
    }
    println!();
    println!(
        "[4] BIOS 段文件数 != 带 bios= 的 rom 数: {}  （共 {with_bios_attr} 个机种有 bios rom）",
        count_mismatch.len()
    );
    for (n, want, got) in count_mismatch.iter().take(8) {
        println!("    {n}: 实际 {want} 个，段里只渲染 {got} 个");
    }

    // kovplus 专项（用户截图那个）
    println!();
    println!("== kovplus ==");
    let v = mvui::core::rominfo::view_of(&lib, "kovplus", true);
    println!(
        "  bios_sets 声明: {}",
        lib.get("kovplus").map(|g| g.bios_sets.len()).unwrap_or(0)
    );
    println!("  Rom 段 {} 行，前 4:", v.roms.len());
    for r in v.roms.iter().take(4) {
        println!("      {}", r.name);
    }
    println!("  Bios 段 {} 套:", v.bios.len());
    for b in &v.bios {
        println!(
            "    {} ({}) default={} roms={:?}",
            b.name,
            b.description,
            b.is_default,
            b.roms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>()
        );
    }
}