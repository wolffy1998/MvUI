//! Data model mirroring MAME's `-listxml` output (origin: prototype.h GameInfo).
//! Status numbering follows the original utils.cpp getStatus():
//! good=1, imperfect=2, preliminary=0, unknown=64.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub type StatusGrade = u8;
/// original: "good"→1, "imperfect"→2, "preliminary"→0, else 64
pub const STATUS_GOOD: StatusGrade = 1;
pub const STATUS_PRELIMINARY: StatusGrade = 0;
pub const STATUS_IMPERFECT: StatusGrade = 2;
pub const STATUS_NA: StatusGrade = 64;

pub const GAME_MISSING: u8 = 0;
pub const GAME_COMPLETE: u8 = 1;

pub fn grade_of(v: &str) -> StatusGrade {
    match v {
        "good" | "supported" => STATUS_GOOD,
        "imperfect" => STATUS_IMPERFECT,
        "preliminary" | "unsupported" => STATUS_PRELIMINARY,
        _ => STATUS_NA,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BiosSet {
    pub name: String,
    pub description: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RomInfo {
    pub name: String,
    pub bios: String,
    pub size: u64,
    pub crc: u32,
    pub merge: String,
    pub region: String,
    pub status: String,
    /// audit result (nodump counts as available)
    #[serde(default)]
    pub available: bool,
}

impl RomInfo {
    pub fn is_nodump(&self) -> bool {
        self.status.eq_ignore_ascii_case("nodump")
    }
    pub fn effective_name(&self) -> &str {
        if self.merge.is_empty() { &self.name } else { &self.merge }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub sha1: String,
    pub merge: String,
    pub region: String,
    pub index: u8,
    pub status: String,
    #[serde(default)]
    pub available: bool,
}

impl DiskInfo {
    pub fn is_nodump(&self) -> bool {
        self.status.eq_ignore_ascii_case("nodump")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChipInfo {
    pub name: String,
    pub tag: String,
    /// "cpu" | "audio"
    pub kind: String,
    pub clock: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub kind: String,
    /// original keeps the raw string ("0"/"90"/"270")
    pub rotate: String,
    pub flipx: bool,
    pub width: u16,
    pub height: u16,
    pub refresh: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControlInfo {
    pub kind: String,
    pub min: u16,
    pub max: u16,
    pub sensitivity: u16,
    pub keydelta: u16,
    pub reverse: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SoftwareListRef {
    pub name: String,
    pub status: String,
    pub filter: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceInfo {
    /// "-<instance> <path>" command line instance name (map key in the original)
    pub instance: String,
    pub kind: String,
    pub tag: String,
    pub mandatory: bool,
    /// set when a rom file matches the device extension (auto-mounted)
    pub mounted_path: String,
    pub is_const: bool,
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameMeta {
    /// ROM name; ext roms use the "dirpath+file[/zipentry]" key
    pub name: String,
    pub sourcefile: String,
    pub is_bios: bool,
    pub is_device: bool,
    pub is_mechanical: bool,
    pub is_ext_rom: bool,
    pub cloneof: String,
    pub romof: String,
    pub sampleof: String,
    pub description: String,
    pub year: String,
    pub manufacturer: String,

    pub bios_sets: Vec<BiosSet>,
    pub roms: Vec<RomInfo>,
    pub disks: Vec<DiskInfo>,
    pub samples: Vec<String>,
    pub chips: Vec<ChipInfo>,
    pub displays: Vec<DisplayInfo>,
    pub channels: u8,
    pub service: bool,
    pub tilt: bool,
    pub players: u8,
    pub buttons: u8,
    pub coins: u8,
    pub controls: Vec<ControlInfo>,
    pub softwarelists: Vec<SoftwareListRef>,
    pub devices: Vec<DeviceInfo>,
    pub driver: DriverStatus,
    pub palettesize: u32,
    pub ram_options: Vec<u32>,
    pub default_ram_option: u32,

    // derived (complete_data)
    #[serde(default)]
    pub clones: HashSet<String>,
    #[serde(default)]
    pub is_horz: bool,
    /// Localized description from `mame_cn.lst`, or empty when untranslated.
    #[serde(default)]
    pub lc_desc: String,
    /// Retained for cache compatibility only — no longer written or read.
    ///
    /// The list's second column is not a manufacturer translation (it repeats
    /// the description in every shipped file), and honouring it put the Chinese
    /// title in the Manufacturer column. Kept so an old cache still deserialises
    /// rather than erroring on an unknown field.
    #[serde(default)]
    pub lc_mftr: String,
    #[serde(default)]
    pub icondata: Vec<u8>,
    /// overall audit: GAME_MISSING/COMPLETE/INCOMPLETE
    #[serde(default)]
    pub available: u8,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DriverStatus {
    pub status: StatusGrade,
    pub emulation: StatusGrade,
    pub color: StatusGrade,
    pub sound: StatusGrade,
    pub graphic: StatusGrade,
    pub cocktail: StatusGrade,
    pub protection: StatusGrade,
    pub savestate: StatusGrade,
}

impl GameMeta {
    /// original GameInfo::biosof(): romof, else grandparent romof; only when
    /// the resolved game is a bios.
    pub fn bios_of<'a>(&'a self, lib: &'a GameLibrary) -> String {
        if self.is_bios {
            return String::new();
        }
        let mut candidate = self.romof.clone();
        if candidate.is_empty() {
            return String::new();
        }
        let parent = match lib.get(&candidate) {
            Some(p) => p,
            None => return String::new(),
        };
        if !parent.romof.is_empty() {
            candidate = parent.romof.clone();
        }
        match lib.get(&candidate) {
            Some(g) if g.is_bios => candidate,
            _ => String::new(),
        }
    }

    pub fn is_vector(&self) -> bool {
        self.displays.iter().any(|d| d.kind.eq_ignore_ascii_case("vector"))
    }

    pub fn is_console(&self) -> bool {
        !self.softwarelists.is_empty()
    }

    /// "256 x 224 (H)" (origin: Gamelist::getResolution)
    pub fn resolution_label(&self, idx: usize) -> String {
        match self.displays.get(idx) {
            Some(d) => format!(
                "{} x {} {}",
                d.width,
                d.height,
                if self.is_horz { "(H)" } else { "(V)" }
            ),
            None => String::new(),
        }
    }
}

use crate::core::library::GameLibrary;
