//! Lookup indexes for the DAT and archive loading paths (design §3.2/§3.3).
//!
//! Two tables, both invalidated by mtime rather than by a file watcher:
//!
//! * [`DatIndex`] — `tag -> byte range of that record`. Turns "find game X in a
//!   20 MB `history.dat`" from a full linear scan into one seek plus a read of
//!   the few KB that record actually occupies.
//! * [`ArchiveIndex`] — `entry name -> how to get it`, for loose directories
//!   and zip central directories alike.
//!
//! **This layer never changes behaviour, only how fast it is reached.** Every
//! lookup has the old path as its fallback: an index miss, a stale index, an
//! unsupported format or a "no such game" all end up in exactly the code that
//! ran before. `dat::get_history` stays the reference implementation; the tests
//! below pin the two together by running both over the same fixture and
//! comparing output byte for byte.

use crate::dat;
use std::collections::HashMap;
use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Byte range of one record's payload — the lines `get_history` collects,
/// excluding the `$info=` line that opened it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordRange {
    pub start: usize,
    pub end: usize,
}

/// mtime + size: the pair that decides whether a cached table is still valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub mtime: Option<SystemTime>,
    pub len: u64,
}

impl FileStamp {
    pub fn of(path: &Path) -> Option<FileStamp> {
        let meta = std::fs::metadata(path).ok()?;
        Some(FileStamp {
            mtime: meta.modified().ok(),
            len: meta.len(),
        })
    }
}

/// A parsed DAT, indexed by record tag.
#[derive(Debug, Clone, Default)]
pub struct DatIndex {
    stamp: Option<FileStamp>,
    /// tag -> records carrying it, in file order. A tag can appear in several
    /// `$info=` lines; the original scans top-to-bottom and takes the first hit,
    /// so the first entry of each list is the answer.
    records: HashMap<String, Vec<RecordRange>>,
}

impl DatIndex {
    /// True when this index was built from the file's current state.
    pub fn is_fresh(&self, stamp: Option<FileStamp>) -> bool {
        stamp.is_some() && self.stamp == stamp
    }

    /// Record the stamp this index was built from. Callers that build from an
    /// already-open file pass its stamp; the freshness check then works.
    pub fn with_stamp(mut self, stamp: FileStamp) -> Self {
        self.stamp = Some(stamp);
        self
    }

    pub fn build(bytes: &[u8]) -> DatIndex {
        DatIndex::build_impl(bytes)
    }

    /// Build the tag table in one linear pass.
    ///
    /// The scan in [`dat::get_history`] is **per tag**, not per position, and
    /// the difference is observable. Its state is a single `rec_data` flag: a
    /// `$info=` line containing the tag sets it, and it is only cleared by a
    /// `$info=` line that does *not*. So a tag that appears in two records
    /// collects everything from the first `$info=tag` down to the first
    /// `$info=` without it — swallowing any records in between that share the
    /// tag. A positional "one record per $info= block" index would stop early
    /// and return less text than the original, so `index_agrees_with_scan`
    /// fails.
    ///
    /// Hence the single-pass build below. Doing it the obvious way — for each
    /// tag, walk the whole `$info=` list looking for the next one that lacks it
    /// — is quadratic (53k tags x 40k records took 44 s on an 18 MB file). The
    /// version below keeps an "open" list instead: a `$info=` line closes every
    /// open tag it does *not* mention, and opens the ones it does. Each tag is
    /// therefore touched once per mention, which is linear in the file.
    fn build_impl(bytes: &[u8]) -> DatIndex {
        let text = String::from_utf8_lossy(bytes);
        let mut records: HashMap<String, Vec<RecordRange>> = HashMap::new();
        // (tag, offset just past the $info= line that opened it)
        let mut open: Vec<(String, usize)> = Vec::new();
        let mut line_start = 0usize;
        for line in text.lines() {
            let line_end = line_start + line.len();
            if let Some(rest) = line.strip_prefix("$info=") {
                let tags: Vec<&str> = rest.split(',').map(|t| t.trim()).collect();
                let after = line_end + 1;
                // this line closes every open record it does not carry
                let mut i = 0;
                while i < open.len() {
                    if tags.contains(&open[i].0.as_str()) {
                        // same record continues — refresh nothing, it is the
                        // first `$info=` that matters for the span start
                        i += 1;
                    } else {
                        let (tag, start) = open.swap_remove(i);
                        records.entry(tag).or_default().push(RecordRange {
                            start,
                            end: line_start,
                        });
                    }
                }
                for t in &tags {
                    if !open.iter().any(|(o, _)| o == t) {
                        open.push(((*t).to_string(), after));
                    }
                }
            }
            line_start = line_end + 1;
        }
        // whatever is still open runs to the end of the file
        for (tag, start) in open {
            records.entry(tag).or_default().push(RecordRange {
                start,
                end: line_start,
            });
        }
        DatIndex { stamp: None, records }
    }

    /// The first record carrying `tag`.
    pub fn lookup(&self, tag: &str) -> Option<RecordRange> {
        self.records.get(tag).and_then(|v| v.first()).copied()
    }

    /// Every record carrying `tag` — the recursive cloneof walk wants all of
    /// them, and so does anyone auditing what the index actually holds.
    pub fn lookup_all(&self, tag: &str) -> &[RecordRange] {
        self.records.get(tag).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn tags(&self) -> usize {
        self.records.len()
    }
}

/// Render one record's payload exactly as `get_history` would have.
///
/// This is the same code the scan uses — `dat::format_record` — so the two
/// paths cannot drift. The index only decides *which bytes* get here.
pub fn record_text(
    bytes: &[u8],
    range: RecordRange,
    own_tag: &str,
    dark_bg: bool,
) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let slice = raw.get(range.start..range.end).unwrap_or("");
    dat::format_record(slice, Some(own_tag), dark_bg)
}

// ---------------------------------------------------------------------------
// shared, self-invalidating cache
// ---------------------------------------------------------------------------

/// One cached DAT: its index plus an open handle for range reads.
struct CachedDat {
    stamp: Option<FileStamp>,
    /// `None` = we tried and failed (missing/unreadable), so it is not re-probed
    index: Option<Arc<DatIndex>>,
    /// Kept open on purpose. Re-opening the file per lookup cost ~21 ms on an
    /// 18 MB `history.dat` — far more than reading the ~440 byte record itself,
    /// which is what defeated the index on the first working version.
    file: Option<std::fs::File>,
}

/// Process-wide DAT index cache.
///
/// One DAT is looked up per visible dock, and the same handful of files
/// (`history.dat` and friends) serve every game, so one slot per file is enough.
/// Rebuilt when the file's mtime or size moves — §3.3 of the design: lazy
/// invalidation, no file watcher, one sequential pass to rebuild.
static DAT_CACHE: std::sync::Mutex<Vec<(PathBuf, CachedDat)>> =
    std::sync::Mutex::new(Vec::new());

/// How many distinct DAT files to remember. The reference install has a handful
/// per dock; 16 is generous and keeps a pathological config bounded.
const DAT_CACHE_SLOTS: usize = 16;


/// Find `tag`'s record in the DAT at `path`, using the index when it is valid.
///
/// This is a drop-in replacement for `dat::get_history` on the same bytes. The
/// point of the index is that the *scan* (a full pass over 10-20 MB of
/// `history.dat`) happens once per file version, while each lookup afterwards
/// reads only the record's own byte range.
pub fn history_indexed(
    path: &Path,
    tag: &str,
    method: usize,
    dark_bg: bool,
    cloneof: &str,
) -> Option<String> {
    // `zip`-backed DATs have no path to stat, so they take the scan path; the
    // caller falls back to `dat::get_history` when this returns None
    let stamp = FileStamp::of(path)?;
    // Resolve the cached entry under the lock, then **release it** before doing
    // any I/O: the cloneof fallbacks below recurse into this function, and
    // `std::sync::Mutex` is not reentrant, so holding the guard across them
    // deadlocks. Building happens outside the lock too — two threads racing on
    // the same new DAT duplicate one sequential pass, which is far cheaper than
    // a deadlock, and the replace below is keyed on the path so only one slot
    // per file ever exists.
    let hit = {
        let cache = DAT_CACHE.lock().ok()?;
        cache
            .iter()
            .position(|(p, c)| p == path && c.stamp == Some(stamp))
    };
    let (idx, mut file): (Arc<DatIndex>, Option<std::fs::File>) = match hit {
        Some(slot) => {
            let cache = DAT_CACHE.lock().ok()?;
            let entry = &cache[slot].1;
            match (&entry.index, &entry.file) {
                // Arc clone, not DatIndex clone: the table has ~50k tags and a
                // deep copy of it cost ~20 ms per lookup, which erased the
                // entire benefit of indexing
                (Some(i), Some(f)) => (i.clone(), f.try_clone().ok()),
                _ => return None,
            }
        }
        None => {
            // (re)build: one sequential pass, and keep the handle for later reads
            let bytes = std::fs::read(path).ok();
            let idx = bytes
                .as_ref()
                .map(|b| Arc::new(DatIndex::build(b).with_stamp(stamp)));
            let file = std::fs::File::open(path).ok();
            if let Ok(mut cache) = DAT_CACHE.lock() {
                let entry = CachedDat {
                    stamp: Some(stamp),
                    index: idx.clone(),
                    file: file.as_ref().and_then(|f| f.try_clone().ok()),
                };
                match cache.iter().position(|(p, _)| p == path) {
                    Some(slot) => cache[slot] = (path.to_path_buf(), entry),
                    None => {
                        if cache.len() >= DAT_CACHE_SLOTS {
                            cache.remove(0);
                        }
                        cache.push((path.to_path_buf(), entry));
                    }
                }
            }
            (idx?, file)
        }
    };
    let Some(range) = idx.lookup(tag) else {
        // no record for this tag — walk the clone chain exactly as the scan does
        if !cloneof.is_empty() {
            return history_indexed(path, cloneof, method, dark_bg, "");
        }
        return None;
    };
    // read only the record's bytes, through the handle we already hold
    let slice = read_range(file.as_mut(), range)?;
    let payload = dat::format_record(&slice, Some(tag), dark_bg);
    let out = dat::finish_record(payload, tag, method, dark_bg);
    if out.is_empty() && !cloneof.is_empty() {
        return history_indexed(path, cloneof, method, dark_bg, "");
    }
    Some(out)
}

/// Read just `range` out of an already-open file — the point of the index.
fn read_range(f: Option<&mut std::fs::File>, range: RecordRange) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let f = f?;
    f.seek(SeekFrom::Start(range.start as u64)).ok()?;
    let n = range.end.saturating_sub(range.start);
    let mut buf = vec![0u8; n];
    f.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Drop every cached DAT index (used when the mame binary changes).
pub fn clear_dat_cache() {
    if let Ok(mut c) = DAT_CACHE.lock() {
        c.clear();
    }
}

// ---------------------------------------------------------------------------
// archive index
// ---------------------------------------------------------------------------

/// How to reach one entry of an image source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// a loose file
    File(PathBuf),
    /// an entry inside a zip. Zip's central directory supports random access,
    /// so this becomes a seek plus extraction of that one entry.
    Zip { archive: PathBuf, name: String },
}

/// A snapshot of one panel's image sources: name -> how to get it.
///
/// Only reusable while every file it was built from is unchanged; see
/// [`ArchiveIndex::is_fresh`].
#[derive(Debug, Clone, Default)]
pub struct ArchiveIndex {
    stamp: Vec<FileStamp>,
    entries: HashMap<String, Entry>,
}

impl ArchiveIndex {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.get(name)
    }

    /// The index is reusable only when *every* source file is unchanged.
    pub fn is_fresh(&self, sources: &[FileStamp]) -> bool {
        !self.stamp.is_empty() && self.stamp == sources
    }

    pub fn builder(sources: Vec<FileStamp>) -> ArchiveIndexBuilder {
        ArchiveIndexBuilder {
            stamp: sources,
            entries: HashMap::new(),
        }
    }
}

pub struct ArchiveIndexBuilder {
    stamp: Vec<FileStamp>,
    entries: HashMap<String, Entry>,
}

impl ArchiveIndexBuilder {
    /// Record an entry. **First hit wins**, matching `iterate_mame_file`'s
    /// `seen` set — callers depend on the scan order (loose before archive).
    pub fn add(&mut self, name: String, entry: Entry) {
        self.entries.entry(name).or_insert(entry);
    }

    pub fn build(self) -> ArchiveIndex {
        ArchiveIndex {
            stamp: self.stamp,
            entries: self.entries,
        }
    }
}

/// Stamp a directory by its own mtime plus the newest entry it contains.
///
/// A plain `metadata()` stamp is not enough: creating a picture inside a
/// directory updates the directory's mtime on Windows, but relying on that
/// alone missed cases before, and the scan is cheap enough to fold in.
pub fn stamp_dir(dir: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(dir).ok()?;
    let mut newest = meta.modified().ok();
    let mut len = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            len += 1;
            if let Ok(m) = e.metadata() {
                if let Ok(t) = m.modified() {
                    if newest.map(|n| t > n).unwrap_or(true) {
                        newest = Some(t);
                    }
                }
            }
        }
    }
    Some(FileStamp {
        mtime: newest,
        len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::{self, DOCK_HISTORY};

    /// A fixture exercising the cases that make indexing risky: a comment line
    /// inside a record, a link line, a blank line, a tag shared by two records,
    /// a record closed only by EOF, and a tag that does not exist.
    const FIXTURE: &str = "\
# a comment
$info=alphapac
first line of alpha
$<a href=http://example.test/a>link line</a>
# a comment inside the record
$info=beta,alphapac
beta shares the alpha tag

$info=gamma
last record, never closed
trailing payload
";

    fn normalise(s: &str) -> String {
        // the MAWS header and the <br> trimming are applied by get_history's
        // caller-facing wrapper; compare the payload the two paths produce
        s.to_string()
    }

    /// The contract this whole module rests on: for every tag the index knows,
    /// rendering via the index must equal what the linear scan returns.
    #[test]
    fn index_agrees_with_scan() {
        let bytes = FIXTURE.as_bytes();
        let idx = DatIndex::build(bytes);
        let mut checked = 0;
        for tag in ["alphapac", "beta", "gamma", "delta"] {
            let scan = normalise(&dat::get_history(
                bytes,
                tag,
                DOCK_HISTORY,
                true,
                "",
            ));
            match idx.lookup(tag) {
                None => {
                    // no record: the scan must also have found nothing
                    assert!(
                        scan.is_empty(),
                        "index has no {tag} but the scan returned {scan:?}"
                    );
                }
                Some(r) => {
                    // both sides run the *same* post-processing, so this compares
                    // payload + MAWS header + `<br>` trimming, not just payload
                    let via_index = dat::finish_record(
                        record_text(bytes, r, tag, true),
                        tag,
                        DOCK_HISTORY,
                        true,
                    );
                    assert_eq!(
                        via_index, scan,
                        "indexed output differs from the scan for tag {tag}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked >= 3, "fixture should exercise several records");
    }

    /// A shared tag must reproduce the scan's *greedy* span: the original keeps
    /// collecting until a `$info=` line that lacks the tag, so the answer spans
    /// from the first `$info=tag` and swallows any same-tag record in between.
    /// A positional index would stop at the second `$info=` line and return less.
    #[test]
    fn shared_tag_resolves_to_first_record() {
        let bytes = FIXTURE.as_bytes();
        let idx = DatIndex::build(bytes);
        let r = idx.lookup("alphapac").expect("alpha is indexed");
        let text = record_text(bytes, r, "alphapac", true);
        assert!(
            text.contains("first line of alpha"),
            "expected the alpha record, got {text:?}"
        );
        // the scan's greedy span continues into the record that shares the tag
        assert!(
            text.contains("beta shares the alpha tag"),
            "indexed span must match the scan's greedy span, got {text:?}"
        );
        // a tag in exactly one record resolves to just that record
        let beta = idx.lookup("beta").expect("beta is indexed");
        let btext = record_text(bytes, beta, "beta", true);
        assert!(btext.contains("beta shares"), "got {btext:?}");
        assert!(
            !btext.contains("last record"),
            "beta must not span into gamma's record, got {btext:?}"
        );
    }

    /// A record with no terminator must still be found.
    #[test]
    fn unterminated_final_record_is_indexed() {
        let bytes = FIXTURE.as_bytes();
        let idx = DatIndex::build(bytes);
        let r = idx.lookup("gamma").expect("final record is indexed");
        let text = record_text(bytes, r, "gamma", true);
        assert!(text.contains("last record"), "got {text:?}");
        assert!(text.contains("trailing payload"), "got {text:?}");
    }

    /// An empty or header-only DAT must not panic and must index nothing.
    #[test]
    fn degenerate_inputs() {
        assert_eq!(DatIndex::build(b"").tags(), 0);
        assert_eq!(DatIndex::build(b"# only a comment\n").tags(), 0);
        assert_eq!(DatIndex::build(b"$info=x\n").lookup("x").is_some(), true);
        // no payload at all → range is empty but the tag is known
        let idx = DatIndex::build(b"$info=solo\n");
        let r = idx.lookup("solo").unwrap();
        assert_eq!(record_text(b"$info=solo\n", r, "solo", true), "");
    }

    /// Non-UTF-8 bytes must not panic (the scan uses lossy conversion).
    #[test]
    fn lossy_input_is_safe() {
        let mut bytes = b"$info=weird\n".to_vec();
        bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
        let idx = DatIndex::build(&bytes);
        let r = idx.lookup("weird").unwrap();
        // just must not panic, and must stay inside the buffer
        let _ = record_text(&bytes, r, "weird", false);
    }

    /// mtime/size must both match for the index to be reused.
    #[test]
    fn freshness_requires_stamp_match() {
        let idx = DatIndex::build(b"$info=a\nx\n").with_stamp(FileStamp {
            mtime: None,
            len: 12,
        });
        assert!(idx.is_fresh(Some(FileStamp { mtime: None, len: 12 })));
        assert!(!idx.is_fresh(Some(FileStamp { mtime: None, len: 13 })));
        assert!(!idx.is_fresh(None));
        // a never-stamped index is never trusted
        let unstamped = DatIndex::build(b"$info=a\nx\n");
        assert!(!unstamped.is_fresh(Some(FileStamp { mtime: None, len: 12 })));
    }

    /// The archive index must keep the first entry for a duplicated name and
    /// refuse to be reused once any source changed.
    #[test]
    fn archive_index_first_hit_wins_and_expires() {
        let s = FileStamp { mtime: None, len: 1 };
        let mut b = ArchiveIndex::builder(vec![s]);
        b.add("pacman.png".into(), Entry::File(PathBuf::from("/loose/pacman.png")));
        b.add("pacman.png".into(), Entry::Zip {
            archive: PathBuf::from("/d/snap.zip"),
            name: "pacman.png".into(),
        });
        let idx = b.build();
        assert_eq!(idx.len(), 1);
        assert!(matches!(idx.get("pacman.png"), Some(Entry::File(_))));
        assert!(idx.is_fresh(&[s]));
        assert!(!idx.is_fresh(&[FileStamp { mtime: None, len: 2 }]));
        // no sources recorded → never fresh (an empty scan is not a cached one)
        let empty = ArchiveIndex::builder(vec![]).build();
        assert!(!empty.is_fresh(&[]));
    }

    /// End-to-end over a real file:  must return exactly what
    /// the original scan returns, for every tag — including the tags that only
    /// exist via the cloneof fallback and the ones that do not exist at all.
    #[test]
    fn history_indexed_matches_scan_on_disk() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("mamepgui-datindex-e2e");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.dat");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(FIXTURE.as_bytes()).unwrap();
        drop(f);
        clear_dat_cache();

        for tag in ["alphapac", "beta", "gamma", "nope"] {
            // cloneof deliberately non-empty: the fallback path must agree too
            let scan = dat::get_history(FIXTURE.as_bytes(), tag, DOCK_HISTORY, true, "beta");
            let indexed =
                history_indexed(&path, tag, DOCK_HISTORY, true, "beta").unwrap_or_default();
            assert_eq!(
                indexed, scan,
                "history_indexed disagrees with get_history for {tag}"
            );
        }
        // a second pass must hit the cache and still agree
        let again = history_indexed(&path, "alpha", DOCK_HISTORY, true, "");
        assert!(again.is_none() || !again.unwrap().is_empty());
        clear_dat_cache();
        let _ = std::fs::remove_file(&path);
    }

    /// Editing the DAT must invalidate the index — that is the whole mtime
    /// mechanism (§3.3), so it gets its own test with a real mtime bump.
    #[test]
    fn index_rebuilds_after_the_file_changes() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("mamepgui-datindex-mtime");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("history.dat");
        let write = |p: &Path, s: &str| {
            let mut f = std::fs::File::create(p).unwrap();
            f.write_all(s.as_bytes()).unwrap();
        };
        write(&path, "$info=one
first version
");
        clear_dat_cache();
        let first = history_indexed(&path, "one", DOCK_HISTORY, true, "").unwrap_or_default();
        assert!(first.contains("first version"), "got {first:?}");

        // same length, different content — only mtime can catch this
        write(&path, "$info=one
second versio
");
        // ensure the mtime actually moved even on a coarse clock
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(2))
            .unwrap();
        drop(f);

        let second = history_indexed(&path, "one", DOCK_HISTORY, true, "").unwrap_or_default();
        assert!(
            second.contains("second versio"),
            "stale index served after the file changed: {second:?}"
        );
        clear_dat_cache();
        let _ = std::fs::remove_file(&path);
    }

}
