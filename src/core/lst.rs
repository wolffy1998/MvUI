//! The localized game list: `mame_cn.lst`.
//!
//! # Format
//!
//! A tab-separated file, one machine per line, no header:
//!
//! ```text
//! pacman\t吃豆人 (Puck Man)\t吃豆人 (Puck Man)
//! 005\t005 情报员 \t005 情报员
//! ```
//!
//! Column 1 is the MAME set name, columns 2 and 3 are the localized description
//! and manufacturer line. In every file 1.8.2-era MAME Plus! shipped, columns 2
//! and 3 hold the same string; they are still read separately so a file that
//! does distinguish them works.
//!
//! # Encoding
//!
//! UTF-8 as documented — but the files in the wild are usually GB18030, and a
//! mis-decoded list is worse than none at all (every title turns into mojibake
//! and the "localized list" looks broken). [`read_text_file`] tries
//! UTF-8-with-BOM, then plain UTF-8, then GB18030, so both work.
//!
//! # What replaced what
//!
//! 1.8.2 read `.mmo` (a binary "MAME output" file) and filled `lc_desc` /
//! `lc_mftr` from it (`gamelist.cpp:1830-1885`). The port never implemented
//! that, so the Localized Game List switch was a no-op: `lc_desc` was always
//! empty and the list rendered exactly like the unlocalized one. This module
//! is the replacement, and it is what the switch now actually toggles.

use std::collections::HashMap;
use std::path::Path;

use crate::core::options::read_text_file;
use crate::core::paths;

/// set name → (localized description, localized manufacturer)
pub type LocalMap = HashMap<String, (String, String)>;

/// Parse the text of a `mame_cn.lst`.
///
/// Tolerant by design — these files are hand-maintained in the community and
/// real ones contain blank lines, `#` comments, single-column rows and stray
/// whitespace. A row that cannot yield both a name and a description is
/// skipped rather than aborting the load, because one bad line should not cost
/// the user the other 49 000.
pub fn parse_list(text: &str) -> LocalMap {
    let mut map = LocalMap::with_capacity(8192);
    for line in text.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let mut cols = line.split('\t');
        let Some(name) = cols.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        // a row with only a name carries no localization — leave the set
        // untranslated instead of blanking its real description
        let Some(desc) = cols.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let mftr = cols
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(desc);
        // first row wins: the list is sorted, and a later duplicate is either a
        // variant entry or an outright mistake
        map.entry(name.to_string())
            .or_insert_with(|| (desc.to_string(), mftr.to_string()));
    }
    map
}

/// Read and parse `mame_cn.lst` from `path`.
pub fn load(path: &Path) -> LocalMap {
    read_text_file(path).map(|t| parse_list(&t)).unwrap_or_default()
}

/// Read the list from its default location (next to `mvui.exe`, or wherever
/// Settings ▸ Directories points it).
pub fn load_default() -> LocalMap {
    load(&paths::localized_list(None))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
pacman\t吃豆人 (Puck Man)\t吃豆人 (Puck Man)
005\t005 情报员 \t005 情报员
100lions\t一百雄狮 (10219211, NSW/ACT)\t一百雄狮 (10219211, NSW/ACT)
";

    #[test]
    fn parses_the_three_column_form() {
        let m = parse_list(SAMPLE);
        assert_eq!(m.len(), 3);
        let (desc, mftr) = &m["pacman"];
        assert_eq!(desc, "吃豆人 (Puck Man)");
        assert_eq!(mftr, "吃豆人 (Puck Man)");
        // the trailing space in the real file must not leak into the title
        assert_eq!(m["005"].0, "005 情报员");
    }

    #[test]
    fn keeps_distinct_manufacturer_when_present() {
        let m = parse_list("neogeo\t Neo Geo \tSNK\n");
        assert_eq!(m["neogeo"], ("Neo Geo".to_string(), "SNK".to_string()));
    }

    #[test]
    fn skips_junk_without_losing_the_rest() {
        let m = parse_list(
            "\
# a comment

pacman\t吃豆人
lonely
\tno name here
puckman\t食人花
",
        );
        assert_eq!(m.len(), 2, "only the two usable rows: {m:?}");
        assert!(m.contains_key("pacman"));
        assert!(m.contains_key("puckman"));
        // a name with no description is left untranslated rather than blanked
        assert!(!m.contains_key("lonely"));
    }

    #[test]
    fn first_duplicate_wins() {
        let m = parse_list("pacman\tfirst\tfirst\npacman\tsecond\tsecond\n");
        assert_eq!(m["pacman"].0, "first");
    }

    #[test]
    fn decodes_gb18030_when_not_utf8() {
        // "吃豆人" in GB18030 — the encoding the shipped files actually use
        let mut bytes = Vec::new();
        for ch in "pacman\t吃豆人".chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            if ch.is_ascii() {
                bytes.extend_from_slice(s.as_bytes());
            } else {
                // encode via the same table read_text_file decodes with
                let (cow, _, _) = encoding_rs::GB18030.encode(s);
                bytes.extend_from_slice(&cow);
            }
        }
        let tmp = std::env::temp_dir().join("mamepgui-lst-encoding-test");
        std::fs::write(&tmp, &bytes).unwrap();
        let m = load(&tmp);
        assert_eq!(m.get("pacman").map(|x| x.0.as_str()), Some("吃豆人"));
        let _ = std::fs::remove_file(&tmp);
    }
}
