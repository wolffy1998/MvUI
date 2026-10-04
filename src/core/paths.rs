//! Where MvUI looks for its content: artwork, DAT files, the localized game
//! list, background images and the external folder lists.
//!
//! # Why this exists
//!
//! 1.8.2 resolved all of these against the **mame.exe directory** (the process
//! cwd), because that is where a MAME install keeps `snap/`, `flyers/` and the
//! `.dat` files. That works for a single-purpose install but couples artwork to
//! the emulator: a portable MvUI next to a read-only MAME tree could not have
//! any pictures at all, and every user had to hand-copy `history.dat` into the
//! MAME folder.
//!
//! The rule here is: **content lives next to `mvui.exe`**; the MAME directory
//! is only for ROMs and MAME's own options. Each location can still be pointed
//! somewhere else from Settings ▸ Directories, and an explicit setting always
//! wins over the default.
//!
//! Resolution order for every path:
//!
//! 1. the configured value, absolute → used as-is
//! 2. the configured value, relative → against the exe directory
//! 3. empty/absent → `<exe dir>/<built-in default>`

use std::path::PathBuf;

use crate::core::settings::GuiSettings;

/// Directory holding `mvui.exe` — the anchor for every default below.
pub fn exe_dir() -> PathBuf {
    GuiSettings::exe_dir()
}

/// Resolve one configured value against the exe directory, falling back to
/// `<exe>/default_rel` when nothing usable is configured.
///
/// A configured value may be a `;`-list; the first entry that is non-empty wins
/// (the callers that genuinely support lists resolve them themselves).
pub fn resolve(configured: Option<&str>, default_rel: &str) -> PathBuf {
    let configured = configured.map(str::trim).filter(|s| !s.is_empty());
    match configured {
        Some(v) => {
            let p = PathBuf::from(v);
            if p.is_absolute() {
                p
            } else {
                exe_dir().join(p)
            }
        }
        None => exe_dir().join(default_rel),
    }
}

/// Same as [`resolve`] but for a `;`-separated list, every entry normalised.
pub fn resolve_list(configured: Option<&str>, default_rel: &str) -> Vec<PathBuf> {
    let configured = configured.map(str::trim).filter(|s| !s.is_empty());
    match configured {
        Some(v) => v
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                let p = PathBuf::from(s);
                if p.is_absolute() {
                    p
                } else {
                    exe_dir().join(p)
                }
            })
            .collect(),
        None => vec![exe_dir().join(default_rel)],
    }
}

// ---------------------------------------------------------------------------
// the table
// ---------------------------------------------------------------------------

/// `(option key, default directory under the exe dir)` for the 7 image docks.
///
/// Note `snapshot_directory` is the odd one out: 1.8.2's template gives it no
/// default, so it inherited MAME's own `$HOME/snap` convention. Here it gets a
/// real default like the rest.
pub const IMAGE_DIRS: [(&str, &str); 7] = [
    ("snapshot_directory", "snap"),
    ("flyer_directory", "flyers"),
    ("cabinet_directory", "cabinets"),
    ("marquee_directory", "marquees"),
    ("title_directory", "titles"),
    ("cpanel_directory", "cpanel"),
    ("pcb_directory", "pcb"),
];

/// Subdirectory of the exe dir holding every `.dat`.
pub const DAT_SUBDIR: &str = "dats";

/// `(option key, file name inside [`DAT_SUBDIR`])` for the 5 document docks.
///
/// `mameinfo_file` backs two docks (MAME信息 and 驱动信息) — same file, different
/// lookup key — so it appears once.
pub const DAT_FILES: [(&str, &str); 4] = [
    ("history_file", "history.dat"),
    ("mameinfo_file", "mameinfo.dat"),
    ("story_file", "story.dat"),
    ("command_file", "command.dat"),
];

/// Default `<exe>/folders` — the external folder lists, incl. `Favorites.ini`.
pub const FOLDERS_SUBDIR: &str = "folders";

/// Default `<exe>/bkground` — the window wallpaper directory (1.8.2's spelling,
/// kept so existing installs keep working).
pub const BG_SUBDIR: &str = "bkground";

/// The localized game list, read from the exe directory. UTF-8, tab separated.
pub const LST_FILE: &str = "mame_cn.lst";

/// Absolute path of one image dock's directory.
pub fn image_dir(configured: Option<&str>, dock: usize) -> PathBuf {
    let (key, default_rel) = IMAGE_DIRS
        .get(dock)
        .copied()
        .unwrap_or(("pcb_directory", "pcb"));
    resolve(Some(configured.unwrap_or(key)), default_rel)
}

/// Absolute path of one document dock's `.dat`.
///
/// The configured value names a **file**, not a directory — that is 1.8.2's
/// `datfile` option type and users may well point it at a single file. When it
/// is unset we look in `<exe>/dats/<name>`.
pub fn dat_file(configured: Option<&str>, dock: usize) -> PathBuf {
    let key = crate::core::dat::dock_file_option(dock);
    let fallback = DAT_FILES
        .iter()
        .find(|(k, _)| Some(*k) == key)
        .map(|(_, f)| *f)
        .unwrap_or("history.dat");
    match configured.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => resolve(Some(v), fallback),
        None => exe_dir().join(DAT_SUBDIR).join(fallback),
    }
}

/// `<exe>/folders`, created if missing — the folder list is unusable without it
/// and 1.8.2 wrote `Favorites.ini` into the first configured directory.
pub fn folders_dir(configured: Option<&str>) -> PathBuf {
    let d = resolve(configured, FOLDERS_SUBDIR);
    let _ = std::fs::create_dir_all(&d);
    d
}

/// `<exe>/bkground`.
pub fn background_dir(configured: Option<&str>) -> PathBuf {
    resolve(configured, BG_SUBDIR)
}

/// `<exe>/mame_cn.lst`.
pub fn localized_list(configured: Option<&str>) -> PathBuf {
    match configured.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => resolve(Some(v), LST_FILE),
        None => exe_dir().join(LST_FILE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every default must land inside the exe directory — that is the whole
    /// point of the change, and it is what a portable install depends on.
    #[test]
    fn defaults_live_under_the_exe_dir() {
        let exe = exe_dir();
        for (_, rel) in IMAGE_DIRS {
            assert!(
                image_dir(None, 0).starts_with(&exe),
                "image dir {rel} escaped the exe dir"
            );
        }
        for dock in 0..5 {
            let p = dat_file(None, dock);
            assert!(p.starts_with(exe.join(DAT_SUBDIR)), "dat escaped: {p:?}");
        }
        assert!(localized_list(None).starts_with(&exe));
        assert!(background_dir(None).starts_with(&exe));
    }

    /// A relative setting is still relative to the exe dir, not the mame one.
    #[test]
    fn relative_settings_resolve_against_the_exe_dir() {
        let p = image_dir(Some("artwork/snap"), 0);
        assert_eq!(p, exe_dir().join("artwork").join("snap"));
    }

    /// An absolute setting is honoured verbatim — that is the escape hatch for
    /// anyone who wants the old mame-directory layout back.
    #[test]
    fn absolute_settings_win() {
        let p = image_dir(Some("D:/art"), 1);
        assert_eq!(p, PathBuf::from("D:/art"));
    }

    /// A `;` list keeps every entry, in order.
    #[test]
    fn lists_resolve_every_entry() {
        let v = resolve_list(Some("C:/a;D:/b"), "snap");
        assert_eq!(v, vec![PathBuf::from("C:/a"), PathBuf::from("D:/b")]);
    }

    /// An explicitly configured dat file stays a file, not a directory join.
    #[test]
    fn configured_dat_file_is_used_as_is() {
        let p = dat_file(Some("D:/info/my.dat"), 7);
        assert_eq!(p, PathBuf::from("D:/info/my.dat"));
    }
}
