//! MvUI 去哪里找它的内容：artwork、DAT 文件、本地化游戏列表、背景
//! 图片，以及外部文件夹列表。
//!
//! # 为什么要有这个模块
//!
//! 1.8.2 把这些全都解析到 **mame.exe 目录**（进程 cwd）下，因为一份
//! MAME 安装就是把 `snap/`、`flyers/` 和那些 `.dat` 放在那里。对单一
//! 用途的安装这没问题，但它把 artwork 和模拟器绑死了：一个放在只读
//! MAME 目录树旁边的便携版 MvUI 会一张图都没有，而且每个用户都得
//! 手动把 `history.dat` 拷进 MAME 文件夹。
//!
//! 这里的规则是：**内容住在 `mvui.exe` 旁边**；MAME 目录只管 ROM 和
//! MAME 自己的选项。每个位置仍然可以在"设置 ▸ 目录"里指向别处，
//! 而且显式设置永远压过默认值。
//!
//! 每个路径的解析顺序：
//!
//! 1. 配置值，绝对路径 → 原样使用
//! 2. 配置值，相对路径 → 按 exe 目录解析
//! 3. 为空/没配 → `<exe 目录>/<内置默认名>`

use std::path::PathBuf;

use crate::core::settings::GuiSettings;

/// 存放 `mvui.exe` 的目录 —— 下面所有默认值的锚点。
pub fn exe_dir() -> PathBuf {
    GuiSettings::exe_dir()
}

/// 把一个配置值按 exe 目录解析；没配出可用值时退回
/// `<exe>/default_rel`。
///
/// 配置值可以是 `;` 分隔的列表；第一个非空条目胜出（真正支持列表的
/// 调用方会自己解析）。
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

/// 同 [`resolve`]，但针对 `;` 分隔的列表，每个条目都做归一化。
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

/// 7 个图片面板各自的 `(选项键, exe 目录下的默认目录名)`。
///
/// 注意 `snapshot_directory` 是个例外：1.8.2 的模板没给它默认值，于是
/// 它继承了 MAME 自己的 `$HOME/snap` 约定。这里跟其他几个一样给了
/// 一个真正的默认值。
pub const IMAGE_DIRS: [(&str, &str); 7] = [
    ("snapshot_directory", "snap"),
    ("flyer_directory", "flyers"),
    ("cabinet_directory", "cabinets"),
    ("marquee_directory", "marquees"),
    ("title_directory", "titles"),
    ("cpanel_directory", "cpanel"),
    ("pcb_directory", "pcb"),
];

/// exe 目录下存放所有 `.dat` 的子目录。
pub const DAT_SUBDIR: &str = "dats";

/// 5 个文档面板各自的 `(选项键, [`DAT_SUBDIR`] 里的默认文件名)`。
///
/// `mameinfo_file` 支撑两个面板（MAME信息 和 驱动信息）——同一个文件、
/// 不同的查询键——所以只出现一次。
///
/// History 用的是 **`history.xml`** 而不是 `history.dat`：Arcade-History
/// 在 MAME 0.228 前后转成了 XML，DAT 版随之停止维护。MAME 和 MAMEUI
/// 现在都要这个新格式的文件（`<mame>/history/history.xml`，MAMEUI 系
/// 则是 `<mame>/dats/`），所以默认值跟着走。名字叫 `.dat` 的文件仍然
/// 能用——加载器是按文件**内容**分派的，DAT 扫描器是兜底路径（见
/// `background::read_one_dat`）。
pub const DAT_FILES: [(&str, &str); 4] = [
    ("history_file", "history.xml"),
    ("mameinfo_file", "mameinfo.dat"),
    ("story_file", "story.dat"),
    ("command_file", "command.dat"),
];

/// 默认 `<exe>/folders` —— 外部文件夹列表，含 `Favorites.ini`。
pub const FOLDERS_SUBDIR: &str = "folders";

/// 默认 `<exe>/bkground` —— 窗口壁纸目录（沿用 1.8.2 的拼写，好让已有
/// 的安装继续能用）。
pub const BG_SUBDIR: &str = "bkground";

/// 本地化游戏列表，从 exe 目录读取。UTF-8，tab 分隔。
pub const LST_FILE: &str = "mame_cn.lst";

/// 某个图片面板目录的绝对路径。
pub fn image_dir(configured: Option<&str>, dock: usize) -> PathBuf {
    let (key, default_rel) = IMAGE_DIRS
        .get(dock)
        .copied()
        .unwrap_or(("pcb_directory", "pcb"));
    resolve(Some(configured.unwrap_or(key)), default_rel)
}

/// 某个文档面板 `.dat` 的绝对路径。
///
/// 配置值指的是一个**文件**而不是目录——那是 1.8.2 的 `datfile` 选项
/// 类型，用户很可能就把它指向单个文件。没配的时候我们去
/// `<exe>/dats/<name>` 里找。
pub fn dat_file(configured: Option<&str>, dock: usize) -> PathBuf {
    let key = crate::core::dat::dock_file_option(dock);
    let fallback = DAT_FILES
        .iter()
        .find(|(k, _)| Some(*k) == key)
        .map(|(_, f)| *f)
        .unwrap_or("history.xml");
    match configured.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => resolve(Some(v), fallback),
        None => exe_dir().join(DAT_SUBDIR).join(fallback),
    }
}

/// `<exe>/folders`，不存在就创建 —— 没有它文件夹列表根本没法用，而
/// 1.8.2 会把 `Favorites.ini` 写进第一个配置目录。
pub fn folders_dir(configured: Option<&str>) -> PathBuf {
    let d = resolve(configured, FOLDERS_SUBDIR);
    let _ = std::fs::create_dir_all(&d);
    d
}

/// `<exe>/bkground`。
pub fn background_dir(configured: Option<&str>) -> PathBuf {
    resolve(configured, BG_SUBDIR)
}

/// `<exe>/mame_cn.lst`。
pub fn localized_list(configured: Option<&str>) -> PathBuf {
    match configured.map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => resolve(Some(v), LST_FILE),
        None => exe_dir().join(LST_FILE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个默认值都必须落在 exe 目录里面 —— 这正是本次改动的全部
    /// 意义，也是便携安装所依赖的东西。
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

    /// 相对设置仍然是相对 exe 目录解析，而不是相对 mame 目录。
    #[test]
    fn relative_settings_resolve_against_the_exe_dir() {
        let p = image_dir(Some("artwork/snap"), 0);
        assert_eq!(p, exe_dir().join("artwork").join("snap"));
    }

    /// 绝对路径的设置原样采纳 —— 这是给想把旧的 mame 目录布局找回来的
    /// 人留的逃生口。
    #[test]
    fn absolute_settings_win() {
        let p = image_dir(Some("D:/art"), 1);
        assert_eq!(p, PathBuf::from("D:/art"));
    }

    /// `;` 列表按顺序保留每一个条目。
    #[test]
    fn lists_resolve_every_entry() {
        let v = resolve_list(Some("C:/a;D:/b"), "snap");
        assert_eq!(v, vec![PathBuf::from("C:/a"), PathBuf::from("D:/b")]);
    }

    /// History 面板默认指向 **XML** 文件。
    ///
    /// 钉住它是因为文件名就是格式分派的全部：如果默认成
    /// `history.dat`，加载器会拿着一个用户根本没有的文件走 DAT 扫描器，
    /// 面板只会一直空着且没有任何报错——这个失败模式已经在 Command
    /// 面板上花掉过一轮排查（见 `dat::tests`）。
    #[test]
    fn history_defaults_to_the_xml_file() {
        let history = crate::core::dat::DOCK_HISTORY;
        assert_eq!(
            dat_file(None, history).file_name().unwrap().to_string_lossy(),
            "history.xml"
        );
        // 另外三个文档面板仍然是 DAT
        for dock in [crate::core::dat::DOCK_MAMEINFO, crate::core::dat::DOCK_STORY, crate::core::dat::DOCK_COMMAND] {
            let name = dat_file(None, dock).file_name().unwrap().to_string_lossy().to_string();
            assert!(name.ends_with(".dat"), "{dock} should stay a .dat, got {name}");
        }
    }

    /// 显式配置的 dat 文件仍然是文件，不会被当成目录去拼接。
    #[test]
    fn configured_dat_file_is_used_as_is() {
        let p = dat_file(Some("D:/info/my.dat"), 7);
        assert_eq!(p, PathBuf::from("D:/info/my.dat"));
    }
}
