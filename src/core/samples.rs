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

/// **本次校验扫到的样本集包名**（小写）。
///
/// 由 [`scan_sample_sets`] 在 `verify_all` 开头一次扫出来，`verify_game_sample`
/// 只查这个集合。**不再按名字现场 `stat`**。
///
/// 为什么改成"扫一次存集合"：
///
/// - 判据只有"包在不在"，而样本集总共**几十个包**（实测 0.284 的 `samples/`
///   是 76 个 zip + 4 个目录）。一次 `read_dir` 毫秒级，比 44387 个条目的
///   rom 枚举小三个数量级，完全搭得起。
/// - 挂在 `verify_all` 里而不是渲染路径上之后，`verified` 门控**名副其实**：
///   灰色真的表示"这一轮还没扫到"，而不是像原先那样"其实查过了但被 rom 的
///   进度挡住了"。要刷新样本状态按 F5 即可。
/// - 散目录形态（0.289 那套 `samples/{name}/`）与归档**混在同一个集合里**，
///   判据统一成"三种形态任一存在"。
///
/// 用 `LazyLock` 包 `RwLock`：`HashSet::new()` 不是 `const`，不能直接进
/// `static`。`LazyLock` 首次解引用时构造，全程不需要 `OnceLock::set` 那套
/// 「第二次返回 `Err` 且不生效」的玩法。
static SAMPLE_SETS: std::sync::LazyLock<std::sync::RwLock<SampleSet>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(SampleSet::new()));

/// 扫 `dirs` 下的样本集，把包名收进全局集合。**每次 `verify_all` 开头调一次。**
///
/// 三种形态都收：
///
/// | 形态 | 收什么
/// |---|---|
/// | `dir/{name}.zip` / `.7z` | `{name}`（`file_stem`，去扩展名）
/// | `dir/{name}/` | `{name}`（`file_name`，目录名本身就是样本集名）
///
/// 一律转小写：Windows/macOS 文件系统不敏感，磁盘上可能是 `GENPIN.ZIP`，
/// 而 `GameMeta::sampleof` 是小写。
/// **测试专用**：串行化所有会改 `SAMPLE_DIRS` / `SAMPLE_SETS` 的测试。
///
/// Rust 的测试默认多线程并行，而这些是**进程级全局**—— 不加锁就会
/// 互相踩：症状是**单跑一个测试全绿、`cargo test` 跑全量间歇性 FAILED，
/// 而且失败的那个常常是无辜的**（2026-06 两次都栽在这：`samples.rs` 内部的
/// 测试互相踩了一次，`rominfo.rs` 的样本测试与`samples.rs` 跨模块踩了一次）。
///
/// 必须是 `pub(crate)` 而不是模块私有：`rominfo.rs` 的测试也要改同一份
/// 集合，**跨模块也要抢同一把锁**。
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn scan_sample_sets(dirs: &[PathBuf]) -> usize {
    let mut set = SampleSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let key = if p.is_dir() {
                // 散目录：`samples/genpin/` → "genpin"
                p.file_name().map(|n| n.to_string_lossy().to_lowercase())
            } else if archive::is_zip(&p) || archive::is_7z(&p) {
                // 归档：`samples/genpin.zip` → "genpin"
                p.file_stem().map(|n| n.to_string_lossy().to_lowercase())
            } else {
                None
            };
            if let Some(k) = key {
                if !k.is_empty() {
                    set.insert(k);
                }
            }
        }
    }
    let n = set.len();
    match SAMPLE_SETS.write() {
        Ok(mut g) => *g = set,
        // 锁 poisoned：上一次写时 panic 了。样本状态只是提示信息，宁可沿用旧值
        // 也不要在这里 panic 把整轮校验带崩。
        Err(p) => *p.into_inner() = set,
    }
    n
}

/// 已扫到的样本集包名集合（供测试与诊断用）。
pub fn sample_sets() -> Vec<String> {
    let Ok(g) = SAMPLE_SETS.read() else {
        return Vec::new();
    };
    let mut v: Vec<String> = g.iter().cloned().collect();
    v.sort();
    v
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
/// **`verified` 门控在这里终于是真的**（2026-06）：判据来自 [`scan_sample_sets`]
/// 那一次扫描，而扫描挂在 `verify_all` 开头 —— 所以灰色确实表示"这一轮还没
/// 扫到"，而不是像原先那样"其实已经 `stat` 过了，只是被 rom 的进度挡住"。
/// 要刷新按 F5。
///
/// 包找不到报 `Missing`（红）不是 `Unknown`（灰）：既然显示的是"这个包在不在"，
/// 那不在就是不在，红色是准确的（用户明确要求）。
pub fn verify_game_sample(g: &GameMeta, verified: bool) -> Option<SampleRow> {
    if g.sampleof.is_empty() || g.samples.is_empty() {
        return None;
    }
    // 自引用守卫：`sampleof` 指向自己的机种（实测全库 61 个）不是样本集，
    // 拿它当包名去找会得到一个毫无意义的"缺失"。
    if g.sampleof.eq_ignore_ascii_case(&g.name) {
        return None;
    }
    // 集合里存的是小写（`scan_sample_sets` 统一转的），而 `sampleof` 在
    // listxml 里已经是小写，仍转一次以防用户手改的 dat 有大写。
    let key = g.sampleof.to_lowercase();
    let present = match SAMPLE_SETS.read() {
        Ok(s) => s.contains(&key),
        Err(p) => p.into_inner().contains(&key),
    };
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

    fn meta_with(name: &str, sampleof: &str, samples: &[&str]) -> GameMeta {
        let mut g = GameMeta {
            name: name.into(),
            sampleof: sampleof.into(),
            ..Default::default()
        };
        g.samples = samples.iter().map(|s| (*s).to_string()).collect();
        g
    }

    /// 没有 `sampleof` 或没有 `<sample>` 的机种不产而行——
    /// 面板上不该出现"样本: -"这种噪音。
    #[test]
    fn a_game_without_samples_produces_no_row() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        scan_sample_sets(&[]);
        assert!(verify_game_sample(&meta_with("x", "", &[]), true).is_none());
        assert!(verify_game_sample(&meta_with("x", "genpin", &[]), true).is_none());
    }

    /// `sampleof` 指向自己的机种（实测 61 个）不算样本集。
    #[test]
    fn a_self_referencing_sampleof_is_not_a_sample_set() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        scan_sample_sets(&[]);
        let g = meta_with("3bagfull", "3bagfull", &["a", "b"]);
        assert!(
            verify_game_sample(&g, true).is_none(),
            "自己不是自己的样本集"
        );
    }

    /// 没校验时一律 Unknown，不能报"拥有"——那是骗人。
    ///
    /// **这条在 2026-06 之后终于名副其实**：判据来自 `verify_all` 开头那次
    /// `scan_sample_sets`，`verified=false` 就是"这一轮真的还没扫"。原先这里
    /// 是假的——判据只做 3 次 `stat`、早就查完了，却因为 rom 还没校验完而
    /// 灰着。
    #[test]
    fn unverified_games_report_unknown() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join("mvui_samples_unver");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建目录");
        std::fs::write(dir.join("genpin.zip"), b"x").expect("写占位");
        scan_sample_sets(&[dir.clone()]);

        let g = meta_with("rctycn", "genpin", &["a"]);
        assert_eq!(
            verify_game_sample(&g, false).expect("出行").state,
            RomState::Unknown,
            "verified=false 就是没扫过，不许报拥有"
        );
        // 同一个包，verified=true 就该报拥有 —— 差别只在这个标志
        assert_eq!(
            verify_game_sample(&g, true).expect("出行").state,
            RomState::Good
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 核心路径：`scan_sample_sets` 扫到 `{sampleof}.zip` → 报拥有。
    ///
    /// **不逐个比对包内文件**（2026-06 用户要求）：包里有什么完全不影响结论。
    /// 样本集是共享包（`genpin` 被 1438 台 pinball 游戏引用），算`9/9` 的分母
    /// 没有意义。
    #[test]
    fn a_scanned_archive_reads_as_owned() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join("mvui_samples_scan");
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
        assert_eq!(scan_sample_sets(&[dir.clone()]), 1, "扫到 1 个样本集");

        // 本机要 a / b / zz 三个 —— zz 不在包里，但**照样报拥有**
        let g = meta_with("rctycn", "genpin", &["a", "b", "zz"]);
        let row = verify_game_sample(&g, true).expect("出行");
        assert_eq!(row.name, "genpin");
        assert_eq!(row.state, RomState::Good, "zip 在就是拥有");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **三种形态都要收进集合**（用户 2026-06 明确要求"散文件也需要，以文件夹名
    /// 匹配"）：`{name}.zip` / `{name}.7z` / 散目录 `{name}/`。
    #[test]
    fn all_three_archive_shapes_are_scanned() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let base = std::env::temp_dir().join("mvui_samples_shapes");
        let _ = std::fs::remove_dir_all(&base);
        // zip
        std::fs::create_dir_all(&base).expect("建目录");
        std::fs::write(base.join("genpin.zip"), b"x").expect("写占位");
        // 7z（内容不用合法，扫描只看名字与扩展名）
        std::fs::write(base.join("ssample.7z"), b"x").expect("写占位");
        // 散目录（0.289 那套：`samples/{name}/`）
        std::fs::create_dir_all(base.join("ssample2")).expect("建散目录");

        assert_eq!(scan_sample_sets(&[base.clone()]), 3, "三种形态各收一个");
        let sets = sample_sets();
        assert!(sets.contains(&"genpin".to_string()), "zip: {sets:?}");
        assert!(sets.contains(&"ssample".to_string()), "7z: {sets:?}");
        assert!(
            sets.contains(&"ssample2".to_string()),
            "散目录按文件夹名匹配: {sets:?}"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 大小写：磁盘上可能是 `GENPIN.ZIP`，而 `sampleof` 是小写。
    #[test]
    fn the_scan_normalises_case() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let base = std::env::temp_dir().join("mvui_samples_case");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("建目录");
        std::fs::write(base.join("GENPIN.ZIP"), b"x").expect("写占位");
        scan_sample_sets(&[base.clone()]);
        assert_eq!(
            verify_game_sample(&meta_with("rctycn", "genpin", &["a"]), true)
                .expect("出行")
                .state,
            RomState::Good,
            "文件系统不敏感，扫描必须统一转小写"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 无关文件不该被当成样本集。
    #[test]
    fn unrelated_files_are_not_collected() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let base = std::env::temp_dir().join("mvui_samples_junk");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("建目录");
        for n in ["readme.txt", "notes.md", "genpin.txt"] {
            std::fs::write(base.join(n), b"x").expect("写占位");
        }
        std::fs::create_dir_all(base.join("_sample")).expect("建目录");
        assert_eq!(
            scan_sample_sets(&[base.clone()]),
            1,
            "只有 `_sample` 目录算样本集，其余三个文本文件不算"
        );
        assert_eq!(sample_sets(), vec!["_sample".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 扫不到就是扫不到 —— 空目录给出空集合，样本一律未拥有（红）。
    ///
    /// 这正是 2026-06 那个热启动 BUG 的形状：引导漏发布目录 → 扫出来是空的
    /// → Samples 段要么全灰（未校验）或全红（已校验）。
    #[test]
    fn an_empty_sample_path_yields_nothing_owned() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join("mvui_samples_empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建目录");
        assert_eq!(scan_sample_sets(&[dir.clone()]), 0);
        assert_eq!(
            verify_game_sample(&meta_with("rctycn", "genpin", &["a"]), true)
                .expect("出行")
                .state,
            RomState::Missing
        );
        // 目录压根不存在也不能 panic
        assert_eq!(
            scan_sample_sets(&[PathBuf::from("Z:////definitely////not////here")]),
            0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `set_sample_dirs` **必须能覆盖**。
    ///
    /// 回归测试：它原先是 `OnceLock`，`set` 第二次返回 `Err` 且**不生效**，
    /// 而返回值被 `let _ =` 吞掉 → "刷新目录"静默变成无操作。引导的热/冷两条
    /// 路径都要设目录，第二次设是常态。
    #[test]
    fn the_sample_dir_table_can_be_replaced() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        set_sample_dirs(vec![PathBuf::from("Z:////first")]);
        assert_eq!(sample_dirs(), vec![PathBuf::from("Z:////first")]);
        set_sample_dirs(vec![PathBuf::from("Y:////second"), PathBuf::from("Y:////third")]);
        assert_eq!(
            sample_dirs(),
            vec![PathBuf::from("Y:////second"), PathBuf::from("Y:////third")],
            "第二次 set 必须真的替换掉第一次的值"
        );
        // 复位，免得污染同进程里其他测试的全局状态
        set_sample_dirs(Vec::new());
    }

    /// 扫出来的集合直接驱动判定：扫一次就够，判据不再碰磁盘。
    ///
    /// **这条钉住"判据不碰磁盘"**：删掉磁盘上的包而**不重扫**，结论不变；
    /// 重扫之后才变。用户新放了包要按 F5（`verify_all` 开头会重扫），
    /// 这是显式的刷新时机，不是隐式的。
    #[test]
    fn the_scan_result_drives_the_verdict() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = std::env::temp_dir().join("mvui_samples_scan2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建目录");
        std::fs::write(dir.join("genpin.zip"), b"not a real zip").expect("写占位");

        let g = meta_with("rctycn", "genpin", &["a", "b"]);
        set_sample_dirs(vec![dir.clone()]);
        assert_eq!(scan_sample_sets(&sample_dirs()), 1);
        let row = verify_game_sample(&g, true).expect("出行");
        assert_eq!(row.name, "genpin");
        assert_eq!(
            row.state,
            RomState::Good,
            "内容不合法也不影响 —— 只判包名在不在"
        );

        // 把文件删掉但**不重扫** → 结论不变（判据读的是集合，不是磁盘）
        std::fs::remove_file(dir.join("genpin.zip")).expect("删文件");
        assert_eq!(
            verify_game_sample(&g, true).expect("出行").state,
            RomState::Good,
            "没重扫就不该变 —— 这正是'按 F5 才刷新'的实现"
        );

        // 重扫之后才变
        assert_eq!(scan_sample_sets(&sample_dirs()), 0);
        assert_eq!(
            verify_game_sample(&g, true).expect("出行").state,
            RomState::Missing
        );

        set_sample_dirs(Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `list_have` 现在**没有产品代码调用**（判据改成"包在不在"之后），
    /// 但它仍是对 zip 中央目录读取的正确性样本，留着并在下面跑一遍真实 zip。
    ///
    /// `_sample` 前缀是 MAME 自己给"内部使用样本"的文件名约定 —— 上面
    /// `unrelated_files_are_not_collected` 已经钉住它算样本集（那是磁盘上的
    /// 目录名，与包内条目名是两回事）。
    #[test]
    fn listing_a_real_archive_takes_basenames() {
        // 串行化：本测试改全局集合，与同模块其他测试不能并行
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
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
        assert!(have.contains("c"));
        let _ = std::fs::remove_dir_all(&dir);
        scan_sample_sets(&[]);
    }
}
