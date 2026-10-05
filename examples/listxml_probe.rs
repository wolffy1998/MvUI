//! Parse a real `-listxml` file and report what the parser actually produced.
//!
//! The Rom panel's "referenced devices" section showed nothing for every one
//! of the 49 676 machines. `src/core/listxml.rs` parses `<device_ref>`
//! correctly as far as reading the code shows, so the suspect is either
//! (//) the fast scanner never hands `device_ref` elements to the parser, or
//! (//) the library that gets cached never went through it.
//!
//! This feeds a real export through the real parser so the answer is data,
//! not speculation.
//!
//! usage: cargo run --release --example listxml_probe -- <file.xml>

fn main() {
    let path = std::env::args().nth(1).expect("path to -listxml output");
    let f = std::fs::File::open(&path).expect("open xml");
    let mut sink = |_done: usize| {};
    let lib = mvui::core::listxml::parse_from_reader(
        std::io::BufReader::new(f),
        false,
        &mut sink,
    )
    .expect("parse");

    println!("machines parsed          {}", lib.len());
    let with_dev: Vec<_> = lib
        .games
        .iter()
        .filter(|g| !g.devices.is_empty())
        .collect();
    println!("machines with devices{}",
        with_dev.len());
    for g in with_dev.iter().take(3) {
        println!(
            "  {}: {} devices, first {:?}",
            g.name,
            g.devices.len(),
            g.devices
                .iter()
                .take(4)
                .map(|d| format!("kind={} instance={} tag={}", d.kind, d.instance, d.tag))
                .collect::<Vec<_>>()
        );
    }
    // 设备段（`<device>`）与引用设备段（`<device_ref>`）分开统计。
    // 判别式是 `is_ref`，**不是** `kind != instance` —— `<device type="cartridge">`
    // 的 instance 名恰好也是 cartridge，用后者会把槽位全算进引用设备。
    let mut refs = 0usize;
    let mut devs = 0usize;
    for g in &lib.games {
        for d in &g.devices {
            if d.is_ref {
                refs += 1;
            } else {
                devs += 1;
            }
        }
    }
    println!("device_ref entries (referenced devices): {refs}");
    println!("device entries (device section): {devs}");

    // `<slot>` 解析情况：带选项的与空槽位各有多少
    let (mut with_slots, mut slot_opts) = (0usize, 0usize);
    for g in &lib.games {
        if !g.slots.is_empty() {
            with_slots += 1;
        }
        slot_opts += g.slots.iter().map(|s| s.options.len()).sum::<usize>();
    }
    println!("machines with <slot>: {with_slots}, total <slotoption>: {slot_opts}");
    for g in &lib.games {
        if !g.slots.is_empty() {
            println!("  {}: {:?}", g.name, g.slots);
            break;
        }
    }
    println!(
        "device machines present in the library: {}",
        ["m68000", "z80", "timer", "palette", "speaker", "nvram"]
            .iter()
            .filter(|n| lib.get(n).is_some())
            .count()
    );
}