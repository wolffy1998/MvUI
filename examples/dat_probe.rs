// Check a real DAT through both extraction paths.
//
//     cargo run --release --example dat_probe -- <dat> <dock> <tag>...
//
// `command.dat` renders blank in the Command panel while `history.dat` works.
// The byte-range index and the linear scan are supposed to agree, so run both
// over the shipped file and show where they diverge.

use mvui::core::dat;
use mvui::core::datindex;

fn dock_from_name(s: &str) -> usize {
    match s.to_ascii_lowercase().as_str() {
        "history" => dat::DOCK_HISTORY,
        "mameinfo" => dat::DOCK_MAMEINFO,
        "driverinfo" => dat::DOCK_DRIVERINFO,
        "story" => dat::DOCK_STORY,
        "command" => dat::DOCK_COMMAND,
        other => panic!("unknown dock {other}"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = std::env::args().nth(1).expect("usage: dat_probe <dat> <dock> <tag>...");
    let _ = &mut args;
    let mut a = std::env::args().skip(1);
    a.next();
    let dock_s = a.next().expect("dock");
    let tags: Vec<String> = a.collect();
    let dock = dock_from_name(&dock_s);

    let raw = std::fs::read(&path).expect("read dat");
    let crlf = raw.windows(2).filter(|w| w == b"\r\n").count();
    let lf = raw.iter().filter(|b| **b == b'\n').count();
    println!(
        "{path}\n  {} bytes, {lf} LF, {crlf} CRLF  ({})",
        raw.len(),
        if crlf > 0 { "CRLF file" } else { "LF file" }
    );

    let idx = datindex::DatIndex::build(&raw);
    println!("  index: {} tags\n", idx.tags());

    for tag in &tags {
        let scan = dat::get_history(&raw, tag, dock, true, "");
        let via_index = idx
            .lookup(tag)
            .map(|r| {
                dat::finish_record(
                    datindex::record_text(&raw, r, tag, true),
                    tag,
                    dock,
                    true,
                )
            })
            .unwrap_or_default();
        let agree = scan == via_index;
        println!("  {tag}");
        println!("    scan  : {} chars", scan.chars().count());
        println!("    index : {} chars  {}", via_index.chars().count(), if agree { "AGREE" } else { "MISMATCH" });
        if !agree {
            println!("    scan head : {:?}", scan.chars().take(70).collect::<String>());
            println!("    index head: {:?}", via_index.chars().take(70).collect::<String>());
        }
        if let Some(r) = idx.lookup(tag) {
            println!("    byte range: {}..{} ({} bytes)", r.start, r.end, r.end - r.start);
            let at = raw.get(r.start..r.end).unwrap_or(b"");
            println!("    range head: {:?}", String::from_utf8_lossy(&at[..at.len().min(60)]));
        } else {
            println!("    NOT IN INDEX");
        }

        // --- full pipeline, exactly as the panel does it ---
        if dock == dat::DOCK_COMMAND {
            // 1. strip_html (background.rs): <br> -> \n, then drop all tags
            let s = scan.replace("<br>", "\n").replace("<hr>", "\n----------------\n");
            let stripped = regex::Regex::new(r"<[^>]+>")
                .unwrap()
                .replace_all(&s, "")
                .replace("&amp;", "&")
                .replace("&quot;", "\"")
                .to_owned();
            println!("    after strip_html: {} chars", stripped.chars().count());
            let lines = dat::convert_command_lines(&stripped);
            let segs: usize = lines.iter().map(|l| l.segments.len()).sum();
            let icons: usize = lines
                .iter()
                .map(|l| {
                    l.segments
                        .iter()
                        .filter(|s| matches!(s, dat::Segment::Icon(_)))
                        .count()
                })
                .sum();
            let chars: usize = lines
                .iter()
                .map(|l| {
                    l.segments
                        .iter()
                        .map(|s| match s {
                            dat::Segment::Text(t) => t.chars().count(),
                            dat::Segment::Icon(_) => 1,
                            dat::Segment::Rule => 0,
                        })
                        .sum::<usize>()
                })
                .sum();
            let rules = lines
                .iter()
                .filter(|l| {
                    l.segments
                        .iter()
                        .any(|s| matches!(s, dat::Segment::Rule))
                })
                .count();
            println!(
                "    convert_command_lines: {} lines, {segs} segments, {icons} icons, {chars} chars, {rules} rules",
                lines.len()
            );
            for l in lines.iter().take(14) {
                let desc: Vec<String> = l
                    .segments
                    .iter()
                    .map(|s| match s {
                        dat::Segment::Text(t) => format!("{t:?}"),
                        dat::Segment::Icon(n) => format!("Icon({n:?})"),
                        dat::Segment::Rule => "--- RULE ---".to_string(),
                    })
                    .collect();
                println!("      | {}", desc.join(" "));
            }
        }
        println!();
    }
}
