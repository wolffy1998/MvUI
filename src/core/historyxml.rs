//! `history.xml` parsing — the modern replacement for `history.dat`.
//!
//! Around MAME 0.228 Arcade-History retired the flat `$info=` DAT format in
//! favour of an XML document. The shipped file (revision 2.89) is 64 MB /
//! 2 094 596 lines / 116 967 `<entry>` blocks, so the shape of the work is
//! different from `history.dat`'s in two ways that matter:
//!
//! * **One `<entry>` can serve many names.** The entry carries either
//!   `<systems><system name="…"/></systems>` (arcade / mainlist, 41 673 names
//!   across 18 550 entries) or `<software><item list="…" name="…"/></software>`
//!   (software-list entries, 133 094 names across 99 650 entries). A single
//!   `<text>` is therefore shared by every name in the block — which is why a
//!   name cannot be used as a direct dictionary key without first flattening
//!   the mapping.
//! * **The section headers are plain text.** Unlike `command.dat` there are no
//!   markup tags inside `<text>`: the block is raw text whose sections are
//!   introduced by lines like `- TECHNICAL -` and `- TRIVIA -`. Only five XML
//!   entities appear (`&apos;` `&quot;` `&amp;` `&gt;` `&lt;`).
//!
//! The output is the same HTML-ish string `dat::get_history` produces, so the
//! existing render pipeline (`strip_html`, and the `Segment` renderer for the
//! section rules) consumes it unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::core::datindex::FileStamp;

/// One `<entry>`: the names it answers to, plus the byte range of its text.
#[derive(Debug, Clone)]
struct Entry {
    /// Byte offset of the first character *inside* `<text>`.
    text_start: usize,
    /// Byte offset just past `</text>`.
    text_end: usize,
}

/// `name -> entry index`. Both mainlist and softlist names land here; a name
/// that is claimed by two entries keeps the **first**, matching the
/// top-to-bottom "first match wins" rule the DAT scanner uses.
#[derive(Debug, Default, Clone)]
pub struct HistoryXmlIndex {
    stamp: Option<FileStamp>,
    entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
}

impl HistoryXmlIndex {
    pub fn is_fresh(&self, stamp: Option<FileStamp>) -> bool {
        stamp.is_some() && self.stamp == stamp
    }

    pub fn with_stamp(mut self, stamp: FileStamp) -> Self {
        self.stamp = Some(stamp);
        self
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn name_count(&self) -> usize {
        self.by_name.len()
    }

    /// Build the whole name table in one linear pass.
    ///
    /// Deliberately not a real XML parser. The file is 64 MB of extremely
    /// regular tag-per-line markup, and a DOM pass would materialise every one
    /// of the 133 094 names plus a copy of all 2 M lines; scanning line by line
    /// and recording byte offsets keeps the resident cost at the two maps while
    /// giving the same answers. The tag forms below are the only ones the
    /// generator emits (verified against revision 2.89 by counting every
    /// `system`/`item`/`text` occurrence).
    pub fn build(bytes: &[u8]) -> HistoryXmlIndex {
        let text = String::from_utf8_lossy(bytes);
        let mut entries: Vec<Entry> = Vec::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();
        // names collected since the last `<entry>`, waiting for its `<text>`
        let mut pending: Vec<String> = Vec::new();
        let mut current: Option<Entry> = None;
        let mut line_start = 0usize;

        // `split_inclusive`, not `lines()`: we need byte offsets, and `lines()`
        // strips the terminator so `start += line.len() + 1` drifts on CRLF
        // (this is the exact bug that made the DAT index read the wrong bytes —
        // see `datindex::DatIndex::build_impl`).
        for raw in text.split_inclusive('\n') {
            let next_start = line_start + raw.len();
            let line = raw.trim_end_matches(['\r', '\n']);
            let t = line.trim();

            if t.starts_with("<entry>") {
                pending.clear();
                current = None;
            } else if t.starts_with("<system ") {
                // <system name="88games" game="yes" />
                if let Some(n) = attr(t, "name") {
                    pending.push(n);
                }
            } else if t.starts_with("<item ") {
                // <item list="nes" name="100mandk" game="yes" />
                if let Some(n) = attr(t, "name") {
                    pending.push(n);
                }
            } else if t.starts_with("<text>") {
                // `<text>` is followed by the first line of content on the same
                // line (`<text>Nintendo Famicom cart. published 37 years ago:`),
                // so the payload starts just past the tag, not past the newline.
                let tag_at = line.find("<text>").unwrap_or(0);
                let text_start = line_start + tag_at + "<text>".len();
                // A block can also open and close on one line
                // (`<text>short entry.</text>`), so check for the close before
                // assuming the entry runs on. Emitting the entry here keeps a
                // one-line record from being swallowed by the branch below.
                if let Some(rel) = line[tag_at + "<text>".len()..].find("</text>") {
                    let idx = entries.len();
                    entries.push(Entry {
                        text_start,
                        text_end: text_start + rel,
                    });
                    for n in pending.drain(..) {
                        by_name.entry(n).or_insert(idx);
                    }
                    current = None;
                } else {
                    current = Some(Entry {
                        text_start,
                        text_end: next_start,
                    });
                }
            } else if let Some(cut) = line.find("</text>") {
                // `</text>` is **always inline** at the end of its content's
                // last line (`Edit this entry: https://…/?o=2</text>`), never on
                // a line of its own — verified across all 116 967 occurrences in
                // revision 2.89. So the test is `find`, not `starts_with`: with
                // `starts_with` this branch never fired, every entry stayed
                // unterminated, and the index came out completely empty while
                // the build still cost 366 ms.
                if let Some(mut e) = current.take() {
                    e.text_end = line_start + cut;
                    // guard against an inverted or zero-length range, which
                    // would panic the slice in `lookup`
                    if e.text_end > e.text_start {
                        let idx = entries.len();
                        entries.push(e);
                        for n in pending.drain(..) {
                            by_name.entry(n).or_insert(idx);
                        }
                    }
                }
            }
            line_start = next_start;
        }

        HistoryXmlIndex {
            stamp: None,
            entries,
            by_name,
        }
    }

    /// Byte range of the `<text>` payload for `name`, if the file has an entry
    /// for it. Reading the range is the caller's job — the index only decides
    /// *which bytes* are the record, the same split `datindex` uses.
    pub fn range_of(&self, name: &str) -> Option<(usize, usize)> {
        let idx = *self.by_name.get(name)?;
        let e = self.entries.get(idx)?;
        Some((e.text_start, e.text_end))
    }
}

/// Read one attribute out of an already-isolated tag line: `attr(line, "name")`
/// on `<system name="foo" game="yes" />` yields `foo`.
fn attr(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let at = line.find(&needle)? + needle.len();
    let rest = &line[at..];
    let end = rest.find('"')?;
    Some(unescape(&rest[..end]))
}

/// Decode the five entities the Arcade-History generator emits.
///
/// Counted over revision 2.89: `&quot;` 6 773, `&apos;` 5 267, `&amp;` 746,
/// `&gt;` 78, `&lt;` 1 per 200 k lines — and nothing else, so a general entity
/// table would be dead weight. `&amp;` is decoded **last** so that a literal
/// `&amp;lt;` in the source does not turn into `<`.
fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

/// Turn one `<text>` payload into the HTML-ish string the document panels
/// render.
///
/// Mirrors what `dat::get_history` does for a DAT record: every content line is
/// emitted followed by `<br>`, blank lines collapse into a paragraph break
/// (they were never dropped in the DAT path either — `<br><br>` is what made
/// the spacing read correctly), and a `- SECTION -` header becomes a rule so the
/// UI can draw the same divider the Command tab uses.
///
/// `finish_record` is applied by the caller so the `<br>` trimming and the MAWS
/// link stay in exactly one place for both formats.
pub fn render_text(payload: &str) -> String {
    let mut out = String::with_capacity(payload.len() + payload.len() / 8);
    for raw in payload.lines() {
        let line = unescape(raw);
        let t = line.trim_end();
        if t.is_empty() {
            continue;
        }
        if is_section_header(t) {
            // `- TRIVIA -` -> the box-drawing rule `dat::is_section_rule`
            // recognises, so `convert_history_lines` can promote it to a
            // divider + heading without a second convention.
            out.push_str("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
            out.push_str("<br>");
            out.push_str(&section_title(t));
            out.push_str("<br>");
            out.push_str("\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
            out.push_str("<br>");
            continue;
        }
        out.push_str(t);
        out.push_str("<br>");
    }
    out
}

/// `- TECHNICAL -` / `- TIPS AND TRICKS -` — a section header line.
///
/// The generator writes these as `- NAME -` with exactly one leading and one
/// trailing space. Requiring two hyphens and nothing else keeps a line of prose
/// that happens to start with a dash from being read as a heading.
fn is_section_header(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("- ") else {
        return false;
    };
    let Some(name) = rest.strip_suffix(" -") else {
        return false;
    };
    !name.is_empty() && !name.contains(" - ")
}

/// `- TECHNICAL -` -> `TECHNICAL`.
fn section_title(line: &str) -> String {
    line.strip_prefix("- ")
        .and_then(|s| s.strip_suffix(" -"))
        .unwrap_or(line)
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// process-wide index cache
// ---------------------------------------------------------------------------

/// Same shape as `datindex::DAT_CACHE`: one slot per file, invalidated by
/// mtime+size. The XML file is 64 MB and its name table is ~133 k entries, so
/// it must not be rebuilt per selection.
static XML_CACHE: std::sync::Mutex<Vec<(PathBuf, CachedXml)>> =
    std::sync::Mutex::new(Vec::new());

const XML_CACHE_SLOTS: usize = 4;

struct CachedXml {
    stamp: Option<FileStamp>,
    index: Option<Arc<HistoryXmlIndex>>,
    file: Option<std::fs::File>,
}

/// Look `name` up in the `history.xml` at `path`, returning the rendered
/// HTML-ish record (without `finish_record` applied).
///
/// `None` means "this is not something we can answer from here" — an unreadable
/// file, a missing name. The caller then falls back to the DAT scanner, which
/// also covers a path that turns out to hold the old format after all.
pub fn lookup(path: &Path, name: &str) -> Option<String> {
    let stamp = FileStamp::of(path)?;
    // Resolve (or build) the cached index, then **release the lock** before
    // reading the record: `std::sync::Mutex` is not reentrant and the build
    // below does file I/O. Two threads racing on a new file duplicate one
    // sequential pass; the push is keyed on the path so only one slot survives.
    //
    // The cached `File` is cloned rather than reopened for the range read:
    // lookups happen once per selection, and reusing the descriptor skips a
    // `CreateFile` + ACL check on a 64 MB file each time.
    let (index, file) = {
        let slot = {
            let cache = XML_CACHE.lock().ok()?;
            cache
                .iter()
                .position(|(p, c)| p == path && c.stamp == Some(stamp))
        };
        match slot {
            Some(s) => {
                let cache = XML_CACHE.lock().ok()?;
                let entry = &cache[s].1;
                (entry.index.clone()?, entry.file.as_ref()?.try_clone().ok()?)
            }
            None => {
                let bytes = std::fs::read(path).ok()?;
                let idx = Arc::new(HistoryXmlIndex::build(&bytes).with_stamp(stamp));
                let file = std::fs::File::open(path).ok()?;
                if let Ok(mut cache) = XML_CACHE.lock() {
                    if cache.len() >= XML_CACHE_SLOTS {
                        cache.remove(0);
                    }
                    cache.push((
                        path.to_path_buf(),
                        CachedXml {
                            stamp: Some(stamp),
                            index: Some(idx.clone()),
                            file: Some(file.try_clone().ok()?),
                        },
                    ));
                }
                (idx, file)
            }
        }
    };
    let (start, end) = index.range_of(name)?;
    let payload = read_range(file, start, end)?;
    Some(render_text(&payload))
}

/// Read `start..end` out of an already-open file handle.
fn read_range(mut file: std::fs::File, start: usize, end: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(start as u64)).ok()?;
    let len = end.checked_sub(start)?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two entries: a mainlist one and a softlist one, in the exact tag-per-line
    /// shape the generator emits, including the two asymmetries that are easy to
    /// get wrong — `<text>` carries its first content line on the same line, and
    /// `</text>` sits inline at the end of the last content line.
    const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<history version="2.89" date="2026-08-07">

	<entry>
		<software>
			<item list="nes" name="100mandk" game="yes" />
		</software>
		<text>Nintendo Famicom cart. published 37 years ago:

Casino Kid&apos;s subtitle is &quot;Maboroshi no Teiou-Hen&quot;.

- TECHNICAL -

GAME ID: SFL-KP

- TRIVIA -

Released on January 6, 1989 in Japan.</text>
	</entry>
	<entry>
		<systems>
			<system name="88games" game="yes" />
		</systems>
		<text>Arcade Video game kit published 38 years ago:

&apos;88 Games (c) 1988 Konami Industry Company, Limited.

- TECHNICAL -

GAME ID : GX861</text>
	</entry>
	<entry>
		<systems>
			<system name="88gamesa" game="yes" />
			<system name="88gamesb" game="yes" />
		</systems>
		<text>Shared by two names: 88 Games kit.</text>
	</entry>
</history>
"#;

    #[test]
    fn mainlist_names_resolve_to_their_text() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        let (s, e) = idx.range_of("88games").expect("88games indexed");
        let payload = &FIXTURE[s..e];
        assert!(payload.starts_with("Arcade Video game kit"), "got {payload:?}");
        assert!(payload.ends_with("GAME ID : GX861"), "got {payload:?}");
    }

    #[test]
    fn softlist_items_are_indexed_too() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        let (s, e) = idx.range_of("100mandk").expect("softlist name indexed");
        let payload = &FIXTURE[s..e];
        assert!(payload.starts_with("Nintendo Famicom cart."), "got {payload:?}");
    }

    /// A `<systems>` block with several `<system>` children must file **every**
    /// name to the same text, or a clone/second name silently has no history.
    #[test]
    fn every_name_in_a_block_maps_to_the_one_text() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        let a = idx.range_of("88gamesa").unwrap();
        let b = idx.range_of("88gamesb").unwrap();
        assert_eq!(a, b, "both names must point at the same text range");
    }

    /// The first entry claiming a name wins, matching the DAT scanner's
    /// top-to-bottom rule. `88games` appears only in entry 2.
    #[test]
    fn first_entry_wins_for_a_duplicated_name() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        let (s, _) = idx.range_of("88games").unwrap();
        let payload = &FIXTURE[s..];
        assert!(
            payload.starts_with("Arcade Video game kit"),
            "unexpected text: {payload:?}"
        );
    }

    #[test]
    fn entities_are_decoded() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        let (s, e) = idx.range_of("100mandk").unwrap();
        let out = render_text(&FIXTURE[s..e]);
        assert!(out.contains("Casino Kid's subtitle"), "apos: {out}");
        assert!(out.contains("\"Maboroshi no Teiou-Hen\""), "quot: {out}");
    }

    /// `&amp;lt;` must decode to the literal text `&lt;`, not to `<`. This is
    /// why `&amp;` is applied last.
    #[test]
    fn ampersand_is_decoded_last() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        let _ = idx;
        assert_eq!(unescape("&amp;lt;"), "&lt;");
        assert_eq!(unescape("Rock &amp; Roll"), "Rock & Roll");
    }

    #[test]
    fn section_headers_become_rules_with_the_title_between_them() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        let (s, e) = idx.range_of("100mandk").unwrap();
        let out = render_text(&FIXTURE[s..e]);
        // the rule is what `dat::is_section_rule` recognises, so the existing
        // renderer can promote it to a divider without a second convention
        let rule = "\u{2500}".repeat(8);
        assert!(out.contains(&format!("{rule}<br>TECHNICAL<br>{rule}")), "got {out}");
        assert!(out.contains(&format!("{rule}<br>TRIVIA<br>{rule}")), "got {out}");
    }

    #[test]
    fn prose_starting_with_a_dash_is_not_a_section_header() {
        assert!(is_section_header("- TRIVIA -"));
        assert!(is_section_header("- TIPS AND TRICKS -"));
        assert!(!is_section_header("- not a header"));
        assert!(!is_section_header("- a - b -"));
        assert!(!is_section_header("plain text"));
    }

    #[test]
    fn section_title_strips_the_markers() {
        assert_eq!(section_title("- TIPS AND TRICKS -"), "TIPS AND TRICKS");
        assert_eq!(section_title("- TECHNICAL -"), "TECHNICAL");
    }

    #[test]
    fn unknown_name_is_absent() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        assert!(idx.range_of("no_such_game").is_none());
    }

    /// An entry with no `<text>` at all must not produce an inverted or
    /// zero-length range that would then panic the slice in the caller.
    #[test]
    fn entry_without_text_is_skipped() {
        let xml = "<history version=\"2.89\">\n<entry>\n<systems>\n<system name=\"ghost\" game=\"yes\" />\n</systems>\n</entry>\n</history>\n";
        let idx = HistoryXmlIndex::build(xml.as_bytes());
        assert!(idx.range_of("ghost").is_none());
    }

    /// The name table and the entry list must agree: 116 967 `<entry>` blocks in
    /// the shipped file, but *more* names than entries because one entry can
    /// carry many `<system>`/`<item>` children.
    #[test]
    fn name_count_is_at_least_entry_count() {
        let idx = HistoryXmlIndex::build(FIXTURE.as_bytes());
        assert!(idx.name_count() >= idx.entry_count());
        assert_eq!(idx.entry_count(), 3);
        assert_eq!(idx.name_count(), 4);
    }
}
