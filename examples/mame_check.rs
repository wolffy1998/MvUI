//! Diagnostic: reproduce MvUI's startup MAME-binary validation.
//!
//! Usage: cargo run --release --example mame_check -- [mame.exe]
//!
//! Mirrors `app.rs` start-up: read `mame_binary` from `.mamepgui/mamepgui.ini`,
//! run the same version detection, and report whether `try_accept_mame` would
//! accept it. Use this instead of guessing why the app opened the picker — the
//! three failure modes (key missing, version not detected, path == self) print
//! differently here.

use std::path::PathBuf;

fn main() {
    let ini = mvui::core::settings::GuiSettings::cfg_prefix().join("mamepgui.ini");
    println!("ini: {}", ini.display());

    let gui = mvui::core::settings::GuiSettings::load();
    let from_ini = gui.get("mame_binary");
    println!("mame_binary in ini: {from_ini:?}");

    let path = match (std::env::args().nth(1), from_ini) {
        (Some(a), _) => a,
        (None, Some(v)) => v.to_string(),
        // the same legacy fallback app.rs uses
        (None, None) => "mamep.exe".to_string(),
    };
    println!("resolved path:     {path:?}");

    let version = match mvui::core::mameproc::MameBinary::detect(std::path::Path::new(&path)) {
        Ok(v) => v.version,
        Err(e) => {
            println!("detect FAILED: {e}");
            String::new()
        }
    };
    println!("detected version:  {version:?}");

    let self_exe = std::env::current_exe().ok();
    let is_self = self_exe
        .as_ref()
        .map(|e| e.canonicalize().ok() == PathBuf::from(&path).canonicalize().ok())
        .unwrap_or(false);
    let empty_version = version.is_empty();
    let empty_path = path.is_empty();

    println!("path empty:        {empty_path}");
    println!("version empty:     {empty_version}");
    println!("path == self exe:  {is_self}");
    println!(
        "=> try_accept_mame would {}",
        if empty_path || empty_version || is_self {
            "REJECT (picker opens)"
        } else {
            "ACCEPT"
        }
    );
}
