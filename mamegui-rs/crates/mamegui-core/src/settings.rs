//! QSettings-equivalent GUI settings store (origin: pGuiSettings, IniFormat).
//! Path: CFG_PREFIX + "mamepgui.ini"; CFG_PREFIX default ".mamepgui/" under the
//! exe dir (portable), overridable by the `-configpath <dir>` argument.

use std::collections::BTreeMap;
use std::path::PathBuf;

pub struct GuiSettings {
    pub path: PathBuf,
    pub map: BTreeMap<String, String>,
}

impl GuiSettings {
    /// main(): CFG_PREFIX resolution + `-configpath` handling
    pub fn cfg_prefix() -> PathBuf {
        let mut prefix: Option<String> = None;
        let args: Vec<String> = std::env::args().collect();
        for (i, a) in args.iter().enumerate() {
            if a == "-configpath" && i + 1 < args.len() {
                prefix = Some(args[i + 1].clone());
                break;
            }
        }
        let base = match prefix {
            Some(p) => PathBuf::from(p),
            None => {
                let exe = std::env::current_exe()
                    .ok()
                    .and_then(|e| e.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_default();
                exe.join(".mamepgui")
            }
        };
        let _ = std::fs::create_dir_all(&base);
        base
    }

    pub fn load() -> Self {
        let dir = Self::cfg_prefix();
        let path = dir.join("mamepgui.ini");
        let map = crate::options::read_text_file(&path)
            .map(|text| {
                let mut m = BTreeMap::new();
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() || line.starts_with('[') || line.starts_with(';') {
                        continue;
                    }
                    if let Some(eq) = line.find('=') {
                        m.insert(line[..eq].trim().to_string(), line[eq + 1..].trim().to_string());
                    }
                }
                m
            })
            .unwrap_or_default();
        Self { path, map }
    }

    /// Returns the write error instead of swallowing it: a failed save used to
    /// lose every setting silently (README P3).
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::from("[General]\n");
        for (k, v) in &self.map {
            out.push_str(&format!("{k}={v}\n"));
        }
        std::fs::write(&self.path, out)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(|s| s.as_str())
    }

    pub fn get_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).unwrap_or(default)
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        self.map.insert(key.to_string(), value.into());
    }

    pub fn set_bool(&mut self, key: &str, v: bool) {
        self.set(key, if v { "1" } else { "0" });
    }

    pub fn get_bool(&self, key: &str) -> bool {
        self.get(key).map(|v| v == "1" || v == "true").unwrap_or(false)
    }

    pub fn remove(&mut self, key: &str) {
        self.map.remove(key);
    }
}

/// old toml app settings retained for the cache dir helper
impl GuiSettings {
    pub fn cache_dir() -> PathBuf {
        let p = GuiSettings::cfg_prefix().join("cache");
        let _ = std::fs::create_dir_all(&p);
        p
    }
}
