//! Binary cache for the parsed game library (origin: MameDat::save/load gamelist.cache).
//!
//! Header: magic + format_version + mame_version. mame_version mismatch invalidates
//! the whole cache (driver metadata changes wholesale); format_version upgrades go
//! through a migration chain.

use crate::core::library::GameLibrary;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

pub const MAGIC: &[u8; 9] = b"MAMEGUIRS";
pub const FORMAT_VERSION: u16 = 2;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cache file missing")]
    Missing,
    #[error("cache corrupt: {0}")]
    Corrupt(String),
    #[error("cache io: {0}")]
    Io(#[from] std::io::Error),
    #[error("mame version changed (cache: {cache}, mame: {mame})")]
    VersionChanged { cache: String, mame: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheData {
    pub mame_version: String,
    pub library: GameLibrary,
    /// false = cached right after -listxml, before the audit ran
    #[serde(default)]
    pub audited: bool,
}

pub fn save(path: &Path, data: &CacheData) -> Result<(), CacheError> {
    save_library(path, &data.mame_version, &data.library, data.audited)
}

/// Write the cache straight from a borrowed library.
///
/// `save()` takes an owned `CacheData`, which forces every audit refresh to
/// deep-clone the whole library first; this serializes in place. The layout is
/// byte-identical to `save()` (same struct, same field order).
///
/// The payload is streamed to the temp file instead of being built in a `Vec`
/// first: with a ~50 MB library the old path held both the serialized buffer and
/// the library in memory (README P2-15).
pub fn save_library(
    path: &Path,
    mame_version: &str,
    library: &GameLibrary,
    audited: bool,
) -> Result<(), CacheError> {
    #[derive(Serialize)]
    struct Borrowed<'a> {
        mame_version: &'a str,
        library: &'a GameLibrary,
        audited: bool,
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(MAGIC)?;
        f.write_all(&FORMAT_VERSION.to_le_bytes())?;
        bincode::serialize_into(
            &mut f,
            &Borrowed {
                mame_version,
                library,
                audited,
            },
        )
        .map_err(|e| CacheError::Corrupt(e.to_string()))?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Returns Ok(data) on full hit; Err(VersionChanged) when a rebuild is required.
pub fn load(path: &Path, current_mame_version: &str) -> Result<CacheData, CacheError> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CacheError::Missing),
        Err(e) => return Err(e.into()),
    };
    if bytes.len() < MAGIC.len() + 2 || &bytes[..MAGIC.len()] != MAGIC {
        return Err(CacheError::Corrupt("bad magic".into()));
    }
    let ver = u16::from_le_bytes([bytes[MAGIC.len()], bytes[MAGIC.len() + 1]]);
    if ver > FORMAT_VERSION {
        return Err(CacheError::Corrupt(format!("cache version {ver} is newer")));
    }
    if ver < FORMAT_VERSION {
        // future migrations hook: v_old -> v_new
        return Err(CacheError::Corrupt(format!("cache version {ver} too old")));
    }
    let mut data: CacheData = bincode::deserialize(&bytes[MAGIC.len() + 2..])
        .map_err(|e| CacheError::Corrupt(e.to_string()))?;
    if data.mame_version != current_mame_version {
        return Err(CacheError::VersionChanged {
            cache: data.mame_version,
            mame: current_mame_version.to_string(),
        });
    }
    // index/ by_crc are #[serde(skip)] — without this every lookup
    // (get/get_idx) returns None on a warm start and half the UI goes dead
    data.library.rebuild_indexes();
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::GameMeta;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join("mamegui-test-cache");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("library.bin");
        let mut lib = GameLibrary::new("0.261".into());
        lib.push(GameMeta {
            name: "pacman".into(),
            description: "Pac-Man".into(),
            ..Default::default()
        });
        lib.rebuild_indexes();
        let data = CacheData {
            mame_version: "0.261".into(),
            library: lib,
            audited: true,
        };
        save(&p, &data).unwrap();
        // warm start must come back with usable indexes
        let back = load(&p, "0.261").unwrap();
        assert_eq!(back.library.get_idx("pacman"), Some(0));
        assert!(load(&p, "0.262").is_err());
        assert!(matches!(load(&p, "0.262"), Err(CacheError::VersionChanged { .. })));
        let _ = std::fs::remove_file(&p);
    }
}
