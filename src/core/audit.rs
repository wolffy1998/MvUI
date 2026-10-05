//! ROM 审计，1:1 移植自 audit.cpp 的 RomAuditor（内部审计 + 主机
//! 扫描 + Logiqx fixdat 导出，含那 4 种导出方式）。
//!
//! 可用性存在数据模型里（每个 rom/disk 的 `available` + 游戏的
//! `available`），与原版 GameInfo 的字段对应。

use crate::core::archive::{self, is_7z, is_archive, is_zip};
use crate::core::library::GameLibrary;
use crate::core::model::*;
use crate::dlog;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// 保留的归档清单条目上限。
///
/// 约 4.4 万个 romsets 序列化后大约 20 MB；取 6.4 万是为了给异常大
/// 或者多 rompath 的收藏留出余量，同时不让病态配置把缓存无限撑大。
/// 超上限时丢弃条目（下次扫描会重建），而不是拒绝保存。
const AUDIT_CACHE_LIMIT: usize = 64 * 1024;

/// 对应旧版 AUDIT_ONLY=0 / EXPORT_COMPLETE / EXPORT_ALL /
/// EXPORT_INCOMPLETE / EXPORT_MISSING。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditMethod {
    Only,
    ExportComplete,
    ExportAll,
    ExportIncomplete,
    ExportMissing,
}

#[derive(Clone)]
pub struct AuditHandle {
    pub cancel: Arc<AtomicBool>,
    /// 一轮跑完就置上，这样即使一个都没扫（total == 0），转发进度
    /// 的线程也能退出。
    pub finished: Arc<AtomicBool>,
    /// 已完成单元数 / 总单元数。
    ///
    /// 这两个曾经是一个 `Mutex<(usize, usize, String)>`，在扫描循环里
    /// 用**循环下标**写入——但那个循环跑在 rayon 下，五个工作线程各
    /// 存各的"已完成"，进度条于是在几个不相干的位置之间乱跳。拆成
    /// 原子量既更便宜，而且天然单调：某个工作线程不可能在发布
    /// "41 完成"之后再发布"37 完成"。
    progress: Arc<(AtomicUsize, AtomicUsize)>,
    /// 当前正在扫什么的尽力而为的标签。天生有竞态——同时有多个单元
    /// 在飞——所以故意和计数器分开，不塞进同一把锁里。
    current: Arc<Mutex<String>>,
    /// Test-only：观察 `set_enumerating` 的每次调用。
    ///
    /// 不用它就测不到枚举阶段——`audit_all` 返回前那行
    /// `set_progress(0, 0, "")` 会把标签清空（那是故意的，好让转发线程最后
    /// 观察到"已完成"），所以跑完之后句柄里只剩空串。
    #[cfg(test)]
    enum_observer: Arc<std::sync::OnceLock<Box<dyn Fn(usize, usize, usize)>>>,
}

impl AuditHandle {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
            progress: Arc::new((AtomicUsize::new(0), AtomicUsize::new(0))),
            current: Arc::new(Mutex::new(String::new())),
            #[cfg(test)]
            enum_observer: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Test-only：装上枚举阶段的观察者。
    #[cfg(test)]
    fn on_enumerating(&self, f: impl Fn(usize, usize, usize) + 'static) {
        let _ = self.enum_observer.set(Box::new(f));
    }
    pub fn snapshot(&self) -> (usize, usize, String) {
        let (done, total) = &*self.progress;
        (
            done.load(Ordering::Relaxed),
            total.load(Ordering::Relaxed),
            self.current.lock().unwrap().clone(),
        )
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }
    pub fn finish(&self) {
        self.finished.store(true, Ordering::Relaxed);
    }
    /// Set the denominator and reset the numerator — called once the unit list
    /// is known.
    fn set_total(&self, total: usize) {
        self.progress.1.store(total, Ordering::Relaxed);
        self.progress.0.store(0, Ordering::Relaxed);
    }
    /// Report one unit finished. Named `_done` rather than `set_progress`
    /// because "here is the next position" is exactly the call shape that broke
    /// the old version.
    fn unit_done(&self, label: &str) {
        self.progress.0.fetch_add(1, Ordering::Relaxed);
        *self.current.lock().unwrap() = label.to_string();
    }
    /// 报"正在枚举待扫单元"，即 `total` 还不存在的那一段。
    ///
    /// 这是一个**独立于 `progress` 的阶段**，不是把 `total` 提前设成 1：
    /// 枚举期间把分母设成任何猜测值都会让 UI 画出一个假的百分比，而
    /// 百分比一旦出现又消失，比没有更糟。所以这里只报"第几个 rompath、
    /// 已经收到多少单元"这种**绝对计数**，让状态栏在分母出现之前也有
    /// 东西在动。
    ///
    /// 不用新原子量：这段是纯串行的（枚举就在 `audit_all` 的主线程上），
    /// 而 `current` 本来就是"尽力而为的标签"、天生有竞态也不影响正确性。
    /// 复用它零成本，且不必让 `AuditProgress` 事件多带一个字段。
    pub fn set_enumerating(&self, done_dirs: usize, total_dirs: usize, units: usize) {
        #[cfg(test)]
        if let Some(obs) = self.enum_observer.get() {
            obs(done_dirs, total_dirs, units);
        }
        *self.current.lock().unwrap() =
            format!("enum {done_dirs}/{total_dirs} dirs, {units} units");
    }
    /// Stage-level progress for the console (MESS) pass, which is sequential.
    fn set_progress(&self, done: usize, total: usize, current: &str) {
        self.progress.1.store(total, Ordering::Relaxed);
        self.progress.0.store(done, Ordering::Relaxed);
        *self.current.lock().unwrap() = current.to_string();
    }
}

impl Default for AuditHandle {
    fn default() -> Self {
        Self::new()
    }
}

enum Mark {
    Rom(usize, usize, bool),
    Disk(usize, usize),
}

/// Scan every audit unit, returning the marks it produced.
///
/// **Deliberately sequential.** The previous version used
/// `units.par_iter()` over four rayon workers and measured *slower* than one
/// thread (52 zips/s sequential vs 37 with four) on this machine: the units are
/// independent files spread across a spinning disk, so four readers just make
/// the head thrash and every unit pays an extra seek. The work is I/O-bound and
/// the drive is the bottleneck, not the CPU.
///
/// Progress is reported by *incrementing a counter when a unit finishes*, never
/// by publishing the loop index — see [`AuditHandle`]. Sequential iteration
/// makes that ordering trivially correct, but the counter form is what the
/// status bar needs either way, and it stays correct if parallelism is ever
/// reintroduced for an SSD.
fn scan_units(units: &[(PathBuf, usize)], lib: &GameLibrary, handle: &AuditHandle) -> Vec<Vec<Mark>> {
    let mut results: Vec<Vec<Mark>> = Vec::with_capacity(units.len());
    for (path, gi) in units {
        if handle.cancelled() {
            break;
        }
        let cur = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut marks: Vec<Mark> = Vec::new();
        if path.is_dir() {
            // CHD dir scan — only disk marks derived here (read-only on lib)
            let game_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if let Some(idx) = lib.get_idx(&game_name) {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            continue;
                        }
                        let fname = p
                            .file_name()
                            .map(|n| n.to_string_lossy().to_lowercase())
                            .unwrap_or_default();
                        if fname.ends_with(".chd") {
                            let stem = archive::file_stem(&fname);
                            if let Some(di) = lib.games[idx]
                                .disks
                                .iter()
                                .position(|d| d.name.to_lowercase() == stem)
                            {
                                marks.push(Mark::Disk(idx, di));
                            }
                        } else if let Some(ri) = lib.games[idx].roms.iter().position(|r| {
                            archive::file_stem(&r.effective_name().to_lowercase())
                                == archive::file_stem(&fname)
                        }) {
                            marks.push(Mark::Rom(idx, ri, true));
                        }
                    }
                }
            }
        } else if let Some(entries) = crate::core::audit_cache::list_cached(path) {
            // origin: RomAuditor::run — a `<game>.zip` is matched only against
            // that game's roms and its clone family, never the whole library.
            // Using the global crc index here was both wrong and pathological:
            // a crc shared by thousands of sets (bios / device roms) produced
            // thousands of marks per entry and the audit never finished.
            let mut table: HashMap<u32, Vec<(usize, usize)>> = HashMap::new();
            {
                let g = &lib.games[*gi];
                for (ri, r) in g.roms.iter().enumerate() {
                    if !r.is_nodump() {
                        table.entry(r.crc).or_default().push((*gi, ri));
                    }
                }
                for clone in &g.clones {
                    let Some(ci) = lib.get_idx(clone) else { continue };
                    for (ri, r) in lib.games[ci].roms.iter().enumerate() {
                        if !r.is_nodump() {
                            table.entry(r.crc).or_default().push((ci, ri));
                        }
                    }
                }
            }
            for e in &entries {
                let Some(crc) = e.crc else { continue };
                for (gj, ri) in table.get(&crc).map(|v| v.as_slice()).unwrap_or(&[]) {
                    marks.push(Mark::Rom(*gj, *ri, true));
                }
            }
        }
        handle.unit_done(&cur);
        results.push(marks);
    }
    results
}

/// internal audit (origin: RomAuditor::run) — mutates per-rom/disk availability
/// and game.available; console scan appends ext roms into the library.
/// Marks the audit finished on *any* exit path, including unwinding.
/// origin: the Qt version always emitted `finished()` from the auditor thread.
struct FinishOnDrop(Arc<AtomicBool>);

impl Drop for FinishOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

pub fn audit_all(
    lib: &mut GameLibrary,
    rom_paths: &[PathBuf],
    extra_software: &HashMap<String, String>,
    handle: &AuditHandle,
) {
    // 从这里往后的任何情况——正常返回、提前退出，还是归档层/utf8
    // 之类的地方 panic——句柄都必须变成 finished，否则那个 200 ms
    // 的进度转发线程永远不退出，会一直往 channel 里灌（README P2-19）。
    let _finish_guard = FinishOnDrop(handle.finished.clone());

    let audit_t0 = std::time::Instant::now();
    dlog!(
        "审计: 开始（{} 台机种, {} 个 rompath）",
        lib.len(),
        rom_paths.len()
    );

    // 把上一轮记住的归档清单拉进来。这就是 10–20 分钟和几秒钟的
    // 区别：成本在于打开 4.4 万个 zip，而清单只在文件变了才变。
    let cache_t0 = std::time::Instant::now();
    crate::core::audit_cache::load();
    dlog!(
        "审计: 归档清单缓存载入完成（{} 条记录），耗时 {:?}",
        crate::core::audit_cache::len(),
        cache_t0.elapsed()
    );

    // 1) 重置：nodump 视为 available
    for g in &mut lib.games {
        for r in &mut g.roms {
            r.available = r.is_nodump();
        }
        for d in &mut g.disks {
            d.available = d.is_nodump();
        }
        g.available = GAME_MISSING;
    }

    // sha1 → every (game, disk) sharing that CHD. Built once so a disk hit
    // propagates in O(1); the port used to rescan the whole library per hit,
    // which is O(disks x games) (origin: RomAuditor::run clone propagation).
    let mut disk_index: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for (gi, g) in lib.games.iter().enumerate() {
        for (di, d) in g.disks.iter().enumerate() {
            if !d.sha1.is_empty() {
                disk_index.entry(d.sha1.clone()).or_default().push((gi, di));
            }
        }
    }

    // 枚举待扫单元。这一步要把每个 rompath 整目录读一遍（`read_dir`），
    // 4.4 万个条目在机械盘上是**整整一段静默期**——在这期间
    // `progress.1` 还是 0，状态栏只能打"正在审计"而给不出百分比。
    // 分母只有枚举完才知道，所以这不是可以"顺手修好"的东西，
    // 而是一次审计里"前若干秒没有百分比"的全部原因。
    let enum_t0 = std::time::Instant::now();
    let mut units: Vec<(PathBuf, usize)> = Vec::new();
    for (di, dir) in rom_paths.iter().enumerate() {
        // 每个 rompath 报一次：分母还不存在，但"第几个目录 / 已收多少单元"
        // 是真的，状态栏靠它证明自己还在动。
        handle.set_enumerating(di, rom_paths.len(), units.len());
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if let Some(gi) = lib.get_idx(&name) {
                    units.push((p, gi));
                }
            } else if is_zip(&p) || is_7z(&p) {
                let name = p
                    .file_stem()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if let Some(gi) = lib.get_idx(&name) {
                    units.push((p, gi));
                }
            }
        }
    }

    handle.set_total(units.len());
    dlog!(
        "审计: 待扫单元 {} 个（枚举耗时 {:?}）——分母此刻才确定，状态栏从这条日志之后才开始有百分比",
        units.len(),
        enum_t0.elapsed()
    );

    let scan_t0 = std::time::Instant::now();
    let results = scan_units(&units, lib, handle);
    dlog!(
        "审计: 单元扫描完成（{} 个单元, 耗时 {:?}）",
        units.len(),
        scan_t0.elapsed()
    );

    // 所有被打开过的单元的归档清单现在已经记下来了。先清掉那些文件
    // 已经消失的条目，再把整份交给持久化缓存，这样**下一次**审计——
    // 哪怕重启过——就退化成一次 stat 遍历。放在下面那个 parent/BIOS
    // 回填之前是故意的：那里万一 panic，也不会把一份好好的清单缓存
    // 一起丢掉。
    {
        let paths: Vec<PathBuf> = units.iter().map(|(p, _)| p.clone()).collect();
        crate::core::audit_cache::prune(&paths, AUDIT_CACHE_LIMIT);
        crate::core::audit_cache::save();
    }

    // 应用标记；disk 的标记会传播给共用同一 sha1 的克隆
    let mut disk_marks: Vec<(usize, usize)> = Vec::new();
    for marks in results {
        for m in marks {
            match m {
                Mark::Rom(gi, ri, ok) => {
                    lib.games[gi].roms[ri].available = ok || lib.games[gi].roms[ri].available;
                }
                Mark::Disk(gi, di) => disk_marks.push((gi, di)),
            }
        }
    }
    // 一个 CHD 由整个克隆家族共用——把所有引用同一 sha1 的游戏都标上
    // （origin: RomAuditor::run 里的那个克隆循环）
    for (gi, di) in &disk_marks {
        let sha1 = lib.games[*gi].disks[*di].sha1.clone();
        if sha1.is_empty() {
            lib.games[*gi].disks[*di].available = true;
            continue;
        }
        if let Some(same) = disk_index.get(&sha1) {
            for (cj, di2) in same {
                lib.games[*cj].disks[*di2].available = true;
            }
        } else {
            lib.games[*gi].disks[*di].available = true;
        }
    }

    // 3) 逐游戏定级（origin 第 6 步）
    let mut parent_maps: HashMap<usize, HashMap<u32, usize>> = HashMap::new();
    for i in 0..lib.games.len() {
        if lib.games[i].is_ext_rom {
            lib.games[i].available = GAME_COMPLETE;
            continue;
        }
        let romof = lib.games[i].romof.clone();
        // crc → 父集里的 rom 槽位，做了记忆化：这个兜底路径原本要为
        // 每一个缺失的 rom 线性扫一遍父集的 rom 列表
        if !romof.is_empty() {
            let parent = lib.get_idx(&romof);
            let grand = parent.and_then(|p| {
                let gp_name = lib.games[p].romof.clone();
                if gp_name.is_empty() {
                    None
                } else {
                    lib.get_idx(&gp_name)
                }
            });
            for ri in 0..lib.games[i].roms.len() {
                if lib.games[i].roms[ri].available {
                    continue;
                }
                let crc = lib.games[i].roms[ri].crc;
                let mut ok = false;
                for p in [parent, grand].iter().flatten() {
                    let map = parent_maps.entry(*p).or_insert_with(|| {
                        lib.games[*p]
                            .roms
                            .iter()
                            .enumerate()
                            .map(|(k, r)| (r.crc, k))
                            .collect()
                    });
                    if let Some(&pri) = map.get(&crc) {
                        if lib.games[*p].roms[pri].available {
                            ok = true;
                            break;
                        }
                    }
                }
                if ok {
                    lib.games[i].roms[ri].available = true;
                }
            }
        }
        // origin: audit.cpp:468-514 —— 等级必须根据**最终**的 rom /
        // disk 状态来判定。1.8.2 读 `allinParent` 的时候，克隆集扫描
        // 已经把父集的 ROM 传播进克隆了，所以一个 ROM 全部来自父集
        // 的克隆算作完整；在这里重算是等价的，而且它还覆盖了散装
        // 文件 / 目录那条路径——那条路径上扫描什么都不传播，只有
        // 这个回填能补齐最后缺的几个 ROM。
        let complete = lib.games[i].roms.iter().all(|r| r.available)
            && lib.games[i].disks.iter().all(|d| d.available);
        lib.games[i].available = if complete { GAME_COMPLETE } else { GAME_MISSING };
    }

    // 4) 主机（MESS）审计 —— 会创建 ext rom
    let console_names: Vec<String> = lib
        .games
        .iter()
        .filter(|g| !g.devices.is_empty() && !g.is_ext_rom)
        .map(|g| g.name.clone())
        .collect();
    let total_consoles = console_names.len();
    let console_t0 = std::time::Instant::now();
    let mut consoles_done = 0usize;
    for (ci, console) in console_names.iter().enumerate() {
        let Some(dirpath) = extra_software.get(console) else {
            continue;
        };
        if dirpath.is_empty() || !Path::new(dirpath).exists() {
            continue;
        }
        handle.set_progress(ci, total_consoles, console);
        dlog!(
            "审计: 主机（console）扫描 {}/{} — {} → {}",
            ci + 1,
            total_consoles,
            console,
            dirpath
        );
        audit_console(lib, console, dirpath);
        consoles_done += 1;
    }
    dlog!(
        "审计: 主机扫描完成（{} 台机种, 处理 {} 个主机目录, 耗时 {:?}）",
        total_consoles,
        consoles_done,
        console_t0.elapsed()
    );
    // 在 `finish()` **之前**清掉阶段标签并停表，这样转发线程最后观察到
    // 的状态是"已完成"，而不是跑到一半时留下的过期百分比。UI 是靠
    // `is_finished` 来把进度条整个撤掉的，所以在这里归零不会让它卡在
    // 100%。
    handle.set_progress(0, 0, "");
    handle.finish();

    lib.complete_data();

    // 统计一下结果，便于在 boot.log 里对照界面上看到的数字
    let complete = lib.games.iter().filter(|g| g.available == GAME_COMPLETE).count();
    dlog!(
        "审计: 完成（{} 台中 {} 台完整），总耗时 {:?}",
        lib.len(),
        complete,
        audit_t0.elapsed()
    );
}

/// 一台机种在单游戏审计里的**审计范围**：它自己 + 它依赖的一切。
///
/// 用户口径：「就是此 ROM + 依赖的主 ROM 文件 + BIOS + Device + Samples
/// + CHD」。这五类依赖在数据模型里落在五个不同地方，凑齐它们是
/// [`audit_scope`] 唯一要解决的问题。
#[derive(Debug, Clone, Default)]
pub struct AuditScope {
    /// 本机种。
    pub game: usize,
    /// `romof` 父集与祖父集（依赖的主 ROM 文件）。
    pub parents: Vec<usize>,
    /// 被引用的设备机种（BIOS 不是独立 machine，见 `core::rominfo`）。
    pub devices: Vec<usize>,
    /// 样本机种（`sampleof`）。
    pub samples: Vec<usize>,
}

impl AuditScope {
    /// 范围内的全部机种索引，本体在前。
    pub fn all(&self) -> Vec<usize> {
        let mut v = vec![self.game];
        v.extend(self.parents.iter().copied());
        v.extend(self.devices.iter().copied());
        v.extend(self.samples.iter().copied());
        v
    }

    pub fn len(&self) -> usize {
        1 + self.parents.len() + self.devices.len() + self.samples.len()
    }
}

/// 算出一台机种的审计范围。
///
/// **BIOS 不在这里**：查真实的 `mame pgm -listxml`，BIOS 集是同一个
/// `<machine>` 上的 `<biosset>` 标签，它的 rom 带着 `bios="v2"` 属性**混在
/// 本机种的 `<rom>` 列表里**——没有独立的机种可扫。审计本机种的归档时那些
/// 条目自然一起被匹配到，所以 BIOS 不需要额外处理。
///
/// 设备机种名在 `DeviceInfo::kind` / `instance` 上（`core/listxml.rs` 的
/// `device_ref` 解析把 `name` 属性填进这两处），`tag` 是父机种里的标签全名
/// （`maincpu`），**不能**拿去查库。
pub fn audit_scope(lib: &GameLibrary, game: &str) -> Option<AuditScope> {
    let gi = lib.get_idx(game)?;
    let g = &lib.games[gi];
    let mut scope = AuditScope {
        game: gi,
        ..Default::default()
    };

    // 依赖的主 ROM 文件：romof 链，带防环（坏 dat 是外部输入）
    let mut visited: std::collections::HashSet<usize> = std::collections::HashSet::new();
    visited.insert(gi);
    let mut cursor = g.romof.as_str();
    let mut hops = 0;
    while !cursor.is_empty() && hops < 8 {
        let Some(pi) = lib.get_idx(cursor) else { break };
        if !visited.insert(pi) {
            break;
        }
        scope.parents.push(pi);
        cursor = lib.games[pi].romof.as_str();
        hops += 1;
    }

    // 引用设备
    for d in &g.devices {
        let name = if !d.kind.is_empty() {
            d.kind.as_str()
        } else if !d.instance.is_empty() {
            d.instance.as_str()
        } else {
            continue;
        };
        let Some(di) = lib.get_idx(name) else { continue };
        if di != gi && visited.insert(di) {
            scope.devices.push(di);
        }
    }

    // 样本
    if !g.sampleof.is_empty() {
        if let Some(si) = lib.get_idx(&g.sampleof) {
            if si != gi && visited.insert(si) {
                scope.samples.push(si);
            }
        }
    }
    Some(scope)
}

/// 按名字在 rompath 里直接定位归档，**不做整目录枚举**。
///
/// 单游戏审计的旧写法是对每个 rompath 做一次 `read_dir`，把 4.4 万个
/// 条目逐个 `is_dir()` 一遍，只为了挑出其中 2~5 个。实测冷盘 13.3 s、
/// 热缓存 5.8 s——这就是「审一个游戏要等 20 秒」的全部原因，而代价里
/// 没有一分钱是花在真正读归档上的。
///
/// 这里换成**已知名字、反查路径**：范围里每台机种试
/// `dir/<名>`（松散目录形态）、`dir/<名>.zip`、`dir/<名>.7z` 三种候选，
/// 大约 30 次 `stat` 取代 44387 次，命中范围完全等价。
///
/// 保留的两条旧语义：
/// - 同一台机种在**多个 rompath** 下都有包时都算数（去重按完整路径）。
/// - 目录形态取整个 `file_name` 当名字，归档形态取 `file_stem`——与
///   1.8.2 枚举时的取值口径一致。
///
/// 大小写：Windows/macOS 文件系统不敏感，磁盘上是 `GTMRUSA.ZIP` 用小写拼
/// 也能找到。但正因为不敏感，**两种拼法会命中同一个文件**——所以去重必须
/// 比路径而不是比字符串，否则同一个包会被扫两遍（白读一遍归档，还让
/// `audit_cache` 里多一条一模一样的记录）。这里用 `canonicalize` 把两条
/// 路径收敛成同一个再比。
pub fn find_units_for(lib: &GameLibrary, gis: &[usize], rom_paths: &[PathBuf]) -> Vec<(PathBuf, usize)> {
    let mut units: Vec<(PathBuf, usize)> = Vec::new();
    // 见过哪些**真实文件**——Windows 上大小写不敏感，两种拼法会撞上同一个
    // 包，按路径字符串去重是去不掉的，必须先 canonicalize 再比。
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for &gi in gis {
        // 越界的 gi 要挡住而不是 panic：`audit_scope` 给的是合法下标，但
        // 这个函数是 pub 的，headless 例子与将来的调用方未必都守约。
        let Some(g) = lib.games.get(gi) else {
            continue;
        };
        let name = &g.name;
        // 库名可能已经是小写，也可能不是（`1943` / `PCB` 之类）。
        // 两种拼法各试一遍，在不敏感的文件系统上只是多几次 stat。
        let mut spellings: Vec<String> = vec![name.to_lowercase()];
        let exact = name.clone();
        if !spellings.iter().any(|s| *s == exact) {
            spellings.push(exact);
        }
        for dir in rom_paths {
            for sp in &spellings {
                // 松散目录形态：`roms/game/gtmr/`
                let as_dir = dir.join(sp);
                if as_dir.is_dir() && seen.insert(real_path(&as_dir)) {
                    units.push((as_dir, gi));
                }
                // 归档形态：`gtmr.zip` / `gtmr.7z`
                for p in [dir.join(format!("{sp}.zip")), dir.join(format!("{sp}.7z"))] {
                    if p.is_file() && is_archive(&p) && seen.insert(real_path(&p)) {
                        units.push((p, gi));
                    }
                }
            }
        }
    }
    units
}

/// 用来做去重的"真实路径"。
///
/// `canonicalize` 会解析 `..`、大小写（Windows 上）和符号链接，所以
/// `GTMRUSA.zip` 与 `gtmrusa.zip` 收敛成同一个值。失败（文件刚被删、
/// 无权限）时退回原路径：宁可漏一次去重，也不要把整个查找变成失败。
fn real_path(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// 只审计一台机种 + 它的依赖（右键/菜单「审计 ROM」）。
///
/// 与 [`audit_all`] 的差别不只是范围小，有三处必须不同：
///
/// 1. **只重置范围内的 `available`。** `audit_all` 那个重置循环扫 5 万台
///    游戏，单游戏审计照抄它就把 4.9 万台没参与审计的游戏的"已拥有"全
///    抹成"缺失"——那会直接毁掉整个审计缓存。范围外的状态必须原样保留。
/// 2. **只枚举相关归档。** 按名字在 rompath 里找那几台机种的包，而不是
///    `read_dir` 整目录收 4.4 万个条目。
/// 3. **不动 MESS 主机扫描**，也不改别的游戏的定级。
///
/// 复用 [`audit_cache`]：包没变过就只 `stat` 不重开，所以这一轮通常在
/// 一秒内结束——这正是"单独审一个游戏"该有的速度。
///
/// 返回实际扫过的归档数，供 UI 报"检查了 N 个文件"。
pub fn audit_game(
    lib: &mut GameLibrary,
    game: &str,
    rom_paths: &[PathBuf],
    handle: &AuditHandle,
) -> usize {
    let _finish_guard = FinishOnDrop(handle.finished.clone());
    let t0 = std::time::Instant::now();
    let Some(scope) = audit_scope(lib, game) else {
        dlog!("单游戏审计: 库里没有 {game}");
        handle.finish();
        return 0;
    };
    dlog!(
        "单游戏审计: {} 范围 {} 台（本机 + {} 父集 + {} 设备 + {} 样本）",
        game,
        scope.len(),
        scope.parents.len(),
        scope.devices.len(),
        scope.samples.len()
    );

    // 1) 载入归档清单缓存。和 audit_all 一样，读一次约 20 MB。
    crate::core::audit_cache::load();

    // 2) 只重置范围内这几台。范围外的一个字节都不碰——这是本函数和
    //    audit_all 最要命的区别，写错会把全库审计成果抹掉。
    //
    //    旧值先留一份：与 audit_all 不同，本函数是**就地**改共享库（没有
    //    快照可回滚），所以中途取消时只有靠这份备份才能还原。UI 不给取消
    //    入口，但 headless 例子用得到，而"取消后把范围外的正确结论改成缺失
    //    并落盘"是不能接受的下场。
    let mut backup: Vec<(usize, Vec<bool>, Vec<bool>, u8)> = Vec::new();
    for &gi in scope.all().iter() {
        backup.push((
            gi,
            lib.games[gi].roms.iter().map(|r| r.available).collect(),
            lib.games[gi].disks.iter().map(|d| d.available).collect(),
            lib.games[gi].available,
        ));
        for r in &mut lib.games[gi].roms {
            r.available = r.is_nodump();
        }
        for d in &mut lib.games[gi].disks {
            d.available = d.is_nodump();
        }
        lib.games[gi].available = GAME_MISSING;
    }

    // 3) 找相关归档。范围外的包一个都不打开。
    let gis = scope.all();
    let units = find_units_for(lib, &gis, rom_paths);
    handle.set_total(units.len());
    dlog!(
        "单游戏审计: {} 待扫 {} 个归档（枚举 {:?}）",
        game,
        units.len(),
        t0.elapsed()
    );

    // 4) 扫。scan_units 内部按"本机种 + 其克隆集"匹配 crc，对单游戏审计
    //    正好合适：它只看得到传入的这台机种的 roms。
    let results = scan_units(&units, lib, handle);

    // 中途取消：`scan_units` 提前返回，只扫了一部分。此时上面刚做的重置
    // 还没被下面的落标记抵消，不还原就等于把范围内那几台判成"全缺失"。
    // 还原后直接返回，交给调用方决定不落盘。
    if handle.cancelled() {
        for (gi, roms, disks, grade) in backup {
            for (r, av) in lib.games[gi].roms.iter_mut().zip(roms) {
                r.available = av;
            }
            for (d, av) in lib.games[gi].disks.iter_mut().zip(disks) {
                d.available = av;
            }
            lib.games[gi].available = grade;
        }
        handle.finish();
        dlog!("单游戏审计: {game} 被取消，已还原范围内的旧结论");
        return 0;
    }

    // 5) 落标记
    let mut disk_marks: Vec<(usize, usize)> = Vec::new();
    for marks in results {
        for m in marks {
            match m {
                Mark::Rom(gi, ri, ok) => {
                    lib.games[gi].roms[ri].available = ok || lib.games[gi].roms[ri].available;
                }
                Mark::Disk(gi, di) => disk_marks.push((gi, di)),
            }
        }
    }
    // CHD 由克隆家族共用——同 sha1 的一起标上（与 audit_all 同一理由）
    let mut disk_index: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for &(gi, di) in &disk_marks {
        let sha1 = lib.games[gi].disks[di].sha1.clone();
        if sha1.is_empty() {
            lib.games[gi].disks[di].available = true;
        } else {
            disk_index.entry(sha1).or_default().push((gi, di));
        }
    }
    for same in disk_index.values() {
        for (gi, di) in same {
            lib.games[*gi].disks[*di].available = true;
        }
    }

    // 6) romof 回填：克隆集缺失的条目可以从父集里找到同名 crc 的。
    //    范围外的父集**不在 scope 里**也能读（只读不改），所以这里直接按
    //    romof 名字查，不依赖 scope.parents。
    for &gi in scope.all().iter() {
        let romof = lib.games[gi].romof.clone();
        if romof.is_empty() {
            continue;
        }
        let Some(pi) = lib.get_idx(&romof) else { continue };
        // 父集也要在范围内才回填：父集不在范围说明它没被重新审过，它的
        // available 还是上一轮的结果，拿它当权威会写出错的结论。
        if !scope.all().contains(&pi) {
            continue;
        }
        let parent_crcs: std::collections::HashMap<u32, bool> = lib.games[pi]
            .roms
            .iter()
            .map(|r| (r.crc, r.available))
            .collect();
        for ri in 0..lib.games[gi].roms.len() {
            if lib.games[gi].roms[ri].available {
                continue;
            }
            if let Some(&ok) = parent_crcs.get(&lib.games[gi].roms[ri].crc) {
                if ok {
                    lib.games[gi].roms[ri].available = true;
                }
            }
        }
    }

    // 7) 只给范围内这几台重新定级
    for &gi in scope.all().iter() {
        let complete = lib.games[gi].roms.iter().all(|r| r.available)
            && lib.games[gi].disks.iter().all(|d| d.available);
        lib.games[gi].available = if complete { GAME_COMPLETE } else { GAME_MISSING };
    }

    // 8) 这轮动过的归档清单存回去（下次全库审计能直接命中）
    let paths: Vec<PathBuf> = units.iter().map(|(p, _)| p.clone()).collect();
    crate::core::audit_cache::prune(&paths, AUDIT_CACHE_LIMIT);
    crate::core::audit_cache::save();

    handle.set_progress(0, 0, "");
    handle.finish();

    let scanned = units.len();
    dlog!(
        "单游戏审计: {} 完成，扫了 {} 个归档，缺失 {} 条，总耗时 {:?}",
        game,
        scanned,
        lib.games[scope.game]
            .roms
            .iter()
            .filter(|r| !r.available)
            .count(),
        t0.elapsed()
    );
    scanned
}

/// origin: RomAuditor::auditConsole — creates ext roms with "dir+file[/zip]" keys
fn audit_console(lib: &mut GameLibrary, console: &str, dirpath: &str) {
    let dir_path = dir_string_of(dirpath);
    let sourcefile = lib.get(console).map(|g| g.sourcefile.clone()).unwrap_or_default();
    let mut all_ext: Vec<String> = Vec::new();
    if let Some(g) = lib.get(console) {
        for d in &g.devices {
            all_ext.extend(d.extensions.iter().cloned());
        }
    }
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dirpath) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                continue;
            }
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let ext_ok = all_ext
                .iter()
                .any(|x| name.to_lowercase().ends_with(&format!(".{}", x.to_lowercase())));
            if ext_ok || is_zip(&p) || is_7z(&p) {
                files.push(p);
            }
        }
    }
    for p in files {
        let file_name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if is_zip(&p) || is_7z(&p) {
            if let Ok(list) = archive::list_archive(&p) {
                for entry in list {
                    let key = format!("{dir_path}{file_name}/{}", entry.name);
                    if lib.get_idx(&key).is_some() {
                        continue;
                    }
                    let mut g = GameMeta {
                        name: key,
                        description: archive::file_stem(&entry.name),
                        is_ext_rom: true,
                        romof: console.to_string(),
                        sourcefile: sourcefile.clone(),
                        available: GAME_COMPLETE,
                        is_horz: true,
                        ..Default::default()
                    };
                    if let Some(sys) = lib.get(console) {
                        g.devices = sys.devices.clone();
                    }
                    lib.push(g);
                }
            }
        } else {
            let key = format!("{dir_path}{file_name}");
            if lib.get_idx(&key).is_some() {
                continue;
            }
            let mut g = GameMeta {
                name: key,
                description: archive::file_stem(&file_name),
                is_ext_rom: true,
                romof: console.to_string(),
                sourcefile: sourcefile.clone(),
                available: GAME_COMPLETE,
                is_horz: true,
                ..Default::default()
            };
            if let Some(sys) = lib.get(console) {
                g.devices = sys.devices.clone();
            }
            lib.push(g);
        }
    }
}

fn dir_string_of(p: &str) -> String {
    let mut s = p.replace('\\', "/");
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

// ---------------------------------------------------------------------------
// fixdat export (origin: RomAuditor::exportDat)
// ---------------------------------------------------------------------------

pub fn export_fixdat(lib: &GameLibrary, method: AuditMethod, out: &Path) -> std::io::Result<usize> {
    if method == AuditMethod::Only {
        return Ok(0);
    }
    let mut names: Vec<String> = lib
        .games
        .iter()
        .filter(|g| g.cloneof.is_empty())
        .map(|g| g.name.clone())
        .collect();
    names.sort();
    let ordered: Vec<String> = {
        let mut v = Vec::new();
        for p in &names {
            v.push(p.clone());
            let mut clones: Vec<String> = lib
                .get(p)
                .map(|g| g.clones.iter().cloned().collect())
                .unwrap_or_default();
            clones.sort();
            v.extend(clones);
        }
        v
    };

    let mut xml = String::from(
        "<?xml version=\"1.0\"?>\r\n<!DOCTYPE datafile PUBLIC \"-//Logiqx//DTD ROM Management Datafile//EN\" \"http://www.logiqx.com/Dats/datafile.dtd\">\r\n<datafile>\r\n",
    );
    let mut count = 0usize;
    for name in &ordered {
        let Some(g) = lib.get(name) else { continue };
        if g.is_device {
            continue;
        }
        // Which sets belong in this dat at all (origin: the include test at the
        // top of the exportDat loop):
        //   * Export All Sets      → every real set, whether or not it is missing
        //   * anything else        → only sets the audit marked as missing
        let wanted = match method {
            AuditMethod::ExportComplete => !g.is_ext_rom,
            _ => g.available == GAME_MISSING,
        };
        if !wanted {
            continue;
        }

        // parent / grandparent(=bios) lookup, as used both for the redundancy
        // filter on missing roms and for the "completely missing clone" test
        let parent = if g.romof.is_empty() {
            None
        } else {
            lib.get(&g.romof)
        };
        let bios = match parent {
            Some(p) if !p.romof.is_empty() => lib.get(&p.romof),
            Some(p) if p.is_bios => Some(p),
            _ => None,
        };
        let bios_crcs: std::collections::HashSet<u32> = bios
            .map(|b| b.roms.iter().map(|r| r.crc).collect())
            .unwrap_or_default();
        let bios_roms_count = bios.map(|b| b.roms.len()).unwrap_or(0);
        let parent_crcs: std::collections::HashSet<u32> = parent
            .map(|p| p.roms.iter().map(|r| r.crc).collect())
            .unwrap_or_default();
        let parent_avail: std::collections::HashMap<u32, bool> = parent
            .map(|p| p.roms.iter().map(|r| (r.crc, r.available)).collect())
            .unwrap_or_default();

        let mut missing: Vec<&RomInfo> = Vec::new();
        let mut missing_count = 0usize;
        let mut nodump_count = 0usize;
        // a clone is "completely missing" once none of its clone-specific roms
        // is present; a set with no parent keeps this false
        let mut completely_missing_clone = !g.romof.is_empty();
        for r in &g.roms {
            if r.is_nodump() {
                nodump_count += 1;
            }
            if !r.available {
                // the parent is missing it too — leave it out, the parent's own
                // entry carries it (origin: the `continue` before missingRoms)
                if matches!(parent_avail.get(&r.crc), Some(false)) {
                    if !bios_crcs.contains(&r.crc) {
                        missing_count += 1;
                    }
                    continue;
                }
                missing.push(r);
                if !bios_crcs.contains(&r.crc) {
                    missing_count += 1;
                }
            } else if completely_missing_clone && !parent_crcs.contains(&r.crc) {
                completely_missing_clone = false;
            }
        }
        if method != AuditMethod::ExportComplete && missing.is_empty() {
            continue;
        }
        // bios roms and nodumps never count towards "everything is missing"
        let denom = g.roms.len().saturating_sub(bios_roms_count + nodump_count);
        let completely_missing = missing_count >= denom || completely_missing_clone;
        match method {
            AuditMethod::ExportIncomplete if completely_missing => continue,
            AuditMethod::ExportMissing if !completely_missing => continue,
            _ => {}
        }
        missing.sort_by(|a, b| a.name.cmp(&b.name));
        count += 1;
        let mut line = format!(
            "\t<game name=\"{}\" sourcefile=\"{}\"",
            x(&g.name),
            x(&g.sourcefile)
        );
        if !g.romof.is_empty() {
            line.push_str(&format!(" romof=\"{}\"", x(&g.romof)));
        }
        line.push_str(">\r\n");
        xml.push_str(&line);
        xml.push_str(&format!(
            "\t\t<description>{}</description>\r\n",
            x(&g.description)
        ));
        if !g.year.is_empty() {
            xml.push_str(&format!("\t\t<year>{}</year>\r\n", x(&g.year)));
        }
        xml.push_str(&format!(
            "\t\t<manufacturer>{}</manufacturer>\r\n",
            x(&g.manufacturer)
        ));
        for r in &missing {
            // origin: exportDat writes name/size/crc only — nodump roms are
            // marked available during the audit, so they never reach this list
            xml.push_str(&format!(
                "\t\t<rom name=\"{}\" size=\"{}\" crc=\"{:08x}\"/>\r\n",
                x(r.effective_name()),
                r.size,
                r.crc
            ));
        }
        xml.push_str("\t</game>\r\n");
    }
    xml.push_str("</datafile>\r\n");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, xml)?;
    Ok(count)
}

fn x(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::GameMeta;

    /// 枚举阶段必须真的往句柄上报，否则状态栏在分母出现之前只能显示
    /// "正在审计"——那正是这次要修的东西。
    ///
    /// 难点：`audit_all` **返回时**标签必然已被清空（结尾那行
    /// `set_progress(0, 0, "")` 是故意的，好让转发线程最后观察到的是
    /// "已完成"）。所以不能在跑完之后断言。
    ///
    /// 办法是把 `set_enumerating` 换成"记录调用序列"的版本：既验证它被
    /// 调用过，也验证报的数是对的。这里用一个真实目录 + 一个不存在目录：
    /// 前者让 `read_dir` 成功（枚举真的走了一遍），后者验证跳过分支也
    /// 会上报。
    #[test]
    fn the_enumeration_stage_is_reported() {
        let mut lib = GameLibrary::new("test".into());
        lib.games.push(GameMeta {
            name: "pacman".into(),
            ..Default::default()
        });
        lib.rebuild_indexes();

        let base = std::env::temp_dir().join("mvui-audit-enum-test");
        let _ = std::fs::remove_dir_all(&base);
        let real_dir = base.join("roms");
        std::fs::create_dir_all(&real_dir).unwrap();

        let handle = AuditHandle::new();
        // 枚举阶段每次上报都记下来
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        {
            let seen = Arc::clone(&seen);
            handle.on_enumerating(move |d, t, u| {
                seen.lock().unwrap().push((d, t, u));
            });
        }
        audit_all(
            &mut lib,
            &[real_dir, base.join("does-not-exist")],
            &HashMap::new(),
            &handle,
        );

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "两个 rompath 各上报一次；拿到 {seen:?}");
        assert_eq!(seen[0].0, 0, "第一个目录的序号");
        assert_eq!(seen[0].1, 2, "rompath 总数");
        assert_eq!(seen[0].2, 0, "还没收到任何单元");
        assert_eq!(seen[1].0, 1, "第二个目录的序号");
        // 第一个目录存在但空 -> 0 单元；第二个不存在 -> 跳过
        assert_eq!(seen[1].2, 0, "两个目录都没有可扫单元");

        // 分母在枚举之后才确定，而这两个目录里没有可扫单元
        let (done, total, _) = handle.snapshot();
        assert_eq!(total, 0, "没有可扫单元时分母是 0");
        assert_eq!(done, 0, "没有单元被扫过");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 标签格式与 `ui::parse_enumerating` 是对偶的：那边改格式这边不改，
    /// 状态栏会安静地退化成纯文案（不报错，所以必须两边都有测试）。
    #[test]
    fn the_enumeration_label_has_the_shape_the_ui_expects() {
        let handle = AuditHandle::new();
        handle.set_enumerating(2, 5, 13824);
        let (_, _, cur) = handle.snapshot();
        assert_eq!(cur, "enum 2/5 dirs, 13824 units");
    }

    /// `find_units_for` 是 `read_dir` 全量枚举的**等价替换**，不是简化。
    ///
    /// 三种形态都要认（松散目录 / `.zip` / `.7z`），同一台机种在多个
    /// rompath 下都有包时都要算数，去重要按完整路径（同一目录里
    /// `pacman.zip` 和 `pacman` 目录是两个不同单元，都该收）。
    /// 少收一个单元 = 那盘游戏的一部分永远显示成"缺失"，而且不报错。
    #[test]
    fn direct_lookup_finds_every_archive_shape_the_enumeration_would() {
        let base = std::env::temp_dir().join("mvui-find-units-test");
        let _ = std::fs::remove_dir_all(&base);
        let roms_a = base.join("roms_a");
        let roms_b = base.join("roms_b");
        std::fs::create_dir_all(&roms_a).unwrap();
        std::fs::create_dir_all(&roms_b).unwrap();
        // A：pacman 是松散目录；A：dkong 是 zip；B：dkong 是 7z（同机种，
        // 两个 rompath 各一份，都算数）；qbert 谁都没有；还有一个无关的 zip
        std::fs::create_dir_all(roms_a.join("pacman")).unwrap();
        std::fs::write(roms_a.join("dkong.zip"), b"x").unwrap();
        std::fs::write(roms_b.join("dkong.7z"), b"x").unwrap();
        std::fs::write(roms_a.join("unrelated.zip"), b"x").unwrap();

        let mut lib = GameLibrary::new("test".into());
        for n in ["pacman", "dkong", "qbert", "unrelated"] {
            lib.games.push(GameMeta {
                name: n.into(),
                ..Default::default()
            });
        }
        lib.rebuild_indexes();

        let gis = vec![lib.get_idx("pacman").unwrap(), lib.get_idx("dkong").unwrap()];
        let units = find_units_for(&lib, &gis, &[roms_a.clone(), roms_b.clone()]);

        let mut got: Vec<String> = units.iter().map(|(p, _)| p.display().to_string()).collect();
        got.sort();
        let mut want = vec![
            roms_a.join("pacman").display().to_string(),
            roms_a.join("dkong.zip").display().to_string(),
            roms_b.join("dkong.7z").display().to_string(),
        ];
        want.sort();
        assert_eq!(got, want, "目录形态 / zip / 7z / 多 rompath 都要收齐");
        // 库下标也要对：单元是"哪个机种的包"，错了就是把 A 的结论写到 B 上
        for (_, gi) in &units {
            let n = lib.games[*gi].name.as_str();
            assert!(n == "pacman" || n == "dkong", "不该扫到 {n}");
        }

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 大小写：磁盘上是大写 `GTMRUSA.ZIP`、库里是 `GtmrUsa`。
    ///
    /// Windows/macOS 上不敏感所以必然命中，但 Linux 上不会——而写错时的
    /// 症状是"明明有 rom 却说缺失"。所以两种拼法都得试。
    ///
    /// 同时钉住**只找到一个**：不敏感的文件系统上两种拼法指向同一个文件，
    /// 按路径字符串去重是去不掉的，会把同一个包收两遍（白读一遍归档）。
    #[test]
    fn direct_lookup_tries_both_the_exact_and_the_lower_case_spelling() {
        let base = std::env::temp_dir().join("mvui-find-units-case");
        let _ = std::fs::remove_dir_all(&base);
        let roms = base.join("roms");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::write(roms.join("GTMRUSA.zip"), b"x").unwrap();

        let mut lib = GameLibrary::new("test".into());
        lib.games.push(GameMeta {
            name: "GtmrUsa".into(),
            ..Default::default()
        });
        lib.rebuild_indexes();

        let gis = vec![0];
        let units = find_units_for(&lib, &gis, &[roms]);
        assert_eq!(units.len(), 1, "两种拼法在 Windows 上撞同一个文件，只能收一次");
        assert!(units[0].0.to_string_lossy().to_uppercase().contains("GTMRUSA.ZIP"));

        let _ = std::fs::remove_dir_all(&base);
    }

    /// 空的 / 不存在的 rompath 不能 panic，也不能凭空造出单元。
    ///
    /// 顺带钉住越界的 `gi`：`find_units_for` 是 `pub` 的，调用方未必都像
    /// `audit_scope` 那样给出合法下标。
    #[test]
    fn direct_lookup_tolerates_missing_rompaths_and_out_of_range_indexes() {
        let lib = GameLibrary::new("test".into());
        let units = find_units_for(
            &lib,
            &[0, 7], // 库是空的，两个下标都越界
            &[std::env::temp_dir().join("mvui-definitely-not-here")],
        );
        assert!(units.is_empty());
    }

    /// 造一个真 zip，让单游戏审计跑在真的解析器上而不是桩上。
    fn make_zip(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        use std::io::Write;
        let p = dir.join(name);
        let f = std::fs::File::create(&p).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (n, data) in entries {
            w.start_file(*n, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
        p
    }

    /// 审计是**按 crc 匹配**的（`scan_units` 建 crc → 槽位的表），所以测试里
    /// 造的 `RomInfo` 必须带**条目内容的真实 crc32**，不能编一个假的。
    /// 编错时的症状很误导人：包明明在、文件名明明对，审计就是找不到。
    fn crc32(data: &[u8]) -> u32 {
        // 与 zip 里的 crc32 同算法（IEEE 反射多项式 0xEDB88320）
        let mut table = [0u32; 256];
        for (i, slot) in table.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        let mut crc = 0xFFFF_FFFFu32;
        for b in data {
            crc = table[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }

    fn rom_named(name: &str, data: &[u8]) -> RomInfo {
        RomInfo {
            name: name.into(),
            crc: crc32(data),
            size: data.len() as u64,
            region: "maincpu".into(),
            ..Default::default()
        }
    }

    /// **单游戏审计最要命的不变量：范围外的游戏一个字节都不能被改。**
    ///
    /// `audit_all` 的重置循环会扫 5 万台游戏；单游戏审计照抄它就会把 4.9
    /// 万台未参与审计的游戏的"已拥有"全抹成"缺失"，那等于毁掉整个审计
    /// 缓存——而这个错误**不会报错**，只会让用户的收藏看起来全丢了。
    #[test]
    fn auditing_one_game_leaves_every_other_game_untouched() {
        const TARGET_DATA: &[u8] = b"target data";
        const BYSTANDER_DATA: &[u8] = b"bystander data";
        let dir = std::env::temp_dir().join("mvui-auditgame-untouched");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        make_zip(&dir, "target.zip", &[("t.rom", TARGET_DATA)]);

        let mut lib = GameLibrary::new("test".into());
        // 目标机种：包里那个文件在，第二个文件缺
        let mut target = GameMeta {
            name: "target".into(),
            ..Default::default()
        };
        target.roms.push(rom_named("t.rom", TARGET_DATA));
        target.roms.push(rom_named("missing.rom", b"never in the zip"));
        // 旁观者：审计前先标成"已拥有"，审计后必须还是
        let mut bystander = GameMeta {
            name: "bystander".into(),
            ..Default::default()
        };
        bystander.roms.push(rom_named("b.rom", BYSTANDER_DATA));
        bystander.roms[0].available = true;
        bystander.available = GAME_COMPLETE;
        lib.games.push(target);
        lib.games.push(bystander);
        lib.rebuild_indexes();

        let handle = AuditHandle::new();
        audit_game(&mut lib, "target", &[dir.clone()], &handle);

        // 目标：包里的那个找到了
        assert!(
            lib.games[0].roms[0].available,
            "包里的 t.rom 必须被认出来"
        );
        assert!(
            !lib.games[0].roms[1].available,
            "不在包里的 missing.rom 必须仍然是缺失"
        );
        assert_eq!(lib.games[0].available, GAME_MISSING);

        // 旁观者：一个字节都没动
        let b = &lib.games[1];
        assert!(b.roms[0].available, "范围外的游戏不能被重置");
        assert_eq!(b.available, GAME_COMPLETE, "范围外的定级不能被改");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 范围要含 romof 父集：克隆集缺的条目能从父集的包里找到。
    /// 范围要含 romof 父集：父集自己的包也要被扫到。
    ///
    /// 注意父集的包**不会**把条目记到克隆集头上——`scan_units` 建的
    /// crc 表只含"本机种 + 其克隆集"（这是 1.8.2 的口径，也是全库审计
    /// 不把 4.4 万个包的 crc 互相串起来的原因）。共享条目靠第 6 步的
    /// romof 回填补上，而那一步要求父集在范围内。所以这里断言的是
    /// "父集的包被打开过，且回填能功。
    #[test]
    fn the_scope_covers_the_romof_parent() {
        const SHARED: &[u8] = b"from parent";
        const OWN: &[u8] = b"from child";
        let dir = std::env::temp_dir().join("mvui-auditgame-parent");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        make_zip(&dir, "parent.zip", &[("shared.rom", SHARED)]);
        make_zip(&dir, "child.zip", &[("own.rom", OWN)]);

        let mut lib = GameLibrary::new("test".into());
        let mut child = GameMeta {
            name: "child".into(),
            romof: "parent".into(),
            ..Default::default()
        };
        child.roms.push(rom_named("shared.rom", SHARED));
        child.roms.push(rom_named("own.rom", OWN));
        let mut parent = GameMeta {
            name: "parent".into(),
            ..Default::default()
        };
        parent.roms.push(rom_named("shared.rom", SHARED));
        lib.games.push(child);
        lib.games.push(parent);
        lib.rebuild_indexes();

        // 范围必须含父集，否则父集的包根本不会被打开、romof 回填也无从起谈
        let scope = audit_scope(&lib, "child").expect("child 在库里");
        assert_eq!(scope.game, 0);
        assert_eq!(scope.parents, vec![1], "父集在范围内");
        assert_eq!(scope.len(), 2);

        let handle = AuditHandle::new();
        let n = audit_game(&mut lib, "child", &[dir.clone()], &handle);
        assert_eq!(n, 2, "child.zip 和 parent.zip 都被扫（父集在范围内）");
        assert!(lib.games[0].roms[1].available, "own.rom 直接命中");
        assert!(
            lib.games[0].roms[0].available,
            "shared.rom 通过 romof 回填从父集补上"
        );
        assert!(lib.games[0].roms.iter().all(|r| r.available));
        assert_eq!(lib.games[0].available, GAME_COMPLETE);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 范围要含引用设备与样本。
    #[test]
    fn the_scope_covers_devices_and_samples() {
        let mut lib = GameLibrary::new("test".into());
        let mut g = GameMeta {
            name: "game".into(),
            sampleof: "ssample".into(),
            ..Default::default()
        };
        g.devices.push(DeviceInfo {
            kind: "m68000".into(),
            instance: "m68000".into(),
            tag: "maincpu".into(),
            ..Default::default()
        });
        let mut cpu = GameMeta {
            name: "m68000".into(),
            is_device: true,
            ..Default::default()
        };
        cpu.roms.push(rom_named("mc68000.bin", b"cpu data"));
        let mut sample = GameMeta {
            name: "ssample".into(),
            ..Default::default()
        };
        sample.roms.push(rom_named("a.wav", b"a wave"));
        lib.games.push(g);
        lib.games.push(cpu);
        lib.games.push(sample);
        lib.rebuild_indexes();

        let scope = audit_scope(&lib, "game").expect("game 在库里");
        assert_eq!(scope.devices, vec![1], "设备在范围内");
        assert_eq!(scope.samples, vec![2], "样本在范围内");
        assert_eq!(scope.len(), 3);
        assert_eq!(scope.all(), vec![0, 1, 2]);
    }

    /// 设备包的可用性要被这一轮更新（设备 rom 也是这盘游戏要的文件）。
    #[test]
    fn device_roms_get_audited_too() {
        const CPU_DATA: &[u8] = b"cpu data";
        let dir = std::env::temp_dir().join("mvui-auditgame-device");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        make_zip(&dir, "m68000.zip", &[("mc68000.bin", CPU_DATA)]);

        let mut lib = GameLibrary::new("test".into());
        let mut g = GameMeta {
            name: "game".into(),
            ..Default::default()
        };
        g.devices.push(DeviceInfo {
            kind: "m68000".into(),
            instance: "m68000".into(),
            tag: "maincpu".into(),
            ..Default::default()
        });
        let mut cpu = GameMeta {
            name: "m68000".into(),
            is_device: true,
            ..Default::default()
        };
        cpu.roms.push(rom_named("mc68000.bin", CPU_DATA));
        lib.games.push(g);
        lib.games.push(cpu);
        lib.rebuild_indexes();

        let handle = AuditHandle::new();
        let n = audit_game(&mut lib, "game", &[dir.clone()], &handle);
        assert_eq!(n, 1, "设备的包被扫了");
        assert!(
            lib.games[1].roms[0].available,
            "设备 rom 认出来了——它也是这盘游戏要的文件"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 库里没有这台游戏时不能 panic：静默返回 0 个归档。
    #[test]
    fn auditing_an_unknown_game_is_a_no_op() {
        let mut lib = GameLibrary::new("test".into());
        lib.rebuild_indexes();
        let handle = AuditHandle::new();
        assert_eq!(
            audit_game(&mut lib, "nope", &[], &handle),
            0,
            "不存在的游戏不该炸，也不该扫任何东西"
        );
        assert!(handle.finished.load(Ordering::Relaxed), "句柄必须收尾");
    }

    /// romof 成环时范围计算必须终止。
    #[test]
    fn a_cyclic_scope_terminates() {
        let mut lib = GameLibrary::new("test".into());
        let mut a = GameMeta {
            name: "a".into(),
            romof: "b".into(),
            ..Default::default()
        };
        a.roms.push(rom_named("a.rom", b"a data"));
        let mut b = GameMeta {
            name: "b".into(),
            romof: "a".into(),
            ..Default::default()
        };
        b.roms.push(rom_named("b.rom", b"b data"));
        lib.games.push(a);
        lib.games.push(b);
        lib.rebuild_indexes();

        let scope = audit_scope(&lib, "a").expect("a 在库里");
        assert_eq!(scope.parents, vec![1], "b 进范围，a 不再进");
    }

    /// 取消必须把范围内的旧结论**原样还原**。
    ///
    /// 这是"就地改库"才有的风险：`audit_game` 先把范围内那几台的 `available`
    /// 全清成 false，再去扫包。扫到一半取消的话，那次清零就成了最终结论——
    /// 一台本来齐备的游戏被判成"全缺失"，而调用方还会把它当审计结果落盘。
    /// 所以取消路径必须回滚，而 `audit_game` 手上只有它自己留的那份备份。
    #[test]
    fn cancelling_restores_the_previous_verdicts() {
        let dir = std::env::temp_dir().join("mvui-auditgame-cancel");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        make_zip(&dir, "game.zip", &[("g.rom", b"g data")]);

        let mut lib = GameLibrary::new("test".into());
        let mut g = GameMeta {
            name: "game".into(),
            ..Default::default()
        };
        g.roms.push(rom_named("g.rom", b"g data"));
        // 审计前的状态：齐备
        g.roms[0].available = true;
        g.available = GAME_COMPLETE;
        lib.games.push(g);
        lib.rebuild_indexes();

        let handle = AuditHandle::new();
        handle.cancel.store(true, Ordering::Relaxed);
        let n = audit_game(&mut lib, "game", &[dir.clone()], &handle);

        assert_eq!(n, 0, "取消的审计不报扫了几个");
        assert!(
            lib.games[0].roms[0].available,
            "取消后 rom 的可用性必须回到审计前——不能停在「刚清零」那个中间态"
        );
        assert_eq!(
            lib.games[0].available, GAME_COMPLETE,
            "取消后整机的定级也必须回到审计前"
        );
        assert!(handle.finished.load(Ordering::Relaxed), "句柄必须收尾");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
