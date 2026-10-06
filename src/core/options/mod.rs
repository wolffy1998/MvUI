//! MAME option system, 1:1 port of mameopt.cpp.
//!
//! Levels: GUI(0) Global(1) Horizont(2) Vertical(3) Source(4) Bios(5)
//! Cloneof(6) Curr(7).
//!
//! Horizont/Vertical are MAME's `horizont.ini`/`vertical.ini` pair in the ini
//! chain. MAME itself loads only the one matching the driver's native
//! orientation (origin: parse_standard_inis); MvUI loads **both** — each fills
//! its own field, and `active` (the game's `is_horz`) decides which one also
//! feeds `currvalue`. That way both editor pages can be shown at once, and the
//! off-chain page still displays real values instead of nothing.

pub use crate::core::library::GameLibrary;
use crate::core::model::GameMeta;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub const OPTLEVEL_GUI: usize = 0;
pub const OPTLEVEL_GLOBAL: usize = 1;
pub const OPTLEVEL_HORIZONT: usize = 2;
pub const OPTLEVEL_VERTICAL: usize = 3;
pub const OPTLEVEL_SRC: usize = 4;
pub const OPTLEVEL_BIOS: usize = 5;
pub const OPTLEVEL_CLONEOF: usize = 6;
pub const OPTLEVEL_CURR: usize = 7;
pub const OPTLEVEL_LAST: usize = 8;
pub const LEVEL_NAMES: [&str; OPTLEVEL_LAST] =
    ["GUI", "Global", "Horizont", "Vertical", "Source", "Bios", "Cloneof", "Game"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptKind {
    Bool,
    Int,
    Float,
    Str,
    StrEditable,
    File,
    DatFile,
    CfgFile,
    ExeFile,
    Dir,
    Dirs,
    Csv,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MameOption {
    pub guiname: String,
    pub defvalue: String,
    pub description: String,
    pub currvalue: String,
    pub max: String,
    pub min: String,
    pub globalvalue: String,
    pub horzvalue: String,
    pub vertvalue: String,
    pub srcvalue: String,
    pub biosvalue: String,
    pub cloneofvalue: String,
    pub kind: Option<OptKind>,
    pub guivisible: bool,
    pub globalvisible: bool,
    pub srcvisible: bool,
    pub biosvisible: bool,
    pub cloneofvisible: bool,
    pub gamevisible: bool,
    /// canonical ini values / display values, index-parallel
    pub values: Vec<String>,
    pub guivalues: Vec<String>,
}

impl Default for OptKind {
    fn default() -> Self {
        OptKind::Unknown
    }
}

impl MameOption {
    fn new(defvalue: &str) -> Self {
        Self {
            defvalue: defvalue.to_string(),
            globalvisible: true,
            srcvisible: true,
            biosvisible: true,
            cloneofvisible: true,
            gamevisible: true,
            ..Default::default()
        }
    }
}

/// hardcoded category table (origin: optCatList, order matters)
const OPT_CAT_LIST: [&str; 46] = [
    "00_Global Misc_00_core configuration",
    "00_Global Misc_01_core palette",
    "00_Global Misc_02_core language",
    "02_MAME Paths_00_core search path",
    "02_MAME Paths_01_core output directory",
    "02_MAME Paths_02_core filename",
    "04_Core Video_00_core rotation",
    "04_Core Video_01_core screen",
    "04_Core Video_02_core performance",
    "04_Core Video_03_core render",
    "05_OSD Video_00_OSD video",
    "05_OSD Video_01_Windows video",
    "05_OSD Video_02_OSD full screen",
    "05_OSD Video_03_full screen",
    "05_OSD Video_04_DirectDraw-specific",
    "05_OSD Video_05_Direct3D-specific",
    "05_OSD Video_06_Direct3D post-processing",
    "05_OSD Video_07_OSD performance",
    "05_OSD Video_08_OpenGL-specific",
    "05_OSD Video_09_NTSC post-processing",
    "05_OSD Video_10_Bloom post-processing",
    "05_OSD Video_11_BGFX post-processing",
    "05_OSD Video_12_OSD accelerated video",
    "06_Screen_00_OSD per-window video",
    "07_Audio_00_OSD sound",
    "07_Audio_01_core sound",
    "08_Control_00_core input",
    "08_Control_01_core input automatic enable",
    "08_Control_02_input device",
    "08_Control_03_SDL keyboard mapping",
    "08_Control_04_SDL joystick mapping",
    "08_Control_05_OSD input options",
    "08_Control_06_OSD input mapping",
    "09_Vector_00_core vector",
    "09_Vector_01_Vector post-processing",
    "10_Misc_01_core misc",
    "10_Misc_02_core artwork",
    "10_Misc_03_core state/playback",
    "10_Misc_04_SDL lowlevel driver",
    "10_Misc_05_MESS specific",
    "10_Misc_06_Windows MESS specific",
    "10_Misc_07_Windows performance",
    "10_Misc_08_core debugging",
    "10_Misc_09_OSD debugging",
    "10_Misc_10_Windows debugging",
    "06_Misc_99_fallback",
];

/// sidebar categories per level: (GUI, [7 categories])
pub const GUI_CATEGORIES: [&str; 3] = ["GUI Paths", "MAME Paths", "MESS Paths"];
pub const CORE_CATEGORIES: [&str; 7] = [
    "Core Video",
    "OSD Video",
    "Screen",
    "Audio",
    "Control",
    "Vector",
    "Misc",
];

/// The one copy of the template lives at the workspace root, next to the icon
/// set `the legacy app crate/build.rs` embeds. It used to be duplicated under
/// `crates/the legacy core crate/assets/` for this `include_str!`, and two copies of a
/// 20 KB hand-edited file silently drift out of sync — editing one changes
/// nothing at all for the user.
const TEMPLATE_XML: &str = include_str!("../../../assets/optiontemplate.xml");

/// One path default from the template: option name → `default=` attribute.
///
/// The Settings ▸ Directories dialog reads its fallbacks here instead of
/// re-deriving them in Rust, so the XML stays the one place a default is
/// written. Works before boot: it parses the embedded string and needs no
/// `OptionCore`.
pub fn template_default(key: &str) -> String {
    static MAP: std::sync::OnceLock<HashMap<String, String>> = std::sync::OnceLock::new();
    MAP.get_or_init(|| {
        let mut reader = quick_xml::Reader::from_str(TEMPLATE_XML);
        reader.config_mut().trim_text(true);
        // 与 load_template 同一个 0.41 陷阱：不展开自闭合标签就什么都收不到
        reader.config_mut().expand_empty_elements = true;
        let mut map = HashMap::new();
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(quick_xml::events::Event::Start(ref e)) if e.name().as_ref() == b"option" => {
                    let mut name = String::new();
                    let mut def = String::new();
                    for a in e.attributes().flatten() {
                        let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                        let v =
                            crate::core::listxml::xml_unescape(&String::from_utf8_lossy(&a.value));
                        match k.as_str() {
                            "name" => name = v,
                            "default" => def = v,
                            _ => {}
                        }
                    }
                    if !name.is_empty() {
                        map.insert(name, def);
                    }
                }
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        map
    })
    .get(key)
    .cloned()
    .unwrap_or_default()
}

#[derive(Debug, Default)]
pub struct OptionCore {
    /// directory of mame.exe; relative ini paths resolve against it
    pub base_dir: PathBuf,
    pub opts: BTreeMap<String, MameOption>,
    /// "NN_Category_MM_label" -> option names (BTreeMap keeps original sort)
    pub opt_cat_map: BTreeMap<String, Vec<String>>,
    pub mame_ini_path: PathBuf,
    pub is_sdl_port: bool,
    pub has_language: bool,
    pub has_ips: bool,
    pub has_devices: bool,
    pub mess_like: bool,
    // saveIniFile "static" quirks, replicated as state
    save_c: i32,
    save_is_entry: bool,
    /// non-fatal problems hit while parsing (template truncation, unreadable
    /// ini …). The GUI logs them instead of the core printing to a console
    /// that does not exist in a windows-subsystem build.
    pub warnings: Vec<String>,
}

/// parseIni(text, isInitOptCatMap) — returns (settings, categories built in-place)
fn parse_ini_core(
    text: &str,
    is_init: bool,
    cat_map: &mut Option<BTreeMap<String, Vec<String>>>,
) -> HashMap<String, String> {
    let text = text.strip_prefix("﻿").unwrap_or(text);
    let mut settings = HashMap::new();
    let mut cur_cat = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            if is_init && line.len() > 2 {
                let mut category = rest.replace("OPTIONS", "");
                category = category.replace('#', "");
                let category = category.trim().to_string();
                let mut found = None;
                for (i, cand) in OPT_CAT_LIST.iter().enumerate() {
                    if cand.to_lowercase().contains(&category.to_lowercase()) {
                        // original quirk: indexOf(...) > 0 — index 0 falls through
                        if i > 0 {
                            found = Some(cand.to_string());
                        }
                        break;
                    }
                }
                cur_cat = found.unwrap_or_else(|| format!("06_Misc_99_{category}"));
                if cat_map.as_mut().is_some() && !cur_cat.is_empty() {
                    cat_map.as_mut().unwrap().entry(cur_cat.clone()).or_default();
                }
            }
            continue;
        }
        if line.starts_with('<') {
            continue;
        }
        let (key, value) = match line.find(' ') {
            Some(sp) => (line[..sp].trim(), line[sp..].trim()),
            None => (line, ""),
        };
        let value = {
            let v = value.trim();
            if v.starts_with('"') && v.ends_with('"') && v.len() >= 2 {
                v[1..v.len() - 1].to_string()
            } else {
                v.to_string()
            }
        };
        if is_init {
            if let Some(cm) = cat_map.as_mut() {
                if !cur_cat.is_empty() {
                    cm.entry(cur_cat.clone()).or_default().push(key.to_string());
                }
            }
        }
        settings.insert(key.to_string(), value);
    }
    settings
}

/// Read a text file the way Qt 4's `QTextStream` did (`autoDetectUnicode`
/// on, codec = locale): honour a UTF-8/UTF-16 BOM, else strict UTF-8, else
/// decode with the system code page — approximated by GB18030, a superset of
/// GBK/CP936 that also covers the whole BMP, so decoding never fails.
///
/// Used for MAME's ini files: MAME itself writes them with a UTF-8 BOM, but
/// hand-edited or tool-written ones are frequently GBK on Chinese Windows.
pub fn read_text_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some(String::from_utf8_lossy(rest).to_string());
    }
    if let Ok(s) = std::str::from_utf8(&bytes) {
        return Some(s.to_string());
    }
    let (cow, _, _) = encoding_rs::GB18030.decode(&bytes);
    Some(cow.to_string())
}

impl OptionCore {
    /// origin: loadDefault(defaultIni) + loadTemplate()
    pub fn load_default(
        default_ini: &str,
        lib: &GameLibrary,
        gui: &HashMap<String, String>,
        base_dir: &Path,
    ) -> Self {
        let mut core = Self {
            base_dir: base_dir.to_path_buf(),
            ..Default::default()
        };
        let mut cat_map = Some(BTreeMap::new());
        let ini_settings = parse_ini_core(default_ini, true, &mut cat_map);
        core.opt_cat_map = cat_map.unwrap();

        for (k, v) in &ini_settings {
            core.opts.insert(k.clone(), MameOption::new(v));
        }

        // mameIniPath: first token of inipath
        if let Some(inipath) = ini_settings.get("inipath") {
            let first = inipath.split(';').next().unwrap_or("");
            let mut p = PathBuf::from(first.trim());
            if p.is_relative() {
                p = base_dir.join(p);
            }
            core.mame_ini_path = p.components().collect::<PathBuf>();
        }
        // patch inipath for unofficial mame (origin: mameopt.cpp:1757-1764 —
        // unconditionally append `mameIniPath + "ini"`, then strip "./").
        // The previous port mixed separators: it compared a `/`-normalised path
        // against the raw value, so a value using `\` looked "absent" and the
        // ini dir got appended a second time.
        if let Some(opt) = core.opts.get_mut("inipath") {
            if opt.defvalue != ".;ini" {
                let mut v = opt.defvalue.clone();
                v.push(';');
                // MAME accepts either separator, but the original tool wrote the
                // forward-slash form into the ini, so normalise what we append
                let ini_dir = core.mame_ini_path.join("ini");
                v.push_str(&ini_dir.to_string_lossy().replace('\\', "/"));
                v = v.replace("./", "");
                opt.defvalue = v;
            }
        }

        // port detection
        core.is_sdl_port = core.opts.contains_key("sdlvideofps") || core.opts.contains_key("videodriver");
        core.has_language =
            core.opts.contains_key("langpath") || core.opts.contains_key("languagepath");
        core.has_ips = core.opts.contains_key("ips");

        core.load_template(gui);

        // MESS devices → <game>_extra_software options
        for g in &lib.games {
            if !g.devices.is_empty() && !g.is_ext_rom {
                let name = format!("{}_extra_software", g.name);
                let mut o = MameOption::new("");
                o.guivisible = true;
                o.kind = Some(OptKind::Dir);
                core.opts.insert(name.clone(), o);
                core.opt_cat_map
                    .entry("03_MESS Paths_00_MESS software directory".into())
                    .or_default()
                    .push(name);
                core.has_devices = true;
            }
        }

        // remaining guivisible options → GUI paths
        let guivisible: Vec<String> = core
            .opts
            .iter()
            .filter(|(_, o)| o.guivisible)
            .map(|(k, _)| k.clone())
            .collect();
        for name in guivisible {
            core.opt_cat_map
                .entry("01_GUI Paths_00_GUI paths".into())
                .or_default()
                .push(name);
        }

        let _ = std::fs::create_dir_all(core.mame_ini_path.join("ini").join("source"));
        core
    }

    /// MAME 0.227 把 langpath 改名为 languagepath；mamep 等旧构建仍用旧名。
    /// 语言功能（启动参数、本地化 DAT 路径）按这个键读写，随二进制自适应。
    pub fn language_path_key(&self) -> &'static str {
        if self.opts.contains_key("languagepath") {
            "languagepath"
        } else {
            "langpath"
        }
    }

    /// origin: OptionXMLHandler over res/optiontemplate.xml
    fn load_template(&mut self, gui: &HashMap<String, String>) {
        let mut cur: Option<String> = None;
        let mut text = String::new();
        let mut in_value = false;
        let mut value_os = String::new();
        // <value guivalue="GDI">gdi</value>: the old GUI shows the guivalue text
        // (`OptionXMLHandler`: guiname of the value wins over the canonical one)
        let mut value_gui = String::new();
        let mut reader = quick_xml::Reader::from_str(TEMPLATE_XML);
        reader.config_mut().trim_text(true);
        // 0.41 默认不展开自闭合标签：`<option …/>` 以 Event::Empty 到达，
        // 下面只认 Event::Start，不设置就会静默丢掉整个模板
        reader.config_mut().expand_empty_elements = true;
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(quick_xml::events::Event::Start(ref e)) => match e.name().as_ref() {
                    b"option" => {
                        let mut name = String::new();
                        let mut guivisible = false;
                        let mut attrs: HashMap<String, String> = HashMap::new();
                        for a in e.attributes() {
                            let a = match a {
                                Ok(a) => a,
                                Err(_) => continue,
                            };
                            let k = String::from_utf8_lossy(a.key.as_ref()).to_string();
                            let v = crate::core::listxml::xml_unescape(&String::from_utf8_lossy(&a.value));
                            match k.as_str() {
                                "name" => name = v.clone(),
                                "guivisible" => guivisible = v == "1",
                                _ => {}
                            }
                            attrs.insert(k, v);
                        }
                        if name.is_empty() {
                            continue;
                        }
                        if guivisible && !self.opts.contains_key(&name) {
                            let def = attrs.get("default").cloned().unwrap_or_default();
                            let mut o = MameOption::new(&def);
                            // GUI options may be overridden by pGuiSettings
                            if let Some(v) = gui.get(&name) {
                                o.defvalue = v.clone();
                            }
                            o.guivisible = true;
                            self.opts.insert(name.clone(), o);
                        }
                        if let Some(o) = self.opts.get_mut(&name) {
                            o.guiname = attrs.get("guiname").cloned().unwrap_or_default();
                            o.max = attrs.get("max").cloned().unwrap_or_default();
                            o.min = attrs.get("min").cloned().unwrap_or_default();
                            o.kind = Some(match attrs.get("type").map(|s| s.as_str()).unwrap_or("") {
                                "string" => OptKind::Str,
                                "stringeditable" => OptKind::StrEditable,
                                "file" => OptKind::File,
                                "datfile" => OptKind::DatFile,
                                "exefile" => OptKind::ExeFile,
                                "cfgfile" => OptKind::CfgFile,
                                "dir" => OptKind::Dir,
                                "dirs" => OptKind::Dirs,
                                "int" => OptKind::Int,
                                "float" => OptKind::Float,
                                "bool" => OptKind::Bool,
                                "csv" => OptKind::Csv,
                                _ => OptKind::Unknown,
                            });
                            o.guivisible = guivisible;
                            o.globalvisible = attrs.get("globalvisible").map(|v| v != "0").unwrap_or(true);
                            o.srcvisible = attrs.get("srcvisible").map(|v| v != "0").unwrap_or(true);
                            o.biosvisible = attrs.get("biosvisible").map(|v| v != "0").unwrap_or(true);
                            o.cloneofvisible = attrs.get("cloneofvisible").map(|v| v != "0").unwrap_or(true);
                            o.gamevisible = attrs.get("gamevisible").map(|v| v != "0").unwrap_or(true);
                            cur = Some(name);
                        }
                    }
                    b"value" => {
                        in_value = true;
                        value_os = String::new();
                        value_gui = String::new();
                        if let Ok(Some(a)) = e.try_get_attribute("os") {
                            value_os = String::from_utf8_lossy(&a.value).to_string();
                        }
                        if let Ok(Some(a)) = e.try_get_attribute("guivalue") {
                            value_gui =
                                crate::core::listxml::xml_unescape(&String::from_utf8_lossy(&a.value));
                        }
                    }
                    _ => {}
                },
                Ok(quick_xml::events::Event::Text(ref t)) => {
                    if in_value {
                        // quick-xml >=0.41 dropped `unescape()`; `decode()` is
                        // its replacement (resolves the predefined XML entities)
                        if let Ok(s) = t.decode() {
                            text.push_str(&s);
                        }
                    }
                }
                Ok(quick_xml::events::Event::End(ref e)) => {
                    if e.name().as_ref() == b"value" {
                        in_value = false;
                        if let Some(name) = &cur {
                            let os_ok =
                                value_os.is_empty() || value_os == if self.is_sdl_port { "sdl" } else { "win" };
                            if os_ok {
                                if let Some(o) = self.opts.get_mut(name) {
                                    o.values.push(text.trim().to_string());
                                    // empty → the backfill below capitalises the
                                    // canonical value, as the template handler does
                                    o.guivalues.push(value_gui.clone());
                                }
                            }
                        }
                        text.clear();
                    }
                }
                Ok(quick_xml::events::Event::Eof) => break,
                Err(e) => {
                    // a truncated/corrupt template used to stop the parse without
                    // a trace and every option silently fell back to defaults
                    self.warnings.push(format!("optiontemplate.xml: {e}"));
                    break;
                }
                _ => {}
            }
            buf.clear();
        }
        // backfill empty guivalues with capitalized canonical values
        for o in self.opts.values_mut() {
            if o.values.is_empty() {
                continue;
            }
            while o.guivalues.len() < o.values.len() {
                o.guivalues.push(String::new());
            }
            for i in 0..o.values.len() {
                if o.guivalues[i].is_empty() {
                    o.guivalues[i] = capitalize_str(&o.values[i]);
                }
            }
        }
    }

    /// origin: loadIni(optLevel, fileName)
    ///
    /// `active` says the level is on the current game's real ini chain. It only
    /// matters for Horizont/Vertical — exactly one of them is ever in a game's
    /// chain (MAME picks by native orientation), but both files are read so both
    /// editor pages have values. The inactive one fills its level field and
    /// leaves `currvalue` alone; touching it would corrupt the effective value
    /// the running game would see.
    pub fn load_ini(
        &mut self,
        level: usize,
        path: &Path,
        gui: &HashMap<String, String>,
        active: bool,
    ) {
        if level == OPTLEVEL_GUI {
            return;
        }
        // GBK/ANSI ini files (README P2-16): `read_to_string` returned Err on
        // them, the level was silently dropped, and a later save rewrote the
        // user's settings from defaults. Decode it the way Qt4's QTextStream
        // did: honour a BOM, otherwise fall back to the locale codec.
        let ini_settings: HashMap<String, String> = match read_text_file(path) {
            Some(text) => parse_ini_core(&text, false, &mut None),
            None => HashMap::new(),
        };
        let names: Vec<String> = self.opts.keys().cloned().collect();
        for name in names {
            // GUI-overlap: GUI settings override global for the 16 GUI keys.
            //
            // "GUI keys" means the options the template marks `guivisible="1"`
            // (exactly 16 of them — the same set `load_default` files under the
            // "GUI paths" sidebar category). Testing the *name* alone also
            // matched purely-GUI settings that share a name with a core option:
            // `language` is both a GUI setting (`pGuiSettings`, holding e.g.
            // "zh_CN") and a real MAME option (holding e.g. "English"), so the
            // core option was overwritten and later written back into mame.ini
            // as an invalid `language zh_CN`.
            let gui_value = match self.opts.get(&name) {
                Some(o) if o.guivisible => gui.get(&name).cloned(),
                _ => None,
            };
            if let Some(v) = gui_value {
                let o = self.opts.get_mut(&name).unwrap();
                o.globalvalue = v.clone();
                o.currvalue = v.clone();
                continue;
            }
            let has = ini_settings.get(&name).cloned();
            let o = self.opts.get_mut(&name).unwrap();
            match (level, has) {
                (_, Some(v)) if name == "ips" => {
                    o.currvalue = v;
                }
                (_, _) if name == "ips" => {
                    o.currvalue = o.defvalue.clone();
                }
                (OPTLEVEL_GLOBAL, Some(v)) => {
                    o.currvalue = v.clone();
                    o.globalvalue = v;
                }
                (OPTLEVEL_GLOBAL, None) => {
                    o.currvalue = o.defvalue.clone();
                    o.globalvalue = o.defvalue.clone();
                }
                (OPTLEVEL_HORIZONT, Some(v)) => {
                    o.horzvalue = v;
                    if active {
                        o.currvalue = o.horzvalue.clone();
                    }
                }
                (OPTLEVEL_HORIZONT, None) => {
                    o.horzvalue = o.globalvalue.clone();
                    if active {
                        o.currvalue = o.horzvalue.clone();
                    }
                }
                (OPTLEVEL_VERTICAL, Some(v)) => {
                    o.vertvalue = v;
                    if active {
                        o.currvalue = o.vertvalue.clone();
                    }
                }
                (OPTLEVEL_VERTICAL, None) => {
                    o.vertvalue = o.globalvalue.clone();
                    if active {
                        o.currvalue = o.vertvalue.clone();
                    }
                }
                (OPTLEVEL_SRC, Some(v)) => {
                    o.currvalue = v.clone();
                    o.srcvalue = v;
                }
                (OPTLEVEL_SRC, None) => {
                    o.currvalue = o.globalvalue.clone();
                    o.srcvalue = o.globalvalue.clone();
                }
                (OPTLEVEL_BIOS, Some(v)) => {
                    o.currvalue = v.clone();
                    o.biosvalue = v;
                }
                (OPTLEVEL_BIOS, None) => {
                    o.currvalue = o.srcvalue.clone();
                    o.biosvalue = o.srcvalue.clone();
                }
                (OPTLEVEL_CLONEOF, Some(v)) => {
                    o.currvalue = v.clone();
                    o.cloneofvalue = v;
                }
                (OPTLEVEL_CLONEOF, None) => {
                    o.currvalue = o.biosvalue.clone();
                    o.cloneofvalue = o.biosvalue.clone();
                }
                (OPTLEVEL_CURR, Some(v)) => {
                    o.currvalue = v;
                }
                (OPTLEVEL_CURR, None) => {
                    o.currvalue = o.cloneofvalue.clone();
                }
                _ => {}
            }
        }
    }

    /// global ini only (boot phase, before any game is selected)
    pub fn load_global(&mut self, gui: &HashMap<String, String>) {
        let f = self
            .mame_ini_path
            .join(if self.mess_like { "mess.ini" } else { "mame.ini" });
        self.load_ini(OPTLEVEL_GLOBAL, &f, gui, true);
    }

/// resolve a ';'-separated dir option against base_dir
    pub fn resolve_dir_list(&self, value: &str) -> Vec<PathBuf> {
        value
            .split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                let p = PathBuf::from(s);
                if p.is_relative() {
                    self.base_dir.join(p)
                } else {
                    p
                }
            })
            .collect()
    }

    /// level → ini file path (origin: chainLoadOptions mapping)
    pub fn ini_file_for(&self, level: usize, meta: &GameMeta, lib: &GameLibrary) -> PathBuf {
        match level {
            OPTLEVEL_GLOBAL => self
                .mame_ini_path
                .join(if self.is_mess_like() { "mess.ini" } else { "mame.ini" }),
            OPTLEVEL_HORIZONT => self.mame_ini_path.join("ini").join("horizont.ini"),
            OPTLEVEL_VERTICAL => self.mame_ini_path.join("ini").join("vertical.ini"),
            OPTLEVEL_SRC => {
                // origin: mameopt.cpp:815-817 — `iniFileName = sourcefile;
                // iniFileName.replace(".c", INI_EXT); mameIniPath + "ini/source/" + ...`.
                // Two corrections against what MAME itself reads
                // (origin: mame-0.168 emuopts.cpp::parse_standard_inis):
                //   * the lookup is `source/<base>.ini` where <base> is
                //     `core_filename_extract_base(source_file, strip_extension=true)`
                //     — i.e. the *file name only* (directory dropped) with the
                //     extension chopped at the last dot. The old code kept the
                //     directory and replaced ".c" globally, so a modern
                //     "sega/naomi.cpp" became "sega/naomi.inip".
                //   * it lives under `<inidir>/source/`, not `<mame>/source/`.
                let file = meta.sourcefile.rsplit(['/', '\\']).next().unwrap_or("");
                let stem = match file.rsplit_once('.') {
                    Some((s, _)) if !s.is_empty() => s,
                    _ => file,
                };
                self.mame_ini_path
                    .join("ini")
                    .join("source")
                    .join(format!("{stem}.ini"))
            }
            OPTLEVEL_BIOS => {
                let b = meta.bios_of(lib);
                if b.is_empty() {
                    PathBuf::new()
                } else {
                    self.mame_ini_path.join("ini").join(format!("{b}.ini"))
                }
            }
            OPTLEVEL_CLONEOF => {
                if meta.cloneof.is_empty() {
                    PathBuf::new()
                } else {
                    self.mame_ini_path.join("ini").join(format!("{}.ini", meta.cloneof))
                }
            }
            _ => {
                let n = if meta.is_ext_rom { meta.romof.clone() } else { meta.name.clone() };
                self.mame_ini_path.join("ini").join(format!("{n}.ini"))
            }
        }
    }

    fn is_mess_like(&self) -> bool {
        self.mess_like
    }

    /// full chain load (origin: chainLoadOptions).
    ///
    /// Always runs to the end: `currvalue` must be the value the game would
    /// actually see, not the value at whichever tab the dialog happens to show.
    /// Both orientation files are read (see `load_ini`'s `active`), one of them
    /// feeding `currvalue` per the game's native orientation.
    pub fn chain_load(&mut self, meta: &GameMeta, lib: &GameLibrary, gui: &HashMap<String, String>) {
        self.load_ini(OPTLEVEL_GLOBAL, &self.ini_file_for(OPTLEVEL_GLOBAL, meta, lib), gui, true);
        let horz = meta.is_horz;
        self.load_ini(
            OPTLEVEL_HORIZONT,
            &self.ini_file_for(OPTLEVEL_HORIZONT, meta, lib),
            gui,
            horz,
        );
        self.load_ini(
            OPTLEVEL_VERTICAL,
            &self.ini_file_for(OPTLEVEL_VERTICAL, meta, lib),
            gui,
            !horz,
        );
        self.load_ini(OPTLEVEL_SRC, &self.ini_file_for(OPTLEVEL_SRC, meta, lib), gui, true);
        let bios_file = self.ini_file_for(OPTLEVEL_BIOS, meta, lib);
        if !bios_file.as_os_str().is_empty() {
            self.load_ini(OPTLEVEL_BIOS, &bios_file, gui, true);
        }
        let clone_file = self.ini_file_for(OPTLEVEL_CLONEOF, meta, lib);
        if !clone_file.as_os_str().is_empty() {
            self.load_ini(OPTLEVEL_CLONEOF, &clone_file, gui, true);
        }
        self.load_ini(OPTLEVEL_CURR, &self.ini_file_for(OPTLEVEL_CURR, meta, lib), gui, true);
    }

    // ---- value conversion (origin: getLongValue/getShortValue) ----

    pub fn get_long_value(&self, name: &str, val: &str) -> String {
        let Some(o) = self.opts.get(name) else { return val.to_string() };
        match o.kind.unwrap_or(OptKind::Unknown) {
            OptKind::Bool => {
                if val == "0" { "false".into() } else { "true".into() }
            }
            OptKind::Float => val.parse::<f64>().map(|f| format!("{f:.2}")).unwrap_or_else(|_| val.into()),
            OptKind::Str | OptKind::StrEditable => {
                match o.values.iter().position(|v| v == val) {
                    Some(i) => o.guivalues.get(i).cloned().unwrap_or_else(|| val.into()),
                    None => val.to_string(),
                }
            }
            _ => val.to_string(),
        }
    }

    pub fn get_short_value(&self, name: &str, val: &str) -> String {
        let Some(o) = self.opts.get(name) else { return val.to_string() };
        match o.kind.unwrap_or(OptKind::Unknown) {
            OptKind::Bool => {
                if val == "true" { "1".into() } else { "0".into() }
            }
            OptKind::Str | OptKind::StrEditable => {
                match o.guivalues.iter().position(|v| v == val) {
                    Some(i) => o.values.get(i).cloned().unwrap_or_else(|| val.into()),
                    None => val.to_string(),
                }
            }
            _ => val.to_string(),
        }
    }

    pub fn long_name(&self, name: &str) -> String {
        match self.opts.get(name) {
            Some(o) if !o.guiname.is_empty() => o.guiname.clone(),
            _ => capitalize_str(name),
        }
    }

    /// effective display value at a level (currvalue after chain load)
    pub fn display_value(&self, name: &str) -> String {
        match self.opts.get(name) {
            Some(o) => self.get_long_value(name, &o.currvalue),
            None => String::new(),
        }
    }

    /// origin: saveIniFile(optLevel, iniFileName) — rewrites through defaultIni template
    pub fn save_ini_file(&mut self, level: usize, path: &Path, default_ini: &str) -> std::io::Result<()> {
        if path.as_os_str().is_empty() {
            return Ok(());
        }
        let mut out = String::new();
        let mut headers: Vec<String> = Vec::new();
        for raw in default_ini.lines() {
            let line = raw;
            if line.starts_with('<') {
                continue;
            }
            if line.starts_with('#') {
                headers.push(line.to_string());
                continue;
            }
            if line.trim().is_empty() {
                out.push('\n');
                continue;
            }
            let sp = line.find(char::is_whitespace).unwrap_or(line.len());
            let opt_name = &line[..sp];
            let Some(o) = self.opts.get(opt_name) else {
                continue;
            };
            let (curr, def) = match level {
                OPTLEVEL_GUI | OPTLEVEL_GLOBAL => {
                    let mut c = o.globalvalue.clone();
                    if opt_name == "bios" {
                        c = o.defvalue.clone();
                    }
                    (c, o.defvalue.clone())
                }
                OPTLEVEL_SRC => {
                    let mut c = o.srcvalue.clone();
                    if opt_name == "bios" {
                        c = o.defvalue.clone();
                    }
                    (c, o.globalvalue.clone())
                }
                OPTLEVEL_HORIZONT => {
                    let mut c = o.horzvalue.clone();
                    if opt_name == "bios" {
                        c = o.defvalue.clone();
                    }
                    (c, o.globalvalue.clone())
                }
                OPTLEVEL_VERTICAL => {
                    let mut c = o.vertvalue.clone();
                    if opt_name == "bios" {
                        c = o.defvalue.clone();
                    }
                    (c, o.globalvalue.clone())
                }
                OPTLEVEL_BIOS => (o.biosvalue.clone(), o.srcvalue.clone()),
                OPTLEVEL_CLONEOF => (o.cloneofvalue.clone(), o.biosvalue.clone()),
                _ => {
                    if opt_name == "ips" {
                        (o.currvalue.clone(), o.defvalue.clone())
                    } else {
                        (o.currvalue.clone(), o.cloneofvalue.clone())
                    }
                }
            };
            let cur_l = self.get_long_value(opt_name, &curr);
            let def_l = self.get_long_value(opt_name, &def);
            let is_changed = cur_l != def_l;
            if is_changed || level == OPTLEVEL_GLOBAL || level == OPTLEVEL_GUI {
                for h in headers.drain(..) {
                    out.push_str(&h);
                    out.push('\n');
                }
                let padded = format!("{opt_name:<26}");
                let value = if cur_l.chars().any(|c| c.is_whitespace())
                    && !(cur_l.starts_with('"') && cur_l.ends_with('"'))
                {
                    format!("\"{cur_l}\"")
                } else {
                    cur_l
                };
                out.push_str(&padded);
                out.push_str(&value);
                out.push('\n');
            }
        }

        // postprocess (origin statics replicated as self state)
        let mut lines: Vec<String> = out
            .split(|c| c == '\r' || c == '\n')
            .map(|s| s.to_string())
            .collect();
        lines.retain(|l| !l.starts_with("language"));
        // reverse pass: eat orphaned header blocks (each header max 3 '#' lines)
        let mut c = self.save_c;
        let mut idx = lines.len();
        while idx > 0 {
            idx -= 1;
            let l = &lines[idx];
            if l.starts_with('#') {
                c -= 1;
            } else if !l.is_empty() {
                c = 3;
            }
            if c < 0 {
                lines.remove(idx);
            }
        }
        self.save_c = c;
        while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            lines.pop();
        }
        let mut out2 = String::new();
        let mut is_entry = self.save_is_entry;
        for (i, l) in lines.iter().enumerate() {
            if l.starts_with('#') {
                if is_entry && i > 3 {
                    out2.push('\n');
                }
                is_entry = false;
            } else {
                is_entry = true;
            }
            if !l.contains("readconfig") {
                out2.push_str(l);
                out2.push('\n');
            }
        }
        self.save_is_entry = is_entry;

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if out2.trim().is_empty() {
            let _ = std::fs::remove_file(path);
            return Ok(());
        }
        // BOM (mame.ini always uses a BOM)
        std::fs::write(path, format!("\u{feff}{out2}"))
    }

    /// dynamic enums (origin: updateSelectableItems) — bios/ctrlr/effect
    pub fn update_selectable_items(&mut self, name: &str, meta: &GameMeta, lib: &GameLibrary) {
        // pre-read directory inputs before taking the mutable borrow
        let dir_input: String = match name {
            "ctrlr" => self
                .opts
                .get("ctrlrpath")
                .map(|p| p.currvalue.clone())
                .unwrap_or_default(),
            "effect" => self
                .opts
                .get("artpath")
                .map(|p| p.currvalue.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        // a relative `ctrlrpath` / `artpath` used to be read against the process
        // working directory instead of the mame.exe one, so the list came up empty
        // whenever the two differed (README P3)
        let dir_input = {
            let dirs = self.resolve_dir_list(&dir_input);
            dirs.first()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        let Some(o) = self.opts.get_mut(name) else { return };
        o.values.clear();
        o.guivalues.clear();
        match name {
            "bios" => {
                let biosof = meta.bios_of(lib);
                let ref_name = if biosof.is_empty() { &meta.name } else { &biosof };
                if let Some(g) = lib.get(ref_name) {
                    let mut sets: Vec<(String, String)> = g
                        .bios_sets
                        .iter()
                        .map(|b| (b.name.clone(), b.description.clone()))
                        .collect();
                    sets.sort();
                    for (n, d) in sets {
                        o.values.push(n);
                        o.guivalues.push(d);
                    }
                }
            }
            "ctrlr" => {
                let dir = dir_input;
                let mut files: Vec<String> = std::fs::read_dir(dir)
                    .map(|rd| {
                        rd.flatten()
                            .filter(|e| {
                                // Windows file names are case-insensitive: .CFG counts too
                                e.path()
                                    .extension()
                                    .map(|x| x.eq_ignore_ascii_case("cfg"))
                                    .unwrap_or(false)
                            })
                            .filter_map(|e| {
                                e.path()
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort();
                for f in files {
                    o.values.push(f.clone());
                    o.guivalues.push(f);
                }
            }
            "effect" => {
                o.values.push("none".into());
                o.guivalues.push("None".into());
                let dir = dir_input;
                let mut files: Vec<String> = std::fs::read_dir(dir)
                    .map(|rd| {
                        rd.flatten()
                            .filter(|e| {
                                e.path()
                                    .extension()
                                    .map(|x| x.eq_ignore_ascii_case("png"))
                                    .unwrap_or(false)
                            })
                            .filter_map(|e| {
                                e.path()
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                files.sort();
                for f in files {
                    o.values.push(f.clone());
                    o.guivalues.push(f);
                }
            }
            _ => {}
        }
        while o.guivalues.len() < o.values.len() {
            o.guivalues.push(capitalize_str(&o.values[o.guivalues.len()]));
        }
    }
}

pub fn capitalize_str(s: &str) -> String {
    s.split('_')
        .enumerate()
        .map(|(i, seg)| {
            if i == 0 {
                let mut c = seg.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => String::new(),
                }
            } else {
                seg.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// utils->getPath: $HOME expansion + clean path + trailing '/'
pub fn clean_dir_path(p: &str) -> PathBuf {
    let p = if p == "$HOME" || p.starts_with("$HOME/") {
        if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
            p.replacen("$HOME", &home, 1)
        } else {
            p.to_string()
        }
    } else {
        p.to_string()
    };
    let pb = PathBuf::from(p.trim());
    pb.components().collect::<PathBuf>()
}

/// normalize to a dir string with trailing separator (getPath semantics)
pub fn dir_string(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    if s.ends_with('/') || s.ends_with('\\') {
        s
    } else {
        format!("{s}/")
    }
}

/// utils->getSinglePath: first dir where dir/fileName opens
pub fn single_path(dir_paths: &str, file_name: &str) -> Option<PathBuf> {
    for d in dir_paths.split(';') {
        if d.trim().is_empty() {
            continue;
        }
        let p = clean_dir_path(d).join(file_name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::library::GameLibrary;
    use crate::core::model::GameMeta;

    fn core_at(mame_ini_path: &str) -> OptionCore {
        let mut core = OptionCore::default();
        core.mame_ini_path = PathBuf::from(mame_ini_path);
        core
    }

    /// `ini_file_for` is what both the loader *and* the writer have to agree on:
    /// the writer used to hand-build a different path, so every source-level
    /// edit was written somewhere the loader never looked (README 4.4).
    #[test]
    fn source_ini_is_the_stem_under_ini_source() {
        let lib = GameLibrary::new("0.261".into());
        let core = core_at("/mame");
        let meta = GameMeta {
            name: "pacman".into(),
            sourcefile: "pacman.cpp".into(),
            ..Default::default()
        };
        let p = core.ini_file_for(OPTLEVEL_SRC, &meta, &lib);
        assert_eq!(p, PathBuf::from("/mame/ini/source/pacman.ini"));

        // MAME's own lookup keeps only the file name of a nested driver path
        let meta = GameMeta {
            sourcefile: "sega/naomi.cpp".into(),
            ..Default::default()
        };
        let p = core.ini_file_for(OPTLEVEL_SRC, &meta, &lib);
        assert_eq!(p, PathBuf::from("/mame/ini/source/naomi.ini"));
    }

    /// the game/bios/cloneof levels live flat under `<inidir>/ini/`
    #[test]
    fn per_machine_ini_paths() {        let lib = GameLibrary::new("0.261".into());
        let core = core_at("/mame");
        let meta = GameMeta {
            name: "pacman".into(),
            cloneof: "puckman".into(),
            romof: "puckman".into(),
            ..Default::default()
        };
        assert_eq!(
            core.ini_file_for(OPTLEVEL_CLONEOF, &meta, &lib),
            PathBuf::from("/mame/ini/puckman.ini")
        );
        assert_eq!(
            core.ini_file_for(OPTLEVEL_CURR, &meta, &lib),
            PathBuf::from("/mame/ini/pacman.ini")
        );
        // a softlist entry is filed under its parent machine
        let mut ext = meta.clone();
        ext.is_ext_rom = true;
        ext.name = "dir/soft.bin".into();
        assert_eq!(
            core.ini_file_for(OPTLEVEL_CURR, &ext, &lib),
            PathBuf::from("/mame/ini/puckman.ini")
        );
        // no bios parent → nothing to save/read for that level
        assert_eq!(core.ini_file_for(OPTLEVEL_BIOS, &meta, &lib), PathBuf::new());
    }

    /// `pGuiSettings` owns the 14 `guivisible` path options and nothing else.
    /// Matching on the *name* let the GUI's own `language` setting ("zh_CN")
    /// replace MAME's core `language` option ("English"), which was then written
    /// back into mame.ini as an invalid value on the next save.
    #[test]
    fn gui_settings_only_override_gui_visible_options() {
        let mut core = core_at("/mame");
        let mut gui_opt = MameOption::new("icons");
        gui_opt.guivisible = true;
        core.opts.insert("icons_directory".into(), gui_opt);
        core.opts.insert("language".into(), MameOption::new("English"));

        let gui: HashMap<String, String> = [
            ("icons_directory".to_string(), "D:/icons".to_string()),
            ("language".to_string(), "zh_CN".to_string()),
        ]
        .into_iter()
        .collect();

        // no ini on disk: every level falls back to its own default/parent
        core.load_ini(OPTLEVEL_GLOBAL, &Path::new("/does/not/exist.ini"), &gui, true);

        // GUI-owned → the GUI setting wins
        assert_eq!(core.opts["icons_directory"].globalvalue, "D:/icons");
        // core option with a GUI namesake → left alone
        assert_eq!(core.opts["language"].globalvalue, "English");
        assert_eq!(core.opts["language"].currvalue, "English");
    }

    /// 模板里的 `default=` 不是摆设：GUI 表和 ini 都没配时，加载器把
    /// defvalue 写进 globalvalue，`content_setting` 再把它当配置值解析；
    /// 「设置 ▸ 目录」对话框的兜底值也直接读模板（`template_default`）。
    /// 两边一漂移，内容查找就指向一个没人创建的位置——history_file 默认
    /// 还是 `history.dat` 时，History 面板在 `<exe>/history.dat` 找一个
    /// 不存在的文件，而真实默认是 `<exe>/dats/history.xml`。
    /// 这里把模板默认钉在 `core::paths` 的内置默认上（运行期兜底仍是
    /// paths，因为启动早期 opts 还没加载，两边必须相等）。
    #[test]
    fn template_defaults_match_paths_module() {
        let mut core = core_at("/mame");
        core.load_template(&HashMap::new());
        let d = |k: &str| core.opts.get(k).map(|o| o.defvalue.clone()).unwrap_or_default();

        for (key, dir) in crate::core::paths::IMAGE_DIRS {
            if key == "snapshot_directory" {
                // MAME 核心选项：不是 guivisible，空 core 里不会插入，
                // 改在下面的 template_default 一侧断言
                continue;
            }
            assert_eq!(d(key), dir, "{key} 默认值必须镜像 IMAGE_DIRS");
        }
        for (key, file) in crate::core::paths::DAT_FILES {
            assert_eq!(
                d(key),
                format!("{}/{}", crate::core::paths::DAT_SUBDIR, file),
                "{key} 默认值必须指向 dats/ 下的文件"
            );
        }
        assert_eq!(d("background_directory"), crate::core::paths::BG_SUBDIR);
        assert_eq!(d("folder_directory"), crate::core::paths::FOLDERS_SUBDIR);
        assert_eq!(d("icons_directory"), "icons");
        assert_eq!(d("m1_directory"), "bin/m1");
        assert_eq!(d("localized_list_file"), crate::core::paths::LST_FILE);
        // mame_binary 故意没有默认值：它由用户首次启动时选择，回退名
        // （mamep.exe）只存在于启动校验里，不该进模板
        assert_eq!(d("mame_binary"), "");

        // 对话框一侧：直接解析嵌入模板，不依赖加载好的 core。
        // snapshot_directory 在这里补上（MAME 自己的默认就是 snap）。
        assert_eq!(template_default("snapshot_directory"), "snap");
        assert_eq!(template_default("command_file"), "dats/command.dat");
        assert_eq!(template_default("localized_list_file"), "mame_cn.lst");
        assert_eq!(template_default("mame_binary"), "");
        assert_eq!(template_default("rompath"), "");
    }

    /// HORIZONT/VERTICAL 层对应 MAME 加载链里的 ini/horizont.ini 与
    /// ini/vertical.ini：两个固定层级，文件名不随游戏方向变
    /// （origin: parse_standard_inis —— 读哪个由 MAME 按驱动原生方向定，
    /// 这里两层都读、都写）。
    #[test]
    fn orientation_levels_map_to_fixed_files() {
        let lib = GameLibrary::new("0.261".into());
        let core = core_at("/mame");
        let meta = GameMeta {
            name: "pacman".into(),
            is_horz: true,
            ..Default::default()
        };
        assert_eq!(
            core.ini_file_for(OPTLEVEL_HORIZONT, &meta, &lib),
            PathBuf::from("/mame/ini/horizont.ini")
        );
        assert_eq!(
            core.ini_file_for(OPTLEVEL_VERTICAL, &meta, &lib),
            PathBuf::from("/mame/ini/vertical.ini")
        );
    }

    /// 两个方向层都被读取（各进各的字段），但只有当前游戏方向上的那层
    /// 喂 currvalue —— 竖屏游戏选中时，横屏层只是一次纯文件读入。
    #[test]
    fn orientation_load_feeds_currvalue_only_when_active() {
        let dir = std::env::temp_dir().join(format!("mvui_orient_chain_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ini")).unwrap();
        let horz_ini = dir.join("ini").join("horizont.ini");
        let vert_ini = dir.join("ini").join("vertical.ini");
        std::fs::write(&horz_ini, "autofire 1\n").unwrap();
        std::fs::write(&vert_ini, "autofire 2\n").unwrap();

        let mut core = core_at(&dir.to_string_lossy());
        core.opts.insert("autofire".into(), MameOption::new("0"));
        core.opts.insert("cheat".into(), MameOption::new("0"));
        core.load_ini(
            OPTLEVEL_GLOBAL,
            &Path::new("/does/not/exist.ini"),
            &HashMap::new(),
            true,
        );

        // 竖屏游戏：vertical 活跃，horizontal 只是填字段
        core.load_ini(
            OPTLEVEL_HORIZONT,
            &horz_ini,
            &HashMap::new(),
            false,
        );
        core.load_ini(OPTLEVEL_VERTICAL, &vert_ini, &HashMap::new(), true);
        {
            let af = &core.opts["autofire"];
            assert_eq!(af.horzvalue, "1");
            assert_eq!(af.vertvalue, "2");
            assert_eq!(af.currvalue, "2");
            assert_eq!(af.globalvalue, "0");
        }
        // 横屏游戏：反过来
        core.opts.insert("autofire".into(), MameOption::new("0"));
        core.load_ini(
            OPTLEVEL_GLOBAL,
            &Path::new("/does/not/exist.ini"),
            &HashMap::new(),
            true,
        );
        core.load_ini(OPTLEVEL_HORIZONT, &horz_ini, &HashMap::new(), true);
        core.load_ini(OPTLEVEL_VERTICAL, &vert_ini, &HashMap::new(), false);
        let af = &core.opts["autofire"];
        assert_eq!(af.horzvalue, "1");
        assert_eq!(af.vertvalue, "2");
        assert_eq!(af.currvalue, "1");

        let _ = std::fs::remove_file(&horz_ini);
        let _ = std::fs::remove_file(&vert_ini);
        let _ = std::fs::remove_dir(&dir);
    }

    /// chain_load 一路读到底，两个方向层各归各位；之后两层的字段都能
    /// 拿到文件里的值（编辑页靠层级字段显示，不靠 currvalue）。
    #[test]
    fn chain_load_fills_both_orientation_fields() {
        let dir = std::env::temp_dir().join(format!("mvui_orient_chain2_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("ini")).unwrap();
        let horz_ini = dir.join("ini").join("horizont.ini");
        let vert_ini = dir.join("ini").join("vertical.ini");
        std::fs::write(&horz_ini, "autofire 1\n").unwrap();
        std::fs::write(&vert_ini, "autofire 2\n").unwrap();

        let lib = GameLibrary::new("0.261".into());
        let mut core = core_at(&dir.to_string_lossy());
        core.opts.insert("autofire".into(), MameOption::new("0"));
        core.opts.insert("cheat".into(), MameOption::new("0"));

        let meta = GameMeta {
            name: "pacman".into(),
            is_horz: true,
            ..Default::default()
        };
        core.chain_load(&meta, &lib, &HashMap::new());

        let af = &core.opts["autofire"];
        assert_eq!(af.horzvalue, "1");
        assert_eq!(af.vertvalue, "2");
        assert_eq!(af.globalvalue, "0");

        let _ = std::fs::remove_file(&horz_ini);
        let _ = std::fs::remove_file(&vert_ini);
        let _ = std::fs::remove_dir(&dir);
    }

    /// 方向层保存与 source 层同规则：只写和 globalvalue 不同的项；
    /// 全部回到全局值时 diff 清空，文件直接删掉不留空壳。
    #[test]
    fn orientation_save_writes_only_diffs_from_global() {
        let dir = std::env::temp_dir().join(format!("mvui_orient_save_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ini = dir.join("vertical.ini");

        let mut core = core_at("/mame");
        core.opts.insert("autofire".into(), MameOption::new("0"));
        core.opts.insert("cheat".into(), MameOption::new("0"));
        core.load_ini(
            OPTLEVEL_GLOBAL,
            &Path::new("/does/not/exist.ini"),
            &HashMap::new(),
            true,
        );
        // 运行期保存前 ensure_chain 一定把两个方向层都读过（文件不存在时
        // 字段回填 globalvalue），这里照做，不然未设置项会以空值入档
        core.load_ini(
            OPTLEVEL_HORIZONT,
            &Path::new("/does/not/exist.ini"),
            &HashMap::new(),
            false,
        );
        core.load_ini(
            OPTLEVEL_VERTICAL,
            &Path::new("/does/not/exist.ini"),
            &HashMap::new(),
            true,
        );
        {
            let af = core.opts.get_mut("autofire").unwrap();
            af.vertvalue = "1".into();
            af.currvalue = "1".into();
        }
        let default_ini = "#\n# CORE CONFIGURATION\n#\nautofire 0\ncheat 0\n";
        core.save_ini_file(OPTLEVEL_VERTICAL, &ini, default_ini).unwrap();
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.lines().any(|l| l.split_whitespace().eq(["autofire", "1"])));
        assert!(!text.lines().any(|l| l.starts_with("cheat")));

        let af = core.opts.get_mut("autofire").unwrap();
        af.vertvalue = "0".into();
        af.currvalue = "0".into();
        core.save_ini_file(OPTLEVEL_VERTICAL, &ini, default_ini).unwrap();
        assert!(!ini.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

