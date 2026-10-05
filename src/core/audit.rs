//! ROM 审计，1:1 移植自 audit.cpp 的 RomAuditor（内部审计 + 主机
//! 扫描 + Logiqx fixdat 导出，含那 4 种导出方式）。
//!
//! 可用性存在数据模型里（每个 rom/disk 的 `available` + 游戏的
//! `available`），与原版 GameInfo 的字段对应。

use crate::core::archive::{self, is_7z, is_zip};
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
}
