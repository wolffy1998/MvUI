//! Unified zip/7z access (origin: utils.cpp iterateMameFile, backed by QuaZip + LZMA).
//!
//! Four original modes map onto these functions:
//! - GETINFO  -> list_archive
//! - GETDATINFO -> list_archive + fixdat crosscheck (caller side)
//! - READ     -> read_entry
//! - EXTRACT  -> extract_entry

use crate::core::options::clean_dir_path;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct EntryInfo {
    pub name: String,
    pub size: u64,
    /// None when the format does not expose crc without decompression (7z)
    pub crc: Option<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("unsupported archive: {0}")]
    Unsupported(String),
    #[error("archive io: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive error: {0}")]
    Other(String),
}

pub fn is_zip(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("zip")).unwrap_or(false)
}

pub fn is_7z(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("7z")).unwrap_or(false)
}

pub fn is_archive(p: &Path) -> bool {
    is_zip(p) || is_7z(p)
}

/// GETINFO: list entries of a zip/7z container.
pub fn list_archive(path: &Path) -> Result<Vec<EntryInfo>, ArchiveError> {
    if is_zip(path) {
        let f = File::open(path)?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| ArchiveError::Other(e.to_string()))?;
        let mut out = Vec::with_capacity(z.len());
        for i in 0..z.len() {
            let f = z.by_index(i).map_err(|e| ArchiveError::Other(e.to_string()))?;
            if f.is_dir() {
                continue;
            }
            out.push(EntryInfo {
                name: f.name().to_string(),
                size: f.size(),
                crc: Some(f.crc32()),
            });
        }
        Ok(out)
    } else if is_7z(path) {
        let mut z = sevenz_rust::SevenZReader::open(path, sevenz_rust::Password::empty())
            .map_err(|e| ArchiveError::Other(e.to_string()))?;
        let mut out = Vec::new();
        // 7z stores a per-entry CRC in the archive header — same source the
        // original used (f->FileCRC), no decompression needed
        z.for_each_entries(|entry, reader| {
            // the zip branch above skips directories; without this a 7z folder
            // entry became a "rom" of size 0 with no CRC (README P3). Nothing is
            // read from the archive here, so the shared decode stream stays put.
            if entry.name().ends_with('/') {
                let _ = std::io::copy(reader, &mut std::io::sink());
                return Ok(true);
            }
            let crc = if entry.has_crc {
                Some(entry.crc as u32)
            } else {
                None
            };
            out.push(EntryInfo {
                name: entry.name().to_string(),
                size: entry.size(),
                crc,
            });
            Ok(true)
        })
        .map_err(|e| ArchiveError::Other(e.to_string()))?;
        Ok(out)
    } else {
        Err(ArchiveError::Unsupported(path.display().to_string()))
    }
}

pub fn file_stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string())
}

// ---------------------------------------------------------------------------
// iterateMameFile (origin: utils.cpp) — the unified archive/file scanner
// ---------------------------------------------------------------------------

/// MAMEFILE_GETINFO=0 GETDATINFO=1 READ=2 EXTRACT=3
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IterateMethod {
    GetInfo,
    GetDatInfo,
    Read,
    Extract,
}

#[derive(Debug, Clone, Default)]
pub struct MameFileInfo {
    pub path: String,
    pub crc: u32,
    pub size: u64,
    pub data: Vec<u8>,
    pub removable: bool,
}

/// Results **in scan order**: the original picked the first hit it met, and a
/// `HashMap` cannot express that (README P3 — a preview used to pick a random
/// one of several same-named images).
pub type MameFileMap = Vec<(String, MameFileInfo)>;

/// CRC32 of a file, read in 64 KiB chunks.
///
/// Used by the loose-file scan so a multi-hundred-MB image is hashed without
/// ever being held in memory (README P2-11).
fn crc32_of_file(path: &Path) -> std::io::Result<u32> {
    let mut f = File::open(path)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize())
}

/// matchMameFile: "*" | "*.ext" | "?<crc-decimal>" | exact/basename ci compare
fn match_mame_file(file_name: &str, filters: &[String], crc: u32) -> bool {
    for f in filters {
        if f == "*" {
            return true;
        }
        if let Some(ext) = f.strip_prefix("*.") {
            if file_name.to_lowercase().ends_with(&format!(".{ext}").to_lowercase()) {
                return true;
            }
            continue;
        }
        if let Some(crcs) = f.strip_prefix('?') {
            if crc != 0 && crcs.parse::<u32>().map(|v| v == crc).unwrap_or(false) {
                return true;
            }
            continue;
        }
        if file_name.eq_ignore_ascii_case(f)
            || Path::new(file_name)
                .file_name()
                .map(|b| b.eq_ignore_ascii_case(f))
                .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

/// utils->iterateMameFile: unified scan over loose files and zip/7z archives.
/// `fixdat_lookup` mirrors the (quirky) pFixDat use for merge-aware extraction.
pub fn iterate_mame_file(
    dir_paths: &str,
    arch_names: &str,
    file_name_filters: &str,
    method: IterateMethod,
    extract_path: &str,
    fixdat_lookup: Option<&dyn Fn(&str) -> Option<Vec<String>>>,
) -> MameFileMap {
    let split = |s: &str| -> Vec<String> {
        s.split(';')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect()
    };
    let dir_path_list = split(dir_paths);
    let arch_name_list = split(arch_names);
    let filters = split(file_name_filters);
    // `is_single_file` used to be computed here and dropped: the loose-file branch
    // treats every archName as a subdirectory either way, and the wildcard/crc
    // forms are handled per filter inside `match_mame_file`.

    let extract_dir = if extract_path.is_empty() {
        temp_rom_dir()
    } else {
        clean_dir_path(extract_path)
    };

    let mut result: MameFileMap = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for dp in &dir_path_list {
        let base_dir = clean_dir_path(dp);

        // 1) loose files: treat each archName as a subdirectory name
        for arch in &arch_name_list {
            let dir = if arch.is_empty() {
                base_dir.clone()
            } else {
                base_dir.join(arch)
            };
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    continue;
                }
                let file_name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if seen.contains(&file_name) {
                    continue;
                }
                let is_chd = file_name.to_lowercase().ends_with(".chd");
                if !match_mame_file(&file_name, &filters, 0) {
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                // hash by streaming and only keep the bytes when the caller
                // actually asked for them: a loose set can hold CD images of
                // hundreds of MB, and GETINFO used to retain every one of them
                // in `result` (README P2-11)
                let crc = if method <= IterateMethod::GetDatInfo && !is_chd {
                    crc32_of_file(&p).unwrap_or(0)
                } else {
                    0
                };
                let data = if method == IterateMethod::Read || is_chd {
                    std::fs::read(&p).unwrap_or_default()
                } else {
                    Vec::new()
                };
                let mfi = MameFileInfo {
                    path: p.to_string_lossy().to_string(),
                    crc,
                    size: meta.len(),
                    data,
                    removable: true,
                };
                seen.insert(file_name.clone());
                result.push((file_name, mfi));
            }
        }

        // 2) archives; archName defaulting per dirPath
        let mut arch_names_here: Vec<String> = arch_name_list.clone();
        if arch_names_here.is_empty() {
            if filters.first().map(|f| f.starts_with("*.")) == Some(true) {
                if let Some(dn) = base_dir.file_name() {
                    arch_names_here.push(dn.to_string_lossy().to_string());
                }
            } else if let Some(f) = filters.first() {
                if let Some(bn) = base_dir.join(f).file_stem() {
                    arch_names_here.push(bn.to_string_lossy().to_string());
                }
            }
        }
        for arch in &arch_names_here {
            let fixdat_roms: Option<Vec<String>> =
                fixdat_lookup.as_ref().and_then(|f| f(arch));

            let zip_path = base_dir.join(format!("{arch}.zip"));
            if zip_path.is_file() {
                let Ok(f) = File::open(&zip_path) else { continue };
                let Ok(mut z) = zip::ZipArchive::new(f) else { continue };
                let meta_list: Vec<(String, u32, u64)> = (0..z.len())
                    .filter_map(|i| {
                        let e = z.by_index_raw(i).ok()?;
                        if e.name().ends_with('/') {
                            return None;
                        }
                        Some((e.name().to_string(), e.crc32(), e.size()))
                    })
                    .collect();
                for (name, crc, size) in meta_list {
                    if seen.contains(&name) {
                        continue;
                    }
                    if !match_mame_file(&name, &filters, crc) {
                        continue;
                    }
                    let mut mfi = MameFileInfo {
                        path: format!("{}::{}", zip_path.display(), name),
                        crc,
                        size,
                        ..Default::default()
                    };
                    if method > IterateMethod::GetDatInfo {
                        if let Ok(mut zf) = z.by_name(&name) {
                            let mut buf = Vec::new();
                            if std::io::Read::read_to_end(&mut zf, &mut buf).is_ok() {
                                mfi.data = buf;
                                if method != IterateMethod::Read {
                                    // `extract_mame_file` writes `mfi.data`; the
                                    // old code passed it an empty buffer and
                                    // produced zero-byte temp roms
                                    extract_mame_file(
                                        &name,
                                        &mfi,
                                        &extract_dir,
                                        fixdat_roms.as_deref(),
                                    );
                                    mfi.data = Vec::new();
                                }
                            }
                        }
                    }
                    seen.insert(name.clone());
                    result.push((name, mfi));
                }
                continue;
            }

            let sz_path = base_dir.join(format!("{arch}.7z"));
            if sz_path.is_file() {
                let Ok(mut z) =
                    sevenz_rust::SevenZReader::open(&sz_path, sevenz_rust::Password::empty())
                else {
                    continue;
                };
                // Single pass: the old version collected the entry list, then
                // re-opened and re-scanned the whole archive inside the loop for
                // every matching entry (O(n²)), and matched with crc 0 which made
                // the `?<crc>` filters unable to hit anything. `for_each_entries`
                // hands us both the entry (with its header CRC) and a reader for
                // the decompressed content (README P2-12).
                let _ = z.for_each_entries(|e, reader| {
                    // sevenz-rust decodes a folder into ONE sequential stream and
                    // hands each entry a reader bounded to its own share of it. An
                    // entry we skip without draining leaves that stream parked in
                    // the middle of the skipped entry, and every later entry of the
                    // folder then decodes from the wrong offset — silent data
                    // corruption (README N1). Draining is only needed when we read
                    // content at all: a name/CRC-only listing never touches the
                    // stream, so it can skip the (expensive) decode.
                    let needs_data = method > IterateMethod::GetDatInfo;
                    let drain = |reader: &mut dyn std::io::Read| {
                        if needs_data {
                            let _ = std::io::copy(reader, &mut std::io::sink());
                        }
                    };
                    let name = e.name().to_string();
                    if name.ends_with('/') {
                        drain(reader);
                        return Ok(true);
                    }
                    if seen.contains(&name) {
                        drain(reader);
                        return Ok(true);
                    }
                    let crc = if e.has_crc { e.crc as u32 } else { 0 };
                    if !match_mame_file(&name, &filters, crc) {
                        drain(reader);
                        return Ok(true);
                    }
                    let mut mfi = MameFileInfo {
                        path: format!("{}::{}", sz_path.display(), name),
                        crc,
                        size: e.size(),
                        ..Default::default()
                    };
                    if needs_data {
                        let mut buf = Vec::new();
                        if std::io::Read::read_to_end(reader, &mut buf).is_err() {
                            // a short read leaves the shared stream mid-entry too
                            drain(reader);
                            return Ok(true);
                        }
                        // some archives store no CRC — hash the one we just read
                        if crc == 0 {
                            mfi.crc = crc32fast::hash(&buf);
                        }
                        mfi.data = buf;
                        if method != IterateMethod::Read {
                            extract_mame_file(&name, &mfi, &extract_dir, fixdat_roms.as_deref());
                            mfi.data = Vec::new();
                        }
                    }
                    seen.insert(name.clone());
                    result.push((name, mfi));
                    Ok(true)
                });
            }
        }
    }
    result
}

/// origin: extractMameFile — merge-aware path reduction from fixdat rom names
/// Directory holding roms unpacked out of a `.7z` for a one-off launch.
///
/// It used to be `%TEMP%` itself: two games with a same-named rom overwrote each
/// other there, and nothing ever cleaned up what a crash left behind (README N7).
pub fn temp_rom_dir() -> PathBuf {
    let d = std::env::temp_dir().join("mamepgui_tmp");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Empty [`temp_rom_dir`]. Called once at startup: the per-game cleanup only knows
/// about games this session launched, so a crash would pile junk up forever.
pub fn clear_temp_rom_dir() {
    let d = temp_rom_dir();
    let Ok(entries) = std::fs::read_dir(&d) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn extract_mame_file(zip_file_name: &str, mfi: &MameFileInfo, out_path: &Path, fixdat_roms: Option<&[String]>) {
    let mut rom_file_path = Path::new(zip_file_name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| zip_file_name.to_string());
    if let Some(roms) = fixdat_roms {
        for name in roms {
            if !name.contains('/') {
                continue;
            }
            let mut bufs: Vec<&str> = name.split('/').collect();
            while bufs.len() > 2 {
                bufs.remove(0);
            }
            let trimmed = bufs.join("/");
            if zip_file_name.ends_with(&trimmed) {
                rom_file_path = name.clone();
                break;
            }
        }
    }
    // Zip Slip guard. Both inputs are attacker-controlled — `zip_file_name` is an
    // archive entry name and the fixdat rom names come from a downloaded .dat —
    // and the fixdat branch above deliberately keeps the `dir/../` prefix the
    // original used for merge roms. Joining that verbatim let a crafted archive
    // (`a/../../evil.dll`) write outside `out_path`, e.g. straight into the
    // user's startup folder. Keep the merge-path semantics but refuse anything
    // that does not land inside `out_path`.
    let dest = match sanitized_join(out_path, &rom_file_path) {
        Some(d) => d,
        None => return,
    };
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&dest, &mfi.data);
}

/// Join `relative` onto `base`, returning `None` when the result would escape
/// `base` (Zip Slip). Rejects absolute paths, drive prefixes and UNC paths too —
/// `Path::join` silently replaces the base on those.
fn sanitized_join(base: &Path, relative: &str) -> Option<PathBuf> {
    // archive names always use `/`; on Windows a `\` would also be a separator
    let rel = relative.replace('\\', "/");
    let rel_path = Path::new(&rel);
    if rel_path.is_absolute() || rel_path.components().any(|c| c.as_os_str() == "..") {
        return None;
    }
    // `C:foo` / `C:/foo` are drive-relative on Windows and escape the base
    let s = rel.trim_start_matches('/');
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        return None;
    }
    // also reject a `..` that only appears after lexical normalisation
    let joined = base.join(rel_path);
    let norm = normalize_lexically(&joined);
    let base_norm = normalize_lexically(base);
    if !norm.starts_with(&base_norm) {
        return None;
    }
    Some(joined)
}

/// Purely lexical `.`/`..` resolution — `std::path` has no public equivalent and
/// `canonicalize` would fail for paths that do not exist yet (the destination
/// directory is created afterwards).
fn normalize_lexically(p: &Path) -> PathBuf {
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    let mut prefix = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                prefix.push(comp.as_os_str());
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Normal(s) => out.push(s.to_os_string()),
        }
    }
    let mut r = prefix;
    for s in out {
        r.push(s);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        let d = std::env::temp_dir().join("mamepgui-zipslip-test");
        let _ = std::fs::create_dir_all(&d);
        d
    }

    /// A crafted archive/dat must not be able to write outside the target dir.
    #[test]
    fn sanitized_join_rejects_traversal() {
        let b = base();
        for evil in [
            "../evil.dll",
            "a/../../evil.dll",
            "..\\evil.dll",           // backslash separator
            "/etc/evil.dll",            // absolute (posix)
            "C:////evil.dll",            // drive-absolute
            "C:evil.dll",               // drive-relative
        ] {
            assert!(
                sanitized_join(&b, evil).is_none(),
                "should have rejected {evil:?}"
            );
        }
    }

    /// The fixdat merge path (which keeps a `dir/` prefix) must still work.
    #[test]
    fn sanitized_join_allows_nested() {
        let b = base();
        for ok in ["rom.dll", "sub/rom.dll", "a/b/c/rom.dll", "./rom.dll"] {
            let j = sanitized_join(&b, ok).unwrap_or_else(|| panic!("rejected {ok:?}"));
            assert!(normalize_lexically(&j).starts_with(normalize_lexically(&b)));
        }
    }

    /// A `..` that only appears after normalisation must still be caught.
    #[test]
    fn sanitized_join_catches_lexical_escape() {
        let b = base();
        assert!(sanitized_join(&b, "sub/../../out.dll").is_none());
    }

    /// The real entry point: an escaping rom name writes nothing at all.
    #[test]
    fn extract_refuses_to_escape() {
        let out = base().join("out");
        let _ = std::fs::create_dir_all(&out);
        let mfi = MameFileInfo {
            data: b"pwn".to_vec(),
            ..Default::default()
        };
        // the merge-name branch keeps the prefix, the loose branch takes file_name
        extract_mame_file("a/../../evil.dll", &mfi, &out, Some(&["a/../../evil.dll".into()]));
        assert!(
            !base().join("evil.dll").exists(),
            "wrote outside the output dir"
        );
    }

    #[test]
    fn extract_writes_normal_rom() {
        let out = base().join("out2");
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::create_dir_all(&out);
        let mfi = MameFileInfo {
            data: b"ok".to_vec(),
            ..Default::default()
        };
        extract_mame_file("pacman.rom", &mfi, &out, None);
        assert_eq!(std::fs::read(out.join("pacman.rom")).unwrap(), b"ok");
    }
}
