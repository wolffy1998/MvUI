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

/// full boot chain (origin: MainWindow::init → pMameDat load → -listxml →
/// -showconfig → loadDefault → autoAudit)
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
    crate::app::perf_log("boot_run: begin");
    let cache_path = GuiSettings::cache_dir().join("gamelist.cache");

    // 1) cache (origin: pMameDat->load(); version mismatch → rebuild)
    crate::app::perf_log("boot_run: cache load begin");
    match cache::load(&cache_path, &mame.version) {
        Ok(data) if data.audited => {
            crate::app::perf_log("boot_run: cache load done (audited)");
            let _ = tx.send(AppEvent::Log("loaded games from cache.".into()));
            // a fully audited cache needs neither -listxml nor a re-audit
            finish_boot_cached(data.library, mame, tx, published);
            return;
        }
        Ok(data) => {
            // -listxml result is cached but the audit never finished last time
            // (closed mid-audit): skip the parse, just finish the audit
            crate::app::perf_log("boot_run: cache load done (not audited)");
            let _ = tx.send(AppEvent::Log(
                "loaded games from cache, audit still pending.".into(),
            ));
            finish_boot(data.library, mame, tx, published);
            return;
        }
        Err(cache::CacheError::Missing) => {}
        Err(e) => {
            let _ = tx.send(AppEvent::Log(format!("cache unusable ({e}); full refresh")));
        }
    }

    // 2) listxml (origin: MameDat(0,1) child chain)
    let _ = tx.send(AppEvent::Log(
        "running mame -listxml (this can take a while)…".into(),
    ));
    let mut child = match mame.spawn_listxml() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(AppEvent::LibraryReady(Err(e.to_string())));
            return;
        }
    };
    let tx2 = tx.clone();
    let parse_result = child
        .stdout
        .take()
        .map(|out| {
            listxml::parse_from_reader(std::io::BufReader::new(out), false, &mut |done| {
                let _ = tx2.send(AppEvent::LibProgress { done, total: 0, stage: "listxml".into() });
            })
        })
        .unwrap_or_else(|| Err("no stdout".into()));
    let mut library = match parse_result {
        Ok(l) => l,
        Err(e) => {
            let _ = child.kill();
            // reap it: a killed child nobody waits for keeps the pipe open
            let _ = child.wait();
            let _ = tx.send(AppEvent::LibraryReady(Err(format!("listxml: {e}"))));
            return;
        }
    };
    let _ = child.wait();
    let _ = tx.send(AppEvent::Log("listxml parsed.".into()));

    // 3) showconfig (origin: loadDefaultIni child chain)
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
        Err(_) => String::new(),
    };
    library.default_ini = default_ini;
    library.mame_version = mame.version.clone();
    library.rebuild_indexes();
    library.complete_data();

    finish_boot(library, mame, tx, published);
}

/// warm start: publish options + library as-is (audit state comes from cache)
fn finish_boot_cached(
    library: GameLibrary,
    mame: &MameBinary,
    tx: &Sender<AppEvent>,
    published: &Arc<AtomicBool>,
) {
    crate::app::perf_log("finish_boot_cached: begin");
    let mame_dir = mame.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let gui: HashMap<String, String> = HashMap::new();
    let is_mess = mame
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().contains("mess"))
        .unwrap_or(false);
    let mut core = OptionCore::load_default(&library.default_ini, &library, &gui, &mame_dir);
    core.mess_like = is_mess;
    core.load_global(&gui);
    for w in &core.warnings {
        let _ = tx.send(AppEvent::Log(w.clone()));
    }
    let _ = tx.send(AppEvent::OptionsReady(Ok(Arc::new(Mutex::new(core)))));
    crate::app::perf_log("finish_boot_cached: building folder cache");
    let folder_cache = Arc::new(folders::compute_folder_cache(&library, is_mess));
    crate::app::perf_log("finish_boot_cached: LibraryReady sent");
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
    let mut core = OptionCore::load_default(&library.default_ini, &library, &gui, &mame_dir);
    core.mess_like = mame
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().contains("mess"))
        .unwrap_or(false);
    core.load_global(&gui);
    for w in &core.warnings {
        let _ = tx.send(AppEvent::Log(w.clone()));
    }
    let opts: SharedOpts = Arc::new(Mutex::new(core));
    let _ = tx.send(AppEvent::OptionsReady(Ok(opts.clone())));
    crate::app::perf_log("finish_boot: options ready");

    // rompath from the loaded global chain (relative → mame dir)
    let rom_paths: Vec<PathBuf> = {
        let guard = opts.lock().unwrap();
        match guard.opts.get("rompath") {
            Some(o) => guard.resolve_dir_list(&o.currvalue),
            None => Vec::new(),
        }
    };
    let _ = tx.send(AppEvent::Log(format!("audit: {} rom dirs", rom_paths.len())));

    // Persist the parsed library *before* auditing. The audit is the slow part
    // and used to be the only thing that wrote the cache, so closing the app
    // mid-audit meant a full -listxml re-parse on every single start.
    let cache_path = GuiSettings::cache_dir().join("gamelist.cache");
    match cache::save_library(&cache_path, &mame.version, &library, false) {
        Ok(()) => {
            let _ = tx.send(AppEvent::Log("parsed library cached.".into()));
        }
        Err(e) => {
            let _ = tx.send(AppEvent::Log(format!("cache save failed: {e}")));
        }
    }

    crate::app::perf_log("finish_boot: building folder cache");
    let is_mess = mame
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().contains("mess"))
        .unwrap_or(false);
    let folder_cache = Arc::new(folders::compute_folder_cache(&library, is_mess));

    // publish the list FIRST (UI appears right after listxml),
    // then audit in a background thread and swap results in
    crate::app::perf_log("finish_boot: LibraryReady sent (pre-audit)");
    let lib_shared: SharedLib = Arc::new(Mutex::new(library.clone()));
    let _ = tx.send(AppEvent::LibraryReady(Ok(ReadyPayload {
        lib: lib_shared.clone(),
        folders: folder_cache,
        from_cache: false,
    })));
    published.store(true, Ordering::Relaxed);

    let extra = extra_software_for(&library);
    let handle = Arc::new(AuditHandle::new());
    // hand the handle to the UI before the (slow) audit starts, otherwise the
    // cold-start audit cannot be cancelled at all
    let _ = tx.send(AppEvent::AuditStarted(handle.clone()));
    {
        // forward audit progress to the status bar
        let tx2 = tx.clone();
        let h2 = handle.clone();
        thread::spawn(move || loop {
            let (done, total, cur) = h2.snapshot();
            let _ = tx2.send(AppEvent::LibProgress {
                done,
                total,
                stage: format!("audit:{cur}"),
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
    let audit_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        audit::audit_all(&mut library, &rom_paths, &extra, &handle);
    }));
    crate::app::perf_log("finish_boot: audit done");
    if let Err(p) = audit_result {
        // the progress forwarder waits on `finished` — make sure it exits
        handle.finish();
        let msg = format!("audit failed: {}", panic_text(p.as_ref()));
        let _ = tx.send(AppEvent::Log(msg.clone()));
        let _ = tx.send(AppEvent::AuditDone(Err(msg)));
        return;
    }
    if handle.cancelled() {
        // discard the half-audited snapshot: the published library stays as it
        // was and the cache keeps audited=false, so the next start resumes
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
                let _ = tx.send(AppEvent::Log("gamelist.cache saved (audited).".into()));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::Log(format!("cache save failed: {e}")));
            }
        }
    }
    let _ = tx.send(AppEvent::AuditDone(Ok("audit finished".into())));

}

fn extra_software_for(lib: &GameLibrary) -> HashMap<String, String> {
    let settings = GuiSettings::load();
    extra_software_with(lib, &settings)
}

/// Same, with the (disk-backed) settings already in hand.
///
/// The manual-audit path used to call `extra_software_for` while holding the
/// library lock — reading and parsing `mamepgui.ini` inside the critical section
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
    thread::spawn(move || {
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
            audit::audit_all(&mut snapshot, &rom_paths, &extra, &handle);

            // A cancelled run leaves `snapshot` only half marked. Swapping it in
            // and persisting it as audited=true would freeze wrong availability
            // data into the cache, so drop it and keep the previous state
            // (README P1-9).
            if handle.cancelled() {
                return Err("cancelled".into());
            }

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
                        let _ = tx.send(AppEvent::Log("gamelist.cache saved.".into()));
                    }
                    Err(e) => {
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
        let _ = tx.send(AppEvent::AuditDone(msg));
        ctx.request_repaint();
    });
}

/// per-dock preview loading (origin: UpdateSelectionThread getScreenshot)
#[allow(clippy::too_many_arguments)]
pub fn load_preview(
    dock: usize,
    dirs: String,
    game: String,
    // parent sets to try, in order, when the game has no picture of its own —
    // the clone chain, like the old recursive `getScreenshot`
    fallbacks: Vec<String>,
    // the `snapname` pattern, read once by the caller: re-reading mamepgui.ini on
    // every preview request hit the disk for every dock of every game (README P3)
    snapname: Option<String>,
    tx: Sender<AppEvent>,
    ctx: egui::Context,
) {
    thread::spawn(move || {
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
        let mut data = dat::load_preview_bytes(&dirs, &arch, &filters);
        for parent in &fallbacks {
            if data.is_some() {
                break;
            }
            let filters = format!("{parent}.png");
            data = dat::load_preview_bytes(&dirs, &arch, &filters);
        }
        let ready = data
            .and_then(|bytes| image::load_from_memory(&bytes).ok())
            .map(|img| {
                let rgba = img.to_rgba8();
                (rgba.width(), rgba.height(), rgba.into_raw())
            });
        match ready {
            Some((w, h, rgba)) => {
                let _ = tx.send(AppEvent::SnapReady { dock, game, width: w, height: h, rgba });
            }
            None => {
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
                let _ = tx.send(AppEvent::IconReady { game, width: w, height: h, rgba });
            }
            None => {
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
        let _ = tx.send(AppEvent::DatReady {
            dock,
            game,
            text: if text.is_empty() { None } else { Some(text) },
        });
        ctx.request_repaint();
    });
}

/// Read one record out of a DAT, by path.
///
/// The byte-range index (design §3.2) answers from `mtime`-validated memory and
/// reads only the record's own bytes, turning a 10-20 MB linear scan per lookup
/// into a few KB read. It only covers plain files, so anything else — a DAT
/// inside a zip, an unreadable file, or an index the tag is absent from —
/// falls back to the original `read_dat_bytes` + `get_history` scan. The two
/// produce identical output; `datindex`'s tests pin that.
fn read_one_dat(path: &std::path::Path, tag: &str, method: usize, dark: bool, cloneof: &str) -> String {
    if path.is_file() {
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
