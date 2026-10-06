//! Dump what the Rom panel would show for one game, from the real library.
//!
//! The GUI click/typing scripts cannot drive the window reliably (egui ignores
//! synthesised input when the window is not focused), so verifying "does the
//! Bios section appear? does the device section show rom files?" by clicking
//! through the app is slow and flaky. This reads the same
//! `gamelist.cache` the app reads and prints the sections `rompanel::render`
//! iterates, so the data shape can be checked without a window.
//!
//! usage: cargo run --release --example rom_view_dump -- <gamelist.cache> <game>

fn main() {
    let cache = std::env::args().nth(1).expect("gamelist.cache path");
    let game = std::env::args().nth(2).unwrap_or_else(|| "gtmro".into());
    let data = mvui::core::cache::load(std::path::Path::new(&cache), "MAME v0.285 (unknown)")
        .expect("load library");
    let lib = data.library;

    // Which machines are worth looking at: the ones with the most sections.
    let mut ranked: Vec<(usize, String, usize, usize, usize, usize)> = lib
        .games
        .iter()
        .enumerate()
        .filter(|(_, g)| !g.is_device && !g.is_bios && !g.is_mechanical)
        .map(|(_i, g)| {
            let v = mvui::core::rominfo::view_of(&lib, &g.name, data.verified);
            (v.roms.len(), g.name.clone(), v.bios.len(), v.devices.len(),
             v.slots.len(), v.samples.len() + v.disks.len())
        })
        .filter(|(_, _, b, d, s, x)| b + d + s + x > 0)
        .collect();
    ranked.sort_by(|a, b| (b.2 + b.3 + b.4 + b.5).cmp(&(a.2 + a.3 + a.4 + a.5)));
    ranked.truncate(12);

    println!("== machines with the most sections ==");
    for (r, name, b, d, s, x) in &ranked {
        println!("  {name:<28} roms={r:<4} bios={b:<2} devices={d:<3} slots={s:<2} samples+chd={x}");
    }

    // The requested game, and if it has nothing interesting, the top-ranked one.
    let target = if lib.get_idx(&game).is_some() { game.clone() } else {
        ranked.first().map(|r| r.1.clone()).unwrap_or(game)
    };
    println!("\n== view_of({target}) verified={} ==", data.verified);
    let v = mvui::core::rominfo::view_of(&lib, &target, data.verified);
    println!("  roms        {}", v.roms.len());
    println!("  disks       {}", v.disks.len());
    println!("  bios        {}", v.bios.len());
    println!("  devices{}",
        v.devices.iter().map(|d| format!("  {} ({}) {}", d.name, d.tag,
            match d.state {
                mvui::core::rominfo::RomState::Good => "good",
                mvui::core::rominfo::RomState::BadDump => "baddump",
                mvui::core::rominfo::RomState::Missing => "missing",
                mvui::core::rominfo::RomState::NoDump => "nodump",
                mvui::core::rominfo::RomState::Unknown => "unknown",
            })).collect::<Vec<_>>().join("\n"));
    println!("  device_roms {}", v.device_roms.len());
    println!("  slots       {}", v.slots.len());
    println!(
        "  slot_decls{}",
        if v.slot_decls.is_empty() {
            String::new()
        } else {
            format!(
                "  {}",
                v.slot_decls
                    .iter()
                    .map(|s| format!("{}({})", s.name, s.option_count))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
    );
    println!("  samples     {}", v.samples.len());
    println!("  missing     {}", v.missing_count());

    // baddump / nodump are rare in a full library — find some so the states
    // are actually exercised, not just defined.
    let mut seen_baddump = 0;
    let mut seen_nodump = 0;
    for g in lib.games.iter() {
        if seen_baddump > 0 && seen_nodump > 0 {
            break;
        }
        for r in &g.roms {
            if r.is_baddump() && seen_baddump == 0 {
                println!("\n== a baddump exists: {} / {} ==", g.name, r.name);
                seen_baddump = 1;
            }
            if r.is_nodump() && seen_nodump == 0 {
                println!("== a nodump exists: {} / {} ==", g.name, r.name);
                seen_nodump = 1;
            }
        }
    }
    diagnose_devices(&lib);
    if seen_baddump == 0 {
        println!("\n(no baddump in this library — the state cannot be exercised here)");
    }
}
/// 附加诊断：设备段为什么空。
///
/// `pgm` 的 `-listxml` 里有 5 个 `<device_ref>`，但如果 `GameMeta::devices`
/// 是空的，那说明**缓存是旧格式**——`device_ref` 的解析是后加的，老缓存里
/// 根本没有这个字段。热启动走`verified=true` 的路径会跳过 `-listxml` 重新
/// 解析，于是 `devices` 永远是空的，而用户看不到任何提示。
#[allow(dead_code)]
fn diagnose_devices(lib: &mvui::core::library::GameLibrary) {
    let mut with_devices = 0usize;
    for g in &lib.games {
        if !g.devices.is_empty() {
            with_devices += 1;
        }
    }
    let pgm = lib.get("pgm");
    println!(
        "\n== devices 诊断 ==\n库里有 devices 的机种: {with_devices} / {}",
        lib.len()
    );
    if let Some(g) = pgm {
        println!(
            "pgm: roms={} bios_sets={} devices={} devices[0..3]={:?}",
            g.roms.len(),
            g.bios_sets.len(),
            g.devices.len(),
            g.devices
                .iter()
                .take(3)
                .map(|d| format!("{}:{}", d.kind, d.tag))
                .collect::<Vec<_>>()
        );
    }
    // 设备机种本身在不在库里？（`m68000` / `igs036` 这些）
    for n in ["m68000", "z80", "timer", "igs036", "palette"] {
        match lib.get(n) {
            Some(g) => println!("  lib.get({n}) = Some(roms={}, is_device={}, is_bios={})",
                g.roms.len(), g.is_device, g.is_bios),
            None => println!("  lib.get({n}) = None"),
        }
    }

    // 「设备」段的活体检查：`<device>`（本机自带的可挂载设备）现在靠
    // `is_ref` 标记区分，不再靠 `kind == instance` 猜（那个判别式会让
    // instance 名恰好等于 type 的槽位全部误判，见 `DeviceInfo::is_ref`）。
    // 修复前这里输出 `0 / 49676`——不是没有设备，是判别式错了。
    let mut with_devs = 0usize;
    let mut sample: Option<&str> = None;
    for g in &lib.games {
        if g.devices.iter().any(|d| !d.is_ref) {
            with_devs += 1;
            if sample.is_none() {
                sample = Some(g.name.as_str());
            }
        }
    }
    println!(
        "带可挂载设备的机种: {with_devs} / {}  首个={:?}",
        lib.len(),
        sample
    );

    // `<slot>` 槽位声明的覆盖情况
    let mut with_slots = 0usize;
    let mut slot_opts = 0usize;
    for g in &lib.games {
        if !g.slots.is_empty() {
            with_slots += 1;
        }
        slot_opts += g.slots.iter().map(|s| s.options.len()).sum::<usize>();
    }
    println!("带 <slot> 声明的机种: {with_slots} / {}（共 {slot_opts} 个可选设备）", lib.len());
}
