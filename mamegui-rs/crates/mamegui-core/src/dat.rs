//! DAT file parsing, 1:1 port of UpdateSelectionThread getHistory/convertHistory/
//! convertMameInfo/convertCommand/getScreenshot.

use crate::archive::IterateMethod;
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// dock indices matching the original DOCK_* enum
pub const DOCK_SNAP: usize = 0;
pub const DOCK_FLYER: usize = 1;
pub const DOCK_CABINET: usize = 2;
pub const DOCK_MARQUEE: usize = 3;
pub const DOCK_TITLE: usize = 4;
pub const DOCK_CPANEL: usize = 5;
pub const DOCK_PCB: usize = 6;
pub const DOCK_HISTORY: usize = 7;
pub const DOCK_MAMEINFO: usize = 8;
pub const DOCK_DRIVERINFO: usize = 9;
pub const DOCK_STORY: usize = 10;
pub const DOCK_COMMAND: usize = 11;
pub const DOCK_LAST: usize = 12;

pub const DOCK_NAMES: [&str; DOCK_LAST] = [
    "Snapshot",
    "Flyer",
    "Cabinet",
    "Marquee",
    "Title",
    "Control Panel",
    "PCB",
    "History",
    "MAMEInfo",
    "DriverInfo",
    "Story",
    "Command",
];

/// archive/dir names searched per image dock (origin: getScreenshot)
pub fn dock_archive_name(t: usize) -> &'static str {
    match t {
        DOCK_SNAP => "snap",
        DOCK_FLYER => "flyers",
        DOCK_CABINET => "cabinets",
        DOCK_MARQUEE => "marquees",
        DOCK_TITLE => "titles",
        DOCK_CPANEL => "cpanel",
        _ => "pcb",
    }
}

/// which mameOpts directory option backs each image dock (validGuiSettings keys)
pub fn dock_directory_option(t: usize) -> &'static str {
    match t {
        DOCK_SNAP => "snapshot_directory",
        DOCK_FLYER => "flyer_directory",
        DOCK_CABINET => "cabinet_directory",
        DOCK_MARQUEE => "marquee_directory",
        DOCK_TITLE => "title_directory",
        DOCK_CPANEL => "cpanel_directory",
        _ => "pcb_directory",
    }
}

/// which mameOpts file option backs each text dock
pub fn dock_file_option(t: usize) -> Option<&'static str> {
    match t {
        DOCK_HISTORY => Some("history_file"),
        DOCK_MAMEINFO | DOCK_DRIVERINFO => Some("mameinfo_file"),
        DOCK_STORY => Some("story_file"),
        DOCK_COMMAND => Some("command_file"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// getHistory — the classic $info DAT format
// ---------------------------------------------------------------------------

/// link color depends on background (origin uses isDarkBg)
pub fn link_color(dark: bool) -> &'static str {
    if dark {
        "#00a0e9"
    } else {
        "#006d9f"
    }
}

/// Render payload lines to HTML-ish output.
///
/// Shared by the linear scan ([get_history]) and the byte-range index
/// (datindex::record_text) so the two cannot drift: the index decides *which
/// bytes* are the record, this decides what they render to.
///
/// Takes the lines already sliced out - the scan collects them as it walks, the
/// index hands over the range's lines. Joining them back into one string would
/// be lossy (str::lines drops the fact that a trailing empty line existed), so
/// they stay a slice.
fn render_lines(lines: &[&str], dark_bg: bool) -> String {
    render_lines_for(lines, None, dark_bg)
}

/// `own_tag` is the tag the record was opened by, if the caller knows it.
///
/// A `$info=` line inside a record is ambiguous: for the scan it ends the
/// payload, but when it carries the record's own tag the scan *keeps going*
/// (that is the greedy span reproduced in datindex). Passing the tag lets both
/// paths share this one function without disagreeing about that case.
fn render_lines_for(lines: &[&str], own_tag: Option<&str>, dark_bg: bool) -> String {
    let link = link_color(dark_bg);
    let mut out: Vec<String> = Vec::new();
    for line in lines {
        let line = *line;
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with("$info=") {
            // the next record's opener ends this payload — unless it carries
            // our own tag, in which case the scan keeps collecting
            let carries = own_tag.is_some_and(|t| {
                line.strip_prefix("$info=")
                    .is_some_and(|rest| rest.split(',').any(|x| x.trim() == t))
            });
            if !carries {
                break;
            }
            continue;
        }
        if line.starts_with('$') {
            if let Some(href) = line.strip_prefix("$<a href=") {
                out.push(format!("<a style=\"color:{link}\" href={href}><br>"));
            } else {
                out.push(format!("{line}<br>"));
            }
        } else {
            out.push(format!("{line}<br>"));
        }
    }
    out.join("")
}

/// Public wrapper for the index path: turn a record's byte range into output.
pub fn format_record(range_text: &str, own_tag: Option<&str>, dark_bg: bool) -> String {
    let lines: Vec<&str> = range_text.lines().collect();
    render_lines_for(&lines, own_tag, dark_bg)
}

/// Trim the `<br>` padding and prepend the MAWS link, i.e. everything
/// [`get_history`] does to a rendered payload before handing it over.
/// Applied *after* either extraction path so the results match exactly.
pub fn finish_record(mut s: String, search_tag: &str, method: usize, dark_bg: bool) -> String {
    if method == DOCK_HISTORY {
        s = format!(
            "<a style=\"color:{}\" href=\"http://maws.mameworld.info/maws/romset/{search_tag}\">View information at MAWS</a><br>",
            link_color(dark_bg)
        ) + &s;
    }
    while s.starts_with("<br>") {
        s = s[4..].to_string();
    }
    while s.ends_with("<br>") {
        s = s[..s.len() - 4].to_string();
    }
    s
}

/// origin: UpdateSelectionThread::getHistory — returns HTML-ish lines
pub fn get_history(
    file_bytes: &[u8],
    search_tag: &str,
    method: usize,
    dark_bg: bool,
    cloneof: &str,
) -> String {
    let text = String::from_utf8_lossy(file_bytes);
    let mut rec_data = false;
    // line-offset state only exists for the scan; the formatter itself is shared
    let mut rec_lines: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('$') {
            if let Some(rest) = line.strip_prefix("$info=") {
                let matched = rest.split(',').any(|t| t.trim() == search_tag);
                if matched {
                    rec_data = true;
                } else if rec_data {
                    break;
                }
            } else if rec_data {
                rec_lines.push(line);
            }
        } else if rec_data {
            rec_lines.push(line);
        }
    }
    let mut s = render_lines(&rec_lines, dark_bg);
    if s.is_empty() {
        // recursive clone fallback
        if !cloneof.is_empty() {
            return get_history(file_bytes, cloneof, method, dark_bg, "");
        }
        return String::new();
    }
    s = finish_record(s, search_tag, method, dark_bg);
    s
}

/// origin: convertMameInfo — prepend "Rom Region" table
pub fn convert_mame_info(text: &str, rom_regions: &[(String, String)]) -> String {
    if rom_regions.is_empty() {
        return text.to_string();
    }
    let mut head = String::from("Rom Region:<table>");
    for (region, name) in rom_regions {
        head.push_str(&format!("<tr><td>{}</td><td> </td><td>{}</td></tr>", region, name));
    }
    head.push_str("</table><hr>");
    format!("{head}{text}")
}

// ---------------------------------------------------------------------------
// convertCommand — the exact replacement table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Notation {
    Dir(u8),
    Hcf,
    Hcb,
    Qdf,
    Qdb,
    Button(String),
    ButtonN(String),
    StarGold,
    StarSilver,
    TriRed,
    CircleYellow,
    CircleRed,
    CircleGreen,
    Arrow,
}

#[derive(Debug, Clone)]
pub enum Segment {
    Text(String),
    Icon(Notation),
}

#[derive(Debug, Clone)]
pub struct DatLine {
    pub segments: Vec<Segment>,
}

/// command.dat token regex — compiled once, this is on the render path
/// (README P2-24)
static CMD_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_([0-9A-DGKNPS+])|_([a-f])|(★|☆|▲|○|◎|●|→)").unwrap());

pub fn convert_command_lines(text: &str) -> Vec<DatLine> {
    // ordered replacement table (origin: convertCommand); first the
    // hcf/hcb duplication fixes, then combo singles, then buttons/symbols
    // The two duplicated combos are *two* arrows, not one: the old table repeats
    // the `4` (`_2_1_4_1_2_3_6` → `_2_1_4_4_1_2_3_6`) and then lets the plain rules
    // fire, so they render as qdb+hcf and qdf+hcb. We used to collapse each into a
    // single icon (README P3). Order matters, exactly as in the original.
    let combos: &[(&str, &[Notation])] = &[
        ("_2_1_4_1_2_3_6", &[Notation::Qdb, Notation::Hcf]),
        ("_2_3_6_3_2_1_4", &[Notation::Qdf, Notation::Hcb]),
        ("_4_1_2_3_6", &[Notation::Hcf]),
        ("_6_3_2_1_4", &[Notation::Hcb]),
        ("_2_3_6", &[Notation::Qdf]), // dir-qdf.png
        ("_2_1_4", &[Notation::Qdb]),
    ];

    let mut out = Vec::new();
    for raw in text.lines() {
        let mut line = raw.to_string();
        for (pat, ns) in combos {
            // every tag is wrapped in its own \0 pair: the parser reads the text
            // *between* two markers as one tag, so two adjacent tags need the
            // delimiter doubled up
            let mut rep = String::new();
            for n in *ns {
                rep.push('\u{0}');
                rep.push_str(notation_tag(n));
                rep.push('\u{0}');
            }
            line = line.replace(pat, &rep);
        }
        let mut segments: Vec<Segment> = Vec::new();
        let mut plain = String::new();
        let mut rest = line.as_str();
        while !rest.is_empty() {
            if let Some(idx) = rest.find('\u{0}') {
                let (head, tail) = rest.split_at(idx);
                plain.push_str(head);
                if let Some(end) = tail[1..].find('\u{0}') {
                    let tag = &tail[1..1 + end];
                    segments.push(Segment::Text(std::mem::take(&mut plain)));
                    segments.push(Segment::Icon(tag_to_notation(tag)));
                    rest = &tail[end + 2..];
                } else {
                    plain.push_str(tail);
                    rest = "";
                }
                continue;
            }
            match CMD_TOKEN.captures(rest) {
                Some(c) => {
                    let m = c.get(0).unwrap();
                    if m.start() > 0 {
                        plain.push_str(&rest[..m.start()]);
                    }
                    if let Some(d) = c.get(1) {
                        segments.push(Segment::Text(std::mem::take(&mut plain)));
                        // origin: `_(\d)` → dir-N.png and `_([A-DGKNPS+])` →
                        // btn-<CHAR>.png. Only the digit case used to be handled,
                        // so every `_P` / `_K` / `_A` had its icon dropped (the
                        // character was swallowed) — "↓↘→ + P" lost the button.
                        match d.as_str().parse::<u8>() {
                            Ok(n) => segments.push(Segment::Icon(Notation::Dir(n))),
                            Err(_) => segments.push(Segment::Icon(Notation::Button(
                                d.as_str().to_string(),
                            ))),
                        }
                    } else if let Some(b) = c.get(2) {
                        segments.push(Segment::Text(std::mem::take(&mut plain)));
                        // origin: `_([a-f])` → btn-n<lower>.png (the icon names are
                        // lowercase, so the letter must not be upper-cased here)
                        segments.push(Segment::Icon(Notation::ButtonN(b.as_str().to_string())));
                    } else if let Some(s) = c.get(3) {
                        segments.push(Segment::Text(std::mem::take(&mut plain)));
                        let n = match s.as_str() {
                            "★" => Notation::StarGold,
                            "☆" => Notation::StarSilver,
                            "▲" => Notation::TriRed,
                            "○" => Notation::CircleYellow,
                            "◎" => Notation::CircleRed,
                            "●" => Notation::CircleGreen,
                            _ => Notation::Arrow,
                        };
                        segments.push(Segment::Icon(n));
                    }
                    rest = &rest[m.end()..];
                }
                None => {
                    plain.push_str(rest);
                    rest = "";
                }
            }
        }
        segments.push(Segment::Text(plain));
        out.push(DatLine { segments });
    }
    out
}

fn notation_tag(n: &Notation) -> &'static str {
    match n {
        Notation::Hcf => "HCF",
        Notation::Hcb => "HCB",
        Notation::Qdf => "QCF",
        Notation::Qdb => "QDB",
        _ => "",
    }
}

fn tag_to_notation(tag: &str) -> Notation {
    match tag {
        "HCF" => Notation::Hcf,
        "HCB" => Notation::Hcb,
        "QCF" => Notation::Qdf,
        "QDB" => Notation::Qdb,
        _ => Notation::Button("?".into()),
    }
}

// ---------------------------------------------------------------------------
// getScreenshot — snapname pattern support (origin: getScreenshot)
// ---------------------------------------------------------------------------

/// expanded filename list for DOCK_SNAP from the snapname option
/// ("snapname" value with %g → game, %i → "0000")
pub fn snapname_variants(snapname: &str, game: &str) -> Vec<String> {
    let pattern = snapname.replace("%g", game).replace("%i", "0000");
    let mut v = vec![format!("{game}.png"), format!("{pattern}.png")];
    v.dedup();
    v
}

/// locate a preview image: try <dir>/<name>.png for each dir variant,
/// then plain file lookup, via iterate_mame_file
pub fn load_preview_bytes(
    dirs: &str,
    archive_names: &str,
    file_filters: &str,
) -> Option<Vec<u8>> {
    let hits = crate::archive::iterate_mame_file(
        dirs,
        archive_names,
        file_filters,
        IterateMethod::Read,
        "",
        None,
    );
    // scan order, not hash order (README P3)
    hits.into_iter().next().map(|(_, m)| m.data)
}

/// read a dat file (plain or inside a zip next to it) as bytes
pub fn read_dat_bytes(path: &str) -> Option<Vec<u8>> {
    let p = Path::new(path);
    if p.is_file() {
        return std::fs::read(p).ok();
    }
    // maybe inside a zip: dat dir + zip name = parent dir name
    let dir = p.parent()?;
    let base = p.file_name()?.to_string_lossy().to_string();
    let zip = dir.join(format!("{}.zip", dir.file_name()?.to_string_lossy()));
    if zip.is_file() {
        let hits = crate::archive::iterate_mame_file(
            &dir.to_string_lossy(),
            &dir.file_name().unwrap().to_string_lossy(),
            &base,
            IterateMethod::Read,
            "",
            None,
        );
        if let Some((_, m)) = hits.into_iter().next() {
            return Some(m.data);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_info_block() {
        let dat = b"# comment\n$info=pacman,pacmana\nHistory line 1\n$more\n";
        let s = get_history(dat, "pacman", DOCK_HISTORY, true, "");
        assert!(s.contains("History line 1"));
        assert!(s.contains("MAWS"));
        let s2 = get_history(dat, "pacmana", DOCK_HISTORY, false, "pacman");
        assert!(s2.contains("History line 1"));
    }

    #[test]
    fn command_tokens() {
        let lines = convert_command_lines("Hadouken: _2_3_6 + _P damage ★");
        assert!(matches!(lines[0].segments[1], Segment::Icon(Notation::Qdf)));
        let lines2 = convert_command_lines("Dash: _4_1_2_3_6");
        assert!(matches!(lines2[0].segments[1], Segment::Icon(Notation::Hcf)));
        // origin: `_2_1_4_1_2_3_6` is drawn as two arrows (qdb + hcf)
        let lines3 = convert_command_lines("QCF: _2_1_4_1_2_3_6");
        let icons: Vec<&Notation> = lines3[0]
            .segments
            .iter()
            .filter_map(|s| match s {
                Segment::Icon(n) => Some(n),
                _ => None,
            })
            .collect();
        assert!(
            icons.len() >= 2
                && matches!(icons[0], Notation::Qdb)
                && matches!(icons[1], Notation::Hcf),
            "got {icons:?}"
        );
    }

    /// `_P` / `_A` / `_+` used to be matched by the digit arm, fail `parse::<u8>`
    /// and vanish without leaving an icon behind (origin: `_([A-DGKNPS+])`).
    #[test]
    fn command_button_tokens() {
        let lines = convert_command_lines("Hadouken: _6_2_3 + _P (strong)");
        let icons: Vec<&Notation> = lines[0]
            .segments
            .iter()
            .filter_map(|s| match s {
                Segment::Icon(n) => Some(n),
                _ => None,
            })
            .collect();
        assert!(
            icons
                .iter()
                .any(|n| matches!(n, Notation::Button(c) if c == "P")),
            "`_P` must produce a button icon: {icons:?}"
        );
        // lowercase letters take the btn-n* family
        let lines2 = convert_command_lines("_a _f");
        let icons2: Vec<&Notation> = lines2[0]
            .segments
            .iter()
            .filter_map(|s| match s {
                Segment::Icon(n) => Some(n),
                _ => None,
            })
            .collect();
        assert_eq!(icons2.len(), 2, "{icons2:?}");
        assert!(matches!(icons2[0], Notation::ButtonN(c) if c == "a"));
        // the text in between must survive
        let text: String = lines[0]
            .segments
            .iter()
            .filter_map(|s| match s {
                Segment::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains('+'), "text around the icon was dropped: {text:?}");
    }
}
