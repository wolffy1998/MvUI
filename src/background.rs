//! Background job spawners (listxml+showconfig boot chain, verify, previews, dats).
//! Long jobs never hold the library mutex while the UI renders: boot verifys a
//! local library before publishing; refresh verifys a snapshot and swaps it back.

use crate::events::{AppEvent, ReadyPayload, SharedLib, SharedOpts};
use crate::core::verify::{self, VerifyHandle};
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

/// 冷启动的完整链路：读缓存 → `mame -listxml` → `mame -showconfig` → 校验。
pub fn boot_library(mame: MameBinary, tx: Sender<AppEvent>, ctx: egui::Context) {
    thread::spawn(move || {
        // set as soon as `LibraryReady` has been sent: from that moment on the
        // UI already owns a usable library, so a later panic must not be
        // reported as a boot failure (that used to overwrite the ready state
        // with an error and leave `VerifyDone` pending forever)
        let published = Arc::new(AtomicBool::new(false));
        let published_for_run = Arc::clone(&published);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            boot_run(&mame, &tx, &published_for_run)
        }));
        if let Err(e) = result {
            let msg = panic_text(e.as_ref());
            if published.load(Ordering::Relaxed) {
                let _ = tx.send(AppEvent::Log(format!("boot: {msg}")));
                let _ = tx.send(AppEvent::VerifyDone(Err(msg)));
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
        Ok(data) if data.verified => {
            dlog!("引导: 缓存命中且已校验（{} 台），跳过 listxml 与校验", data.library.len());
            let _ = tx.send(AppEvent::Log("loaded games from cache.".into()));
            // a fully verified cache needs neither -listxml nor a re-verify
            finish_boot_cached(data.library, mame, tx, published);
            dlog!("引导: 热启动完成，耗时 {:?}", boot_t0.elapsed());
            return;
        }
        Ok(data) => {
            // -listxml result is cached but the verify never finished last time
            // (closed mid-verify): skip the parse, just finish the verify
            dlog!(
                "引导: 缓存命中但未校验（{} 台），跳过 listxml 解析，只补校验",
                data.library.len()
            );
            let _ = tx.send(AppEvent::Log(
                "loaded games from cache, verify still pending.".into(),
            ));
            finish_boot(data.library, mame, tx, published, true);
            dlog!("引导: 补校验完成，耗时 {:?}", boot_t0.elapsed());
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

    finish_boot(library, mame, tx, published, false);
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

/// 从已加载的选项链里取 `samplepath`，写进 `core::samples` 的全局目录表。
///
/// **热启动和冷启动都必须调它。** 这正是本轮修的那个 BUG：`set_sample_dirs`
/// 原先只写在 `finish_boot`（冷启动/补校验）里，而 `finish_boot_cached`
/// （缓存命中且 `verified=true`——**绝大多数正常启动都走这条**）完全没有这一段。
/// 于是热启动时全局目录表恒为空，`find_sample_archive` 一律返回 `None`，
/// Samples 段对**所有**游戏恒为灰色"未知"，永远不显示 `拥有 18/18`。
/// 单独跑 `examples/samples_probe` 却能验出 `genpin 18/18 Good`——因为探针
/// 自己调了 `set_sample_dirs`，绕过了引导。这就是"探针绿、程序灰"的原因。
fn publish_sample_dirs(opts: &SharedOpts) {
    let dirs: Vec<PathBuf> = {
        let guard = opts.lock().unwrap();
        match guard.opts.get("samplepath") {
            Some(o) if !o.currvalue.trim().is_empty() => guard.resolve_dir_list(&o.currvalue),
            // mame.ini 没这一项（或留空）时用 MAME 官方默认（相对 MAME 目录）
            _ => guard.resolve_dir_list(crate::core::samples::DEFAULT_SAMPLEPATH),
        }
    };
    crate::core::samples::set_sample_dirs(dirs.clone());
    dlog!(
        "引导: 样本目录 {} 个: {}",
        dirs.len(),
        dirs.iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    );
}

/// warm start: publish options + library as-is (verify state comes from cache)
fn finish_boot_cached(
    library: GameLibrary,
    mame: &MameBinary,
    tx: &Sender<AppEvent>,
    published: &Arc<AtomicBool>,
) {
    dlog!("引导: 热启动路径，发布选项与游戏库（不校验）");
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
    let opts: SharedOpts = Arc::new(Mutex::new(core));
    let _ = tx.send(AppEvent::OptionsReady(Ok(Arc::clone(&opts))));
    // 样本目录：热启动同样要发布（见 `publish_sample_dirs` 的注释——漏了它
    // 面板上的 Samples 段会恒为灰色"未知"）。
    publish_sample_dirs(&opts);
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
        // 这条路径只在 `data.verified` 为真时走到（boot_run 的三分支），
        // 所以这里一定是已校验的库
        verified: true,
    })));
    published.store(true, Ordering::Relaxed);
}

/// cold start: options + verify + cache — all on the local (unlocked) library
///
/// `from_cache` = 这次的游戏库是从 `gamelist.cache` 读出来的（上次校验没跑完就
/// 关了程序）。那条路上**跳过校验前那次落盘** —— 缓存**刚读过、内容一模一样**，
/// 再写一遍是纯浪费：`save_library` 要把 49676 台序列化一遍（实测 ~2s），
/// 而它挡在 `LibraryReady` 之前，于是用户看到的是"重启后卡在黑屏两秒才出列表"。
///
/// 冷启动（`-listxml` 刚解析出来）那次落盘**必须留着**：那是把解析结果存下来，
/// 否则中途关掉下次得重新解析 40 秒。
fn finish_boot(
    mut library: GameLibrary,
    mame: &MameBinary,
    tx: &Sender<AppEvent>,
    published: &Arc<AtomicBool>,
    from_cache: bool,
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
        "引导: 校验 rompath {} 个: {}",
        rom_paths.len(),
        rom_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("; ")
    );
    // 样本目录（`mame.ini` 的 `samplepath`）。**与 rompath 分开取**：样本集
    // 是独立包（`samples/{name}.zip`），不跟 rom 放一起，而且它不进校验
    // 单元——只用来算面板上那一行 `拥有 9/9`。
    // 没有这一段时样本行全是"未校验"（灰），而不是错的"缺失"。
    publish_sample_dirs(&opts);
    let _ = tx.send(AppEvent::Log(format!("verify: {} rom dirs", rom_paths.len())));

    // 把游戏库落盘，**必须在校验之前** —— 校验是慢的那一步，而校验跑完才落盘
    // 的话，中途关掉程序下次得从 `-listxml` 重新解析 40 秒。
    let cache_path = GuiSettings::cache_dir().join("gamelist.cache");
    if from_cache {
        // 缓存刚读过，内容一样，不必再写（见 `finish_boot` 的参数注释）
        dlog!("引导: 库来自缓存，跳过校验前落盘（verified 仍是 false）");
    } else {
        let save_t0 = std::time::Instant::now();
        match cache::save_library(&cache_path, &mame.version, &library, false) {
            Ok(()) => {
                dlog!(
                    "引导: 校验前落盘缓存（verified=false），耗时 {:?}",
                    save_t0.elapsed()
                );
                let _ = tx.send(AppEvent::Log("parsed library cached.".into()));
            }
            Err(e) => {
                dlog!("引导: 校验前落盘缓存失败：{e}");
                let _ = tx.send(AppEvent::Log(format!("cache save failed: {e}")));
            }
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
    // then verify in a background thread and swap results in
    dlog!(
        "引导: LibraryReady 已发布（校验前, {} 台机种）",
        library.len()
    );
    let lib_shared: SharedLib = Arc::new(Mutex::new(library.clone()));
    let _ = tx.send(AppEvent::LibraryReady(Ok(ReadyPayload {
        lib: lib_shared.clone(),
        folders: folder_cache,
        from_cache: false,
        // 校验**还没跑**（这一步就是为校验腾出界面），所以是 false
        verified: false,
    })));
    published.store(true, Ordering::Relaxed);

    let extra = extra_software_for(&library);
    dlog!(
        "引导: 校验开始前准备就绪（{} 个 extra_software 条目, {} 台机种）",
        extra.len(),
        lib_shared.lock().map(|g| g.len()).unwrap_or(0)
    );
    let handle = Arc::new(VerifyHandle::new());
    // hand the handle to the UI before the (slow) verify starts, so the status
    // bar can show "Verifying nn%" from the first tick instead of waiting for
    // the verify to report anything itself
    let _ = tx.send(AppEvent::VerifyStarted(handle.clone()));
    {
        // forward verify progress to the status bar
        let tx2 = tx.clone();
        let h2 = handle.clone();
        thread::spawn(move || loop {
            let (done, total, cur) = h2.snapshot();
            let _ = tx2.send(AppEvent::VerifyProgress {
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
    // The verify works on a private copy, so a panic in it must not escape to the
    // outer handler: the library has already been published and `VerifyDone` has
    // to arrive anyway, or the status bar spins forever and the start button
    // stays disabled (README P1-8).
    let verify_t0 = std::time::Instant::now();
    let verify_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        verify::verify_all(&mut library, &rom_paths, &extra, &handle);
    }));
    dlog!("引导: 校验返回，耗时 {:?}", verify_t0.elapsed());
    if let Err(p) = verify_result {
        // the progress forwarder waits on `finished` — make sure it exits
        handle.finish();
        let msg = format!("verify failed: {}", panic_text(p.as_ref()));
        dlog!("引导: 校验 panic：{msg}");
        let _ = tx.send(AppEvent::Log(msg.clone()));
        let _ = tx.send(AppEvent::VerifyDone(Err(msg)));
        return;
    }
    if handle.cancelled() {
        // discard the half-verified snapshot: the published library stays as it
        // was and the cache keeps verified=false, so the next start resumes
        dlog!("引导: 校验被取消，丢弃半成品快照（缓存保持 verified=false）");
        let _ = tx.send(AppEvent::Log("verify cancelled.".into()));
        let _ = tx.send(AppEvent::VerifyDone(Err("cancelled".into())));
        return;
    }
    {
        // move, not clone — the library is ~46k entries
        let mut guard = lib_shared.lock().unwrap();
        *guard = library;
    }
    // Availability / Unavailability counts depend on the verify result, so the
    // folder tree has to be rebuilt once the verify is in.
    let folders = {
        let guard = lib_shared.lock().unwrap();
        folders::compute_folder_cache(&guard, is_mess)
    };
    let _ = tx.send(AppEvent::FoldersReady(Arc::new(folders)));
    // re-save with verified = true: from now on a start is cache-only
    {
        let guard = lib_shared.lock().unwrap();
        match cache::save_library(&cache_path, &mame.version, &guard, true) {
            Ok(()) => {
                dlog!("引导: 校验后落盘缓存（verified=true）");
                let _ = tx.send(AppEvent::Log("gamelist.cache saved (verified).".into()));
            }
            Err(e) => {
                dlog!("引导: 校验后落盘缓存失败：{e}");
                let _ = tx.send(AppEvent::Log(format!("cache save failed: {e}")));
            }
        }
    }
    dlog!("引导: 校验完成，VerifyDone 已发出");
    let _ = tx.send(AppEvent::VerifyDone(Ok("verify finished".into())));

}

fn extra_software_for(lib: &GameLibrary) -> HashMap<String, String> {
    let settings = GuiSettings::load();
    extra_software_with(lib, &settings)
}

/// Same, with the (disk-backed) settings already in hand.
///
/// The manual-verify path used to call `extra_software_for` while holding the
/// library lock — reading and parsing `the original GUI ini` inside the critical section
/// stalled the UI for the frame that started the verify (README P2-18).
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

/// refresh verify on a snapshot; UI keeps rendering (origin: RomVerifyor thread)
pub fn run_verify(
    lib: SharedLib,
    opts: SharedOpts,
    handle: Arc<VerifyHandle>,
    tx: Sender<AppEvent>,
    ctx: egui::Context,
    is_mess: bool,
) {
    dlog!("手动校验: 后台线程启动（F5 刷新）");
    thread::spawn(move || {
        let verify_t0 = std::time::Instant::now();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // read the settings file *before* taking the library lock (README P2-18)
            let settings = GuiSettings::load();
            // Lock order: the option chain on the UI thread takes `opts` and
            // then `lib` (`MameApp::ensure_chain`), so taking the two the other
            // way round here can deadlock an verify against an options dialog
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
                "手动校验: 快照 {} 台机种, {} 个 rompath, {} 个 extra_software",
                snapshot.len(),
                rom_paths.len(),
                extra.len()
            );
            verify::verify_all(&mut snapshot, &rom_paths, &extra, &handle);

            // A cancelled run leaves `snapshot` only half marked. Swapping it in
            // and persisting it as verified=true would freeze wrong availability
            // data into the cache, so drop it and keep the previous state
            // (README P1-9).
            if handle.cancelled() {
                dlog!("手动校验: 已取消，丢弃快照，不改缓存");
                return Err("cancelled".into());
            }
            dlog!("手动校验: 校验完成，开始回填（耗时 {:?}）", verify_t0.elapsed());

            // Swap the verified snapshot back (move, not clone) and persist
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
                        dlog!("手动校验: 缓存已落盘（verified=true）");
                        let _ = tx.send(AppEvent::Log("gamelist.cache saved.".into()));
                    }
                    Err(e) => {
                        dlog!("手动校验: 缓存落盘失败：{e}");
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
            Ok(format!("verify finished: {complete} complete"))
        }));
        let msg = match result {
            Ok(m) => m,
            // `panic!("literal")` hands over a `&str`, not a `String`: matching
            // only the String case reported an empty reason for half of all
            // panics (README N9)
            Err(e) => Err(format!("verify panicked: {}", panic_text(e.as_ref()))),
        };
        dlog!(
            "手动校验: 结束（{}），总耗时 {:?}",
            match &msg {
                Ok(m) => m.as_str(),
                Err(m) => m.as_str(),
            },
            verify_t0.elapsed()
        );
        let _ = tx.send(AppEvent::VerifyDone(msg));
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

/// pump mame -verifyroms/-verifysamples output (origin: MameExeRomVerifyor)
///
/// **当前没有菜单入口**（见 `run_game_verify` 的注释：菜单里的校验项已按
/// 用户要求删除，统一走「刷新档案」）。这是 1.8.2 那条原样搬过来的输出泵，
/// 保留以便将来接回 MAME 原生校验。
#[allow(dead_code)]
pub fn run_verify_output_pump(mame: MameBinary, args: Vec<String>, tx: Sender<AppEvent>, ctx: egui::Context) {
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
        let _ = tx.send(AppEvent::VerifyOutputDone);
        ctx.request_repaint();
    });
}

#[cfg(test)]
mod tests {
    /// **重启续校验时，必须先发布游戏列表，再开始枚举/校验。**
    ///
    /// 用户 2026-10-06 明确要求：「已经解析了 listxml 且已经缓存，但是没校验完
    /// 就重启，第二次重启应该是先加载 XML 缓存，等游戏信息显示出来后再重新开始
    /// 枚举、校验」。
    ///
    /// 这条用**源码结构**钉住而不是跑真的引导（真引导要 40 秒起，而且要真MAME）。
    /// 它守的不变量很具体：`finish_boot` 里发`LibraryReady` 的那一行必须排在
    /// 调 `verify_all` 的那一行**之前**。一旦有人把发布挪到校验后面（比如"等数据
    /// 齐了再一次性给 UI"，听起来很合理），用户就要盯着黑屏等几分钟。
    #[test]
    fn the_library_is_published_before_the_verify_starts() {
        let src = include_str!("background.rs");
        let body = src
            .split("fn finish_boot(")
            .nth(1)
            .expect("找不到 finish_boot");
        let ready = body
            .find("AppEvent::LibraryReady")
            .expect("finish_boot 里没有发 LibraryReady");
        let work = body
            .find("verify::verify_all")
            .expect("finish_boot 里没有调 verify_all");
        assert!(
            ready < work,
            "LibraryReady({ready}) 必须排在 verify_all({work}) 之前，\
             否则用户要等校验完才看到游戏列表"
        );
    }

    /// 缓存命中那条路**不许再落一次盘**。
    ///
    /// `save_library` 要把 49676 台序列化一遍（实测 ~2s），而它挡在
    /// `LibraryReady` 之前 —— 每次"校验没跑完就重启"都要白等这两秒才出列表。
    /// 缓存**刚读过、内容一模一样**，重写是纯浪费。
    ///
    /// 反过来冷启动那次落盘**必须留着**：那是把刚解析出来的 listxml 存下来，
    /// 否则中途关掉下次得重新解析 40 秒。所以判据是"来自缓存"这个flag。
    #[test]
    fn the_cached_boot_does_not_rewrite_the_cache() {
        let src = include_str!("background.rs");
        let body = src.split("fn finish_boot(").nth(1).expect("找不到 finish_boot");
        // 落盘必须在 `if from_cache` 的 else 分支里
        let guard = body.find("if from_cache {").expect("没有 from_cache 分支");
        let save = body
            .find("cache::save_library")
            .expect("finish_boot 里没有落盘");
        assert!(
            guard < save,
            "落盘必须在 `if from_cache {{` 之后（缓存路径要跳过它）"
        );
    }
}
