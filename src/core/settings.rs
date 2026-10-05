//! GUI 设置存储，等价旧版的 QSettings（origin: pGuiSettings, IniFormat）。
//!
//! 路径：CFG_PREFIX + `mvui.ini`；CFG_PREFIX 默认取 exe 目录下的
//! `.mvui/`（便携安装），可用 `-configpath <dir>` 命令行参数覆盖。
//!
//! 改名说明：目录与文件名随程序名从 `.the original GUI/` + `the original GUI ini`
//! 换成 `.mvui/` + `mvui.ini`，**不做迁移**——旧配置不再生效，
//! 用户需要重新配置一次。旧的 `.the original GUI` 目录不会被自动删除，
//! 残留在磁盘上，可手动清理。

use crate::dlog;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// `cfg_prefix()` 的结果，进程内只算一次。
static CFG_PREFIX: OnceLock<PathBuf> = OnceLock::new();

pub struct GuiSettings {
    pub path: PathBuf,
    pub map: BTreeMap<String, String>,
}

impl GuiSettings {
    /// 存放 `mvui.exe` 的目录。
    ///
    /// 工程拍平之后，这里成了所有**内容目录**的锚点：`snap/`、
    /// `flyers/`、`dats/`、`folders/`、`bkground/` 和 `mame_cn.lst`
    /// 默认都住在程序旁边，除非用户另行指定。
    ///
    /// 这**故意不是** mame 目录了——rom 集和 artwork 是两回事，
    /// 便携安装不该要求对 MAME 目录树的写权限。
    pub fn exe_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// 配置根目录。对应旧版 main() 里的 CFG_PREFIX 解析 +
    /// `-configpath` 处理。
    ///
    /// 优先级：`-configpath <dir>` 参数 > exe 目录下的 `.mvui/`。
    ///
    /// 结果缓存：`perf_log` 每写一条日志都会问一次 `cache_dir()` → 这里，
    /// 不缓存就是每条日志一遍参数扫描 + 一遍 `create_dir_all`。
    ///
    /// **这个函数里不许有 `dlog!`**——它是日志落点自己的上游：
    /// `dlog!` → `perf_log` → `cache_dir()` → 这里 → `dlog!`，
    /// 无限递归直接栈溢出（Windows 上表现为 0xC0000005，无报错）。
    /// `log.rs` 那道重入闸是最后一道保险，不是让它绕过去的理由。
    pub fn cfg_prefix() -> PathBuf {
        CFG_PREFIX.get_or_init(Self::compute_cfg_prefix).clone()
    }

    fn compute_cfg_prefix() -> PathBuf {
        let mut prefix: Option<String> = None;
        let args: Vec<String> = std::env::args().collect();
        for (i, a) in args.iter().enumerate() {
            if a == "-configpath" && i + 1 < args.len() {
                prefix = Some(args[i + 1].clone());
                break;
            }
        }
        let base = match prefix {
            // 显式指定：原样用，不再拼子目录
            Some(p) => PathBuf::from(p),
            // 默认：exe 目录下的 .mvui/
            None => {
                let exe = std::env::current_exe()
                    .ok()
                    .and_then(|e| e.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_default();
                exe.join(".mvui")
            }
        };
        let _ = std::fs::create_dir_all(&base);
        base
    }

    /// 载入设置。文件不存在或损坏时返回空表（不报错）。
    pub fn load() -> Self {
        let dir = Self::cfg_prefix();
        let path = dir.join("mvui.ini");
        let map = crate::core::options::read_text_file(&path)
            .map(|text| {
                let mut m = BTreeMap::new();
                for line in text.lines() {
                    let line = line.trim();
                    // 跳过空行、节名 `[General]`、注释 `;...`
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
        dlog!(
            "设置: 从 {} 载入 {} 条",
            path.display(),
            map.len()
        );
        Self { path, map }
    }

    /// 把写错误返回给调用方，而不是吞掉：保存失败曾经会静默丢掉
    /// 全部设置（README P3）。
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::from("[General]\n");
        for (k, v) in &self.map {
            out.push_str(&format!("{k}={v}\n"));
        }
        std::fs::write(&self.path, out)?;
        dlog!(
            "设置: 保存 {} 条到 {}",
            self.map.len(),
            self.path.display()
        );
        Ok(())
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

/// 缓存目录助手，挂在配置根目录下。
impl GuiSettings {
    /// `<配置根>/cache`。清单缓存、审计缓存、boot.log 都住这里。
    pub fn cache_dir() -> PathBuf {
        let p = GuiSettings::cfg_prefix().join("cache");
        let _ = std::fs::create_dir_all(&p);
        p
    }
}
