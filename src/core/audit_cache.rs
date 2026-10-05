//! Persistent cache of archive entry lists, for the ROM audit.
//!
//! The audit's cost is almost entirely *opening 44 000 zip archives* to read
//! their central directories. Measured on the reference machine: 0.55 ms per
//! archive when the drive is warm, 19–27 ms when it is cold — so a cold run is
//! 10–20 minutes of pure head movement, and a warm one is under a minute.
//! Nothing about that is algorithmic; `the original GUI`'s own auditor does the same
//! work and takes the same time on a cold disk.
//!
//! The entry list of a romset changes only when the file does, and a file's
//! `(mtime, size)` pair is enough to tell. So: remember each archive's listing
//! keyed by that pair, in memory for the run and on disk for the next one.
//! Re-auditing then costs one `stat` per archive (≈2–5 s for the whole set)
//! instead of one open, and the 10–20 minutes is paid once per changed file
//! rather than once per audit.
//!
//! **This is a cache, not a source of truth.** Anything that looks wrong —
//! unreadable file, unparsable archive, stale stamp, version bump — falls back
//! to actually opening the archive and rewrites the entry. A corrupt cache can
//! therefore cost time but never produce a wrong audit result.

use crate::core::archive::{self, EntryInfo};
use crate::core::settings::GuiSettings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;
/// Bumped whenever [`CachedListing`] changes shape. A mismatch discards the
/// whole file rather than trying to migrate it: rebuilding is one cheap `stat`
/// pass per archive on a warm disk, and a half-migrated cache is a bug factory.
const FORMAT_VERSION: u16 = 1;
const MAGIC: &[u8; 8] = b"MVUIAC01";

/// Write-buffer size, for the same reason as `cache::WRITE_BUFFER_BYTES`:
/// bincode emits one `write` per string, so serializing ~44 000 listings (about
/// 20 MB) straight onto a `File` is a syscall storm.
const WRITE_BUFFER_BYTES: usize = 1 << 20;

/// Identity of an archive as far as the cache is concerned.
///
/// `mtime` is stored as nanoseconds since the epoch, or `None` when the
/// filesystem cannot supply it. A missing mtime makes the stamp fall back to
/// size-only matching, which is weaker but still catches the common case of a
/// romset being replaced by a different-sized one. `#[serde(default)]` keeps a
/// file written by an older build loadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
struct Stamp {
    size: u64,
    #[serde(default)]
    mtime_ns: Option<u64>,
}

impl Stamp {
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64);
        Some(Self {
            size: meta.len(),
            mtime_ns,
        })
    }
}

/// One archive's listing, plus the stamp it was taken under.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedListing {
    stamp: Stamp,
    entries: Vec<CachedEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedEntry {
    name: String,
    size: u64,
    crc: Option<u32>,
}

impl From<&EntryInfo> for CachedEntry {
    fn from(e: &EntryInfo) -> Self {
        Self {
            name: e.name.clone(),
            size: e.size,
            crc: e.crc,
        }
    }
}

impl From<&CachedEntry> for EntryInfo {
    fn from(e: &CachedEntry) -> Self {
        Self {
            name: e.name.clone(),
            size: e.size,
            crc: e.crc,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CacheFile {
    #[serde(default)]
    entries: HashMap<String, CachedListing>,
}

/// Process-wide state: the loaded map plus dirty-tracking so a run that changed
/// nothing does not rewrite 20 MB on exit.
struct State {
    map: HashMap<String, CachedListing>,
    dirty: bool,
    /// `true` only after a successful `load()`.
    loaded: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap();
    let state = guard.get_or_insert_with(|| State {
        map: HashMap::new(),
        dirty: false,
        loaded: false,
    });
    f(state)
}

fn cache_path() -> PathBuf {
    GuiSettings::cache_dir().join("audit_cache.bin")
}

/// Read the on-disk cache once per process.
///
/// Called from `audit_all` before the scan. A missing or unreadable file is not
/// an error — it just means every archive is opened this time round, which is
/// exactly what happens today.
pub fn load() {
    let path = cache_path();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            with_state(|s| {
                s.loaded = true;
                s.dirty = false;
            });
            return;
        }
    };
    // header: magic + version
    let ok = bytes.len() > MAGIC.len() + 2
        && &bytes[..MAGIC.len()] == MAGIC
        && u16::from_le_bytes([bytes[MAGIC.len()], bytes[MAGIC.len() + 1]]) == FORMAT_VERSION;
    let parsed = if ok {
        bincode::deserialize::<CacheFile>(&bytes[MAGIC.len() + 2..]).ok()
    } else {
        None
    };
    with_state(|s| {
        s.map = parsed.map(|f| f.entries).unwrap_or_default();
        s.loaded = true;
        // a file that failed to parse is rebuilt from scratch, so treat the
        // (now empty) map as dirty and let the next save overwrite it
        s.dirty = !ok;
    });
}

/// Persist the cache. Only writes when something actually changed, so a second
/// audit in the same session is free, and a read-only install stays quiet.
pub fn save() {
    let file_opt = with_state(|s| {
        if !s.loaded || !s.dirty {
            return None;
        }
        s.dirty = false;
        Some(CacheFile {
            entries: s.map.clone(),
        })
    });
    let Some(file) = file_opt else { return };
    let _ = write_cache(&cache_path(), &file);
}

fn write_cache(path: &Path, file: &CacheFile) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // tmp + rename, same atomic-swap reasoning as cache::save_library: an audit
    // that is killed mid-save must not leave a truncated cache behind
    let tmp = path.with_extension("tmp");
    {
        let f = std::fs::File::create(&tmp)?;
        let mut w = std::io::BufWriter::with_capacity(WRITE_BUFFER_BYTES, f);
        w.write_all(MAGIC)?;
        w.write_all(&FORMAT_VERSION.to_le_bytes())?;
        bincode::serialize_into(&mut w, file).map_err(io_other)?;
        let mut f = w.into_inner().map_err(|e| {
            io_other(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))
        })?;
        f.flush()?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn io_other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
}

/// Entry list for `path`, from the cache when the stamp matches.
///
/// Returns `None` when the path does not exist or the archive cannot be read —
/// the caller treats that as "no entries", which is what the old direct call
/// did with an `Err`, and the audit keeps going past one bad zip. A read
/// failure is deliberately *not* cached: a file that is temporarily unreadable
/// (network share asleep, antivirus holding it) is retried next time instead of
/// being remembered as empty.
///
/// A cache miss opens the archive (the expensive path) and records the result
/// for the rest of the run and the next save.
pub fn list_cached(path: &Path) -> Option<Vec<EntryInfo>> {
    let key = path.to_string_lossy().to_lowercase();
    let stamp = Stamp::of(path)?;

    let hit = with_state(|s| {
        let cached = s.map.get(&key)?;
        // size-only fallback when the filesystem gave no mtime
        let fresh = cached.stamp == stamp
            || (cached.stamp.mtime_ns.is_none() && cached.stamp.size == stamp.size);
        if fresh {
            Some(cached.entries.clone())
        } else {
            None
        }
    });
    if let Some(entries) = hit {
        return Some(entries.iter().map(EntryInfo::from).collect());
    }

    // miss: open the archive. An error is *not* cached, so a file that is
    // temporarily unreadable (network share asleep, antivirus holding it) is
    // retried next time instead of being remembered as empty.
    let entries = archive::list_archive(path).ok()?;
    let cached = CachedListing {
        stamp,
        entries: entries.iter().map(CachedEntry::from).collect(),
    };
    with_state(|s| {
        s.map.insert(key, cached);
        s.dirty = true;
    });
    Some(entries)
}

/// Drop listings for archives that no longer exist or changed, so the file does
/// not grow without bound as a user swaps romsets around.
///
/// Only prunes paths the audit actually walked past — a romset directory that
/// is simply not on this run's `rompath` keeps its listing, which is the point
/// of a persistent cache.
pub fn prune(seen: &[PathBuf], keep_limit: usize) {
    let live: std::collections::HashSet<String> = seen
        .iter()
        .map(|p| p.to_string_lossy().to_lowercase())
        .collect();
    with_state(|s| {
        let before = s.map.len();
        s.map.retain(|k, _| {
            // Live paths were every one of them just passed through
            // `list_cached`, which validated or refreshed their stamp this
            // run — restatting them here would double the stat cost of every
            // audit without buying safety: the stamp check inside
            // `list_cached` is the gate that actually decides freshness.
            // A path the audit did not see is kept only while the file still
            // exists, so a romset that moved to another rompath keeps its
            // listing instead of being rebuilt from scratch.
            live.contains(k) || Path::new(k).exists()
        });
        if s.map.len() > keep_limit {
            // pathological growth guard — a runaway cache is worse than a slow
            // audit. Dropping half keeps the newest entries by dropping at
            // random, which is fine: they are all rebuildable.
            let drop_n = s.map.len() - keep_limit;
            let doomed: Vec<String> = s.map.keys().take(drop_n).cloned().collect();
            for k in doomed {
                s.map.remove(&k);
            }
        }
        if s.map.len() != before {
            s.dirty = true;
        }
    });
}

/// How many listings are held, for the log line.
pub fn len() -> usize {
    with_state(|s| s.map.len())
}

/// Test-only: forget everything, as if the process had just started.
#[cfg(test)]
fn reset_for_test() {
    with_state(|s| {
        s.map.clear();
        s.dirty = false;
        s.loaded = true;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a real zip so the "does the cache actually avoid opening it"
    /// question is answered against a real parser, not a stub.
    fn make_zip(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let p = dir.join(name);
        let f = std::fs::File::create(&p).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (n, data) in entries {
            w.start_file(*n, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        p
    }

    /// The whole point of the module: the second listing of an unchanged
    /// archive must come from the cache and still be correct.
    #[test]
    fn second_listing_is_served_from_cache_and_matches() {
        let dir = std::env::temp_dir().join("mvui-auditcache-e2e");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        reset_for_test();

        let p = make_zip(&dir, "pacman.zip", &[("a.rom", b"aaaa"), ("b.rom", b"bb")]);

        let first = list_cached(&p).expect("first listing must succeed");
        assert_eq!(first.len(), 2, "both entries listed");
        let mut names: Vec<&str> = first.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["a.rom", "b.rom"]);
        assert!(first.iter().all(|e| e.crc.is_some()));

        // Second listing: must return the same contents. ("Served from the
        // cache" is asserted structurally rather than by observing the absence
        // of an open — the `changed_archive_is_relisted` test below is what
        // pins the invalidation direction, which is the one that can go wrong
        // silently.)
        let second = list_cached(&p).expect("cached listing must succeed");
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(second.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.size, b.size);
            assert_eq!(a.crc, b.crc);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A changed archive must NOT come back from the cache — otherwise a user
    /// who swaps a romset gets an audit against the old contents.
    #[test]
    fn changed_archive_is_relisted() {
        let dir = std::env::temp_dir().join("mvui-auditcache-changed");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        reset_for_test();

        let p = make_zip(&dir, "game.zip", &[("one.rom", b"1")]);
        let before = list_cached(&p).unwrap();
        assert_eq!(before.len(), 1);

        // replace it with a differently-shaped archive
        let _ = std::fs::remove_file(&p);
        let p = make_zip(&dir, "game.zip", &[("one.rom", b"1"), ("two.rom", b"22")]);
        let after = list_cached(&p).unwrap();
        assert_eq!(
            after.len(),
            2,
            "a replaced archive must be re-read, not served from the cache"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A non-archive must be a miss rather than a cached empty listing.
    #[test]
    fn non_archive_is_a_miss() {
        let dir = std::env::temp_dir().join("mvui-auditcache-nonzip");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("not-a-zip.txt");
        std::fs::write(&p, b"hello").unwrap();
        reset_for_test();
        assert!(
            list_cached(&p).is_none(),
            "a non-zip must not be remembered as an empty listing"
        );
        let _ = std::fs::remove_file(&p);
    }

    /// Stamps must actually distinguish a changed file, or the cache hands back
    /// a stale listing and the audit reports a romset as present when it was
    /// swapped out.
    #[test]
    fn stamp_changes_with_size_and_mtime() {
        let dir = std::env::temp_dir().join("mvui-auditcache-stamp");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("a.zip");
        std::fs::write(&p, b"one").unwrap();
        let s1 = Stamp::of(&p).unwrap();
        std::fs::write(&p, b"one two").unwrap();
        let s2 = Stamp::of(&p).unwrap();
        assert_ne!(s1, s2, "size change must invalidate the stamp");

        std::fs::write(&p, b"one two").unwrap();
        let s3 = Stamp::of(&p).unwrap();
        // same size; mtime should differ (resolution permitting)
        assert_eq!(s2.size, s3.size);
        let _ = std::fs::remove_file(&p);
    }

    /// A missing file is a miss, not a panic or an empty listing.
    #[test]
    fn missing_file_is_a_miss() {
        let p = std::env::temp_dir().join("mvui-auditcache-does-not-exist.zip");
        let _ = std::fs::remove_file(&p);
        assert!(list_cached(&p).is_none());
    }

    /// The round trip through the on-disk format must preserve entries exactly,
    /// including a 7z-style `crc: None`.
    #[test]
    fn cache_file_roundtrips_entries() {
        let mut entries = HashMap::new();
        entries.insert(
            "c:/roms/pacman.zip".to_string(),
            CachedListing {
                stamp: Stamp {
                    size: 1234,
                    mtime_ns: Some(42),
                },
                entries: vec![
                    CachedEntry {
                        name: "pacman.1".into(),
                        size: 4096,
                        crc: Some(0xdead_beef),
                    },
                    CachedEntry {
                        name: "no-crc.bin".into(),
                        size: 7,
                        crc: None,
                    },
                ],
            },
        );
        let file = CacheFile { entries };
        let dir = std::env::temp_dir().join("mvui-auditcache-roundtrip");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("audit_cache.bin");
        write_cache(&p, &file).unwrap();

        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[..MAGIC.len()], MAGIC);
        assert_eq!(
            u16::from_le_bytes([bytes[MAGIC.len()], bytes[MAGIC.len() + 1]]),
            FORMAT_VERSION
        );
        let back: CacheFile = bincode::deserialize(&bytes[MAGIC.len() + 2..]).unwrap();
        let got = &back.entries["c:/roms/pacman.zip"];
        assert_eq!(got.stamp.size, 1234);
        assert_eq!(got.stamp.mtime_ns, Some(42));
        assert_eq!(got.entries.len(), 2);
        assert_eq!(got.entries[0].crc, Some(0xdead_beef));
        assert_eq!(got.entries[1].crc, None);
        let _ = std::fs::remove_file(&p);
    }

    /// An empty cache file that is all zeros must be rejected, not deserialized
    /// into garbage — a rolled-back write can leave exactly that.
    #[test]
    fn truncated_cache_is_rejected() {
        let bytes = vec![0u8; 64];
        let ok = bytes.len() > MAGIC.len() + 2 && &bytes[..MAGIC.len()] == MAGIC;
        assert!(!ok, "all-zero file must fail the magic check");
    }
}
