//! Category folder engine — 1:1 port of gamelist.cpp folder logic
//! (FOLDER_* enum, intFolderNames0 strings, children lists, custom folders,
//! console/bios maps, hidden folders).

use crate::core::library::GameLibrary;
use crate::core::model::{GAME_COMPLETE, STATUS_GOOD, STATUS_PRELIMINARY};
use crate::core::model::GameMeta;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// FOLDER_* indices (order is load-bearing; FOLDER_EXT = 26)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FolderKind {
    AllGame,
    AllArc,
    Available,
    Unavailable,
    Console,
    Manufacturer,
    Year,
    Source,
    Bios,
    Cpu,
    Snd,
    HardDisk,
    Samples,
    Dumping,
    Working,
    NonWorking,
    Originals,
    Clones,
    Resolution,
    PaletteSize,
    Refresh,
    Display,
    Controls,
    Channels,
    SaveState,
    Mechanical,
    NonMechanical,
    Ext(String),
}

/// original intFolderNames0 strings (MESS variants depend on isMESS)
pub fn root_folder_names(is_mess: bool) -> Vec<(FolderKind, &'static str)> {
    if is_mess {
        vec![
            (FolderKind::AllGame, "All Games"),
            (FolderKind::AllArc, "All Systems"),
            (FolderKind::Available, "Available Systems"),
            (FolderKind::Unavailable, "Unavailable Systems"),
            (FolderKind::Console, "Softwares"),
            (FolderKind::Manufacturer, "Manufacturer"),
            (FolderKind::Year, "Year"),
            (FolderKind::Source, "Driver"),
            (FolderKind::Bios, "BIOS"),
            (FolderKind::Cpu, "CPU"),
            (FolderKind::Snd, "Sound"),
            (FolderKind::HardDisk, "CHD"),
            (FolderKind::Samples, "Samples"),
            (FolderKind::Dumping, "Dumping Status"),
            (FolderKind::Working, "Working"),
            (FolderKind::NonWorking, "Not working"),
            (FolderKind::Originals, "Originals"),
            (FolderKind::Clones, "Clones"),
            (FolderKind::Resolution, "Resolution"),
            (FolderKind::PaletteSize, "Colors"),
            (FolderKind::Refresh, "Refresh Rate"),
            (FolderKind::Display, "Display"),
            (FolderKind::Controls, "Control Type"),
            (FolderKind::Channels, "Channels"),
            (FolderKind::SaveState, "Save State"),
            (FolderKind::Mechanical, "Mechanical"),
            (FolderKind::NonMechanical, "Non Mechanical"),
        ]
    } else {
        vec![
            (FolderKind::AllGame, "All Games"),
            (FolderKind::AllArc, "All Arcades"),
            (FolderKind::Available, "Available Arcades"),
            (FolderKind::Unavailable, "Unavailable Arcades"),
            (FolderKind::Console, "Consoles"),
            (FolderKind::Manufacturer, "Manufacturer"),
            (FolderKind::Year, "Year"),
            (FolderKind::Source, "Driver"),
            (FolderKind::Bios, "BIOS"),
            (FolderKind::Cpu, "CPU"),
            (FolderKind::Snd, "Sound"),
            (FolderKind::HardDisk, "CHD"),
            (FolderKind::Samples, "Samples"),
            (FolderKind::Dumping, "Dumping Status"),
            (FolderKind::Working, "Working"),
            (FolderKind::NonWorking, "Not working"),
            (FolderKind::Originals, "Originals"),
            (FolderKind::Clones, "Clones"),
            (FolderKind::Resolution, "Resolution"),
            (FolderKind::PaletteSize, "Colors"),
            (FolderKind::Refresh, "Refresh Rate"),
            (FolderKind::Display, "Display"),
            (FolderKind::Controls, "Control Type"),
            (FolderKind::Channels, "Channels"),
            (FolderKind::SaveState, "Save State"),
            (FolderKind::Mechanical, "Mechanical"),
            (FolderKind::NonMechanical, "Non Mechanical"),
        ]
    }
}

/// utils->getLongName descMap (input/display tokens)
pub fn long_name(s: &str) -> &str {
    match s {
        "joy2way" => "Joy 2-Way",
        "joy4way" => "Joy 4-Way",
        "joy8way" => "Joy 8-Way",
        "paddle" => "Paddle",
        "doublejoy2way" => "Double Joy 2-Way",
        "doublejoy4way" => "Double Joy 4-Way",
        "doublejoy8way" => "Double Joy 8-Way",
        "dial" => "Dial",
        "lightgun" => "Lightgun",
        "pedal" => "Pedal",
        "stick" => "Stick",
        "trackball" => "Trackball",
        "vjoy2way" => "Joy 2-Way (V)",
        "vdoublejoy2way" => "Double Joy 2-Way (V)",
        "baddump" => "Bad Dump",
        "nodump" => "No Dump",
        "raster" => "Raster",
        "vector" => "Vector",
        "lcd" => "LCD",
        "card" => "PC Card",
        "cdrom" | "cdrom0" | "cdrom1" => "CD-ROM",
        "cfcard" => "CompactFlash Card",
        "disk" | "disks" => "Disk",
        "gdrom" => "GD-ROM",
        "ide" => "IDE",
        "laserdisc" | "laserdisc2" => "Laserdisc",
        "scsi0" | "scsi1" => "SCSI",
        "vhs" => "VHS",
        other => other,
    }
}

/// maps built once per library refresh (origin: initFolders)
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct FolderMaps {
    /// description → system name (consoles)
    pub console_map: BTreeMap<String, String>,
    /// description → bios name
    pub bios_map: BTreeMap<String, String>,
}

impl FolderMaps {
    pub fn build(lib: &GameLibrary) -> Self {
        let mut m = Self::default();
        for g in &lib.games {
            if !g.devices.is_empty() && !g.is_ext_rom {
                m.console_map.insert(g.description.clone(), g.name.clone());
            }
            if g.is_bios {
                m.bios_map.insert(g.description.clone(), g.name.clone());
            }
        }
        m
    }
}

/// children (subfolder keys) for a root folder, with counts
pub struct FolderChild {
    pub label: String,
    pub key: String,
    pub count: usize,
}

/// filter predicate — mirrors GameListSortFilterProxyModel folder roles (§10)
pub fn matches(kind: &FolderKind, key: &str, g: &GameMeta) -> bool {
    match kind {
        FolderKind::AllGame => !g.is_bios,
        FolderKind::AllArc => !g.is_bios && !g.is_ext_rom && !g.is_console(),
        FolderKind::Available => !g.is_bios && !g.is_ext_rom && g.available == GAME_COMPLETE,
        FolderKind::Unavailable => !g.is_bios && !g.is_ext_rom && g.available != GAME_COMPLETE,
        FolderKind::Console => !g.is_ext_rom && g.is_console(),
        FolderKind::Manufacturer => !g.is_bios && g.manufacturer == key,
        FolderKind::Year => !g.is_bios && {
            let y = if g.year.is_empty() { "?".to_string() } else { g.year.clone() };
            y == key
        },
        FolderKind::Source => g.sourcefile == key,
        FolderKind::Bios => g.is_bios,
        FolderKind::Cpu => g.chips.iter().any(|c| c.kind == "cpu" && c.name == key),
        FolderKind::Snd => g.chips.iter().any(|c| c.kind == "audio" && c.name == key),
        FolderKind::HardDisk => {
            !g.disks.is_empty()
                && (key.is_empty() || g.disks.iter().any(|d| long_name(&d.region) == key))
        }
        FolderKind::Samples => !g.samples.is_empty(),
        FolderKind::Dumping => g
            .roms
            .iter()
            .any(|r| long_name(&r.status) == key)
            || g.disks.iter().any(|d| long_name(&d.status) == key),
        FolderKind::Working => !g.is_ext_rom && g.driver.status == STATUS_GOOD,
        FolderKind::NonWorking => !g.is_ext_rom && g.driver.status != STATUS_GOOD,
        FolderKind::Originals => !g.is_bios && !g.is_ext_rom && g.cloneof.is_empty(),
        FolderKind::Clones => !g.is_bios && !g.is_ext_rom && !g.cloneof.is_empty(),
        FolderKind::Resolution => !g.is_ext_rom
            && (0..g.displays.len()).any(|i| g.resolution_label(i) == key),
        FolderKind::PaletteSize => !g.is_ext_rom && g.palettesize.to_string() == key,
        FolderKind::Refresh => !g.is_ext_rom
            && g.displays.iter().any(|d| format!("{} Hz", d.refresh) == key),
        FolderKind::Display => !g.is_ext_rom && match key {
            "__horizontal__" => g.is_horz,
            "__vertical__" => !g.is_horz,
            k => g.displays.iter().any(|d| long_name(&d.kind) == k),
        },
        FolderKind::Controls => !g.is_ext_rom && {
            if key.ends_with('P') && key[..key.len() - 1].chars().all(|c| c.is_ascii_digit()) {
                format!("{}P", g.players) == key
            } else {
                g.controls.iter().any(|c| long_name(&c.kind) == key)
            }
        },
        FolderKind::Channels => !g.is_ext_rom && g.channels.to_string() == key,
        FolderKind::SaveState => !g.is_ext_rom && {
            key == "supported" && g.driver.savestate == STATUS_GOOD
                || key == "unsupported" && g.driver.savestate == STATUS_PRELIMINARY
        },
        FolderKind::Mechanical => !g.is_bios && !g.is_ext_rom && g.is_mechanical,
        FolderKind::NonMechanical => !g.is_bios && !g.is_ext_rom && !g.is_mechanical,
        FolderKind::Ext(_) => true,
    }
}

/// bios children use bios names as keys (biosMap reverse)
pub fn bios_child_key(bios_desc: &str, maps: &crate::core::folders::FolderMaps) -> String {
    maps.bios_map.get(bios_desc).cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// custom (external) folders — MAMEUI folder ini format
// ---------------------------------------------------------------------------

pub const EXTFOLDER_MAGIC: &str = "**00_";
pub const ROOT_FOLDER_SECTION: &str = "**00_ROOT_FOLDER";

pub const FAVORITES_INI_BYTES: &str =
    "[FOLDER_SETTINGS]\nRootFolderIcon golden\nSubFolderIcon cust2\n\n[ROOT_FOLDER]\n";

/// parseExtFolders: sections → multimap; ROOT_FOLDER/FOLDER_SETTINGS keep magic
pub fn parse_ext_folders(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut key = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];
            key = match section {
                "ROOT_FOLDER" | "FOLDER_SETTINGS" => format!("{EXTFOLDER_MAGIC}{section}"),
                s => s.to_string(),
            };
            map.entry(key.clone()).or_default();
            continue;
        }
        if !key.is_empty() {
            map.entry(key.clone()).or_default().push(line.to_string());
        }
    }
    map
}

/// saveExtFolders: magic sections written back as [ROOT_FOLDER]/[FOLDER_SETTINGS]
pub fn save_ext_folders(map: &BTreeMap<String, Vec<String>>) -> String {
    let mut out = String::new();
    for (k, values) in map {
        let section = k.strip_prefix(EXTFOLDER_MAGIC).unwrap_or(k);
        out.push_str(&format!("[{section}]\n"));
        let mut sorted = values.clone();
        sorted.sort();
        for v in sorted {
            out.push_str(&v);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// runtime state for one custom-folder ini file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtFolderStore {
    pub name: String,
    pub entries: BTreeMap<String, Vec<String>>,
    pub writable: bool,
}

impl ExtFolderStore {
    pub fn add(&mut self, section: &str, game: &str) {
        let key = if section == "ROOT_FOLDER" {
            format!("{EXTFOLDER_MAGIC}ROOT_FOLDER")
        } else {
            section.to_string()
        };
        let v = self.entries.entry(key).or_default();
        if !v.iter().any(|x| x == game) {
            v.push(game.to_string());
        }
    }
    pub fn remove_game(&mut self, section: &str, game: &str) {
        let key = if section == "ROOT_FOLDER" {
            format!("{EXTFOLDER_MAGIC}ROOT_FOLDER")
        } else {
            section.to_string()
        };
        if let Some(v) = self.entries.get_mut(&key) {
            v.retain(|x| x != game);
        }
    }
    pub fn games_in(&self, section: &str) -> Vec<String> {
        let key = if section == "ROOT_FOLDER" {
            format!("{EXTFOLDER_MAGIC}ROOT_FOLDER")
        } else {
            section.to_string()
        };
        self.entries.get(&key).cloned().unwrap_or_default()
    }
}


// ---------------------------------------------------------------------------
// precomputed folder tree (single pass over the library) — keeps the UI frame
// free of O(kinds x games x children) rescans
// ---------------------------------------------------------------------------

pub struct RootNode {
    pub kind: FolderKind,
    pub label: String,
    pub count: usize,
    pub children: Vec<FolderChild>,
}

#[derive(Default)]
pub struct FolderCache {
    pub roots: Vec<RootNode>,
}

/// Root folders that carry a boolean predicate instead of a key: their
/// subfolder key is meaningless, so the root shows every member of the
/// dimension (origin: filterFolderChanged, root branch with empty filterText).
fn is_flag_dimension(kind: &FolderKind) -> bool {
    matches!(
        kind,
        FolderKind::AllGame
            | FolderKind::AllArc
            | FolderKind::Available
            | FolderKind::Unavailable
            | FolderKind::HardDisk
            | FolderKind::Samples
            | FolderKind::Working
            | FolderKind::NonWorking
            | FolderKind::Originals
            | FolderKind::Clones
            | FolderKind::Mechanical
            | FolderKind::NonMechanical
    )
}

/// Every subfolder key this game contributes to, plus that key's display label.
/// Keys are exactly what `matches()` compares against — Display / Dumping /
/// Controls / Refresh therefore use the *long* name, because both the filter
/// and the tree label show the long name. `lib` is only needed by Bios and
/// Console, which resolve the parent machine for the label; pass None to skip.
pub fn for_each_child_key(
    kind: &FolderKind,
    g: &GameMeta,
    lib: Option<&GameLibrary>,
    f: &mut impl FnMut(&str, &str),
) {
    match kind {
        FolderKind::Manufacturer => {
            if !g.is_bios && !g.manufacturer.is_empty() {
                f(&g.manufacturer, &g.manufacturer);
            }
        }
        FolderKind::Year => {
            if !g.is_bios {
                let y = if g.year.is_empty() { "?" } else { &g.year };
                f(y, y);
            }
        }
        FolderKind::Source => {
            if !g.is_device && !g.sourcefile.is_empty() {
                f(&g.sourcefile, &g.sourcefile);
            }
        }
        FolderKind::Bios => {
            if let Some(lib) = lib {
                let b = g.bios_of(lib);
                if !b.is_empty() {
                    match lib.get(&b).map(|x| x.description.clone()) {
                        Some(d) => f(&b, &d),
                        None => f(&b, &b),
                    }
                }
            }
        }
        FolderKind::Console => {
            if g.is_ext_rom && !g.romof.is_empty() {
                match lib
                    .and_then(|l| l.get(&g.romof))
                    .map(|x| x.description.clone())
                {
                    Some(d) => f(&g.romof, &d),
                    None => f(&g.romof, &g.romof),
                }
            }
        }
        FolderKind::Cpu | FolderKind::Snd => {
            let want = if *kind == FolderKind::Cpu { "cpu" } else { "audio" };
            for c in &g.chips {
                if c.kind == want {
                    f(&c.name, long_name(&c.name));
                }
            }
        }
        FolderKind::HardDisk => {
            for d in &g.disks {
                let r = long_name(&d.region);
                if !r.is_empty() {
                    f(r, r);
                }
            }
        }
        FolderKind::Dumping => {
            for r in &g.roms {
                if !r.status.is_empty() {
                    let l = long_name(&r.status);
                    f(l, l);
                }
            }
            for d in &g.disks {
                if !d.status.is_empty() {
                    let l = long_name(&d.status);
                    f(l, l);
                }
            }
        }
        FolderKind::Display => {
            if !g.is_ext_rom {
                for d in &g.displays {
                    let l = long_name(&d.kind);
                    f(l, l);
                }
                if g.is_horz {
                    f("__horizontal__", "Horizontal");
                } else {
                    f("__vertical__", "Vertical");
                }
            }
        }
        FolderKind::Refresh => {
            if !g.is_ext_rom {
                for d in &g.displays {
                    let l = format!("{} Hz", d.refresh);
                    f(&l, &l);
                }
            }
        }
        FolderKind::Resolution => {
            if !g.is_ext_rom {
                // one game with two identical displays must count once
                let mut seen: Vec<String> = Vec::new();
                for i in 0..g.displays.len() {
                    if g.displays[i].kind == "vector" {
                        continue;
                    }
                    let l = g.resolution_label(i);
                    if seen.contains(&l) {
                        continue;
                    }
                    seen.push(l.clone());
                    f(&l, &l);
                }
            }
        }
        FolderKind::Controls => {
            if !g.is_ext_rom {
                for c in &g.controls {
                    let l = long_name(&c.kind);
                    f(l, l);
                }
                if g.players > 0 {
                    let l = format!("{}P", g.players);
                    f(&l, &l);
                }
            }
        }
        FolderKind::PaletteSize => {
            if !g.is_ext_rom {
                let l = g.palettesize.to_string();
                f(&l, &l);
            }
        }
        FolderKind::Channels => {
            if !g.is_ext_rom {
                let l = g.channels.to_string();
                f(&l, &l);
            }
        }
        FolderKind::SaveState => {
            if !g.is_ext_rom {
                if g.driver.savestate == STATUS_GOOD {
                    f("supported", "Supported");
                } else if g.driver.savestate == STATUS_PRELIMINARY {
                    f("unsupported", "Unsupported");
                }
            }
        }
        _ => {}
    }
}

/// Root membership. `has_keys` is what `for_each_child_key` produced and is
/// only consulted for the keyed dimensions; Bios / Console keep their own
/// predicate because their children are keyed by the *parent* machine.
fn root_member(kind: &FolderKind, g: &GameMeta, has_keys: bool) -> bool {
    match kind {
        FolderKind::Bios => g.is_bios,
        FolderKind::Console => !g.is_ext_rom && g.is_console(),
        FolderKind::SaveState => {
            !g.is_ext_rom
                && (g.driver.savestate == STATUS_GOOD
                    || g.driver.savestate == STATUS_PRELIMINARY)
        }
        k if is_flag_dimension(k) => matches(k, "", g),
        _ => has_keys,
    }
}

/// Does this game belong to this root folder at all?
///
/// The port used to call `matches(kind, "", g)` here, which is wrong: every
/// keyed dimension compares its key against "" and rejects every game, so
/// Manufacturer / Year / Driver / CPU / ... rendered as "(0)" with no
/// children at all — the folder tree looked empty.
pub fn in_dimension(kind: &FolderKind, g: &GameMeta) -> bool {
    let mut has_keys = false;
    for_each_child_key(kind, g, None, &mut |_, _| has_keys = true);
    root_member(kind, g, has_keys)
}

/// Single pass over the library (origin: initFolders) — every dimension's
/// tally is filled while walking the games once, instead of one full pass per
/// root kind.
pub fn compute_folder_cache(lib: &GameLibrary, is_mess: bool) -> FolderCache {
    let kinds = root_folder_names(is_mess);
    let n = kinds.len();
    let mut tallies: Vec<BTreeMap<String, usize>> = vec![BTreeMap::new(); n];
    let mut labels: Vec<BTreeMap<String, String>> = vec![BTreeMap::new(); n];
    let mut root_counts: Vec<usize> = vec![0; n];

    for g in &lib.games {
        // The tree must agree with the list, and the list never shows device
        // machines (the refilter skips them first): counting them here is what
        // made 全部街机 read 46 885 while the status bar said 39 903, and what
        // double-counted software-list devices under both 全部街机 and 游戏机.
        if g.is_device {
            continue;
        }
        for i in 0..n {
            let kind = &kinds[i].0;
            let mut has_keys = false;
            for_each_child_key(kind, g, Some(lib), &mut |k, l| {
                if k.is_empty() {
                    return;
                }
                has_keys = true;
                *tallies[i].entry(k.to_string()).or_insert(0) += 1;
                labels[i]
                    .entry(k.to_string())
                    .or_insert_with(|| l.to_string());
            });
            if root_member(kind, g, has_keys) {
                root_counts[i] += 1;
            }
        }
    }

    let mut cache = FolderCache::default();
    for (i, (kind, label)) in kinds.into_iter().enumerate() {
        let children = tallies[i]
            .iter()
            .filter(|(_, c)| **c > 0)
            .map(|(key, count)| FolderChild {
                label: labels[i].get(key).cloned().unwrap_or_else(|| key.clone()),
                key: key.clone(),
                count: *count,
            })
            .collect();
        cache.roots.push(RootNode {
            kind,
            label: label.to_string(),
            count: root_counts[i],
            children,
        });
    }
    cache
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{ControlInfo, DisplayInfo};

    fn lib_with(games: Vec<GameMeta>) -> GameLibrary {
        let mut lib = GameLibrary::new("test".into());
        for g in games {
            lib.push(g);
        }
        lib.rebuild_indexes();
        lib.complete_data();
        lib
    }

    fn sample() -> GameLibrary {
        lib_with(vec![
            GameMeta {
                name: "puckman".into(),
                description: "Puck Man".into(),
                manufacturer: "Namco".into(),
                year: "1980".into(),
                sourcefile: "pacman.cpp".into(),
                ..Default::default()
            },
            GameMeta {
                name: "pacman".into(),
                description: "Pac-Man".into(),
                manufacturer: "Namco".into(),
                year: "1980".into(),
                sourcefile: "pacman.cpp".into(),
                cloneof: "puckman".into(),
                romof: "puckman".into(),
                players: 2,
                channels: 1,
                palettesize: 32,
                displays: vec![DisplayInfo {
                    kind: "raster".into(),
                    rotate: "0".into(),
                    width: 288,
                    height: 224,
                    refresh: "60.606060".into(),
                    ..Default::default()
                }],
                controls: vec![ControlInfo {
                    kind: "joy4way".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        ])
    }

    /// regression: keyed dimensions used to end up with "(0)" and no children
    /// because the root path compared every key against "".
    #[test]
    fn keyed_dimensions_have_children() {
        let lib = sample();
        let cache = compute_folder_cache(&lib, false);

        let mftr = cache
            .roots
            .iter()
            .find(|r| r.kind == FolderKind::Manufacturer)
            .expect("manufacturer root");
        assert_eq!(mftr.count, 2);
        assert!(mftr.children.iter().any(|c| c.key == "Namco" && c.count == 2));

        let year = cache
            .roots
            .iter()
            .find(|r| r.kind == FolderKind::Year)
            .expect("year root");
        assert_eq!(year.count, 2);
        assert!(year.children.iter().any(|c| c.key == "1980" && c.count == 2));

        let src = cache
            .roots
            .iter()
            .find(|r| r.kind == FolderKind::Source)
            .expect("driver root");
        assert!(src.children.iter().any(|c| c.key == "pacman.cpp"));

        let clones = cache
            .roots
            .iter()
            .find(|r| r.kind == FolderKind::Clones)
            .expect("clones root");
        assert_eq!(clones.count, 1);
    }

    /// the tree key must be exactly what `matches()` compares against
    #[test]
    fn child_keys_roundtrip_through_matches() {
        let lib = sample();
        let g = lib.get("pacman").unwrap();

        for kind in [
            FolderKind::Manufacturer,
            FolderKind::Year,
            FolderKind::Source,
            FolderKind::Display,
            FolderKind::Resolution,
            FolderKind::PaletteSize,
            FolderKind::Channels,
            FolderKind::Controls,
        ] {
            let mut checked = false;
            for_each_child_key(&kind, g, Some(&lib), &mut |k, _| {
                if k.is_empty() {
                    return;
                }
                assert!(
                    matches(&kind, k, g),
                    "{kind:?} produced key {k:?} that matches() rejects"
                );
                checked = true;
            });
            assert!(checked, "{kind:?} produced no key");
        }

        assert!(in_dimension(&FolderKind::Manufacturer, g));
        assert!(in_dimension(&FolderKind::Year, g));
    }

    /// The tree counts must agree with the list, which never shows devices:
    /// a device machine must appear in no root count and no child tally.
    #[test]
    fn devices_are_not_counted() {
        let lib = lib_with(vec![
            GameMeta {
                name: "pacman".into(),
                description: "Pac-Man".into(),
                manufacturer: "Namco".into(),
                year: "1980".into(),
                sourcefile: "pacman.cpp".into(),
                ..Default::default()
            },
            GameMeta {
                name: "joy_card".into(),
                description: "Joy Card".into(),
                is_device: true,
                sourcefile: "joy.cpp".into(),
                ..Default::default()
            },
        ]);
        let cache = compute_folder_cache(&lib, false);
        let all_arc = cache
            .roots
            .iter()
            .find(|r| r.kind == FolderKind::AllArc)
            .expect("all-arc root");
        assert_eq!(all_arc.count, 1, "a device must not inflate 全部街机");
        let driver = cache
            .roots
            .iter()
            .find(|r| r.kind == FolderKind::Source)
            .expect("driver root");
        assert!(
            driver.children.iter().all(|c| c.key != "joy.cpp"),
            "a device must not contribute to the driver tally"
        );
    }
}
