//! Game icon lookup (origin: `gamelist.cpp::loadIconWorkder`, which reads the
//! icon pack through `utils->iterateMameFile(icons_directory, "icons;.",
//! "*.ico", MAMEFILE_READ)`).
//!
//! README §6.1, layer ③: icons are *external data*, one `.ico` per machine, in
//! whatever `icons_directory` points at — packed as `icons.zip` / `icons.7z` or
//! loose on disk. The port reads one icon at a time instead of slurping the
//! whole pack into `GameInfo::icondata` as the original did: a modern pack is
//! tens of thousands of files, and only the rows currently on screen are drawn.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// split a ';'-separated directory option into paths
fn dirs_of(dir_paths: &str) -> Vec<PathBuf> {
    dir_paths
        .split(';')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// One icon: `<game>.ico` from the first icon directory that has it.
///
/// Per directory the order is the one the original used: the packed `icons.zip`
/// first (by far the common case, and a single indexed lookup), then `icons.7z`,
/// then loose files — both directly in the directory and in its `icons/`
/// subdirectory, the two `archNames` ("icons;.") it passes.
pub fn read_game_icon(dir_paths: &str, game: &str) -> Option<Vec<u8>> {
    if game.is_empty() {
        return None;
    }
    let file = format!("{game}.ico");
    for dir in dirs_of(dir_paths) {
        if let Some(b) = read_zip_entry(&dir.join("icons.zip"), &file) {
            return Some(b);
        }
        if let Some(b) = read_7z_entry(&dir, &file) {
            return Some(b);
        }
        for sub in ["", "icons"] {
            let p = if sub.is_empty() {
                dir.join(&file)
            } else {
                dir.join(sub).join(&file)
            };
            if let Ok(b) = std::fs::read(&p) {
                return Some(b);
            }
        }
    }
    None
}

/// Central-directory lookup inside `icons.zip`, with a case-insensitive
/// fallback: packs assembled elsewhere do not always lower-case the machine
/// name, and MAME's own set names are always lower case.
fn read_zip_entry(zip_path: &Path, entry: &str) -> Option<Vec<u8>> {
    if !zip_path.is_file() {
        return None;
    }
    let f = File::open(zip_path).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    if let Ok(mut e) = z.by_name(entry) {
        let claimed = e.size();
        return read_capped(&mut e, claimed);
    }
    let want = entry.to_lowercase();
    for i in 0..z.len() {
        let Ok(mut e) = z.by_index(i) else { continue };
        if e.name().to_lowercase() == want {
            let claimed = e.size();
            return read_capped(&mut e, claimed);
        }
    }
    None
}

/// A real `.ico` is at most a few hundred KB (MAME's own set stays under 128 KB).
/// The entry's uncompressed size comes from the archive header, i.e. from
/// whoever built the pack — a crafted `icons.zip` claiming 100 GB made
/// `Vec::with_capacity` abort the process before a single byte was read. Cap the
/// *pre-allocation* and read incrementally so a lying header costs nothing.
const MAX_ICON_BYTES: u64 = 16 * 1024 * 1024;

fn read_capped<R: std::io::Read>(r: &mut R, claimed: u64) -> Option<Vec<u8>> {
    let mut buf = Vec::with_capacity(claimed.min(MAX_ICON_BYTES) as usize);
    // `.take` bounds the real transfer too, in case the header under-declares
    let mut limited = r.take(MAX_ICON_BYTES + 1);
    limited.read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > MAX_ICON_BYTES {
        return None;
    }
    Some(buf)
}

/// The 7z pack has to be opened through `iterate_mame_file` (its entries share
/// one decode stream), which decodes only the entry asked for.
fn read_7z_entry(dir: &Path, entry: &str) -> Option<Vec<u8>> {
    let hits = crate::core::archive::iterate_mame_file(
        &dir.to_string_lossy(),
        "icons",
        entry,
        crate::core::archive::IterateMethod::Read,
        "",
        None,
    );
    hits.into_iter()
        .next()
        .map(|(_, m)| m.data)
        .filter(|d| !d.is_empty())
}

/// The games whose icon a game may inherit, in the order the original tries
/// them: a clone takes its parent's icon, and a softlist entry (an ext rom)
/// takes the icon of the machine it runs on (README §6.1③).
///
/// `lib` may be `None` when only the direct links matter; the clone chain is
/// then cut after the first step.
pub fn icon_candidates(g: &crate::core::model::GameMeta) -> Vec<String> {
    let mut out = Vec::new();
    if g.is_ext_rom && !g.romof.is_empty() {
        out.push(g.romof.clone());
    }
    if !g.cloneof.is_empty() {
        out.push(g.cloneof.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join("mamegui-icon-test").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn bytes_of(dir: &Path) -> Option<Vec<u8>> {
        read_game_icon(&dir.to_string_lossy(), "pacman")
    }

    #[test]
    fn finds_loose_icon_in_the_directory() {
        let d = scratch("loose");
        std::fs::write(d.join("pacman.ico"), b"ICO0").unwrap();
        assert_eq!(bytes_of(&d), Some(b"ICO0".to_vec()));
    }

    /// the original searches `icons;` as well as `.`
    #[test]
    fn finds_loose_icon_in_the_icons_subdir() {
        let d = scratch("subdir");
        std::fs::create_dir_all(d.join("icons")).unwrap();
        std::fs::write(d.join("icons").join("pacman.ico"), b"ICO1").unwrap();
        assert_eq!(bytes_of(&d), Some(b"ICO1".to_vec()));
    }

    #[test]
    fn finds_icon_inside_icons_zip() {
        let d = scratch("zip");
        let zip_path = d.join("icons.zip");
        let f = File::create(&zip_path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        // Stored: no compression feature needed, and the entry name is all that
        // this test cares about
        w.start_file("pacman.ico", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut w, b"ICOZ").unwrap();
        w.finish().unwrap();
        assert_eq!(bytes_of(&d), Some(b"ICOZ".to_vec()));
        // and a miss stays a miss
        assert_eq!(read_game_icon(&d.to_string_lossy(), "puckman"), None);
    }

    /// a pack with mixed-case names still resolves a lower-case set name
    #[test]
    fn zip_lookup_is_case_insensitive() {
        let d = scratch("zipcase");
        let f = File::create(d.join("icons.zip")).unwrap();
        let mut w = zip::ZipWriter::new(f);
        w.start_file("PacMan.ico", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut w, b"ICOC").unwrap();
        w.finish().unwrap();
        assert_eq!(bytes_of(&d), Some(b"ICOC".to_vec()));
    }

    #[test]
    fn first_directory_wins() {
        let a = scratch("multi-a");
        let b = scratch("multi-b");
        std::fs::write(a.join("pacman.ico"), b"FIRST").unwrap();
        std::fs::write(b.join("pacman.ico"), b"SECOND").unwrap();
        let dirs = format!("{};{}", a.display(), b.display());
        assert_eq!(read_game_icon(&dirs, "pacman"), Some(b"FIRST".to_vec()));
    }

    #[test]
    fn candidates_cover_parent_and_host_machine() {
        use crate::core::model::GameMeta;
        let clone = GameMeta {
            name: "pacman".into(),
            cloneof: "puckman".into(),
            ..Default::default()
        };
        assert_eq!(icon_candidates(&clone), vec!["puckman".to_string()]);

        let soft = GameMeta {
            name: "dir/cart.bin".into(),
            is_ext_rom: true,
            romof: "nes".into(),
            ..Default::default()
        };
        assert_eq!(icon_candidates(&soft), vec!["nes".to_string()]);
    }
}
