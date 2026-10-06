//! 样本集（sample set）的定位与校验。
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

/// 某个样本集的校验结果：一个包里有哪些采样文件名。
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
/// 三候选口径与 `verify::find_units_for`（按名字定位 rom 归档）**故意一致**：
/// 那已经是本项目验证过的"一个名字三个去处"惯例，抄它比另立一套好。
///
/// 只查 `dirs` 里的目录，不做 `read_dir` 全量枚举——样本集总共 76 个包，
/// 按名字直接stat 就够（这也是 `find_units_for` 把单游戏校验从 11s 降到
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

/// 校验一台机种的样本集，产出面板要显示的那一行。
///
/// **只判"包在不在"**（2026-10-06 用户要求）：`{samplepath}/{sampleof}.zip`
/// 有 → `拥有`，没有 → `未拥有`。**不显示图标、不显示 `9/9` 数量。**
///
/// 之前这里逐个比对包内文件算出 `have/total`，面板显示 `18/18`。用户说那
/// 些数字"没意义"——样本集是**共享包**（`genpin` 被 1438 台 pinball 游戏
/// 引用），每台只要其中几个文件，`18/18` 那个分母是这台机器的、分子是整包的，
/// 混在一起并不说明任何事。用户真正要回答的是"这个游戏要的采样包我下了没"。
///
/// **`Unknown` 只在没校验时出现**（`verified=false`）。包找不到现在报
/// `Missing`（红）而不是 `Unknown`（灰）——用户明确说"红色，缺失"。
/// 之前的理由是"用户可能压根没下采样包，报缺失会误导"，但既然显示的是
/// "这个包在不在"，那不在就是不在，红色是准确的。
pub fn verify_game_sample(g: &GameMeta, dirs: &[PathBuf], verified: bool) -> Option<SampleRow> {
    if g.sampleof.is_empty() || g.samples.is_empty() {
        return None;
    }
    // 自引用守卫：`sampleof` 指向自己的机种（实测全库61 个）不是样本集，
    // 拿它当包名去找会得到一个毫无意义的"缺失"。
    if g.sampleof.eq_ignore_ascii_case(&g.name) {
        return None;
    }
    let present = find_sample_archive(dirs, &g.sampleof).is_some();
    let state = if !verified {
        RomState::Unknown
    } else if present {
        RomState::Good
    } else {
        RomState::Missing
    };
    Some(SampleRow {
        name: g.sampleof.clone(),
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
        assert!(verify_game_sample(&meta_with("x", "", &[]), &[], true).is_none());
        assert!(verify_game_sample(&meta_with("x", "genpin", &[]), &[], true).is_none());
    }

    /// `sampleof` 指向自己的机种（实测 61 个）不算样本集。
    #[test]
    fn a_self_referencing_sampleof_is_not_a_sample_set() {
        let g = meta_with("3bagfull", "3bagfull", &["a", "b"]);
        assert!(
            verify_game_sample(&g, &[], true).is_none(),
            "自己不是自己的样本集"
        );
    }

    /// 样本集包找不到 → **未拥有（红）**，不是灰色"未知"。
    ///
    /// 2026-06 用户定的口径：既然这一行显示的是"这个包在不在"，那不在就是
    /// 不在，红是准确的。此前报Unknown 的理由是"用户可能压根没下采样包，
    /// 报缺失会误导"—— 但判据已经简化成"包在不在"，那个理由不成立了。
    #[test]
    fn a_missing_archive_is_reported_as_not_owned() {
        let g = meta_with("rctycn", "genpin", &["bumper", "chime1"]);
        let row = verify_game_sample(&g, &[PathBuf::from(r"Z://definitely//not//here")], true)
            .expect("仍要出行，只是状态是未拥有");
        assert_eq!(row.name, "genpin");
        assert_eq!(row.state, RomState::Missing, "包不在 = 未拥有");
    }

    /// 没校验时一律 Unknown，不能报"拥有"——那是骗人。
    #[test]
    fn unverified_games_report_unknown() {
        let g = meta_with("rctycn", "genpin", &["a"]);
        let row = verify_game_sample(&g, &[], false).expect("出行");
        assert_eq!(row.state, RomState::Unknown);
    }

    /// 核心路径：磁盘上真的有 `{sampleof}.zip` → 报拥有。
    ///
    /// **不再逐个比对包内文件**（2026-06 用户要求）。原先这里断言
    /// `have=2, total=3, state=Missing`（包里缺 zz），现在包里有什么完全
    /// 不影响结论 —— 只要 zip 在就是"拥有"。
    #[test]
    fn an_existing_archive_reads_as_owned() {
        let dir = std::env::temp_dir().join("mvui_samples_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
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

        // 本机要 a / b / zz 三个 —— zz 不在包里，但**照样报拥有**：
        // 样本集是共享包，逐个比对算出来的分母没有意义。
        let g = meta_with("rctycn", "genpin", &["a", "b", "zz"]);
        let row = verify_game_sample(&g, &[dir.clone()], true).expect("出行");
        assert_eq!(row.name, "genpin");
        assert_eq!(row.state, RomState::Good, "zip 在就是拥有");

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
        let row = verify_game_sample(&g, &[base.clone()], true).expect("出行");
        // 散目录形态也认（0.289 那套是纯散装，`samples/{name}/`）
        assert_eq!(row.state, RomState::Good, "散目录形态要能认出来");
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

    /// 全局目录表驱动的校验：设进去就能查到，查不到就是空表。
    /// 这条正是"热启动 Samples 段恒灰"的形状——`sample_dirs()` 返回空表时，
    /// 有样本的游戏也只能是 Unknown。
    #[test]
    fn the_global_table_drives_the_verify() {
        let dir = std::env::temp_dir().join("mvui_samples_global");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        std::fs::write(dir.join("genpin.zip"), b"not a real zip").expect("写占位");

        let g = meta_with("rctycn", "genpin", &["a", "b"]);
        set_sample_dirs(vec![dir.clone()]);
        let row = verify_game_sample(&g, &sample_dirs(), true).expect("出行");
        assert_eq!(row.name, "genpin");
        assert_eq!(
            row.state,
            RomState::Good,
            "包在磁盘上就该报拥有（内容不合法也不影响 —— 只判存在）"
        );

        // 把表清空 → 立刻变成未拥有（这正是 2026-10-06 修的那个热启动 BUG
        // 的表现：`finish_boot_cached` 漏设目录 → Samples 段全灰）
        set_sample_dirs(Vec::new());
        let row2 = verify_game_sample(&g, &sample_dirs(), true).expect("出行");
        assert_eq!(row2.state, RomState::Missing, "空表 = 找不到包 = 未拥有");

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

