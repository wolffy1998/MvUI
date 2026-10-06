//! 解析后的游戏库二进制缓存（origin: MameDat::save/load gamelist.cache）。
//!
//! 文件头：魔数 + format_version + mame_version。mame_version 不一致会
//! 让整份缓存作废（驱动元数据是整体变化的）；format_version 升级走
//! 迁移链。
//!
//! 魔数随程序名从 `旧版魔数` 改成 `MVUICACHE`（两者都是 9 字节，
//! 布局不变）。**改魔数的代价是已有的 gamelist.cache 全部作废**，
//! 首次启动要重跑一次完整 listxml 解析。

use crate::core::library::GameLibrary;
use crate::dlog;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// 缓存文件魔数。9 字节，改了就会让所有旧缓存失效。
pub const MAGIC: &[u8; 9] = b"MVUICACHE";

/// 缓存格式版本。
///
/// **每给 `GameMeta` / `DeviceInfo` / `RomInfo` 加一个字段就要 +1。**
///
/// 这不是形式主义：`bincode` 反序列化时**不认识新增的字段**，而它对"文件
/// 结尾还有多余字节"是宽容的——所以旧缓存会**静默地**读出来，只是新增字段
/// 全部留空。实测踩过：`device_ref` 的解析加上以后，全库 49676 台机器的
/// `GameMeta::devices` 都是空的，"引用设备"面板永远空着，而没有任何报错。
/// 用户看到的现象是"面板缺一段"，不是"缓存坏了"。
///
/// 换 MAME 版本本来就会让缓存重建（版本串变了），但那只在升级 MAME 时发生；
/// 字段变更必须靠这个号，否则同一个 MAME 版本下改代码就永远读旧缓存。
///
/// 3 → 4：`DeviceInfo::is_ref` + `GameMeta::slots`。加 `is_ref` 尤其必须
/// bump —— 判别式从"猜 `kind == instance`"换成读标记位，而旧缓存里这个位
/// 全是false，于是**所有** `<device>` 都会被当成引用设备（正好把上一版刚
/// 修对的东西又弄坏），比留空更难查。
///
/// 4 → 5：**`GameMeta::bios_sets` 现在真的有内容了**（2026-10-06）。
/// `listxml.rs` 里 `<biosset>` 的解析门控 `is_mess || is_bios` 去掉了——
/// 实测它只放行了全库 3655 个带 biosset 的机种中的 42 个。而 Rom 段是按
/// `r.bios.is_empty()` 排除 BIOS 条目的，所以旧缓存里"BIOS 文件既不在 Rom
/// 段、又没有Bios 段可渲染" —— **全库 64576 个 BIOS rom 凭空消失**，
/// 零报错。**不 bump 这个号，热启动会永远读那份残缺缓存**，且没有任何提示。
pub const FORMAT_VERSION: u16 = 5;

/// [`save_library`] 的写缓冲大小。
///
/// `bincode::serialize_into` 对每个字符串、每个长度前缀、每个标量
/// 各发一次 `write`。不加缓冲的话，一份完整 MAME 库（约 5 万机种、
/// 序列化后约 58 MB）会产生几百万次 `WriteFile` 系统调用——实测
/// 约 50 秒；换成 1 MiB 缓冲后约 0.12 秒。冷启动要写两次（见
/// [`save_library`]），所以不带缓冲的版本每换一次 MAME 版本就要多
/// 花大约 100 秒。
///
/// 暴露在 crate 内是为了让回归测试断言的是**写入器实际用的**那个数，
/// 而不是一份可能与之脱节的副本。
pub const WRITE_BUFFER_BYTES: usize = 1 << 20;

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cache file missing")]
    Missing,
    #[error("cache corrupt: {0}")]
    Corrupt(String),
    #[error("cache io: {0}")]
    Io(#[from] std::io::Error),
    #[error("mame version changed (cache: {cache}, mame: {mame})")]
    VersionChanged { cache: String, mame: String },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheData {
    pub mame_version: String,
    pub library: GameLibrary,
    /// false = cached right after -listxml, before the audit ran
    #[serde(default)]
    pub audited: bool,
}

pub fn save(path: &Path, data: &CacheData) -> Result<(), CacheError> {
    save_library(path, &data.mame_version, &data.library, data.audited)
}

/// 直接从一份**借用**的游戏库写缓存。
///
/// `save()` 要求传入 owned 的 `CacheData`，那会迫使每次审计刷新都先
/// 深拷贝整个游戏库；这个版本是就地序列化的。两者产出的布局逐字节
/// 一致（同一个结构、同样的字段顺序）。
///
/// 载荷是**流式**写进临时文件的，而不是先攒进一个 `Vec`：库有约
/// 50 MB 时，老做法会把序列化缓冲和游戏库同时留在内存里
/// （README P2-15）。
///
/// **写必须带缓冲，这不是可有可无的优化。** 冷启动会把这个文件写
/// 两次——审计前一次（`audited = false`），审计后一次
/// （`audited = true`，见 `background.rs::finish_boot`）——所以每换
/// 一次 MAME 版本这个代价都要付两遍。直接往 `File` 上序列化在这台
/// 机器上是每次约 50 秒（49 676 个机种，写出 58 MB）；走 1 MiB
/// 的 `BufWriter` 是约 0.12 秒。别把它"简化"回裸 `File`：这种退化
/// 在只有两个游戏的单测里看不出来，到了用户机器上就是一分钟。
/// 下面的 `saving_coalesces_writes_instead_of_one_syscall_per_field`
/// 是唯一能抓住它的测试。
///
/// [`WRITE_BUFFER_BYTES`] 就是所用的缓冲大小；测试断言的是同一个
/// 常量，所以两边不会脱节。
pub fn save_library(
    path: &Path,
    mame_version: &str,
    library: &GameLibrary,
    audited: bool,
) -> Result<(), CacheError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    dlog!(
        "缓存: 开始写入 {}（{} 台机种, audited={}）",
        path.display(),
        library.len(),
        audited
    );
    let write_t0 = std::time::Instant::now();
    let tmp = path.with_extension("tmp");
    {
        // **写入必须过缓冲。** `bincode::serialize_into` 对每个字符串和
        // 每个长度前缀各发一次 `write`，所以直接往 `File` 上写会把
        // 50 MB 的库变成几百万次极小的 `WriteFile` 调用——那是系统
        // 调用风暴，不是 I/O。本机实测（49 676 个机种，写出 58 MB）：
        // 不带缓冲 50.2 秒，走 1 MiB 缓冲 0.12 秒。
        //
        // tmp + rename 的两步不变，所以文件仍然是原子出现的。
        // `sync_all` 落在真实句柄上而不是缓冲上，因为 `drain_buffer`
        // 在返回前已经把 `File` 交还回来了。
        let f = fs::File::create(&tmp)?;
        // Test-only observer: sits directly on the real handle, *below* the
        // `BufWriter`, so `write_probe::count()` reports the writes that
        // actually reach the OS on the shipped path. Without it a regression
        // that removed the buffer would be invisible to the suite — the
        // remaining tests compare a `BufWriter` they build themselves against
        // a bare writer, which says nothing about what `save_library` does.
        #[cfg(test)]
        let f = write_probe::CountingFile(f);
        let mut f = drain_buffer(io::BufWriter::with_capacity(WRITE_BUFFER_BYTES, f), |w| {
            write_payload(w, mame_version, library, audited)
        })?;
        f.flush()?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    // 耗时值得记：这个文件在冷启动要被写两次（审计前后各一次），而
    // 去掉 BufWriter 会让它从 0.12 秒变成约 50 秒——日志里没有这个
    // 数字的话，那次退化只能靠用户投诉才发现。
    dlog!(
        "缓存: 写入完成 {}，耗时 {:?}",
        path.display(),
        write_t0.elapsed()
    );
    Ok(())
}

/// 把缓存载荷（魔数、版本、正文）序列化进 `w`。
///
/// 从 [`save_library`] 里拆出来，是为了让回归测试能拿**真正的**序列化
/// 器配一个自己选的 `Write`——包括一个会数每次 `write` 的实现。一旦
/// 它被内联回 `save_library`，那个数系统调用的测试就会悄悄不再测
/// 线上代码路径了。
///
/// 参数取 `&mut W` 而不是 `W`，这样带缓冲的写入器可以直接传进来并且
/// 保住自己的缓冲。
fn write_payload<W: Write>(
    w: &mut W,
    mame_version: &str,
    library: &GameLibrary,
    audited: bool,
) -> Result<(), CacheError> {
    #[derive(Serialize)]
    struct Borrowed<'a> {
        mame_version: &'a str,
        library: &'a GameLibrary,
        audited: bool,
    }
    w.write_all(MAGIC)?;
    w.write_all(&FORMAT_VERSION.to_le_bytes())?;
    bincode::serialize_into(
        w,
        &Borrowed {
            mame_version,
            library,
            audited,
        },
    )
    .map_err(|e| CacheError::Corrupt(e.to_string()))?;
    Ok(())
}

/// 让 `f` 对着带缓冲的写入器干活，然后 flush 并把底层句柄交还回来。
///
/// 缓冲类型写进签名是故意的：只有一个调用点，它必须是 `BufWriter`，
/// 而且调用方拿到 `File` 的时候 `into_inner` 已经 flush 过了——所以
/// `save_library` 里少一个 `sync_all` 会是**真的**持久化 bug，而不是
/// 一个无声的空操作。
fn drain_buffer<W: Write, T>(
    mut w: io::BufWriter<W>,
    f: impl FnOnce(&mut io::BufWriter<W>) -> Result<T, CacheError>,
) -> Result<W, CacheError> {
    let out = f(&mut w)?;
    w.flush()?;
    w.into_inner()
        .map_err(|e| CacheError::Io(e.into_error()))
        .map(|inner| {
            // keep `out` alive across the fallible conversion above
            let _ = out;
            inner
        })
}

/// 完全命中返回 Ok(data)；需要重建时返回 `Err(VersionChanged)`。
pub fn load(path: &Path, current_mame_version: &str) -> Result<CacheData, CacheError> {
    let load_t0 = std::time::Instant::now();
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            dlog!("缓存: 不存在 {}（需要冷启动）", path.display());
            return Err(CacheError::Missing);
        }
        Err(e) => return Err(e.into()),
    };
    if bytes.len() < MAGIC.len() + 2 || &bytes[..MAGIC.len()] != MAGIC {
        // 魔数对不上：可能是改名前的旧缓存，也可能是坏文件
        dlog!(
            "缓存: 魔数不匹配 {}（{} 字节）",
            path.display(),
            bytes.len()
        );
        return Err(CacheError::Corrupt("bad magic".into()));
    }
    let ver = u16::from_le_bytes([bytes[MAGIC.len()], bytes[MAGIC.len() + 1]]);
    if ver > FORMAT_VERSION {
        return Err(CacheError::Corrupt(format!("cache version {ver} is newer")));
    }
    if ver < FORMAT_VERSION {
        // 未来做迁移时挂钩子的地方：v_old -> v_new
        return Err(CacheError::Corrupt(format!("cache version {ver} too old")));
    }
    let mut data: CacheData = bincode::deserialize(&bytes[MAGIC.len() + 2..])
        .map_err(|e| CacheError::Corrupt(e.to_string()))?;
    if data.mame_version != current_mame_version {
        dlog!(
            "缓存: mame 版本变了（缓存 {}, 当前 {}）→ 整库重建",
            data.mame_version,
            current_mame_version
        );
        return Err(CacheError::VersionChanged {
            cache: data.mame_version,
            mame: current_mame_version.to_string(),
        });
    }
    // index 是 #[serde(skip)] —— 不重建的话热启动时每次 get/get_idx
    // 都返回 None，半个界面会变成死的
    data.library.rebuild_indexes();
    dlog!(
        "缓存: 命中 {}（{} 台机种, audited={}, {} 字节, 读+反序列化耗时 {:?}）",
        path.display(),
        data.library.len(),
        data.audited,
        bytes.len(),
        load_t0.elapsed()
    );
    Ok(data)
}

/// 统计真正落到文件句柄上的 `write` 次数。
///
/// 只在 `cfg(test)` 下编译，而且只会被**包在** [`save_library`] 的句柄
/// 外面，所以 release 构建既不带这个类型也不带那个原子量。意义在于让
/// 缓冲变得可观测——`BufWriter` 的其他方面就算被重构掉，测试套件照样
/// 全绿，因为别的测试是自己造一个带缓冲的写入器，而不是在测应用程序
/// 真正在用的那条路径。
#[cfg(test)]
pub(crate) mod write_probe {
    use std::io::{self, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static WRITES: AtomicUsize = AtomicUsize::new(0);

    pub fn reset() {
        WRITES.store(0, Ordering::Relaxed);
    }

    pub fn count() -> usize {
        WRITES.load(Ordering::Relaxed)
    }

    /// 一个在转发之前先给每次 `write` 记数的 `File`。
    pub struct CountingFile(pub std::fs::File);

    impl Write for CountingFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            WRITES.fetch_add(1, Ordering::Relaxed);
            self.0.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.flush()
        }
    }

    /// `save_library` 会对 `drain_buffer` 交还的句柄调 `sync_all`，
    /// 所以这层壳也得把持久化调用转发过去——但不计数，因为那是
    /// `FlushFileBuffers`，不是 `write`。
    impl CountingFile {
        pub fn sync_all(&self) -> io::Result<()> {
            self.0.sync_all()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::GameMeta;

    #[test]
    fn roundtrip() {
        let dir = std::env::temp_dir().join("mvui-test-cache");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("library.bin");
        let mut lib = GameLibrary::new("0.261".into());
        lib.push(GameMeta {
            name: "pacman".into(),
            description: "Pac-Man".into(),
            ..Default::default()
        });
        lib.rebuild_indexes();
        let data = CacheData {
            mame_version: "0.261".into(),
            library: lib,
            audited: true,
        };
        save(&p, &data).unwrap();
        // 热启动读回来必须带着可用的索引
        let back = load(&p, "0.261").unwrap();
        assert_eq!(back.library.get_idx("pacman"), Some(0));
        assert!(load(&p, "0.262").is_err());
        assert!(matches!(load(&p, "0.262"), Err(CacheError::VersionChanged { .. })));
        let _ = std::fs::remove_file(&p);
    }

    /// 守住 `save_library` 里的那个 `BufWriter`。
    ///
    /// 两个游戏的往返测试抓不到这个退化：不带缓冲的版本一样**正确**，
    /// 只是慢得灾难性，因为 bincode 对每个字符串和长度前缀各发一次
    /// `write`。参考机实测（49 676 个机种，写出 58 MB）：不带缓冲约
    /// 50 秒，走 1 MiB 缓冲约 0.12 秒，而冷启动要付**两次**，因为
    /// `finish_boot` 在审计前后各写一次缓存。
    ///
    /// 故意**不**断言的：墙钟时间上限。它是最容易想到的手段，在这里
    /// 却没用——0.12 秒和 50 秒的差距只在完整的 5 万机种库上才显现，
    /// 而任何小到能在单测里造出来的库，不带缓冲也跑得完。改成数
    /// `write` 次数，测的是同一个缺陷，却不依赖机器有多快。
    ///
    /// 四条断言，按"真正能保护多少"递增排列：
    ///
    /// 1. 缓冲容量常量仍然大到值得存在；
    /// 2. 机制成立——[`write_payload`] 每个字段发一次 `write`，
    ///    而 `BufWriter<WRITE_BUFFER_BYTES>` 把它们合并成几次；
    /// 3. `save_library` 写出的文件仍然能读回来；
    /// 4. **`save_library` 自己只发几次写** —— 通过 [`write_probe`]
    ///    包住真实句柄来测。这条才是真正咬人的：把 `save_library`
    ///    里的 `BufWriter` 删掉，(1)–(3) 全都照样通过，因为它们玩的
    ///    是测试自己造的那个缓冲。
    #[test]
    fn saving_coalesces_writes_instead_of_one_syscall_per_field() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        // (1) 常量本身
        assert!(
            WRITE_BUFFER_BYTES >= 64 * 1024,
            "写缓冲只有 {WRITE_BUFFER_BYTES} 字节，逐字段的系统调用风暴会回来"
        );

        /// 统计真正到达底层句柄的 `write` 次数。
        struct Counting<W: Write> {
            inner: W,
            calls: Arc<AtomicUsize>,
        }
        impl<W: Write> Write for Counting<W> {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                self.inner.write(buf)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.inner.flush()
            }
        }

        let lib = sample_library(400);

        // 两条支路，同一个序列化器：都是应用程序真正调用的那个
        // [`write_payload`]，一次直接打在计数句柄上，一次走缓冲。
        // 计数壳必须待在缓冲**下面**，这样数的是系统调用而不是
        // bincode 打进缓冲的调用——放错边的话两条支路报出一样的数，
        // 下面的比值断言就成了空过。
        let count = |buffered: bool| -> usize {
            let calls = Arc::new(AtomicUsize::new(0));
            let shim = Counting {
                inner: std::io::sink(),
                calls: calls.clone(),
            };
            if buffered {
                let mut w = io::BufWriter::with_capacity(WRITE_BUFFER_BYTES, shim);
                write_payload(&mut w, "0.261", &lib, true).unwrap();
            } else {
                let mut shim = shim;
                write_payload(&mut shim, "0.261", &lib, true).unwrap();
            }
            calls.load(Ordering::Relaxed)
        };

        let buffered_calls = count(true);
        let raw_calls = count(false);

        // (2) 机制。`raw_calls` 先证明计数壳确实观察到了逐字段的写；
        // 没有它，比值可能因为错误的原因而通过（比如某次重构让
        // `write_payload` 一次写完整个结构体——那本身没问题，但会让
        // 这个测试变得没有意义）。
        assert!(
            raw_calls > 1000,
            "计数壳没有观察到逐字段的写（raw={raw_calls}），下面的比较将没有意义"
        );
        assert!(
            buffered_calls * 50 < raw_calls,
            "缓冲没有把写合并掉: buffered={buffered_calls} raw={raw_calls}"
        );
        // 400 个机种序列化后远不到 1 MB，所以 1 MiB 的缓冲不该溢出；
        // 给文件头和对齐留出充裕余量。
        assert!(
            buffered_calls <= 8,
            "预期只有几次大块写，实际 {buffered_calls} 次"
        );

        // (3) **线上**路径，实测而非推断。这条才是真正守住
        // `save_library` 的断言：把那里的 `BufWriter` 删掉，这个数会从
        // 个位数跳到约 19 000，而上面那些比值断言全都照样通过。
        let dir = std::env::temp_dir().join("mvui-test-writecount");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("real.bin");
        write_probe::reset();
        save_library(&p, "0.261", &lib, true).unwrap();
        let shipped = write_probe::count();

        assert!(
            shipped <= 8,
            "save_library 为 {} 个机种发了 {shipped} 次写；\
             BufWriter 被删掉或被绕过了",
            400
        );

        // 而且它产出的文件仍然能读回来，索引也重建好了
        let back = load(&p, "0.261").unwrap();
        assert!(back.audited);
        assert!(back.library.get_idx("machine0000").is_some());
        let _ = std::fs::remove_file(&p);
    }

    /// 带缓冲的写入器必须产出与不带缓冲时**逐字节相同**的结果，
    /// 否则所有已存在的 `gamelist.cache` 都会读不出来。
    ///
    /// 两条支路都走 [`write_payload`]，所以比的是真实布局，而不是手搓
    /// 的仿制品。
    #[test]
    fn buffered_output_is_byte_identical_to_unbuffered() {
        let lib = sample_library(50);

        let mut buffered_bytes = Vec::new();
        {
            let mut w = io::BufWriter::new(&mut buffered_bytes);
            write_payload(&mut w, "0.261", &lib, false).unwrap();
            w.flush().unwrap();
        }

        let mut raw_bytes = Vec::new();
        write_payload(&mut raw_bytes, "0.261", &lib, false).unwrap();

        assert_eq!(
            buffered_bytes, raw_bytes,
            "BufWriter changed the serialized layout"
        );
        // 而且它仍然是一份合法的缓存，文件头也在
        assert_eq!(&buffered_bytes[..MAGIC.len()], MAGIC);
        assert_eq!(
            u16::from_le_bytes([buffered_bytes[MAGIC.len()], buffered_bytes[MAGIC.len() + 1]]),
            FORMAT_VERSION
        );
    }

    /// 给 `GameMeta` 加了字段之后，**旧缓存必须读不出来**（而不是静默地
    /// 读出空字段）。
    ///
    /// 这是本文件最容易踩且最难发现的坑：`bincode` 对"结构体多了字段"
    /// 完全宽容——反序列化读完已知字段就停，剩下没读掉的字节被忽略，
    /// 新字段留空。实测踩过：`device_ref` 解析加上以后，全库 49676 台的
    /// `devices` 全是空的，"引用设备"面板永远空着，而**没有任何报错**。
    ///
    /// 所以这个测试不能只断言"写出去的版本号等于常量"（换号它照样过），
    /// 必须断言**换掉版本号真的会作废旧文件**。
    #[test]
    fn a_stale_format_version_is_rejected_not_silently_accepted() {
        let dir = std::env::temp_dir().join("mvui-cache-ver-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("gamelist.cache");

        let mut lib = GameLibrary::new("0.261".into());
        lib.push(GameMeta {
            name: "pacman".into(),
            devices: vec![crate::core::model::DeviceInfo {
                kind: "z80".into(),
                instance: "z80".into(),
                tag: "maincpu".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        save_library(&p, "0.261", &lib, true).unwrap();

        // 现行版本读得回来，且字段没丢
        let back = load(&p, "0.261").expect("现行版本应当可读");
        assert_eq!(back.library.games[0].devices.len(), 1);

        // 伪造一份"上一个版本号"的缓存：只改文件头那两字节，载荷一模一样
        let mut bytes = fs::read(&p).unwrap();
        bytes[MAGIC.len()..MAGIC.len() + 2].copy_from_slice(&(FORMAT_VERSION - 1).to_le_bytes());
        let stale = dir.join("stale.cache");
        fs::write(&stale, &bytes).unwrap();

        assert!(
            load(&stale, "0.261").is_err(),
            "旧格式版本必须被拒绝：读成功意味着新增字段会被静默留空"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// 造一个足够大的库，让两种写策略的差距肉眼可见。上面几个测试共用。
    fn sample_library(n: usize) -> GameLibrary {
        let mut lib = GameLibrary::new("0.261".into());
        for i in 0..n {
            lib.push(GameMeta {
                name: format!("machine{i:04}"),
                description: format!("Description number {i} with some length to it"),
                manufacturer: "Acme".into(),
                ..Default::default()
            });
        }
        lib.rebuild_indexes();
        lib
    }
}
