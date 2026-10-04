//! Full pipeline check: history.xml -> index -> render_text -> strip_html ->
//! convert_history_lines, i.e. exactly what the History panel consumes.
//!
//! Usage: history_pipe <history.xml> <game>

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(game)) = (args.next(), args.next()) else {
        eprintln!("usage: history_pipe <history.xml> <game>");
        std::process::exit(2);
    };
    let t0 = std::time::Instant::now();
    let bytes = std::fs::read(&path).expect("read");
    let idx = mvui::core::historyxml::HistoryXmlIndex::build(&bytes);
    let Some((s, e)) = idx.range_of(&game) else {
        println!("MISS {game}");
        return;
    };
    let raw = String::from_utf8_lossy(&bytes[s..e]).to_string();
    let (start, end) = (s, e);
    let _ = (start, end);

    // what read_one_dat would return
    let html = mvui::core::historyxml::render_text(&raw);
    println!("=== render_text: {} B ===", html.len());

    // the tag stripper the loader applies before the UI sees it.
    // `strip_html` is private to the binary, so this mirrors it exactly:
    // `<br>` -> newline, `<hr>` -> the dashed rule, then every remaining tag
    // removed. Kept in step with `background::strip_html`.
    let stripped = strip_html_for_probe(&html);
    println!("=== strip_html: {} B ===", stripped.len());

    let lines = mvui::core::dat::convert_history_lines(&stripped);
    println!("=== convert_history_lines: {} lines ===", lines.len());
    for (i, l) in lines.iter().enumerate().take(60) {
        use mvui::core::dat::Segment;
        match l.segments.first() {
            Some(Segment::Rule) => println!("  {i:3} [RULE]"),
            Some(Segment::Text(t)) if t.is_empty() => println!("  {i:3} [blank]"),
            Some(Segment::Text(t)) => {
                let cut: String = t.chars().take(70).collect();
                println!("  {i:3} {cut}");
            }
            _ => println!("  {i:3} [icon]"),
        }
    }
    println!("total elapsed {:?}", t0.elapsed());
}

/// Byte-for-byte the same steps as `background::strip_html`.
fn strip_html_for_probe(s: &str) -> String {
    let s = s.replace("<br>", "\n").replace("<hr>", "\n----------------\n");
    let re = regex::Regex::new(r"<[^>]+>").unwrap();
    re.replace_all(&s, "")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
}
