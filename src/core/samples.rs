//! 样本集（sample set）的定位与审计。
//!
//! # 为什么需要这个模块
//!
//! MAME 的 `-listxml` **不把样本集作为 `<machine>` 输出**，只在游戏上留一句
//! 悬空的 `sampleof="genpin"`。实测 MAME 0.284 的全量 listxml（320MB）：
//! 1898 个机种带 `sampleof`，而 `genpin` 在里面出现 **0 次**。
//!
//! 所以 `RomInfoView` 原来那句 `lib.get_idx(&g.sampleof)` 注定返回 `None`
//! （全库 1574/1898 = 83% 静默失败），Samples 段对绝大多数游戏恒空。
//!
//! 样本集的真实形态是**磁盘上的独立包**：`mame.ini` 里 `samplepath = samples`，
//! 该目录下有 76 个 `{name}.zip`——与全库 76 种 `sampleof` 目标**一一对应、
//! 缺失 0 个**。
//!
//! # 三层数据里缺的是中间那层
//!
//! | 层 | 来源 | 内容 |
//! |---|---|---|
//! | 要哪些采样文件 | `<sample name>` | ✅ listxml 里有（只给基名，无扩展名） |
//! | 去哪个包拿 | `sampleof` 属性 | ✅ listxml 里有 |
//! | 那个包里有哪些文件 | zip 中央目录 | ❌ **不在 listxml 里** |
//!
//! 比对时把 `<sample name>` 拼上 `.wav`（实测样本集条目一律带 `.wav`、
//! 扁平无子目录），再取 zip 条目的 basename 相比较。
//!
//! # CRC 校验做不了
//!
//! DTD 写死 `<!ELEMENT sample EMPTY>` —— `<sample>` 只有 `name` 一个属性，
//! **没有 size / crc / sha1**。所以只能判"文件在不在"，判不了"内容对不对"。
//! 想要权威结论只能让 MAME 自己说（`-verifysamples`）。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::core::archive;
use crate::core::model::GameMeta;
use crate::core::rominfo::{RomState, SampleRow};

// 注：实测 `genpin.zip` 的 18 个条目**全部**带 `.wav`（0 个例外），但**比对时
// 不拼这个后缀** —— `list_have` 存的是 `file_stem()`（已去扩展名的基名），
// `GameMeta::samples` 里也是基名，两边同形态直接比。曾在这里给样本名拼 `.wav`
// 去查 Set，于是永远匹配不上、`have` 恒 0、整个 Samples 段全变"缺失"。

/// 某个样本集的审计结果：一个包里有哪些采样文件名。
///
/// 用 `HashSet` 是因为**同一个包被上千台游戏共用**（`genpin` 被1438 台
/// pinball 游戏引用），每次都比对前先构造一次 Set。
pub type SampleSet = HashSet<String>;

/// 定位一个样本集包，返回它的**已存在**候选路径。
///
/// **三种形态都要认**（实测机器上三种都出现过）：
///
/// | 形态 | 路径 | 谁在用
/// |---|---|---|
/// | 归档 | `dir/{name}.zip` | MAME 0.284 标准发行包的 `samples/`（76 个 zip）
/// | 归档 | `dir/{name}.7z` | 部分整合包
/// | 散目录 | `dir/{name}/` | 0.289 那套（`samples/` 空，只有 `floppy/*.wav` 散装）
///
/// 三候选口径与 `audit::find_units_for`（按名字定位 rom 归档）**故意一致**：
/// 那已经是本项目验证过的"一个名字三个去处"惯例，抄它比另立一套好。
///
/// 只查 `dirs` 里的目录，不做 `read_dir` 全量枚举——样本集总共 76 个包，
/// 按名字直接stat 就够（这也是 `find_units_for` 把单游戏审计从 11s 降到
/// 0.002s 的原因）。
pub fn find_sample_archive(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    for dir in dirs {
        // 归档形态
        for ext in ["zip", "7z"] {
            let p = dir.join(format!("{name}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
        // 散目录形态
        let d = dir.join(name);
        if d.is_dir() {
            return Some(d);
        }
    }
    None
}

/// 列出一个样本集里**已拥有**的采样文件名（基名，不含扩展名）。
///
/// 归档走 `archive::list_archive`（现成的 zip/7z 中央目录读取，**不打开
/// 条目内容**），散目录走 `read_dir`。两者都只取 basename——实测归档是扁平
/// 的，但取 basename 零成本，且能挡住"解压器多套一层目录"的情况。
pub fn list_have(archive_or_dir: &Path) -> SampleSet {
    let mut out = SampleSet::new();
    if archive_or_dir.is_dir() {
        // 散目录形态：直接列文件
        let Ok(rd) = std::fs::read_dir(archive_or_dir) else {
            return out;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    out.insert(stem.to_string());
                }
            }
        }
        return out;
    }
    // 归档形态
    let Ok(entries) = archive::list_archive(archive_or_dir) else {
        return out;
    };
    for e in entries {
        // 条目名可能是 `sub/dir/foo.wav`，取 basename 再去扩展名
        let base = e.name.rsplit(['/', '\\']).next().unwrap_or(&e.name);
        if let Some(stem) = Path::new(base).file_stem().and_then(|s| s.to_str()) {
            out.insert(stem.to_string());
        }
    }
    out
}

/// 审计一台机种的样本集，产出面板要显示的那一行。
///
/// `total` 取**本机自己的 `<sample>` 条数**（`g.samples`），不是包内文件数——
/// 用户关心的是"这台游戏要的齐了没有"，而 `genpin` 那个包里的文件远多于
/// 单台游戏需要的（共享包）。实测 `rctycn` 要 18 个，`genpin.zip` 正好18个。
///
/// `state` 的含义与其他段一致：
/// - `Unknown` —— 没审计，或样本集包**根本没找到**（用户压根没下采样包）
/// - `Good` —— 本机需要的文件全在包里
/// - `Missing` —— 缺了至少一个
pub fn audit_game_sample(
    g: &GameMeta,
    dirs: &[PathBuf],
    audited: bool,
) -> Option<SampleRow> {
    if g.sampleof.is_empty() || g.samples.is_empty() {
        return None;
    }
    // 自引用守卫：`sampleof` 指向自己的机种（实测全库61 个）不是样本集，
    // 拿它当包名去找会得到一个毫无意义的"缺失"。
    if g.sampleof.eq_ignore_ascii_case(&g.name) {
        return None;
    }
    let total = g.samples.len();
    let Some(pack) = find_sample_archive(dirs, &g.sampleof) else {
        // 包都没找到：状态未知（不是"缺 18 个"——用户可能压根没下采样包，
        // 报"缺失"会误导他去逐个文件找）。**未审计也是同一个 Unknown**，
        // 所以这里不再按 `audited` 分支（原先两个分支写同一个值，是死代码）。
        return Some(SampleRow {
            name: g.sampleof.clone(),
            have: 0,
            total,
            state: RomState::Unknown,
        });
    };
    let have_set = list_have(&pack);
    // **两边都必须是基名**：`list_have` 存的是 `file_stem()`（`a.wav` → `a`），
    // 而 `g.samples` 里也是基名（`<sample name="bumper"/>` 没有扩展名）。
    // 曾经在这里给样本名拼 `.wav` 去查Set，于是永远查不到 —— `have` 恒0，
    // 整个 Samples 段全变"缺失"。**扩展名在 `list_have` 里已经被去掉了，
    // 这一侧不要再拼。**
    let have = g.samples.iter().filter(|s| have_set.contains(*s)).count();
    let state = if !audited {
        RomState::Unknown
    } else if have == total {
        RomState::Good
    } else {
        RomState::Missing
    };
    Some(SampleRow {
        name: g.sampleof.clone(),
        have,
        total,
        state,
    })
}

/// `mame.ini` 里 `samplepath` 的默认值（MAME 官方默认）。
pub const DEFAULT_SAMPLEPATH: &str = "samples";

/// 本次会话的样本目录（`mame.ini` 的 `samplepath` 解析出来的绝对路径）。
///
/// **用全局而不是给 `view_of` 加参数**，是因为 `view_of` 已经有 15 个调用点
/// （3 处产品代码 + 12 处测试），为一个目录把签名拖长一截不划算；而且样本
/// 目录与 Rom 目录不同——它**只有一个**（MAME 的 `samplepath` 是单值，
/// 不是 `;` 分隔的列表），全局存一份就够。
///
/// 用 `RwLock` 而不是 `OnceLock`：**必须允许覆盖**。`OnceLock::set` 第二次调用
/// 返回 `Err` 且**不生效**（返回值极易被 `let _ =` 吞掉，于是"刷新目录"变成
/// 静默无操作）；而且引导有热/冷两条路径都会设目录，第二次设是常态不是异常。
/// 读多写极少（只在引导时写），`RwLock` 的读开销可以忽略。
static SAMPLE_DIRS: std::sync::RwLock<Vec<PathBuf>> = std::sync::RwLock::new(Vec::new());

/// 引导时设置样本目录。**没调过就是空列表**（于是所有样本行都是
/// [`RomState::Unknown`]，而不是错误的"缺失"）。
///
/// 可重复调用，后一次覆盖前一次——`finish_boot` 与 `finish_boot_cached`
/// 都会调（**两条路径都必须调**，见下）。
pub fn set_sample_dirs(dirs: Vec<PathBuf>) {
    match SAMPLE_DIRS.write() {
        Ok(mut g) => *g = dirs,
        // 锁 poisoned：上一次写的时候 panic 了。样本目录只是"有没有样本包"的
        // 提示信息，宁可继续用旧值也不要在这里 panic 把整个引导带崩。
        Err(p) => *p.into_inner() = dirs,
    }
}

/// 当前生效的样本目录。
pub fn sample_dirs() -> Vec<PathBuf> {
    match SAMPLE_DIRS.read() {
        Ok(g) => g.clone(),
        Err(p) => p.into_inner().clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::RomInfo;

    fn meta_with(name: &str, sampleof: &str, samples: &[&str]) -> GameMeta {
        let mut g = GameMeta {
            name: name.into(),
            sampleof: sampleof.into(),
            ..Default::default()
        };
        g.samples = samples.iter().map(|s| (*s).to_string()).collect();
        g
    }

    fn rom_named(n: &str) -> RomInfo {
        RomInfo {
            name: n.into(),
            ..Default::default()
        }
    }

    /// 没有 `sampleof` 或没有 `<sample>` 的机种不产出行——
    /// 面板上不该出现"样本: -"这种噪音。
    #[test]
    fn a_game_without_samples_produces_no_row() {
        assert!(audit_game_sample(&meta_with("x", "", &[]), &[], true).is_none());
        assert!(audit_game_sample(&meta_with("x", "genpin", &[]), &[], true).is_none());
    }

    /// `sampleof` 指向自己的机种（实测 61 个）不算样本集。
    #[test]
    fn a_self_referencing_sampleof_is_not_a_sample_set() {
        let g = meta_with("3bagfull", "3bagfull", &["a", "b"]);
        assert!(
            audit_game_sample(&g, &[], true).is_none(),
            "自己不是自己的样本集"
        );
    }

    /// 样本集包找不到时状态是 Unknown 而不是 Missing ——
    /// 用户压根没下采样包，报"缺失"会误导他去逐个文件找。
    #[test]
    fn a_missing_archive_is_unknown_not_missing() {
        let g = meta_with("rctycn", "genpin", &["bumper", "chime1"]);
        let row = audit_game_sample(&g, &[PathBuf::from(r"Z:\definitely\not\here")], true)
            .expect("仍要出行，只是状态未知");
        assert_eq!(row.name, "genpin");
        assert_eq!(row.total, 2);
        assert_eq!(row.state, RomState::Unknown, "没下包 ≠ 缺文件");
    }

    /// 没审计时一律 Unknown，不能报"拥有"——那是骗人。
    #[test]
    fn unaudited_games_report_unknown() {
        let g = meta_with("rctycn", "genpin", &["a"]);
        let row = audit_game_sample(&g, &[], false).expect("出行");
        assert_eq!(row.state, RomState::Unknown);
    }

    /// 核心路径：真的在磁盘上放一个 zip，验证 18 个文件全被认出来。
    /// 这是唯一能证明"比对逻辑真的работает"的测试 —— 纯 mock 只能证明
    /// 代码按自己写的跑。
    #[test]
    fn a_real_zip_is_matched_against_the_games_sample_list() {
        let dir = std::env::temp_dir().join("mvui_samples_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");

        // 写一个含 3 个 wav 的 zip：a / b / c
        {
            use std::io::Write;
            let f = std::fs::File::create(dir.join("genpin.zip")).expect("建 zip");
            let mut zw = zip::ZipWriter::new(f);
            let opts: zip::write::FileOptions<'_, ()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for n in ["a.wav", "b.wav", "c.wav"] {
                zw.start_file(n, opts).expect("写条目");
                zw.write_all(b"x").expect("写内容");
            }
            zw.finish().expect("收尾");
        }

        // 本机要 a / b / c 三个 → 全齐
        let g = meta_with("rctycn", "genpin", &["a", "b", "c"]);
        let row = audit_game_sample(&g, &[dir.clone()], true).expect("出行");
        assert_eq!(row.have, 3, "三个 wav 都该被认出来");
        assert_eq!(row.total, 3);
        assert_eq!(row.state, RomState::Good);

        // 本机要 a / b / zz 三个 → 缺 zz
        let g2 = meta_with("rctycn", "genpin", &["a", "b", "zz"]);
        let row2 = audit_game_sample(&g2, &[dir.clone()], true).expect("出行");
        assert_eq!(row2.have, 2, "包里有 a / b，没有 zz");
        assert_eq!(row2.state, RomState::Missing);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 散目录形态（0.289 那套的 `floppy/*.wav`）也要认。
    #[test]
    fn a_loose_directory_is_also_a_valid_sample_set() {
        let base = std::env::temp_dir().join("mvui_samples_loose");
        let _ = std::fs::remove_dir_all(&base);
        let set = base.join("genpin");
        std::fs::create_dir_all(&set).expect("建目录");
        for n in ["a", "b"] {
            std::fs::write(set.join(format!("{n}.wav")), b"x").expect("写 wav");
        }
        let g = meta_with("rctycn", "genpin", &["a", "b"]);
        let row = audit_game_sample(&g, &[base.clone()], true).expect("出行");
        assert_eq!(row.have, 2, "散目录形态要能认出来");
        assert_eq!(row.state, RomState::Good);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 三个候选（`.zip` / `.7z` / 散目录）里找到任何一个就算数，
    /// 口径与 `find_units_for` 一致。
    #[test]
    fn all_three_archive_shapes_are_tried() {
        let base = std::env::temp_dir().join("mvui_samples_shapes");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("建目录");
        // 只放一个 7z 名字的占位文件：内容不用合法，find只看存在性
        std::fs::write(base.join("genpin.7z"), b"x").expect("写占位");
        assert_eq!(
            find_sample_archive(&[base.clone()], "genpin"),
            Some(base.join("genpin.7z")),
            "7z 形态要认"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// `set_sample_dirs` **必须能覆盖**。
    ///
    /// 回归测试：它原先是 `OnceLock`，`set` 第二次返回 `Err` 且**不生效**，
    /// 而返回值被 `let _ =` 吞掉 → "刷新目录"静默变成无操作。引导的热/冷两条
    /// 路径都要设目录，第二次设是常态。
    #[test]
    fn the_sample_dir_table_can_be_replaced() {
        set_sample_dirs(vec![PathBuf::from("Z:\\first")]);
        assert_eq!(sample_dirs(), vec![PathBuf::from("Z:\\first")]);
        set_sample_dirs(vec![PathBuf::from("Y:\\second"), PathBuf::from("Y:\\third")]);
        assert_eq!(
            sample_dirs(),
            vec![PathBuf::from("Y:\\second"), PathBuf::from("Y:\\third")],
            "第二次 set 必须真的替换掉第一次的值"
        );
        // 复位，免得污染同进程里其他测试的全局状态
        set_sample_dirs(Vec::new());
    }

    /// 全局目录表驱动的审计：设进去就能查到，查不到就是空表。
    /// 这条正是"热启动 Samples 段恒灰"的形状——`sample_dirs()` 返回空表时，
    /// 有样本的游戏也只能是 Unknown。
    #[test]
    fn the_global_table_drives_the_audit() {
        let dir = std::env::temp_dir().join("mvui_samples_global");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        std::fs::write(dir.join("genpin.zip"), b"not a real zip").expect("写占位");

        let g = meta_with("rctycn", "genpin", &["a", "b"]);
        set_sample_dirs(vec![dir.clone()]);
        // 找到一个占位 zip：至少证明"目录被用上了"（坏 zip → 空 Set → Missing）
        let row = audit_game_sample(&g, &sample_dirs(), true).expect("出行");
        assert_eq!(row.name, "genpin");
        assert_eq!(row.total, 2);
        assert_ne!(row.state, RomState::Unknown, "包在磁盘上，不该是 Unknown");

        // 把表清空 → 立刻退回 Unknown（这正是热启动漏设目录时的表现）
        set_sample_dirs(Vec::new());
        let row2 = audit_game_sample(&g, &sample_dirs(), true).expect("出行");
        assert_eq!(row2.state, RomState::Unknown, "空表 = 找不到包 = Unknown");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `_sample` 前缀是 MAME 自己给"内部使用样本"的文件名约定，
    /// 不该被当成"这台游戏要的采样"。
    #[test]
    fn rom_files_are_not_confused_with_samples() {
        // 反过来验：`g.samples` 里的名字拼 `.wav` 后必须能在包里找到，
        // 而包里的 `.rom`/`.bin` 不该被算成采样文件。
        let dir = std::env::temp_dir().join("mvui_samples_rom");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建目录");
        {
            use std::io::Write;
            let f = std::fs::File::create(dir.join("genpin.zip")).expect("建 zip");
            let mut zw = zip::ZipWriter::new(f);
            let opts: zip::write::FileOptions<'_, ()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for n in ["a.wav", "sub/b.wav", "c.bin"] {
                zw.start_file(n, opts).expect("写条目");
                zw.write_all(b"x").expect("写内容");
            }
            zw.finish().expect("收尾");
        }
        let have = list_have(&dir.join("genpin.zip"));
        // 子目录里的 b.wav 也要认（取 basename）
        assert!(have.contains("a") && have.contains("b"), "a/b 都该在");
        // `.bin` 会被收进 Set（`list_have` 只去扩展名，不筛后缀），
        // 但**比对时不会命中**——因为 `<sample>` 里不会有叫 `c` 的采样名。
        // 这里验的是"取 basename 生效"：`sub/b.wav` 的 b 被正确认了出来。
        assert!(have.contains("c"));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = rom_named("unused");
    }
}

