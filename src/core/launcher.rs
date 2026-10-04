//! Launch argument assembly, 1:1 port of Gamelist::runMame.

use crate::core::model::GameMeta;
pub use self::mode::RunMode;

mod mode {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RunMode {
        Normal,
        ExtRom,
        Cmd,
    }
}

pub fn savestate_args(state_file: &str) -> Vec<String> {
    if state_file.is_empty() {
        return vec![];
    }
    vec!["-state".into(), state_file.into()]
}

pub fn playback_args(inp_dir: &str, file_name: &str) -> Vec<String> {
    vec![
        "-input_directory".into(),
        inp_dir.into(),
        "-playback".into(),
        file_name.into(),
        "-nvram_directory".into(),
        std::env::temp_dir().to_string_lossy().to_string(),
    ]
}

pub fn record_args(inp_dir: &str, file_name: &str) -> Vec<String> {
    vec![
        "-input_directory".into(),
        inp_dir.into(),
        "-record".into(),
        file_name.into(),
    ]
}

pub fn mng_args(snap_dir: &str, file_name: &str) -> Vec<String> {
    vec![
        "-snapshot_directory".into(),
        snap_dir.into(),
        "-mngwrite".into(),
        file_name.into(),
    ]
}

pub fn avi_args(snap_dir: &str, file_name: &str) -> Vec<String> {
    vec![
        "-snapshot_directory".into(),
        snap_dir.into(),
        "-aviwrite".into(),
        file_name.into(),
    ]
}

pub fn wave_args(full_path: &str) -> Vec<String> {
    vec!["-wavwrite".into(), full_path.into()]
}

#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub args: Vec<String>,
    pub warnings: Vec<String>,
}

/// build the final command line (origin runMame steps 2-6)
pub fn build_args(
    mode: RunMode,
    meta: &GameMeta,
    cmd_diff: &[(String, String, bool)], // (name, value, is_bool)
    play_args: &[String],
    temp_rom: Option<&std::path::Path>,
) -> LaunchSpec {
    let mut args: Vec<String> = play_args.to_vec();
    let mut warnings = Vec::new();

    if meta.devices.is_empty() {
        args.push(meta.name.clone());
    } else {
        // MESS: temp rom extraction handled by the caller before this
        let system = if meta.is_ext_rom { meta.romof.clone() } else { meta.name.clone() };
        args.push(system);
        let mut first_mount = true;
        for d in &meta.devices {
            if !d.mounted_path.is_empty() {
                args.push(format!("-{}", d.instance));
                if first_mount && mode == RunMode::ExtRom {
                    if let Some(tr) = temp_rom {
                        args.push(tr.to_string_lossy().to_string());
                        first_mount = false;
                        continue;
                    }
                }
                args.push(d.mounted_path.clone());
                first_mount = false;
            } else if d.mandatory {
                warnings.push(d.instance.clone());
            }
        }
    }

    if mode == RunMode::Cmd {
        for (name, value, is_bool) in cmd_diff {
            if name.ends_with("_extra_software") || name == "langpath" || name == "language" {
                continue;
            }
            if *is_bool {
                if value == "0" {
                    args.push(format!("-no{name}"));
                } else {
                    args.push(format!("-{name}"));
                }
            } else {
                args.push(format!("-{name}"));
                args.push(value.clone());
            }
        }
    }

    LaunchSpec { args, warnings }
}

/// command-line-mode diff (origin: defvalue != currvalue, excluding GUI keys)
pub fn cmd_diff(core: &crate::core::options::OptionCore, gui_keys: &std::collections::HashSet<String>) -> Vec<(String, String, bool)> {
    use crate::core::options::OptKind;
    let mut out = Vec::new();
    for (name, o) in &core.opts {
        if o.defvalue != o.currvalue
            && !name.ends_with("_extra_software")
            && !gui_keys.contains(name.as_str())
            && name != "langpath"
            && name != "language"
        {
            let is_bool = o.kind == Some(OptKind::Bool);
            out.push((name.clone(), o.currvalue.clone(), is_bool));
        }
    }
    out.sort();
    out
}
