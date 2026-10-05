//! Background job spawners (listxml+showconfig boot chain, audit, previews, dats).
//! Long jobs never hold the library mutex while the UI renders: boot audits a
//! local library before publishing; refresh audits a snapshot and swaps it back.

use crate::events::{AppEvent, ReadyPayload, SharedLib, SharedOpts};
use crate::core::audit::{self, AuditHandle};
use crate::core::cache;
use crate::core::dat;
use crate::core::folders;
use crate::core::library::GameLibrary;
use crate::core::listxml;
use crate::core::mameproc::{pump_lines, MameBinary};
use crate::core::options::OptionCore;
use crate::core::settings::GuiSettings;
use mvui::dlog;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

/// unwrap a `catch_unwind` payload into a printable string
fn panic_text(e: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = e.downcast_ref::<&str>() {
        (*s).to_string()
    } else {
        "unknown panic".into()
    }
}

/// 冷启动的完整链路：读缓存 → `mame -listxml` → `mame -showconfig` → 审计。
pub fn boot_library(mame: MameBinary, tx: Sender<AppEvent>, ctx: egui::Context) {
    thread::spawn(move || {
        // set as soon as `LibraryReady` has been sent: from that moment on the
        // UI already owns a usable library, so a later panic must not be
        // reported as a boot failure (that used to overwrite the ready state
        // with an error and leave `AuditDone` pending forever)
        let published = Arc::new(AtomicBool::new(false));
        let published_for_run = Arc::clone(&published);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            boot_run(&mame, &tx, &published_for_run)
        }));
        if let Err(e) = result {
            let msg = panic_text(e.as_ref());
            if published.load(Ordering::Relaxed) {
                let _ = tx.send(AppEvent::Log(format!("boot: {msg}")));
                let _ = tx.send(AppEvent::AuditDone(Err(msg)));
            } else {
                let _ = tx.send(AppEvent::LibraryReady(Err(format!("boot failed: {msg}"))));
            }
        }
        ctx.request_repaint();
    });
}

fn boot_run(mame: &MameBinary, tx: &Sender<AppEvent>, published: &Arc<AtomicBool>) {
    let boot_t0 = std::time::Instant::now();
    dlog!("引导: boot_run 开始（mame={} 版本 {:?}）", mame.path.display(), mame.version);
    let cache_path = GuiSettings::cache_dir().join("gamelist.cache");

    // 1) cache (origin: pMameDat->load(); version mismatch → rebuild)
    dlog!("引导: 读缓存 {}", cache_path.display());
    match cache::load(&cache_path, &mame.version) {
        Ok(data) if data.audited => {
            dlog!("引导: 缓存命中且已审计（{} 台），跳过 listxml 与审计", data.library.len());
            let _ = tx.send(AppEvent::Log("loaded games from cache.".into()));
            // a fully audited cache needs neither -listxml nor a re-audit
            finish_boot_cached(data.library, mame, tx, published);
            dlog!("引导: 热启动完成，耗时 {:?}", boot_t0.elapsed());
            return;
        }
        Ok(data) => {
            // -listxml result is cached but the audit never finished last time
            // (closed mid-audit): skip the parse, just finish the audit
            dlog!(
                "引导: 缓存命中但未审计（{} 台），跳过 listxml 解析，只补审计",
                data.library.len()
            );
            let _ = tx.send(AppEvent::Log(
                "loaded games from cache, audit still pending.".into(),
            ));
            finish_boot(data.library, mame, tx, published);
            dlog!("引导: 补审计完成，耗时 {:?}", boot_t0.elapsed());
            return;
        }
        Err(cache::CacheError::Missing) => {
            dlog!("引导: 无缓存，走完整流程");
        }
        Err(e) => {
            dlog!("引导: 缓存不可用（{e}），走完整流程");
            let _ = tx.send(AppEvent::Log(format!("cache unusable ({e}); full refresh")));
        }
    }

    // 2) listxml
    let _ = tx.send(AppEvent::Log(
        "running mame -listxml (this can take a while)…".into(),
    ));
    let listxml_t0 = std::time::Instant::now();
    let mut child = match mame.spawn_listxml() {
        Ok(c) => c,
        Err(e) => {
            dlog!("引导: -listxml 启动失败：{e}");
            let _ = tx.send(AppEvent::LibraryReady(Err(e.to_string())));
            return;
        }
    };
    let mut library = match parse_listxml(&mut child, tx) {
        Ok(l) => l,
        Err(e) => {
            dlog!("引导: -listxml 解析失败：{e}");
            let _ = tx.send(AppEvent::LibraryReady(Err(e)));
            return;
        }
    };
    dlog!(
        "引导: -listxml 解析出 {} 台机种，耗时 {:?}",
        library.len(),
        listxml_t0.elapsed()
    );
    let _ = tx.send(AppEvent::Log(format!(
        "listxml parsed: {} machines.",
        library.len()
    )));

    // 3) showconfig (origin: loadDefaultIni child chain)
    let showconfig_t0 = std::time::Instant::now();
    let default_ini = match mame.spawn_showconfig() {
        Ok(mut c) => {
            let mut buf = String::new();
            if let Some(mut out) = c.stdout.take() {
                use std::io::Read;
                let mut raw = Vec::new();
                let _ = out.read_to_end(&mut raw);
                buf = String::from_utf8_lossy(&raw).to_string();
            }
            let _ = c.wait();
            buf
        }
        Err(e) => {
            dlog!("引导: -showconfig 启动失败（{e}），选项模板将为空");
            String::new()
        }
    };
    dlog!(
        "引导: -showconfig 读到 {} 字节，耗时 {:?}",
        default_ini.len(),
        showconfig_t0.elapsed()
    );
    library.default_ini = default_ini;
    library.mame_version = mame.version.clone();
    library.rebuild_indexes();
    library.complete_data();

    finish_boot(library, mame, tx, published);
    dlog!("引导: 冷启动完成，耗时 {:?}", boot_t0.elapsed());
}

/// 收完 `mame -listxml` 的输出并解析成游戏库。
///
/// 两阶段是必须的：**机种总数只有收完整份输出才知道**。把子进程 stdout 直接
/// 喂给解析器（流式）能省掉这份缓冲，但分母就永远无从得知，解析百分比只能
/// 拿常量瞎估。所以先攒缓冲、边攒边数，攒完再解析——进度从第一帧起就是真
/// 百分比。
fn parse_listxml(
    child: &mut std::process::Child,
    tx: &Sender<AppEvent>,
) -> Result<GameLibrary, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "mame -listxml: no stdout".to_string())?;

    // 阶段一：收输出并数机种。这一步只报台数——总数还没收完，给不出百分比
    let phase1_t0 = std::time::Instant::now();
    dlog!("listxml 阶段一: 开始收子进程输出并数机种");
    let mut xml: Vec<u8> = Vec::new();
    let tx2 = tx.clone();
    let total = match listxml::buffer_and_count(stdout, &mut xml, &mut |machines| {
        let _ = tx2.send(AppEvent::LibProgress {
            done: machines,
            total: 0,
        });
    }) {
        Ok(n) => n,
        Err(e) => {
            let _ = child.kill();
            // reap it: a killed child nobody waits for keeps the pipe open
            let _ = child.wait();
            dlog!("listxml 阶段一: 失败（{e}），耗时 {:?}", phase1_t0.elapsed());
            return Err(format!("listxml: {e}"));
        }
    };
    let _ = child.wait();
    dlog!(
        "listxml 阶段一: 收完 {} 字节 / {} 台机种，耗时 {:?}",
        xml.len(),
        total,
        phase1_t0.elapsed()
    );

    // 阶段二：解析，分母是阶段一数出来的真总数
    let phase2_t0 = std::time::Instant::now();
    dlog!("listxml 阶段二: 开始解析（分母 {}）", total);
    let tx2 = tx.clone();
    let library = listxml::parse_from_reader(&xml[..], false, &mut |done| {
        let _ = tx2.send(AppEvent::LibProgress { done, total });
    })
    .map_err(|e| format!("listxml: {e}"))?;
    dlog!(
        "listxml 阶段二: 解析完成，{} 台机种，耗时 {:?}",
        library.len(),
        phase2_t0.elapsed()
    );
    Ok(library)
}

/// warm start: publish options + library as-is (audit state comes from cache)
fn finish_boot_cached(
    library: GameLibrary,
    mame: &MameBinary,
    tx: &Sender<AppEvent>,
    published: &Arc<AtomicBool>,
) {
    dlog!("引导: 热启动路径，发布选项与游戏库（不审计）");
    let mame_dir = mame.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let gui: HashMap<String, String> = HashMap::new();
    let is_mess = mame
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().contains("mess"))
        .unwrap_or(false);
    let opts_t0 = std::time::Instant::now();
    let mut core = OptionCore::load_default(&library.default_ini, &library, &gui, &mame_dir);
    core.mess_like = is_mess;
    core.load_global(&gui);
    for w in &core.warnings {
        let _ = tx.send(AppEvent::Log(w.clone()));
    }
    let _ = tx.send(AppEvent::OptionsReady(Ok(Arc::new(Mutex::new(core)))));
    dlog!(
        "引导: 热启动选项就绪（{} 个选项），耗时 {:?}",
        library.len(),
        opts_t0.elapsed()
    );
    dlog!("引导: 热启动构建文件夹缓存");
    let folder_cache = Arc::new(folders::compute_folder_cache(&library, is_mess));
    dlog!("引导: 热启动 LibraryReady 已发布（{} 台机种）", library.len());
    let _ = tx.send(AppEvent::LibraryReady(Ok(ReadyPayload {
        lib: Arc::new(Mutex::new(library)),
        folders: folder_cache,
        from_cache: true,
    })));
    published.store(true, Ordering::Relaxed);
}

/// cold start: options + audit + cache — all on the local (unlocked) library
fn finish_boot(
    mut library: GameLibrary,
    mame: &MameBinary,
    tx: &Sender<AppEvent>,
    published: &Arc<AtomicBool>,
) {
    let mame_dir = mame.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let gui: HashMap<String, String> = HashMap::new();

    // options template (defaultIni); relative ini paths resolve to the mame dir
    let opts_t0 = std::time::Instant::now();
    let mut core = OptionCore::load_default(&library.default_ini, &library, &gui, &mame_dir);
    core.mess_like = mame
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().contains("mess"))
        .unwrap_or(false);
    core.load_global(&gui);
    let warn_count = core.warnings.len();
    for w in &core.warnings {
        let _ = tx.send(AppEvent::Log(w.clone()));
    }
    let opt_count = core.opts.len();
    let opts: SharedOpts = Arc::new(Mutex::new(core));
    let _ = tx.send(AppEvent::OptionsReady(Ok(opts.clone())));
    dlog!(
        "引导: 选项就绪（{} 个选项, {} 条警告, {} 台机种），耗时 {:?}",
        opt_count,
        warn_count,
        library.len(),
        opts_t0.elapsed()
    );

    // rompath from the loaded global chain (relative → mame dir)
    let rom_paths: Vec<PathBuf> = {
        let guard = opts.lock().unwrap();
        match guard.opts.get("rompath") {
            Some(o) => guard.resolve_dir_list(&o.currvalue),
            None => Vec::new(),
        }
    };
    dlog!(
        "引导: 审计 rompath {} 个: {}",
        rom_paths.len(),
        rom_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    );
    let _ = tx.send(AppEvent::Log(format!("audit: {} rom dirs", rom_paths.len())));

    // Persist the parsed library *before* auditing. The audit is the slow part
    // and used to be the only thing that wrote the cache, so closing the app
    // mid-audit meant a full -listxml re-parse on every single start.
    let cache_path = GuiSettings::cache_dir().join("gamelist.cache");
    let save_t0 = std::time::Instant::now();
    match cache::save_library(&cache_path, &mame.version, &library, false) {
        Ok(()) => {
            dlog!(
                "引导: 审计前落盘缓存（audited=false），耗时 {:?}",
                save_t0.elapsed()
            );
            let _ = tx.send(AppEvent::Log("parsed library cached.".into()));
        }
        Err(e) => {
            dlog!("引导: 审计前落盘缓存失败：{e}");
            let _ = tx.send(AppEvent::Log(format!("cache save failed: {e}")));
        }
    }

    dlog!("引导: 构建文件夹缓存");
    let is_mess = mame
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().contains("mess"))
        .unwrap_or(false);
    let folder_cache = Arc::new(folders::compute_folder_cache(&library, is_mess));

    // publish the list FIRST (UI appears right after listxml),
    // then audit in a background thread and swap results in
    dlog!(
        "引导: LibraryReady 已发布（审计前, {} 台机种）",
        library.len()
    );
    let lib_shared: SharedLib = Arc::new(Mutex::new(library.clone()));
    let _ = tx.send(AppEvent::LibraryReady(Ok(ReadyPayload {
        lib: lib_shared.clone(),
        folders: folder_cache,
        from_cache: false,
    })));
    published.store(true, Ordering::Relaxed);

    let extra = extra_software_for(&library);
    dlog!(
        "引导: 审计开始前准备就绪（{} 个 extra_software 条目, {} 台机种）",
        extra.len(),
        lib_shared.lock().map(|g| g.len()).unwrap_or(0)
    );
    let handle = Arc::new(AuditHandle::new());
    // hand the handle to the UI before the (slow) audit starts, so the status
    // bar can show "Auditing nn%" from the first tick instead of waiting for
    // the audit to report anything itself
    let _ = tx.send(AppEvent::AuditStarted(handle.clone()));
    {
        // forward audit progress to the status bar
        let tx2 = tx.clone();
        let h2 = handle.clone();
        thread::spawn(move || loop {
            let (done, total, cur) = h2.snapshot();
            let _ = tx2.send(AppEvent::AuditProgress {
                done,
                total,
                system: cur,
            });
            if h2.is_finished() || (total > 0 && done >= total) {
                break;
            }
            thread::sleep(std::time::Duration::from_millis(200));
        });
    }
    // The audit works on a private copy, so a panic in it must not escape to the
    // outer handler: the library has already been published and `AuditDone` has
    // to arrive anyway, or the status bar spins forever and the start button
    // stays disabled (README P1-8).
    let audit_t0 = std::time::Instant::now();
    let audit_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        audit::audit_all(&mut library, &rom_paths, &extra, &handle);
    }));
    dlog!("引导: 审计返回，耗时 {:?}", audit_t0.elapsed());
    if let Err(p) = audit_result {
        // the progress forwarder waits on `finished` — make sure it exits
        handle.finish();
        let msg = format!("audit failed: {}", panic_text(p.as_ref()));
        dlog!("引导: 审计 panic：{msg}");
        let _ = tx.send(AppEvent::Log(msg.clone()));
        let _ = tx.send(AppEvent::AuditDone(Err(msg)));
        return;
    }
    if handle.cancelled() {
        // discard the half-audited snapshot: the published library stays as it
        // was and the cache keeps audited=false, so the next start resumes
        dlog!("引导: 审计被取消，丢弃半成品快照（缓存保持 audited=false）");
        let _ = tx.send(AppEvent::Log("audit cancelled.".into()));
        let _ = tx.send(AppEvent::AuditDone(Err("cancelled".into())));
        return;
    }
    {
        // move, not clone — the library is ~46k entries
        let mut guard = lib_shared.lock().unwrap();
        *guard = library;
    }
    // Availability / Unavailability counts depend on the audit result, so the
    // folder tree has to be rebuilt once the audit is in.
    let folders = {
        let guard = lib_shared.lock().unwrap();
        folders::compute_folder_cache(&guard, is_mess)
    };
    let _ = tx.send(AppEvent::FoldersReady(Arc::new(folders)));
    // re-save with audited = true: from now on a start is cache-only
    {
        let guard = lib_shared.lock().unwrap();
        match cache::save_library(&cache_path, &mame.version, &guard, true) {
            Ok(()) => {
                dlog!("引导: 审计后落盘缓存（audited=true）");
                let _ = tx.send(AppEvent::Log("gamelist.cache saved (audited).".into()));
            }
            Err(e) => {
                dlog!("引导: 审计后落盘缓存失败：{e}");
                let _ = tx.send(AppEvent::Log(format!("cache save failed: {e}")));
            }
        }
    }
    dlog!("引导: 审计完成，AuditDone 已发出");
    let _ = tx.send(AppEvent::AuditDone(Ok("audit finished".into())));

}

fn extra_software_for(lib: &GameLibrary) -> HashMap<String, String> {
    let settings = GuiSettings::load();
    extra_software_with(lib, &settings)
}

/// Same, with the (disk-backed) settings already in hand.
///
/// The manual-audit path used to call `extra_software_for` while holding the
/// library lock — reading and parsing `the original GUI ini` inside the critical section
/// stalled the UI for the frame that started the audit (README P2-18).
fn extra_software_with(lib: &GameLibrary, settings: &GuiSettings) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for g in &lib.games {
        if !g.devices.is_empty() && !g.is_ext_rom {
            let key = format!("{}_extra_software", g.name);
            if let Some(v) = settings.get(&key) {
                m.insert(g.name.clone(), v.to_string());
            }
        }
    }
    m
}

/// refresh audit on a snapshot; UI keeps rendering (origin: RomAuditor thread)
pub fn run_audit(
    lib: SharedLib,
    opts: SharedOpts,
    handle: Arc<AuditHandle>,
    tx: Sender<AppEvent>,
    ctx: egui::Context,
    is_mess: bool,
) {
    dlog!("手动审计: 后台线程启动（F5 刷新）");
    thread::spawn(move || {
        let audit_t0 = std::time::Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // read the settings file *before* taking the library lock (README P2-18)
            let settings = GuiSettings::load();
            // Lock order: the option chain on the UI thread takes `opts` and
            // then `lib` (`MameApp::ensure_chain`), so taking the two the other
            // way round here can deadlock an audit against an options dialog
            // opened while it runs. Read the paths first, then the library.
            let rom_paths = {
                let core = opts.lock().unwrap();
                match core.opts.get("rompath") {
                    Some(o) => core.resolve_dir_list(&o.currvalue),
                    None => Vec::new(),
                }
            };
            let (mut snapshot, extra) = {
                let guard = lib.lock().unwrap();
                (guard.clone(), extra_software_with(&guard, &settings))
            };
            dlog!(
                "手动审计: 快照 {} 台机种, {} 个 rompath, {} 个 extra_software",
                snapshot.len(),
                rom_paths.len(),
                extra.len()
            );
            audit::audit_all(&mut snapshot, &rom_paths, &extra, &handle);

            // A cancelled run leaves `snapshot` only half marked. Swapping it in
            // and persisting it as audited=true would freeze wrong availability
            // data into the cache, so drop it and keep the previous state
            // (README P1-9).
            if handle.cancelled() {
                dlog!("手动审计: 已取消，丢弃快照，不改缓存");
                return Err("cancelled".into());
            }
            dlog!("手动审计: 审计完成，开始回填（耗时 {:?}）", audit_t0.elapsed());

            // Swap the audited snapshot back (move, not clone) and persist
            // under one lock. The original only wrote the cache on exit —
            // reading the whole cache back here just to overwrite it was
            // pure overhead on every refresh.
            // move the snapshot back, then re-lock once per heavy step so the
            // UI still gets to paint in between
            {
                let mut guard = lib.lock().unwrap();
                *guard = snapshot;
            }
            let cache_path = GuiSettings::cache_dir().join("gamelist.cache");
            {
                let guard = lib.lock().unwrap();
                match cache::save_library(&cache_path, &guard.mame_version, &guard, true) {
                    Ok(()) => {
                        dlog!("手动审计: 缓存已落盘（audited=true）");
                        let _ = tx.send(AppEvent::Log("gamelist.cache saved.".into()));
                    }
                    Err(e) => {
                        dlog!("手动审计: 缓存落盘失败：{e}");
                        let _ = tx.send(AppEvent::Log(format!("cache save failed: {e}")));
                    }
                }
            }
            let folders = {
                let guard = lib.lock().unwrap();
                folders::compute_folder_cache(&guard, is_mess)
            };
            let _ = tx.send(AppEvent::FoldersReady(Arc::new(folders)));
            let complete = {
                let guard = lib.lock().unwrap();
                guard.games.iter().filter(|g| g.available == 1).count()
            };
            Ok(format!("audit finished: {complete} complete"))
        }));
        let msg = match result {
            Ok(m) => m,
            // `panic!("literal")` hands over a `&str`, not a `String`: matching
            // only the String case reported an empty reason for half of all
            // panics (README N9)
            Err(e) => Err(format!("audit panicked: {}", panic_text(e.as_ref()))),
        };
        dlog!(
            "手动审计: 结束（{}），总耗时 {:?}",
            match &msg {
                Ok(m) => m.as_str(),
                Err(m) => m.as_str(),
            },
            audit_t0.elapsed()
        );
        let _ = tx.send(AppEvent::AuditDone(msg));
        ctx.request_repaint();
    });
}

/// Cache of parsed artwork archives, keyed by path and invalidated by mtime+size.
///
/// Walking a 48 585-entry central directory costs ~35 ms; doing that per game
/// selection while the user clicks through the list would be visible, and the
/// old code paid far worse than that. One entry per archive, rebuilt only when
/// the file changes, is the whole optimisation.
static PACKED_CACHE: std::sync::Mutex<
    Vec<(std::path::PathBuf, crate::core::datindex::FileStamp, Arc<crate::core::zip64::CentralDirectory>)>,
> = std::sync::Mutex::new(Vec::new());

/// How many archives to remember. One per image dock is 7; 16 leaves room for
/// the extra artwork sets users point at without unbounded growth.
const PACKED_CACHE_SLOTS: usize = 16;

/// One packed artwork archive's central directory, cached until the file moves.
fn packed_directory(
    path: &std::path::Path,
) -> Option<Arc<crate::core::zip64::CentralDirectory>> {
    let stamp = crate::core::datindex::FileStamp::of(path)?;
    // Resolve under the lock, then release it: the build below is a ~35 ms read
    // and must not serialise every other dock's preview request behind it.
    let hit = {
        let cache = PACKED_CACHE.lock().ok()?;
        cache
            .iter()
            .find(|(p, s, _)| p == path && *s == stamp)
            .map(|(_, _, d)| Arc::clone(d))
    };
    if let Some(d) = hit {
        return Some(d);
    }
    let dir = Arc::new(crate::core::zip64::read_directory(path)?);
    if let Ok(mut cache) = PACKED_CACHE.lock() {
        match cache.iter().position(|(p, _, _)| p == path) {
            Some(slot) => cache[slot] = (path.to_path_buf(), stamp, Arc::clone(&dir)),
            None => {
                if cache.len() >= PACKED_CACHE_SLOTS {
                    cache.remove(0);
                }
                cache.push((path.to_path_buf(), stamp, Arc::clone(&dir)));
            }
        }
    }
    Some(dir)
}

/// Fetch a preview from a packed artwork archive (`<dir>/<arch>.zip`).
///
/// Loose files still win: a user who extracted `snap/` expects the loose PNG to
/// be used, and this returns `None` without touching any archive in that case
/// unless the archive is actually present and holds the name.
fn packed_preview_bytes(dirs: &str, arch_names: &str, file_filters: &[String]) -> Option<Vec<u8>> {
    for dp in dirs.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let base = std::path::Path::new(dp);
        for arch in arch_names.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            // `arch` may be "." meaning "the directory itself is the artwork dir"
            let zip_path = if arch == "." {
                continue;
            } else {
                base.join(format!("{arch}.zip"))
            };
            if !zip_path.is_file() {
                continue;
            }
            // A loose file of the same name takes precedence, matching the
            // scan order in `iterate_mame_file` (loose before archive).
            let loose_wins = file_filters.iter().any(|f| {
                base.join(arch)
                    .join(f)
                    .is_file()
            });
            if loose_wins {
                return None;
            }
            let dir = packed_directory(&zip_path)?;
            for f in file_filters {
                if let Some(bytes) = crate::core::zip64::read_entry(&zip_path, &dir, f) {
                    return Some(bytes);
                }
            }
        }
    }
    None
}

/// per-dock preview loading (origin: UpdateSelectionThread getScreenshot)
#[allow(clippy::too_many_arguments)]
pub fn load_preview(    dock: usize,
    dirs: String,
    game: String,
    // parent sets to try, in order, when the game has no picture of its own —
    // the clone chain, like the old recursive `getScreenshot`
    fallbacks: Vec<String>,
    // the `snapname` pattern, read once by the caller: re-reading the original GUI ini on
    // every preview request hit the disk for every dock of every game (README P3)
    snapname: Option<String>,
    tx: Sender<AppEvent>,
    ctx: egui::Context,
) {
    thread::spawn(move || {
        dlog!("预览: 装载 dock {} 的 {}（目录 {}）", dock, game, dirs);
        let arch = format!("{};.", dat::dock_archive_name(dock));
        let mut filter_list = vec![format!("{game}.png")];
        if dock == dat::DOCK_SNAP {
            if let Some(snapname) = snapname.as_deref().filter(|s| !s.is_empty()) {
                for v in dat::snapname_variants(snapname, &game) {
                    if !filter_list.contains(&v) {
                        filter_list.push(v);
                    }
                }
            }
        }
        let filters = filter_list.join(";");
        // Try the packed-archive reader first: `zip::ZipArchive` cannot open
        // the 6 GB MAME Plus! snap pack (see `core::zip64`), and a failed
        // attempt there costs minutes of blocked I/O before it gives up.
        let mut data = packed_preview_bytes(&dirs, &arch, &filter_list)
            .or_else(|| dat::load_preview_bytes(&dirs, &arch, &filters));
        for parent in &fallbacks {
            if data.is_some() {
                break;
            }
            data = packed_preview_bytes(&dirs, &arch, &[format!("{parent}.png")])
                .or_else(|| dat::load_preview_bytes(&dirs, &arch, &format!("{parent}.png")));
        }
        let ready = data
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|img| {
                let rgba = img.to_rgba8();
                (rgba.width(), rgba.height(), rgba.into_raw())
            });
        match ready {
            Some((w, h, rgba)) => {
                dlog!("预览: dock {} 的 {} 就绪（{}x{}）", dock, game, w, h);
                let _ = tx.send(AppEvent::SnapReady { dock, game, width: w, height: h, rgba });
            }
            None => {
                dlog!("预览: dock {} 的 {} 没有图", dock, game);
                let _ = tx.send(AppEvent::SnapReady { dock, game, width: 0, height: 0, rgba: Vec::new() });
            }
        }
        ctx.request_repaint();
    });
}

static TAG_RE: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]+>").unwrap());

fn strip_html(s: &str) -> String {
    let s = s.replace("<br>", "\n").replace("<hr>", "\n----------------\n");
    TAG_RE.replace_all(&s, "").replace("&amp;", "&").replace("&quot;", "\"")
}

/// one machine icon (origin: loadIconWorkder over icons_directory).
///
/// `fallbacks` is the inheritance chain — parent set for a clone, host machine
/// for a softlist entry — tried in order until one of them has an icon; the
/// result is filed under `game` either way, so the row caches what it drew.
pub fn load_icon(
    dirs: String,
    game: String,
    fallbacks: Vec<String>,
    tx: Sender<AppEvent>,
    ctx: egui::Context,
) {
    thread::spawn(move || {
        dlog!("图标: 装载 {}（目录 {}, {} 个回退）", game, dirs, fallbacks.len());
        let mut bytes = crate::core::icons::read_game_icon(&dirs, &game);
        for parent in &fallbacks {
            if bytes.is_some() {
                break;
            }
            bytes = crate::core::icons::read_game_icon(&dirs, parent);
        }
        let decoded = bytes
            .and_then(|b| image::load_from_memory(&b).ok())
            .map(|img| {
                let rgba = img.to_rgba8();
                (rgba.width(), rgba.height(), rgba.into_raw())
            });
        match decoded {
            Some((w, h, rgba)) => {
                dlog!("图标: {} 就绪（{}x{}）", game, w, h);
                let _ = tx.send(AppEvent::IconReady { game, width: w, height: h, rgba });
            }
            None => {
                dlog!("图标: {} 没有图", game);
                let _ = tx.send(AppEvent::IconReady {
                    game,
                    width: 0,
                    height: 0,
                    rgba: Vec::new(),
                });
            }
        }
        ctx.request_repaint();
    });
}

/// per-dock DAT loading (origin: getHistory / local-language pass)
#[allow(clippy::too_many_arguments)]
pub fn load_dat(
    dock: usize,
    file_path: String,
    game: String,
    search_tag: String,
    cloneof: String,
    sourcefile: String,
    dark: bool,
    // `<first langpath>/<language>`, or empty when no language directory is
    // configured. The dat file name is appended here, not by the caller (N2).
    lang_dir: String,
    tx: Sender<AppEvent>,
    ctx: egui::Context,
) {
    thread::spawn(move || {
        dlog!(
            "文档: 装载 dock {} 的 {}（主文件 {}, 语言目录 {:?}）",
            dock, game, file_path, lang_dir
        );
        let method = dock;
        let tag = if method == dat::DOCK_DRIVERINFO {
            sourcefile.clone()
        } else {
            search_tag.clone()
        };
        let mut html = String::new();
        if !lang_dir.is_empty() {
            // `file_path` is absolute, so joining it would discard `lang_dir`
            // entirely — take its file name and put that under the language dir
            let name = std::path::Path::new(&file_path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| file_path.clone());
            let local = std::path::Path::new(&lang_dir).join(name);
            html = read_one_dat(&local, &tag, method, dark, &cloneof);
        }
        if html.is_empty() {
            html = read_one_dat(std::path::Path::new(&file_path), &tag, method, dark, &cloneof);
        } else {
            html.push_str("<hr>");
            html.push_str(&read_one_dat(
                std::path::Path::new(&file_path),
                &tag,
                method,
                dark,
                &cloneof,
            ));
        }
        let text = strip_html(&html);
        match text.is_empty() {
            true => dlog!("文档: dock {} 的 {} 没有内容", dock, game),
            false => dlog!("文档: dock {} 的 {} 就绪（{} 字节）", dock, game, text.len()),
        }
        let _ = tx.send(AppEvent::DatReady {
            dock,
            game,
            text: if text.is_empty() { None } else { Some(text) },
        });
        ctx.request_repaint();
    });
}

/// Read one record out of a document file, by path.
///
/// Two formats answer to the same dock, and the loader picks by **what the file
/// is**, not by what it is called:
///
/// * `history.xml` — the modern Arcade-History format (since MAME ~0.228). It is
///   the default for the History dock, so it gets the fast path: a
///   mtime-validated name→byte-range index (build one 64 MB pass, then seek to
///   the record).
/// * a `$info=` DAT — the legacy format. Still read, because users have these
///   from older installs and the language packs under `lang/<code>/` are all
///   DATs.
///
/// The dispatch is by extension, and the DAT scanner is the fallback for
/// everything else — including a `.xml` file that turns out to be something
/// unrelated. Both produce identical output for the History dock, so a wrong
/// guess costs a scan, never a wrong panel.
fn read_one_dat(path: &std::path::Path, tag: &str, method: usize, dark: bool, cloneof: &str) -> String {
    if path.is_file() {
        if is_xml(path) {
            // `.xml` is only meaningful for the docks that ship as XML today;
            // anything else falls through to the DAT scanner below.
            if method == dat::DOCK_HISTORY {
                if let Some(hit) = crate::core::historyxml::lookup(path, tag) {
                    if !hit.is_empty() {
                        return dat::finish_record(hit, tag, method, dark);
                    }
                }
            }
        }
        if let Some(hit) = crate::core::datindex::history_indexed(path, tag, method, dark, cloneof) {
            if !hit.is_empty() {
                return hit;
            }
        }
    }
    dat::read_dat_bytes(&path.to_string_lossy())
        .map(|b| dat::get_history(&b, tag, method, dark, cloneof))
        .unwrap_or_default()
}

/// True when the path carries an `.xml` extension, case-insensitively.
fn is_xml(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
}

/// pump mame -verifyroms/-verifysamples output (origin: MameExeRomAuditor)
pub fn run_verify(mame: MameBinary, args: Vec<String>, tx: Sender<AppEvent>, ctx: egui::Context) {
    thread::spawn(move || {
        match mame.spawn_run(&args) {
            Ok(mut child) => {
                let tx2 = tx.clone();
                pump_lines(&mut child, |line| {
                    let _ = tx2.send(AppEvent::VerifyLine(line));
                });
            }
            // a silent failure left the verify window empty with no explanation
            Err(e) => {
                let _ = tx.send(AppEvent::VerifyLine(format!("failed to start mame: {e}")));
            }
        }
        let _ = tx.send(AppEvent::VerifyDone);
        ctx.request_repaint();
    });
}
