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
    /// verify result (nodump counts as available)
    #[serde(default)]
    pub available: bool,
}

impl RomInfo {
    pub fn is_nodump(&self) -> bool {
        self.status.eq_ignore_ascii_case("nodump")
    }
    /// `baddump`：条目有，但 MAME 标了内容是坏的。
    ///
    /// 与 `is_nodump` 成对存在，两个都不是 `good`——但含义相反：nodump 是
    /// "永远不会有这个文件"，baddump 是"文件在，但是坏的"。用户拿到 baddump
    /// 该去重下，拿到 nodump 只需要知道没辙。
    pub fn is_baddump(&self) -> bool {
        self.status.eq_ignore_ascii_case("baddump")
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
    /// `baddump` 的 CHD：文件在，但内容是坏的。与 `RomInfo::is_baddump` 同义。
    pub fn is_baddump(&self) -> bool {
        self.status.eq_ignore_ascii_case("baddump")
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
    /// 来自 `<device_ref>`（引用一个**设备机种**）还是 `<device>`（本机自带的
    /// 可挂载设备 / 槽位）。
    ///
    /// 必须显式记，不能靠 `kind == instance` 猜：`<device type="cartridge">` 里
    /// 的 `<instance name="cartridge">` 名字恰好与 `type` 相同，猜法会把 cartridge
    /// 槽位误判成"引用设备"（实测 nes 的 9 个 `<device>` 全中），结果是引用设备
    /// 段混进一堆假设备，而真正的设备段永远是空的。
    pub is_ref: bool,
}

/// `<slot name="ctrl1">` —— 一个可插拔槽位（如 NES 的手柄口、卡带口）。
///
/// 这是 `<device>`（本机自带的槽位设备）之外的另一种表达：`<slot>` 只给槽位
/// 名和**可选**设备列表，不含instance / extension。MAME 里两者并存，
/// `nes` 同时有 9 个 `<device>` 和 12 个 `<slot>`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlotInfo {
    /// 槽位名（命令行 `<machine>:<slot>=<devname>` 用的就是它）。
    pub name: String,
    /// `<slotoption>`：可选设备。空槽位（只有 name 没有 option）也照样记一条。
    pub options: Vec<SlotOption>,
}

/// `<slotoption name="vboy" devname="nes_vboyctrl" default="no"/>`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlotOption {
    /// 选项名（显示用）。
    pub name: String,
    /// 设备机种名，可拿去 `GameLibrary::get` 查它的 rom。
    pub devname: String,
    pub default: bool,
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
    /// `<slot>` 槽位（含 `<slotoption>` 可选设备）。
    pub slots: Vec<SlotInfo>,
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
    /// overall verify: GAME_MISSING/COMPLETE/INCOMPLETE
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
