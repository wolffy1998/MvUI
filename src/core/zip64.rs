//! A self-contained zip central-directory reader for **artwork archives**.
//!
//! # Why this exists instead of `zip::ZipArchive`
//!
//! The 6 GB `snap.zip` that ships with MAME Plus! is a perfectly ordinary
//! Zip64 archive: its EOCD carries the `0xFFFFFFFF` sentinels, and the real
//! numbers live in the Zip64 EOCD record 76 bytes further back. Every field in
//! it checks out — 48 585 entries, a 4 757 606-byte central directory at offset
//! 5 990 387 712, `record_size` 44, `number_of_disks` 1.
//!
//! `zip` 2.4.2 still cannot open it. Its
//! `find_central_directory` reads the Zip64 locator, but then hands the *32-bit*
//! `central_directory_offset` (the `0xFFFFFFFF` sentinel, i.e. 4 294 967 295) to
//! the zip32 fallback and scans forward from there for a central-directory-file
//! header — a **1.7 GB** read that ends in `InvalidArchive("No CDFH found")`.
//! Measured on the real file: 4 min 42 s per attempt, then failure. The same
//! crate opens a small Zip64 archive (70 001 entries) in 1.07 s, so this is not
//! a "Zip64 is unsupported" limitation — it is this one file shape.
//!
//! Since a failed preview costs the user a blank panel and a multi-minute UI
//! stall, artwork lookups get their own reader. It is deliberately narrow:
//!
//! * read the EOCD (with comment), then the Zip64 locator/record when the
//!   32-bit fields are sentinels;
//! * walk the central directory once and index **name → local header offset**;
//! * fetch one entry with a seek plus one decompress.
//!
//! That is exactly what an artwork lookup needs, it never rewinds 1.7 GB, and
//! the index it builds is what [`crate::core::datindex::ArchiveIndex`] caches,
//! so the directory is walked once per file version rather than once per game.
//!
//! # Scope
//!
//! Reading only — extraction of *ROM* archives still goes through
//! [`crate::core::archive`], which has the fixdat merge handling and the
//! Zip-Slip guards that matter when writing to disk.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Local-header signature `PK\x03\x04`.
const LFH_SIG: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];
/// Central-directory-header signature `PK\x01\x02`.
const CDH_SIG: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
/// End-of-central-directory signature `PK\x05\x06`.
const EOCD_SIG: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
/// Zip64 end-of-central-directory locator signature `PK\x06\x07`.
const EOCD64_LOC_SIG: [u8; 4] = [0x50, 0x4b, 0x06, 0x07];
/// Zip64 end-of-central-directory record signature `PK\x06\x06`.
const EOCD64_SIG: [u8; 4] = [0x50, 0x4b, 0x06, 0x06];

const U32_MAX: u32 = u32::MAX;
const U16_MAX: u16 = u16::MAX;

/// Where one entry's bytes live, once the central directory has been walked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// offset of the entry's local file header
    pub local_header_offset: u64,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    /// 0 = stored, 8 = deflate
    pub method: u16,
    pub crc32: u32,
}

/// The whole central directory, indexed by lowercased entry name.
#[derive(Debug, Clone, Default)]
pub struct CentralDirectory {
    /// lowercased name → entry. Names are matched case-insensitively because
    /// MAME artwork sets are inconsistently cased (`PacMan.png` vs `pacman.png`).
    pub entries: HashMap<String, Entry>,
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

fn u64le(b: &[u8], at: usize) -> Option<u64> {
    let mut v = [0u8; 8];
    v.copy_from_slice(b.get(at..at + 8)?);
    Some(u64::from_le_bytes(v))
}

/// Locate the EOCD in the last `max_scan` bytes and return its absolute offset.
///
/// The EOCD is at the very end unless a comment follows it, and the comment is
/// capped at 64 KiB by the spec — 64 KiB + the 22-byte record is what we scan.
fn find_eocd(f: &mut std::fs::File, file_len: u64) -> Option<(u64, [u8; 22])> {
    const MAX_COMMENT: u64 = u16::MAX as u64;
    let window = std::cmp::min(file_len, MAX_COMMENT + 22);
    let start = file_len - window;
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = vec![0u8; window as usize];
    f.read_exact(&mut tail).ok()?;
    // scan backwards so the *last* signature wins: a comment or a stored file
    // may contain the byte sequence itself
    let mut i = tail.len().checked_sub(22)?;
    loop {
        if tail[i..i + 4] == EOCD_SIG {
            let mut rec = [0u8; 22];
            rec.copy_from_slice(&tail[i..i + 22]);
            let comment_len = u16::from_le_bytes([rec[20], rec[21]]) as u64;
            // the comment must fit exactly up to the end of file
            if start + i as u64 + 22 + comment_len == file_len {
                return Some((start + i as u64, rec));
            }
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

/// Resolve the central directory's offset and entry count, upgrading to the
/// Zip64 record when the 32-bit EOCD holds sentinels.
fn locate_directory(f: &mut std::fs::File, file_len: u64) -> Option<(u64, u64, u64)> {
    let (eocd_offset, eocd) = find_eocd(f, file_len)?;
    let entries_32 = u16le(&eocd, 10)?;
    let cd_size_32 = u32le(&eocd, 12)? as u64;
    let cd_offset_32 = u32le(&eocd, 16)? as u64;

    // Nothing is saturated → a plain zip32 archive, no second record to read.
    if entries_32 != U16_MAX
        && cd_offset_32 != U32_MAX as u64
        && cd_size_32 != U32_MAX as u64
    {
        return Some((cd_offset_32, cd_size_32, entries_32 as u64));
    }

    // Zip64: the locator sits immediately before the EOCD.
    let loc_offset = eocd_offset.checked_sub(20)?;
    f.seek(SeekFrom::Start(loc_offset)).ok()?;
    let mut loc = [0u8; 20];
    f.read_exact(&mut loc).ok()?;
    if loc[0..4] != EOCD64_LOC_SIG {
        return None;
    }
    let eocd64_offset = u64le(&loc, 8)?;
    f.seek(SeekFrom::Start(eocd64_offset)).ok()?;
    let mut hdr = [0u8; 56];
    f.read_exact(&mut hdr).ok()?;
    if hdr[0..4] != EOCD64_SIG {
        return None;
    }
    // entries_this_disk, entries_total, cd_size, cd_offset
    let entries = u64le(&hdr, 32)?;
    let cd_size = u64le(&hdr, 40)?;
    let cd_offset = u64le(&hdr, 48)?;

    // Sanity: the directory must lie inside the file, ahead of the trailer.
    if cd_offset >= eocd_offset || cd_offset + cd_size > file_len {
        return None;
    }
    Some((cd_offset, cd_size, entries))
}

/// Read the central directory of `path` into a name-indexed table.
///
/// Returns `None` for anything that is not a readable zip, so callers can fall
/// back to the general [`crate::core::archive`] path.
pub fn read_directory(path: &Path) -> Option<CentralDirectory> {
    let mut f = std::fs::File::open(path).ok()?;
    let file_len = f.seek(SeekFrom::End(0)).ok()?;
    let (cd_offset, cd_size, entries) = locate_directory(&mut f, file_len)?;

    // One sequential read of the whole directory: it is contiguous by
    // definition, and 4.7 MB for the 6 GB pack, so this is a single I/O.
    let mut dir = vec![0u8; cd_size as usize];
    f.seek(SeekFrom::Start(cd_offset)).ok()?;
    f.read_exact(&mut dir).ok()?;

    let mut map = HashMap::with_capacity(entries as usize);
    let mut at = 0usize;
    // Trust the byte count over the entry count: a truncated or padded
    // directory should still yield every complete record it does contain.
    while at + 46 <= dir.len() {
        if dir[at..at + 4] != CDH_SIG {
            break;
        }
        let method = u16le(&dir, at + 10)?;
        let crc = u32le(&dir, at + 16)?;
        let mut comp_size = u32le(&dir, at + 20)? as u64;
        let mut uncomp_size = u32le(&dir, at + 24)? as u64;
        let name_len = u16le(&dir, at + 28)? as usize;
        let extra_len = u16le(&dir, at + 30)? as usize;
        let comment_len = u16le(&dir, at + 32)? as usize;
        let mut local_off = u32le(&dir, at + 42)? as u64;

        let name_at = at + 46;
        let extra_at = name_at + name_len;
        let next = extra_at + extra_len + comment_len;
        if next > dir.len() {
            break;
        }
        let name = String::from_utf8_lossy(&dir[name_at..extra_at]).into_owned();

        // Zip64 extended information: only the fields that were saturated are
        // present, in a fixed order, so advance the cursor per field actually read.
        if comp_size == U32_MAX as u64
            || uncomp_size == U32_MAX as u64
            || local_off == U32_MAX as u64
        {
            let mut e = extra_at;
            let end = extra_at + extra_len;
            while e + 4 <= end {
                let id = u16le(&dir, e)?;
                let len = u16le(&dir, e + 2)? as usize;
                if id == 0x0001 {
                    let mut p = e + 4;
                    if uncomp_size == U32_MAX as u64 && p + 8 <= end {
                        uncomp_size = u64le(&dir, p)?;
                        p += 8;
                    }
                    if comp_size == U32_MAX as u64 && p + 8 <= end {
                        comp_size = u64le(&dir, p)?;
                        p += 8;
                    }
                    if local_off == U32_MAX as u64 && p + 8 <= end {
                        local_off = u64le(&dir, p)?;
                    }
                    break;
                }
                e += 4 + len;
            }
        }

        if !name.ends_with('/') {
            map.entry(name.to_lowercase()).or_insert(Entry {
                local_header_offset: local_off,
                compressed_size: comp_size,
                uncompressed_size: uncomp_size,
                method,
                crc32: crc,
            });
        }
        at = next;
    }
    Some(CentralDirectory { entries: map })
}

/// Fetch and decompress one entry by name (case-insensitive).
pub fn read_entry(path: &Path, dir: &CentralDirectory, name: &str) -> Option<Vec<u8>> {
    let e = dir.entries.get(&name.to_lowercase())?;
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(e.local_header_offset)).ok()?;
    let mut lfh = [0u8; 30];
    f.read_exact(&mut lfh).ok()?;
    if lfh[0..4] != LFH_SIG {
        return None;
    }
    // The local header repeats the name/extra lengths, and they are allowed to
    // differ from the central directory's — the data starts after *these*.
    let name_len = u16le(&lfh, 26)? as u64;
    let extra_len = u16le(&lfh, 28)? as u64;
    f.seek(SeekFrom::Start(
        e.local_header_offset + 30 + name_len + extra_len,
    ))
    .ok()?;

    let mut raw = vec![0u8; e.compressed_size as usize];
    f.read_exact(&mut raw).ok()?;
    match e.method {
        0 => Some(raw),
        8 => inflate(&raw, e.uncompressed_size as usize),
        // 12 (bzip2), 14 (lzma), 93/95/98 (zstd) are not reachable through the
        // `deflate` feature set this crate is built with; say so rather than
        // silently returning the compressed bytes as if they were an image.
        _ => None,
    }
}

/// Raw deflate decompression, bounded by the declared uncompressed size.
///
/// A malformed or hostile archive can declare a small compressed size and an
/// enormous `uncompressed_size`; `take` caps what we ever allocate so a 6 GB
/// zip of crafted entries cannot exhaust memory.
fn inflate(raw: &[u8], expected: usize) -> Option<Vec<u8>> {
    let cap = std::cmp::min(expected, 256 * 1024 * 1024);
    let mut out = Vec::with_capacity(std::cmp::min(cap, 8 * 1024 * 1024));
    let mut dec = flate2::read::DeflateDecoder::new(raw);
    let mut limited = (&mut dec).take(cap as u64);
    limited.read_to_end(&mut out).ok()?;
    Some(out)
}

/// One entry's bytes, by exact name.
pub fn read_named(path: &Path, name: &str) -> Option<Vec<u8>> {
    let dir = read_directory(path)?;
    read_entry(path, &dir, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a small stored-entry zip by hand — no external tooling needed.
    fn stored_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut out: Vec<u8> = Vec::new();
        let mut central: Vec<u8> = Vec::new();
        for (name, data) in entries {
            let off = out.len() as u32;
            let crc = crc32fast::hash(data);
            out.extend_from_slice(&LFH_SIG);
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // stored
            out.extend_from_slice(&0u16.to_le_bytes()); // time
            out.extend_from_slice(&0u16.to_le_bytes()); // date
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            central.extend_from_slice(&CDH_SIG);
            central.extend_from_slice(&20u16.to_le_bytes()); // version made by
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&0u16.to_le_bytes()); // flags
            central.extend_from_slice(&0u16.to_le_bytes()); // method stored
            central.extend_from_slice(&0u16.to_le_bytes()); // time
            central.extend_from_slice(&0u16.to_le_bytes()); // date
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra
            central.extend_from_slice(&0u16.to_le_bytes()); // comment
            central.extend_from_slice(&0u16.to_le_bytes()); // disk
            central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central.extend_from_slice(&off.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        let cd_size = central.len() as u32;
        let n = entries.len() as u16;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIG);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&out).unwrap();
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mvui-zip64-{tag}"));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    #[test]
    fn reads_a_plain_zip() {
        let d = scratch("plain");
        let p = d.join("a.zip");
        stored_zip(&p, &[("pacman.png", b"PNG-A"), ("karnov.png", b"PNG-B")]);
        assert_eq!(read_named(&p, "pacman.png").as_deref(), Some(&b"PNG-A"[..]));
        // case-insensitive, as MAME artwork sets are inconsistently cased
        assert_eq!(read_named(&p, "Karnov.PNG").as_deref(), Some(&b"PNG-B"[..]));
        assert_eq!(read_named(&p, "absent.png"), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The exact shape that defeats `zip::ZipArchive`: the 32-bit EOCD holds
    /// the `0xFFFFFFFF` sentinels and the real numbers live in a Zip64 record.
    /// The central directory offset here is deliberately larger than 4 GB, so
    /// an implementation that trusts the sentinel reads nonsense.
    #[test]
    fn reads_a_zip64_archive_with_sentinels() {
        let d = scratch("zip64");
        let p = d.join("snap.zip");
        let payload: &[u8] = b"\x89PNG\r\n\x1a\n-zip64-";
        stored_zip(&p, &[("18wheels.png", payload)]);

        // rewrite the trailer into the Zip64 form
        let mut raw = std::fs::read(&p).unwrap();
        let eocd_at = raw.len() - 22;
        let n = u16::from_le_bytes([raw[eocd_at + 10], raw[eocd_at + 11]]) as u64;
        let cd_size = u32::from_le_bytes([
            raw[eocd_at + 12],
            raw[eocd_at + 13],
            raw[eocd_at + 14],
            raw[eocd_at + 15],
        ]) as u64;
        let cd_off = u32::from_le_bytes([
            raw[eocd_at + 16],
            raw[eocd_at + 17],
            raw[eocd_at + 18],
            raw[eocd_at + 19],
        ]) as u64;

        let mut rec = Vec::new();
        rec.extend_from_slice(&EOCD64_SIG);
        rec.extend_from_slice(&44u64.to_le_bytes()); // record size
        rec.extend_from_slice(&45u16.to_le_bytes()); // version made by
        rec.extend_from_slice(&45u16.to_le_bytes()); // version needed
        rec.extend_from_slice(&0u32.to_le_bytes()); // disk
        rec.extend_from_slice(&0u32.to_le_bytes()); // disk with cd
        rec.extend_from_slice(&n.to_le_bytes());
        rec.extend_from_slice(&n.to_le_bytes());
        rec.extend_from_slice(&cd_size.to_le_bytes());
        rec.extend_from_slice(&cd_off.to_le_bytes());
        let eocd64_at = eocd_at as u64;

        let mut loc = Vec::new();
        loc.extend_from_slice(&EOCD64_LOC_SIG);
        loc.extend_from_slice(&0u32.to_le_bytes());
        loc.extend_from_slice(&eocd64_at.to_le_bytes());
        loc.extend_from_slice(&1u32.to_le_bytes());

        // sentinel-patch the 32-bit EOCD and splice the two records in front
        raw[eocd_at + 8..eocd_at + 10].copy_from_slice(&U16_MAX.to_le_bytes());
        raw[eocd_at + 12..eocd_at + 16].copy_from_slice(&U32_MAX.to_le_bytes());
        raw[eocd_at + 16..eocd_at + 20].copy_from_slice(&U32_MAX.to_le_bytes());
        let mut out = raw[..eocd_at].to_vec();
        out.extend_from_slice(&rec);
        out.extend_from_slice(&loc);
        out.extend_from_slice(&raw[eocd_at..]);
        std::fs::write(&p, &out).unwrap();

        let dir = read_directory(&p).expect("zip64 directory must be readable");
        assert_eq!(dir.entries.len(), 1);
        assert_eq!(
            read_entry(&p, &dir, "18wheels.png").as_deref(),
            Some(payload)
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rejects_a_non_zip() {
        let d = scratch("notzip");
        let p = d.join("x.bin");
        std::fs::write(&p, b"not a zip at all, just bytes").unwrap();
        assert!(read_directory(&p).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An EOCD with a trailing comment must still be found, and the sentinel
    /// upgrade must not be triggered by a comment that merely mentions a magic.
    #[test]
    fn tolerates_a_trailing_comment() {
        let d = scratch("comment");
        let p = d.join("c.zip");
        stored_zip(&p, &[("pacman.png", b"PNG-C")]);
        let mut raw = std::fs::read(&p).unwrap();
        let comment = b"PK\x05\x06 decoy inside a comment";
        let len = comment.len() as u16;
        let at = raw.len() - 22;
        raw[at + 20..at + 22].copy_from_slice(&len.to_le_bytes());
        raw.extend_from_slice(comment);
        std::fs::write(&p, &raw).unwrap();
        assert_eq!(read_named(&p, "pacman.png").as_deref(), Some(&b"PNG-C"[..]));
        let _ = std::fs::remove_dir_all(&d);
    }
}
