//! List filtering, selection, launching and list rendering
//! (origin: Gamelist + GameListSortFilterProxyModel + GameListDelegate).

use crate::app::{
    MameApp, PlayKind, ListMode, COL_CLONEOF, COL_DEFAULT_WIDTH, COL_DESC, COL_LAST, COL_MFTR,
    COL_MIN_WIDTH, COL_NAME, COL_ROM, COL_SRC, COL_YEAR, COLUMN_TITLES, F_CLONES, F_MECHANICAL,
    F_NONWORKING, F_UNAVAILABLE,
};
use crate::core::folders::{self, FolderKind};
use crate::core::launcher::RunMode;
use crate::core::library::GameLibrary;
use crate::core::model::{GameMeta, GAME_COMPLETE, STATUS_GOOD};
use crate::icons;
use egui_extras::{Column, TableBuilder};
use crate::core::launcher;
use std::cell::Cell;
use std::path::PathBuf;

/// the status-bar view of the current game (see `MameApp::status_info`)
pub struct StatusInfo {
    /// driver grades in the order ui.rs renders them
    pub badges: [u8; 8],
    pub title: String,
    pub year: String,
}

impl MameApp {
    pub fn has_game(&self) -> bool {
        !self.current_game.is_empty() && self.selected.is_some()
    }

    pub fn src_properties_label(&self) -> String {
        self.current_meta()
            .map(|m| self.tf("Properties for {}", m.sourcefile))
            .unwrap_or_else(|| self.tr("Properties for ..."))
    }

    pub fn current_meta(&self) -> Option<crate::core::model::GameMeta> {
        let lib = self.lib.as_ref()?;
        let guard = lib.lock().ok()?;
        guard.get(&self.current_game).cloned()
    }

    /// Everything the status bar needs, without cloning a whole `GameMeta`.
    ///
    /// `draw_status` runs every frame, and `current_meta()` deep-copies the game
    /// including its `roms`/`disks` vectors (hundreds of `String`s for a big
    /// driver) — pure per-frame garbage (README P2-23).
    pub fn status_info(&self) -> Option<StatusInfo> {
        let lib = self.lib.as_ref()?;
        let guard = lib.lock().ok()?;
        let m = guard.get(&self.current_game)?;
        let d = &m.driver;
        Some(StatusInfo {
            badges: [
                d.status,
                d.emulation,
                d.color,
                d.sound,
                d.graphic,
                d.cocktail,
                d.protection,
                d.savestate,
            ],
            // origin: getDesc(useLocal) — localized description when enabled
            title: if self.local_game_list && !m.lc_desc.is_empty() {
                m.lc_desc.clone()
            } else {
                m.description.clone()
            },
            year: m.year.clone(),
        })
    }

    // ------------------------------------------------------------------
    // filtering (origin: filterFolderChanged + filterAcceptsRow)
    // ------------------------------------------------------------------

    pub fn refilter(&mut self) {
        self.needs_refilter = false;
        let Some(lib) = self.lib.clone() else {
            self.visible = std::rc::Rc::new(Vec::new());
            return;
        };
        let guard = lib.lock().unwrap();
        let kind = self.folder_kind.clone();
        let key = self.folder_key.clone().unwrap_or_default();
        let query = {
            // origin: skip 1-char latin search; whitespace → wildcard AND
            let t = self.search.trim().to_string();
            if t.chars().count() == 1 && t.chars().next().map(|c| c < '\u{3000}').unwrap_or(true) {
                String::new()
            } else {
                let mut q = t;
                while q.contains("  ") {
                    q = q.replace("  ", " ");
                }
                q.replace(' ', "*")
            }
        };
        let ext_member: Option<Vec<String>> = match &kind {
            FolderKind::Ext(name) => {
                let section = self
                    .current_folder
                    .rsplit('/')
                    .next()
                    .unwrap_or("ROOT_FOLDER");
                self.ext_folder_data
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, s)| {
                        if section == *name {
                            s.games_in("ROOT_FOLDER")
                        } else {
                            s.games_in(section)
                        }
                    })
            }
            _ => None,
        };
        let mut rows: Vec<usize> = Vec::new();
        for (i, g) in guard.games.iter().enumerate() {
            // flag filters (origin §10)
            if g.is_device {
                continue;
            }
            if self.filter_flags & F_CLONES != 0 && !g.is_ext_rom && !g.cloneof.is_empty() {
                continue;
            }
            if self.filter_flags & F_NONWORKING != 0 && !g.is_ext_rom && g.driver.status == STATUS_GOOD {
                continue;
            }
            if self.filter_flags & F_UNAVAILABLE != 0 && !g.is_ext_rom && g.available == GAME_COMPLETE {
                continue;
            }
            if self.filter_flags & F_MECHANICAL != 0 && !g.is_ext_rom && g.is_mechanical {
                continue;
            }
            // folder filter
            let folder_ok: bool = match (&kind, &key) {
                (FolderKind::Ext(_), _) => ext_member
                    .as_ref()
                    .map(|list| list.contains(&g.name))
                    .unwrap_or(false),
                (k, k0) if k0.is_empty() => folders::in_dimension(k, g),
                (FolderKind::Console, k) => g.is_ext_rom && g.romof == *k,
                (FolderKind::Bios, k) => !g.is_bios && g.bios_of(&guard) == *k,
                (k, key) => folders::matches(k, key, g),
            };
            if !folder_ok {
                continue;
            }
            // search filter: name/description (wildcard)
            if !query.is_empty() {
                let hay = format!("{} {} {}", g.name, g.description, {
                    if self.local_game_list && !g.lc_desc.is_empty() {
                        g.lc_desc.clone()
                    } else {
                        String::new()
                    }
                });
                if !wildcard_match(&hay.to_lowercase(), &query.to_lowercase()) {
                    continue;
                }
            }
            rows.push(i);
        }
        // sort (origin: SORT_STR key nests clones under parents in Grouped)
        let col = self.sort_column;
        let rev = self.sort_reverse;
        let grouped = self.list_mode == ListMode::Grouped;
        let local = self.local_game_list;
        // One allocation per game for the common (ungrouped) case. The previous
        // version cloned the field, then `format!`ed the grouped prefix, then
        // lowercased — three allocations per game, on every refilter, over 46k
        // games (README P2-22).
        let keyfn = |i: usize| -> String {
            let g = &guard.games[i];
            let val = match col {
                COL_NAME => g.name.as_str(),
                COL_ROM => return g.available.to_string(),
                // The localized list replaces the *description* only. Every
                // shipped `mame_cn.lst` carries the description in both of its
                // text columns, so honouring the second one put the Chinese game
                // title in the Manufacturer column — which is not a translation
                // of anything, just the same string in the wrong place.
                COL_MFTR => g.manufacturer.as_str(),
                COL_SRC => g.sourcefile.as_str(),
                COL_YEAR => {
                    if g.year.is_empty() {
                        "?"
                    } else {
                        g.year.as_str()
                    }
                }
                COL_CLONEOF => g.cloneof.as_str(),
                _ => {
                    if local && !g.lc_desc.is_empty() {
                        g.lc_desc.as_str()
                    } else {
                        g.description.as_str()
                    }
                }
            };
            if grouped {
                let (prefix, rank) = if g.cloneof.is_empty() {
                    (g.name.as_str(), " _0")
                } else {
                    (g.cloneof.as_str(), " _9")
                };
                let mut out = String::with_capacity(prefix.len() + rank.len() + val.len());
                out.push_str(prefix);
                out.push_str(rank);
                out.push_str(val);
                out.to_lowercase()
            } else {
                val.to_lowercase()
            }
        };
        let mut keyed: Vec<(String, usize)> = rows.into_iter().map(|i| (keyfn(i), i)).collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        if rev {
            keyed.reverse();
        }
        self.visible = std::rc::Rc::new(keyed.into_iter().map(|(_, i)| i).collect());
        // default selection like restoreGameSelection
        if self.selected.is_none() || !self.visible.contains(self.selected.as_ref().unwrap()) {
            self.selected = self.visible.first().copied();
            if let Some(i) = self.selected {
                self.current_game = guard.games[i].name.clone();
            }
        }
    }

    pub fn search_changed(&mut self) {
        self.needs_refilter = true;
    }

    // ---- folder selection (origin: filterFolderChanged) ----

    pub fn folder_matches_root(&self, kind: &FolderKind) -> bool {
        self.folder_kind == *kind && self.folder_key.is_none()
    }

    pub fn select_root(&mut self, kind: FolderKind, label: &str) {
        self.folder_kind = kind;
        self.folder_key = None;
        self.current_folder = label.to_string();
        self.needs_refilter = true;
    }

    pub fn select_ext_root(&mut self, name: &str) {
        self.folder_kind = FolderKind::Ext(name.to_string());
        self.folder_key = Some("ROOT_FOLDER".into());
        self.current_folder = format!("/{name}");
        self.needs_refilter = true;
    }

    pub fn select_ext_sub(&mut self, name: &str, label: &str) {
        self.folder_kind = FolderKind::Ext(name.to_string());
        self.folder_key = Some(label.to_string());
        self.current_folder = format!("/{name}/{label}");
        self.needs_refilter = true;
    }

    pub fn can_remove_from_folder(&self) -> bool {
        matches!(self.folder_kind, FolderKind::Ext(_)) && self.has_game()
    }

    pub fn remove_from_folder(&mut self) {
        // currentFolder: "/Name[/Sub]" — split last
        let parts: Vec<&str> = self.current_folder.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() < 2 {
            return;
        }
        let (name, sub) = if parts.len() == 2 {
            (parts[0].to_string(), "ROOT_FOLDER".to_string())
        } else {
            (parts[0].to_string(), parts[1..].join("/"))
        };
        if let Some((_, store)) = self.ext_folder_data.iter_mut().find(|(n, _)| *n == name) {
            store.remove_game(&sub, &self.current_game.clone());
        }
        self.save_ext_folder(&name);
        self.needs_refilter = true;
    }

    // ------------------------------------------------------------------
    // delete cfg menu (origin: updateDeleteCfgMenu — ini/cfg/nv/dif)
    // ------------------------------------------------------------------

    /// Files the "delete machine configuration" action offers to remove.
    ///
    /// Every directory option goes through `resolve_dir_list`, which resolves a
    /// relative value against the mame.exe directory. Splitting the raw option
    /// value by ';' instead made every relative entry (`cfg`, `nvram`, `ini`, …)
    /// resolve against the *process* working directory, so the candidates went
    /// missing whenever it differed from the mame directory (README N8).
    pub fn delete_cfg_candidates(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Some(m) = self.current_meta() else { return out };
        let Some(opts) = self.opts.clone() else { return out };
        let Ok(core) = opts.try_lock() else { return out };
        let dirs_of = |key: &str| -> Vec<PathBuf> {
            core.opts
                .get(key)
                .map(|o| core.resolve_dir_list(&o.currvalue))
                .unwrap_or_default()
        };
        for d in dirs_of("inipath") {
            for n in [&m.name, &m.cloneof] {
                if n.is_empty() {
                    continue;
                }
                let p = d.join(format!("{n}.ini"));
                if p.is_file() {
                    out.push(p);
                }
            }
        }
        for (dir_opt, ext) in [
            ("cfg_directory", "cfg"),
            ("nvram_directory", "nv"),
        ] {
            for d in dirs_of(dir_opt) {
                let p = d.join(format!("{}.{}", m.name, ext));
                if p.is_file() {
                    out.push(p);
                }
            }
        }
        out
    }

    // ------------------------------------------------------------------
    // export dialogs (origin: exportFixDat/exportGameList)
    // ------------------------------------------------------------------

    pub fn pick_fixdat_target(&mut self, method: crate::core::verify::VerifyMethod) {
        let start = self
            .mame
            .as_ref()
            .and_then(|m| m.path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();
        let Some(path) = rfd::FileDialog::new()
            .set_title(self.tr("File name:"))
            .set_directory(&start)
            .add_filter("Dat files (*.dat)", &["dat"])
            .add_filter("All Files (*)", &["*"])
            .set_file_name("fixdat.dat")
            .save_file()
        else {
            return;
        };
        self.exporting_method = Some(method);
        self.export_target = Some(path);
        // verify first, export on VerifyDone (origin verify(false, method, file))
        self.start_internal_verify();
    }

    pub fn pick_list_target(&mut self, have: bool) {
        let start = self
            .mame
            .as_ref()
            .and_then(|m| m.path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_default();
        let Some(path) = rfd::FileDialog::new()
            .set_title(self.tr("File name:"))
            .set_directory(&start)
            .add_filter("Txt files (*.txt)", &["txt"])
            .add_filter("All Files (*)", &["*"])
            .set_file_name(if have { "have.txt" } else { "miss.txt" })
            .save_file()
        else {
            return;
        };
        self.export_target = Some(path);
        self.export_game_list(have);
    }

    // ------------------------------------------------------------------
    // launch (origin: runMame)
    // ------------------------------------------------------------------

    pub fn launch(&mut self, mode: RunMode, play_args: Vec<String>) {
        let Some(mame) = self.mame.clone() else {
            self.poplog(self.tr("Could not find valid MAME/MESS."));
            return;
        };
        let Some(lib) = self.lib.clone() else { return };
        let mut meta = match self.current_meta() {
            Some(m) => m,
            None => return,
        };
        // ext roms: materialize devices from system + auto-mount const device
        if meta.is_ext_rom {
            let system = meta.romof.clone();
            let mut devices = lib
                .lock()
                .unwrap()
                .get(&system)
                .map(|s| s.devices.clone())
                .unwrap_or_default();
            for d in &mut devices {
                if let Some(mnt) = self.mounted.get(&(system.clone(), d.instance.clone())) {
                    d.mounted_path = mnt.clone();
                }
            }
            meta.devices = devices;
        }
        // 7z temp-rom extraction (origin step 4a)
        let mut temp_rom: Option<PathBuf> = None;
        if meta.is_ext_rom && self.current_game.contains(".7z/") {
            let parts: Vec<&str> = self.current_game.splitn(2, ".7z/").collect();
            if parts.len() == 2 {
                let arch_dir = PathBuf::from(parts[0]).parent().map(|p| p.to_path_buf()).unwrap_or_default();
                let arch_base = PathBuf::from(parts[0])
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let rom_file = parts[1].to_string();
                // `extract_mame_file` reduces the entry to its basename, so the
                // path we hand to MAME has to be reduced the same way — joining
                // the raw (possibly `subdir/rom.bin`) name pointed at a file that
                // was never written (README N6)
                let base = std::path::Path::new(&rom_file)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| rom_file.clone());
                let dest = crate::core::archive::temp_rom_dir().join(base);
                let hits = crate::core::archive::iterate_mame_file(
                    &arch_dir.to_string_lossy(),
                    &arch_base,
                    &rom_file,
                    crate::core::archive::IterateMethod::Extract,
                    "",
                    None,
                );
                if hits.is_empty() {
                    self.poplog(self.tf(
                        "Could not load:\n\n{}\n\nPlease refresh the game list.",
                        format!("{}\n{}", parts[0], rom_file),
                    ));
                    return;
                }
                // remember it: the file used to stay in %TEMP% forever
                self.temp_roms.push((self.current_game.clone(), dest.clone()));
                temp_rom = Some(dest);
            }
        }
        let cmd_diff = if mode == RunMode::Cmd {
            let gui_keys: std::collections::HashSet<String> =
                self.gui.map.keys().cloned().collect();
            self.opts
                .as_ref()
                .and_then(|o| o.try_lock().ok())
                .map(|o| crate::core::launcher::cmd_diff(&o, &gui_keys))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // language args (origin step 5)
        let mut args = play_args;
        if self.opts.as_ref().and_then(|o| o.try_lock().ok()).map(|o| o.has_language).unwrap_or(false)
            && self.lang != "ru_RU"
        {
            if let Some(o) = self.opts.as_ref().and_then(|o| o.try_lock().ok()) {
                if let Some(lp) = o.opts.get("langpath") {
                    args.push("-langpath".into());
                    args.push(crate::core::options::dir_string(
                        &crate::core::options::clean_dir_path(&lp.globalvalue),
                    ));
                    args.push("-language".into());
                    args.push(self.lang.clone());
                }
            }
        }
        let spec = launcher::build_args(mode, &meta, &cmd_diff, &args, temp_rom.as_deref());
        if !spec.warnings.is_empty() {
            self.poplog(self.tf(
                "{} requires that these device(s)\nmust be mounted:\n\ncouldn't start MESS.",
                format!("{}\n{}", meta.romof, spec.warnings.join("\n")),
            ));
            return;
        }
        self.log(format!("launch: {} {:?}", mame.path.display(), spec.args));
        match mame.spawn_run(&spec.args) {
            Ok(child) => {
                let game = self.current_game.clone();
                self.running.insert(game.clone());
                let tx = self.events_tx.clone();
                let ctx = self.ctx();
                std::thread::spawn(move || {
                    let mut child = child;
                    let code = child.wait().ok().and_then(|s| s.code());
                    let _ = tx.send(crate::events::AppEvent::MameExited { game, code });
                    ctx.request_repaint();
                });
            }
            Err(e) => self.poplog(e.to_string()),
        }
    }

    // ---- PlayWith dialogs glue ----

    pub fn open_play_dialog(&mut self, kind: PlayKind) {
        let game = self.current_game.clone();
        let dir = self
            .opts
            .as_ref()
            .and_then(|o| o.try_lock().ok())
            .and_then(|o| {
                let key = match kind {
                    PlayKind::Savestate => "state_directory",
                    PlayKind::Playback | PlayKind::Record | PlayKind::Wave => "input_directory",
                    PlayKind::Mng | PlayKind::Avi => "snapshot_directory",
                };
                o.opts.get(key).map(|p| p.currvalue.clone())
            })
            .unwrap_or_default();
        let dir0 = dir.split(';').next().unwrap_or("").to_string();
        let file = format!("{}_{:03}.{}", game, 0, kind.ext());
        let _ = dir;
        self.play_dialog = Some((kind, format!("{dir0}/{file}")));
    }

    pub fn open_cmd_dialog(&mut self) {
        // build the full command line preview (origin step 6)
        let Some(mame) = self.mame.clone() else { return };
        let gui_keys: std::collections::HashSet<String> = self.gui.map.keys().cloned().collect();
        let diff = self
            .opts
            .as_ref()
            .and_then(|o| o.try_lock().ok())
            .map(|o| crate::core::launcher::cmd_diff(&o, &gui_keys))
            .unwrap_or_default();
        let spec = crate::core::launcher::build_args(
            RunMode::Cmd,
            &self.current_meta().unwrap_or_default(),
            &diff,
            &[],
            None,
        );
        self.cmd_text = format!("{} {}", mame.path.display(), spec.args.join(" "));
        self.show_cmd = true;
    }

    /// 跑MAME 自己的 `-verifyroms` / `-verifysamples`，把 stdout 收进
    /// `verify_lines`。
    ///
    /// **当前没有菜单入口**（2026-10-05 用户要求删掉「校验 Rom」「校验全部
    /// Rom」「校验全部样本」）。这三条路最后都等价于「刷新档案」(F5) 的
    /// `refresh_all`，留两个入口只会让人以为是两件事。
    /// 代码留着：要接回 MAME 原生校验输出（比校验缓存更权威）时直接启用。
    #[allow(dead_code)]
    pub fn verify(&mut self, current_only: bool, samples: bool) {
        let Some(mame) = self.mame.clone() else { return };
        let mut args = vec![if samples { "-verifysamples".to_string() } else { "-verifyroms".to_string() }];
        if current_only {
            args.push(self.current_game.clone());
        }
        self.verify_lines.clear();
        self.show_verify = true;
        crate::background::run_verify_output_pump(mame, args, self.events_tx.clone(), self.ctx());
    }

    /// Hand a URL to the system default browser. `rundll32 url.dll,FileProtocolHandler`
    /// is the documented way to do that without pulling in a dependency; it takes
    /// the URL as a single argument, so nothing in it is interpreted as a command.
    pub fn open_url(&mut self, url: &str) {
        let _ = std::process::Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(url)
            .spawn();
    }

    // ------------------------------------------------------------------
    // preview / dat requests (origin updateSelection thread triggers)
    // ------------------------------------------------------------------

    pub fn request_preview(&mut self, dock: usize) {
        let game = self.current_game.clone();
        if game.is_empty() || self.snap_requested.contains(&dock_key(dock, &game)) {
            return;
        }
        // design doc §12: wait for the selection to settle, otherwise scrolling
        // the list queues a load job for every machine the cursor passes
        if self.selection_settling() {
            return;
        }
        // artwork lives under the MvUI directory by default, not the MAME one
        let dirs = self.content_image_dirs(dock);
        if dirs.is_empty() {
            return;
        }
        // origin: MameDat::getScreenshot recurses into `cloneof` until a picture is
        // found, so the whole clone chain is the fallback list — not `romof`, which
        // is what the verify uses. We used to pass a single `cloneof` step and threw
        // the second name away (README P3).
        let mut fallbacks: Vec<String> = Vec::new();
        if let Some(m) = self.current_meta() {
            let mut next = m.cloneof.clone();
            let lib = self.lib.clone();
            if let Some(lib) = lib {
                if let Ok(guard) = lib.try_lock() {
                    let guard = guard;
                    while !next.is_empty() && !fallbacks.contains(&next) && fallbacks.len() < 16 {
                        let parent = guard
                            .games
                            .iter()
                            .find(|g| g.name == next)
                            .map(|g| g.cloneof.clone())
                            .unwrap_or_default();
                        fallbacks.push(next);
                        next = parent;
                    }
                }
            }
        }
        // Latch the key only now that the request is really going out: `SnapReady`
        // never arrives for the early returns above, so a key latched before them
        // blocked that dock for this game until the next restart (README N3).
        self.snap_requested.insert(dock_key(dock, &game));
        // read `snapname` here, once per request, instead of inside the worker
        let snapname = if dock == crate::core::dat::DOCK_SNAP {
            Some(self.gui.get("snapname").unwrap_or_default().to_string())
        } else {
            None
        };
        crate::background::load_preview(
            dock,
            dirs,
            game,
            fallbacks,
            snapname,
            self.events_tx.clone(),
            self.ctx(),
        );
    }

    pub fn request_dat(&mut self, dock: usize) {
        let game = self.current_game.clone();
        if game.is_empty() || self.dat_requested.contains(&(dock, game.clone())) {
            return;
        }
        // same debounce as the preview docks (design doc §12)
        if self.selection_settling() {
            return;
        }
        // Same as N3: no early return may happen after the key is latched.
        // DATs live in `<exe>/dats` by default
        let file_path = self.content_dat_file(dock);
        if file_path.is_empty() {
            return;
        }
        let (search_tag, cloneof, sourcefile) = match self.current_meta() {
            // origin: UpdateSelectionThread::getHistory — an ext-rom entry is
            // filed under its parent (`romof`), not under its own name
            Some(m) => {
                let tag = if m.is_ext_rom && !m.romof.is_empty() {
                    m.romof.clone()
                } else {
                    m.name.clone()
                };
                (tag, m.cloneof.clone(), m.sourcefile.clone())
            }
            None => return,
        };
        self.dat_requested.insert((dock, game.clone()));
        // `load_dat` assembles the localized path itself (README N2): handing it a
        // ready-made path made it `join()` an absolute path over itself, so the
        // localized DAT was never read and the English one was appended to itself
        // behind an <hr>. Only the *language directory* travels over the wire, and
        // only the first langpath entry: a ';'-joined list cannot be a path.
        let langdir = self
            .opt_resolved_dirs("langpath")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let local_dat = if langdir.is_empty() {
            String::new()
        } else {
            format!("{}/{}", langdir.trim_end_matches(['/', '\\']), self.lang)
        };
        crate::background::load_dat(
            dock,
            file_path,
            game,
            search_tag,
            cloneof,
            sourcefile,
            self.dark_bg,
            local_dat,
            self.events_tx.clone(),
            self.ctx(),
        );
    }

    // ------------------------------------------------------------------
    // list rendering: Details/Grouped table + context menu
    // ------------------------------------------------------------------

    /// Move the column in slot `from` to slot `to`, shifting the ones in between.
    ///
    /// A swap (what this used to do) only looks right when the drag lands on the
    /// neighbouring column: dragged two columns away, the two swapped ends and
    /// everything in between stayed put, so the header no longer matched what the
    /// pointer had been dragged across.
    fn move_column(&mut self, from: usize, to: usize) {
        shift_slot(&mut self.col_order, from, to);
    }

    pub fn draw_table(&mut self, ui: &mut egui::Ui) {
        if self.lib_status != crate::app::LibStatus::Ready {
            ui.centered_and_justified(|ui| self.loading_or_error(ui));
            return;
        }
        let Some(lib) = self.lib.clone() else { return };
        let guard = lib.lock().unwrap();
        let vis = self.visible.clone();
        let sel = self.selected;
        let grouped = self.list_mode == ListMode::Grouped;
        let local = self.local_game_list;
        let row_h = 22.0;
        let clicked: Cell<Option<usize>> = Cell::new(None);
        let launched: Cell<Option<usize>> = Cell::new(None);
        // Icons are asked for while the row is drawn (that is the only place the
        // visible rows are known) and dispatched after the table pass, which
        // still holds `&mut self`.
        let icon_reqs: std::rc::Rc<std::cell::RefCell<Vec<(String, Vec<String>)>>> =
            Default::default();
        // the response of the right-clicked cell plus its row: egui anchors a
        // context menu at the pointer, and the menu has to be built after the
        // table pass (it needs `&mut self`), so the response is parked here
        // the right-clicked cell's response, parked so the menu can be built
        // after the table pass (it needs `&mut self`) and then kept alive in
        // `self.ctx_menu` — egui only draws it while `context_menu` is called
        let ctx_menu: std::rc::Rc<std::cell::RefCell<Option<usize>>> = Default::default();
        // `Rc<Cell<..>>`, not `Cell<..>`: a `Cell<T: Copy>` is itself `Copy`, and
        // the header closure below is `move`, so a bare `Cell` would be *copied*
        // into it and every write (sort request, column swap) would be lost.
        let header_sort_requested =
            std::rc::Rc::new(std::cell::Cell::new((usize::MAX, false)));
        let header_sort_requested_clone = std::rc::Rc::clone(&header_sort_requested);

        let sort_rev0 = self.sort_reverse;
        let sort_col0 = self.sort_column;
        // grab the context before `TableBuilder` takes a mutable borrow of `ui`
        let ctx0 = ui.ctx().clone();
        // the whole row is highlighted on selection, so cell text has to switch
        // to the selection foreground itself (there is no per-cell SelectableLabel
        // to do it any more)
        let visuals0 = ui.visuals().clone();
        let fg_normal = visuals0.text_color();
        let fw_normal = visuals0.weak_text_color();
        let fg_sel = visuals0.selection.stroke.color;
        // ---- column widths ---------------------------------------------------
        // Every visible column is a plain resizable `Column::initial(w)` seeded
        // from the width measured on the previous frame, and a trailing
        // `Column::remainder()` spacer soaks up whatever is left, so the columns
        // always add up to exactly the viewport width: no horizontal overflow,
        // and therefore no scrollbar appearing/disappearing under the drag (the
        // old code re-derived the description and spacer widths from
        // `available_width` every frame, and rebuilt the whole table state
        // whenever that crossed a threshold — that state rebuild was the
        // flicker).
        let order = self.col_order;
        // egui_extras keeps its column widths positionally, so a reorder has to
        // rebuild the state — otherwise every column would inherit the width of
        // whatever used to sit in its slot. Mixing the order into the id salt
        // does that, and `self.col_widths` (below) hands each column its own
        // width back, keyed by column id.
        let order_key = order.iter().fold(0i64, |a, &v| a * 8 + v as i64);
        let wid = |c: usize| self.col_widths.get(c).copied().unwrap_or(COL_DEFAULT_WIDTH[c]);
        // The row pitch, used to make the per-cell hover fills meet without a seam
        // (`rows()` spaces rows by `row_height + item_spacing.y`). Read before the
        // table is built, because `TableBuilder::new(ui)` borrows `ui` from here on.
        let mut tb = TableBuilder::new(ui)
            .id_salt(("gamelist_table", order_key, self.col_reset_salt))
            // stripes fight a wallpaper: every other row gets `faint_bg_color`
            // laid over the picture, so the list ends up half-transparent /
            // half-opaque and reads as noise. 1.8.2's list view had no zebra
            // striping at all — drop it while a background is selected.
            .striped(self.background_file.is_none())
            // NOT `.resizable(true)`: egui_extras' resize handle is one line from
            // the top of the body to its bottom (table.rs `p0 = pos2(x, table_top)
            // .. p1 = pos2(x, bottom)`), so with it on, dragging sideways anywhere in
            // the *game rows* resized a column and the header could not be resized
            // at all. Column widths are driven by hand from the header instead —
            // see `header_resize`.
            .resizable(false)
            // every cell senses clicks so that a click anywhere on the row selects
            // the game: `TableRow::response()` is the union of all cell responses
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
        // column order is user-draggable
        for &i in order.iter() {
            let col = if !self.col_visible[i] && i != COL_DESC {
                // hidden columns keep a zero-width slot so header/body cells
                // still map 1:1 onto the column list
                Column::exact(0.0)
            } else {
                Column::initial(wid(i))
                    .at_least(COL_MIN_WIDTH[i])
                    .clip(true)
            };
            tb = tb.column(col);
        }
        // spacer: fills whatever the visible columns leave over, so the striped
        // rows always reach the right edge and no horizontal overflow can occur
        tb = tb.column(Column::remainder().resizable(false).clip(true));
        let hs_req = header_sort_requested_clone;
        let reset_widths: Cell<bool> = Cell::new(false);
        let move_req = std::rc::Rc::new(std::cell::Cell::new(None::<(usize, usize)>));
        let move_cell = std::rc::Rc::clone(&move_req);
        // The header cells' geometry, collected so the click/drag test below can be
        // done by hand: egui's hit-test resolves against the widget rects registered
        // in the *previous* pass (`Context::begin_pass` → `hit_test(prev_pass)`, and
        // `Ui::response` reads `prev_pass` too), and for the table header that never
        // yields a hit — clicks on a header cell (sort) and header drags (column
        // reorder) were silently dropped no matter whether they came from
        // `TableRow::response()`, `ui.interact()`, a widget inside the cell or
        // `dnd_drag_source`.
        let hdr_rects: std::rc::Rc<std::cell::RefCell<Vec<(usize, egui::Rect)>>> =
            Default::default();
        let hdr_rects_tx = std::rc::Rc::clone(&hdr_rects);
        let dnd_order = order;
        let header_lang = self.lang.clone();
        // pointer state for the by-hand header hit tests below
        let ptr0 = ctx0.input(|i| i.pointer.latest_pos());
        let grab0 = ctx0.input(|i| i.pointer.press_origin());
        let down0 = ctx0.input(|i| i.pointer.primary_down());
        // the row cells need their own copy: `header_lang` is moved into the
        // header closure below
        let cell_lang = self.lang.clone();
        // likewise for the in-flight resize, read inside that `move` closure only
        // to keep the resize cursor on the column being dragged
        let resizing_col: Option<usize> = self.header_resize.map(|(c, ..)| c);
        // header-only copies of grid/drag state: the closure below is `move` and
        // cannot borrow `self`
        let show_grid0 = self.show_grid;
        let drag_now = self.header_drag;
        // Separator positions, from the previous frame's header rects: the header
        // closure runs before this frame's rects exist, and a frame-old position
        // is what the pointer is compared against anyway.
        let sep_x0: Vec<f32> = column_separators(&hdr_rects.borrow())
            .into_iter()
            .map(|(_, x)| x)
            .collect();
        let table = tb.header(20.0, move |mut header| {
            for &i in dnd_order.iter() {
                let title = crate::i18n::tr(&header_lang, COLUMN_TITLES[i]);
                header.col(|ui| {
                    let arrow = if sort_col0 == i {
                        if sort_rev0 { " ▼" } else { " ▲" }
                    } else {
                        ""
                    };
                    let rect = ui.max_rect();
                    hdr_rects_tx.borrow_mut().push((i, rect));
                    // Header feedback, tested by hand against the pointer: the
                    // table header never yields a hit through egui's own test (see
                    // the note above `hdr_rects`), and `SelectableLabel` is not an
                    // option either. The tint covers the **whole** header cell (not
                    // just the text), so the entire column lights up — plus the one
                    // being dragged, so a reorder shows what it picked up.
                    // the tint covers the entire cell — including the few points
                    // next to the separators, which only the click test avoids
                    let over = ptr0.is_some_and(|p| rect.contains(p));
                    let on_sep = ptr0.is_some_and(|p| {
                        rect.y_range().contains(p.y)
                            && sep_x0.iter().any(|x| (p.x - x).abs() <= RESIZE_GRAB)
                    });
                    let grabbed = down0 && grab0.is_some_and(|p| in_header(p, rect));
                    let tint = if over || grabbed {
                        0.45
                    } else if sort_col0 == i {
                        0.25
                    } else {
                        0.0
                    };
                    if tint > 0.0 {
                        ui.painter().rect_filled(
                            rect,
                            2.0,
                            ui.visuals().selection.bg_fill.gamma_multiply(tint),
                        );
                    }
                    // A live reorder marks the column it picked up: the tint above
                    // is too faint to read as "holding something", so outline the
                    // cell as well (user feedback: the move happened but nothing
                    // looked draggable).
                    if drag_now.is_some_and(|(from, _, moved)| moved && from == i) {
                        ui.painter().rect_stroke(
                            rect,
                            2.0,
                            egui::Stroke::new(1.5_f32, ui.visuals().selection.stroke.color),
                        );
                    }
                    // 1.8.2 resizes a column by dragging the `QHeaderView` separator,
                    // so show the resize cursor over the trailing edge of the cell.
                    if on_sep || resizing_col == Some(i) {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeColumn);
                    }
                    // left-aligned, like the Qt header of 1.8.2
                    ui.add(
                        egui::Label::new(egui::RichText::new(format!("{title}{arrow}")).strong())
                            .selectable(false),
                    );
                });
            }
            // the spacer column has no header of its own
            header.col(|_ui| {});
        });
        {
            let rects = hdr_rects.borrow();
            let col_at = |p: egui::Pos2| {
                rects
                    .iter()
                    .find(|(_, r)| in_header(p, *r))
                    .map(|(i, _)| *i)
            };
            let slot_of = |c: usize| order.iter().position(|&x| x == c);
            // `latest_pos`, not `interact_pos`: while a button is held egui reports
            // the *press origin* as `interact_pos`, so the drop test would always
            // land back on the column the drag started from.
            let ptr = ctx0.input(|i| i.pointer.latest_pos());
            let pressed = ctx0.input(|i| i.pointer.primary_pressed());
            let down = ctx0.input(|i| i.pointer.primary_down());
            // Column resize, handled here rather than by egui_extras: its handle
            // spans the whole table body, but 1.8.2 only lets you resize from the
            // header separator (`QHeaderView::sectionResized`). A press on a
            // separator therefore starts a width drag *instead of* a reorder, and
            // remember the start x + width so the new width tracks the pointer
            // exactly (the header rects are re-measured every frame, and the
            // layout only picks a new width up after a state rebuild).
            // The separator sits in the *gap* between two cells, not on a cell
            // edge: the columns are laid out edge to edge with `item_spacing.x`
            // (~8pt) in between, so a pointer in that gap was more than the grab
            // width away from both edges and the handle could not be grabbed at
            // all. Boundaries are therefore the gap midpoints, and the one nearest
            // the pointer wins.
            let seps = column_separators(&rects);
            // restricted to the header band: a press on a *game row* at the same x
            // must not start a width drag (the rows are not draggable for width)
            let hdr_top = rects.first().map(|(_, r)| r.top()).unwrap_or(0.0);
            let hdr_bottom = rects.first().map(|(_, r)| r.bottom()).unwrap_or(0.0);
            let sep_at = |p: egui::Pos2| {
                if p.y < hdr_top || p.y > hdr_bottom {
                    return None;
                }
                seps
                    .iter()
                    .filter(|(_, x)| (p.x - x).abs() <= RESIZE_GRAB)
                    .min_by(|(_, a), (_, b)| {
                        (p.x - a)
                            .abs()
                            .partial_cmp(&(p.x - b).abs())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|(i, _)| *i)
            };
            // One frame of the header pointer state machine (see
            // `header_step`). This used to be a flat `else if` chain with a
            // second `if down` arm that could never be reached, so a drag never
            // picked up the column it was released over: `moved` stayed false
            // and the reorder never fired — the release was then treated as a
            // plain click and re-sorted the column instead.
            let step = header_step(
                pressed,
                down,
                self.header_resize.is_some(),
                self.header_drag.is_some(),
                ptr.and_then(&sep_at),
                ptr.and_then(col_at).and_then(slot_of),
            );
            match step {
                HeaderStep::Idle => {}
                HeaderStep::BeginResize(c) => {
                    if self.col_visible[c] || c == COL_DESC {
                        if let Some(px) = ptr {
                            let w = self.col_widths[c];
                            self.header_resize = Some((c, px.x, w));
                            self.header_drag = None;
                        }
                    }
                }
                HeaderStep::BeginDrag(slot) => {
                    self.header_resize = None;
                    self.header_drag = Some((slot, slot, false));
                    // the floating ghost tracks `pointer.x - header_drag_x`
                    if let Some(px) = ptr {
                        self.header_drag_x = px.x;
                    }
                }
                HeaderStep::UpdateResize => {
                    if let (Some((c, x0, w0)), Some(px)) = (self.header_resize, ptr) {
                        let w = (w0 + px.x - x0).max(COL_MIN_WIDTH[c]);
                        if (w - self.col_widths[c]).abs() > 0.5 {
                            // no state rebuild needed: for a *non-resizable*
                            // `Column::initial`, `TableState::load` re-derives the
                            // width from `initial_width` on every single frame
                            self.col_widths[c] = w;
                        }
                    }
                }
                HeaderStep::UpdateDragTo(slot) => {
                    if let Some((from, _, moved)) = self.header_drag {
                        self.header_drag = Some((from, slot, moved || slot != from));
                    }
                }
                HeaderStep::EndResize => {
                    // released: a width drag, never a sort
                    self.header_resize = None;
                }
                HeaderStep::EndDrag => {
                    // The move is applied on release, as one shift of the column: doing
                    // it live on every column the pointer crossed re-aimed the target
                    // each time, because the reorder itself moves the columns under the
                    // pointer — one drag used to shuffle three columns instead of one.
                    let drag = self.header_drag.take();
                    if let Some((from, to, moved)) = drag {
                        if moved {
                            move_cell.set(Some((from, to)));
                        } else if let Some(i) = ptr.and_then(col_at) {
                            // a plain click (press and release on one column) sorts
                            // double-click on a header restores the default widths (the
                            // old auto-fill gesture)
                            let dbl = ctx0.input(|i| {
                                i.pointer.button_double_clicked(egui::PointerButton::Primary)
                            });
                            if dbl {
                                reset_widths.set(true);
                            } else {
                                hs_req.set((i, sort_col0 == i && !sort_rev0));
                            }
                        }
                    }
                }
            }
        }
        let table_out = table
        .body(|body| {
            body.rows(row_h, vis.len(), |mut row| {
                let idx = row.index();
                let gi = vis[idx];
                let g = &guard.games[gi];
                let is_sel = Some(gi) == sel;
                // Whether the pointer is on this row: decided in the first cell
                // from its own rect (which spans the full row height) and then read
                // by the remaining cells, so the whole line lights up as one band.
                // Using the rect rather than computed row geometry keeps it correct
                // while the body scrolls.
                // Row highlight is **hover-only**: hover tints the whole line,
                // leaving it restores the row — like 1.8.2's item view, and
                // whether the row is selected or not makes no difference.
                //
                // Painted per cell rather than through egui_extras' `hovered` flag
                // for two reasons: that flag is fed from the *previous* frame's
                // pointer position (`capture_hover_state`), and it is suppressed
                // entirely for a selected row (`StripLayout::add`:
                // `flags.hovered && !flags.selected`). Each cell's rect is inflated
                // by half the item spacing, so the fills join into one band.
                let fg = if is_sel { fg_sel } else { fg_normal };
                let fg_w = if is_sel { fg_sel } else { fw_normal };
                let clone_indent = grouped && !g.cloneof.is_empty();
                // Whole-row selection paint: `set_selected` propagates to every
                // cell drawn afterwards, and egui_extras' `StripLayout::add`
                // fills the selection background across the full cell — the
                // "整行浅蓝色" of MvUI. (Origin: QItemDelegate paints a single
                // selection rect over the whole row; egui_extras paints per
                // cell, but each cell's fill spans its own `max_rect`, so the
                // line lights up end-to-end.)
                row.set_selected(is_sel);
                // one cell per column, in the user's order. The status square
                // lives inside the description cell (origin: GameListDelegate
                // paints icon + text in a single column).
                for &c in order.iter() {
                    let icon_reqs = std::rc::Rc::clone(&icon_reqs);
                    row.col(|ui| {
                        let cell_rect = ui.max_rect();
                        match c {
                        COL_DESC => {
                            // clone members are indented, as in the 1.8.2 tree view
                            if clone_indent {
                                ui.add_space(16.0);
                            }
                            let grade = if g.is_ext_rom || g.driver.status == 1 {
                                crate::core::model::STATUS_GOOD
                            } else if g.driver.status == 2 {
                                crate::core::model::STATUS_IMPERFECT
                            } else {
                                crate::core::model::STATUS_PRELIMINARY
                            };
                            // origin: GameListDelegate paints icon + text in one
                            // column — the machine icon from the icon pack when
                            // there is one, otherwise the driver-status square
                            // (README §6.1, layer ③). The square is procedural
                            // rather than the 16×16 `sqr-*.png` bitmaps: a solid
                            // colour block has no detail to lose, and painting it
                            // directly keeps the edge crisp at every DPI (the PNG
                            // path was 2× upscaled and visibly soft on a 4K screen)
                            let ictx = ui.ctx().clone();
                            let resp = match self.game_icon(&g.name) {
                                Some(tex) => ui.add(
                                    egui::Image::new(&tex)
                                        .max_size(egui::Vec2::splat(16.0))
                                        // passive: the row-wide hit rect behind it
                                        // must keep the hover and the press
                                        .sense(egui::Sense {
                                            click: false,
                                            drag: false,
                                            focusable: false,
                                        }),
                                ),
                                None => {
                                    if self.icon_needs_request(&g.name) {
                                        icon_reqs
                                            .borrow_mut()
                                            .push((g.name.clone(), icon_fallbacks(g, &guard)));
                                    }
                                    icons::draw_square(ui, icons::status_color(grade), 16.0)
                                }
                            };
                            if g.available != GAME_COMPLETE {
                                // the Qt delegate stamps `res/status-na.png` over
                                // the bottom-right corner of the status square
                                let na = egui::Rect::from_min_size(
                                    resp.rect.left_top() + egui::Vec2::splat(8.0),
                                    egui::Vec2::splat(8.0),
                                );
                                if !icons::put(ui, &ictx, "status-na.png", na) {
                                    ui.painter().rect_filled(na, 1.0, icons::GRAY);
                                }
                            }
                            ui.add_space(4.0);
                            let label = if local && !g.lc_desc.is_empty() {
                                g.lc_desc.clone()
                            } else {
                                g.description.clone()
                            };
                            let color = if g.driver.emulation == 0 && !g.is_ext_rom {
                                egui::Color32::from_rgb(255, 96, 96)
                            } else {
                                fg
                            };
                            // plain left-aligned `Label`, *not* `SelectableLabel`:
                            // the latter centres its galley inside the rect handed
                            // to `add_sized`, which shoved the description to the
                            // far right of the column and away from its icon
                            cell_text(ui, &label, color);
                        }
                        COL_NAME => {
                            let n = if g.is_ext_rom { &g.romof } else { &g.name };
                            cell_text(ui, n, fg);
                        }
                        COL_ROM => {
                            // origin: GameListModel::data() → tr("Yes") / tr("No")
                            let v = match g.available {
                                1 | 2 => crate::i18n::tr(&cell_lang, "Yes"),
                                0 => crate::i18n::tr(&cell_lang, "No"),
                                _ => String::new(),
                            };
                            cell_text(ui, &v, fg_w);
                        }
                        COL_MFTR => {
                            // MAME's own manufacturer, never localized — see the
                            // sort key above for why.
                            cell_text(ui, &g.manufacturer, fg);
                        }
                        COL_SRC => cell_text(ui, &g.sourcefile, fg),
                        COL_YEAR => {
                            cell_text(ui, if g.year.is_empty() { "?" } else { &g.year }, fg)
                        }
                        _ => cell_text(ui, &g.cloneof, fg),
                        }
                        // hit-test the full cell as the LAST widget inside it:
                        // egui_extras drives `hovered_row_index` from each
                        // cell's `child_ui.response()`, which is the last-added
                        // widget. Without this the only last widget is the
                        // `Label` from `cell_text`, whose rect is just the text
                        // bounding box — so the row only lit up where the text
                        // was. Putting `interact` last makes the response cover
                        // the whole cell, and the row lights up end-to-end.
                        let hit = ui.interact(
                            cell_rect,
                            egui::Id::new(("row_hit", idx, c)),
                            egui::Sense::click(),
                        );
                        if hit.clicked() {
                            clicked.set(Some(gi));
                        }
                        if hit.double_clicked() {
                            launched.set(Some(gi));
                        }
                        if hit.secondary_clicked() {
                            *ctx_menu.borrow_mut() = Some(gi);
                        }
                    });
                }
                // spacer cell: keeps the striped/selection band reaching the
                // right edge, and lets a click on the empty area still select
                // the game
                row.col(|ui| {
                    let hit = ui.interact(
                        ui.max_rect(),
                        egui::Id::new(("row_hit", idx, usize::MAX)),
                        egui::Sense::click(),
                    );
                    if hit.clicked() {
                        clicked.set(Some(gi));
                    }
                    if hit.double_clicked() {
                        launched.set(Some(gi));
                    }
                    if hit.secondary_clicked() {
                        *ctx_menu.borrow_mut() = Some(gi);
                    }
                });
            });
        });

        // Vertical grid lines: one full-height stroke per column boundary, drawn
        // after the body so a single line spans header and rows without a break
        // at every row gap (per-cell segments left exactly that). The x positions
        // reuse `column_separators` — the very boundaries the header's resize
        // handle tests against — so a line can never drift away from its grab
        // strip. Purely decorative: resizing lives in the header only, these
        // lines never take the pointer.
        if show_grid0 {
            let rects = hdr_rects.borrow();
            let top = rects.first().map(|(_, r)| r.top());
            let bottom = table_out.inner_rect.bottom();
            if let Some(top) = top {
                if bottom > top + 20.0 {
                    let st = ui.visuals().widgets.noninteractive.bg_stroke;
                    for (_, x) in column_separators(&rects) {
                        ui.painter()
                            .line_segment([egui::pos2(x, top), egui::pos2(x, bottom)], st);
                    }
                }
            }
        }

        // remember the widths the table actually used, so a later state rebuild
        // (a reorder, a window resize) hands every column its own width back.
        // Skipped while a resize drag is running: the pointer *is* the width then,
        // and echoing the measured rect back would fight it.
        if self.header_resize.is_none() {
            let rects = hdr_rects.borrow();
            for (i, r) in rects.iter() {
                // a hidden column is laid out as `exact(0)`; storing that would
                // lose the width the user had set for it
                if *i < COL_LAST && (self.col_visible[*i] || *i == COL_DESC) {
                    self.col_widths[*i] = r.width();
                }
            }
        }

        // a double-click on a header put `reset_widths` up: restore the default
        // widths and rebuild the table state (fresh id salt) so they take effect
        if reset_widths.get() {
            self.col_widths = COL_DEFAULT_WIDTH;
            self.col_reset_salt = self.col_reset_salt.wrapping_add(1);
        }

        // While a reorder drag is live, mark the slot it would insert into: a
        // full-height line at the leading/trailing edge of the drop column, the
        // way Qt's QHeaderView does. The body rect is only known after the scroll
        // area has run, which is why the line is painted here and not in the
        // header pass.
        if let Some((from, to, moved)) = self.header_drag {
            if moved && to < COL_LAST {
                let target = order[to];
                let rects = hdr_rects.borrow();
                if let Some((_, tr)) = rects.iter().find(|(i, _)| *i == target) {
                    let x = if to > from { tr.right() } else { tr.left() };
                    let top = rects.first().map(|(_, r)| r.top());
                    if let Some(top) = top {
                        let bottom = table_out.inner_rect.bottom();
                        if bottom > top + 20.0 {
                            let accent = ui.visuals().selection.stroke.color;
                            egui::Area::new(egui::Id::new("header_drag_insert"))
                                .fixed_pos(egui::pos2(x - 1.0, top))
                                .order(egui::Order::Foreground)
                                .interactable(false)
                                .show(&ctx0, |ui| {
                                    ui.set_min_size(egui::vec2(2.0, bottom - top));
                                    ui.painter().rect_filled(ui.max_rect(), 1.0, accent);
                                });
                        }
                    }
                }
            }
        }

        // Floating ghost of the source column's header, following the pointer
        // horizontally. The Qt `QHeaderView` lifts the dragged section and
        // carries it with the cursor; we can't relayout the table mid-drag
        // without losing cells, so the lookalike header floats above the table
        // at `source_left + (pointer.x - press.x)`. The press-x is latched in
        // `BeginDrag` as `header_drag_x` (egui clears `press_origin` on release).
        if let Some((from, _to, moved)) = self.header_drag {
            if moved && from < COL_LAST {
                let src_col = order[from];
                let rects = hdr_rects.borrow();
                if let Some((_, src_rect)) = rects.iter().find(|(i, _)| *i == src_col) {
                    if let Some(px) = ptr0 {
                        let dx = px.x - self.header_drag_x;
                        let ghost_x = src_rect.left() + dx;
                        let title =
                            crate::i18n::tr(&self.lang, COLUMN_TITLES[src_col]);
                        let accent = ui.visuals().selection.stroke.color;
                        // `panel_fill`, not `window_fill`: the latter goes
                        // translucent when a wallpaper is set, and the ghost
                        // floats over the list where a see-through header (and
                        // doubled-up title text) would be unreadable
                        let bg = ui.visuals().panel_fill;
                        let text_color = ui.visuals().text_color();
                        let (gw, gh) = (src_rect.width(), src_rect.height());
                        egui::Area::new(egui::Id::new("header_drag_ghost"))
                            .fixed_pos(egui::pos2(ghost_x, src_rect.top()))
                            .order(egui::Order::Foreground)
                            .interactable(false)
                            .show(&ctx0, |ui| {
                                ui.set_min_size(egui::vec2(gw, gh));
                                let r = ui.max_rect();
                                ui.painter().rect_filled(r, 2.0, bg);
                                ui.painter().rect_stroke(
                                    r,
                                    2.0,
                                    egui::Stroke::new(1.5_f32, accent),
                                );
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(title)
                                            .strong()
                                            .color(text_color),
                                    )
                                    .selectable(false),
                                );
                            });
                    }
                }
            }
        }

        drop(guard);
        // Icon requests collected during the row pass go out here: they need
        // `&mut self`, which the closures could not hold.
        for (game, fallbacks) in icon_reqs.take() {
            self.request_game_icon(game, fallbacks);
        }
        if let Some((from, to)) = move_req.get() {
            self.move_column(from, to);
            self.needs_refilter = true;
        }
        let (sc, sr) = header_sort_requested.get();
        if sc != usize::MAX {
            if sr {
                self.sort_reverse = true;
            } else if self.sort_column == sc {
                self.sort_reverse = !self.sort_reverse;
            } else {
                self.sort_column = sc;
                self.sort_reverse = false;
            }
            self.needs_refilter = true;
        }
        if let Some(gi) = clicked.get() {
            self.select_game(gi);
        }
        if let Some(gi) = launched.get() {
            self.selected = Some(gi);
            if let Some(lib) = &self.lib {
                if let Ok(guard) = lib.try_lock() {
                    if let Some(g) = guard.games.get(gi) {
                        self.current_game = g.name.clone();
                    }
                }
            }
            self.launch(RunMode::Normal, vec![]);
        }
        // A fresh right-click selects its row and opens the menu at the pointer.
        // The old 1.8.2 pops the menu without changing the selection (Qt emits
        // customContextMenuRequested on its own), which made the menu act on a game
        // the user had not pointed at.
        if let Some(gi) = ctx_menu.borrow_mut().take() {
            self.select_game(gi);
            let pos = ctx0
                .input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
            if let Some(pos) = pos {
                self.ctx_menu = Some((pos, gi));
            }
        }
        if let Some((pos, _gi)) = self.ctx_menu {
            let menu_rect: std::cell::Cell<Option<egui::Rect>> = std::cell::Cell::new(None);
            let mut close = false;
            let ctx = ui.ctx().clone();
            // origin: Gamelist::showContextMenu → menuContext->popup(mapToGlobal(p)):
            // the menu opens at the click, not at some remembered corner. The
            // anchor used to be `Pos2::ZERO` in a field nobody ever read, so the
            // menu never appeared at all (README P3).
            egui::Area::new(egui::Id::new("gamelist_context_menu"))
                .fixed_pos(pos)
                .order(egui::Order::Foreground)
                .show(&ctx, |ui| {
                    // `panel_fill` is opaque in both themes, so the menu stays
                    // readable over a wallpaper — a context menu you cannot read
                    // is worse than one that hides the picture
                    let resp = egui::Frame::none()
                        .fill(ui.visuals().panel_fill)
                        .stroke(ui.visuals().window_stroke)
                        .rounding(egui::Rounding::same(6.0))
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                        self.play_section(ui);
                        self.delete_cfg_submenu(ui);
                        ui.separator();
                        self.add_folder_section(ui);
                        let rm = self.tr("Remove From This Folder");
                        if ui
                            .add_enabled(self.can_remove_from_folder(), crate::ui::button(rm))
                            .clicked()
                        {
                            self.remove_from_folder();
                            close = true;
                        }
                        ui.separator();
                        // 原来这里还有一项「校验 Rom」调 `start_game_verify()`，
                        // 与文件菜单的「刷新档案」(F5) 同源（都走校验），
                        // 用户要求删掉。下面直接是「导出列表」。
                        self.verify_submenu(ui);
                        ui.separator();
                        let src = self.src_properties_label();
                        if ui
                            .add_enabled(self.has_game(), crate::ui::button(src))
                            .clicked()
                        {
                            self.open_properties(crate::core::options::OPTLEVEL_SRC);
                            close = true;
                        }
                        let props = self.tr("Properties");
                        if ui
                            .add_enabled(self.has_game(), crate::ui::button(props))
                            .clicked()
                        {
                            self.open_properties(crate::core::options::OPTLEVEL_CURR);
                            close = true;
                        }
                    });
                    menu_rect.set(Some(resp.response.rect));
                });
            // any click outside the menu dismisses it
            let outside = ctx.input(|i| {
                i.pointer.any_pressed()
                    && !menu_rect
                        .get()
                        .is_some_and(|r| i.pointer.latest_pos().is_some_and(|p| r.contains(p)))
            });
            if close || outside {
                self.ctx_menu = None;
            }
        }
    }

    /// Make `gi` the current game and publish it to the rest of the UI.
    fn select_game(&mut self, gi: usize) {
        if self.selected == Some(gi) {
            return;
        }
        self.selected = Some(gi);
        if let Some(lib) = &self.lib {
            if let Ok(guard) = lib.try_lock() {
                if let Some(g) = guard.games.get(gi) {
                    self.current_game = g.name.clone();
                }
            }
        }
    }
}

/// A pointer this close to a header cell's left/right edge belongs to the resize
/// handle living on that boundary, not to the header itself — without the guard,
/// a drag meant to grab the Description|Name boundary would also reorder or
/// re-sort instead.
const HEADER_EDGE_GRAB: f32 = 4.0;

/// Half-width of the header's column-resize handle, in points.
const RESIZE_GRAB: f32 = 4.0;

/// Is `p` over the header cell `r`, away from its resize handles?
///
/// The game list resolves every header interaction (hover tint, click-to-sort,
/// drag-to-reorder) by hand: a `TableBuilder` header cell is not in egui's
/// hit-test, so `Response::hovered()`/`clicked()` never fire for one.
fn in_header(p: egui::Pos2, r: egui::Rect) -> bool {
    r.contains(p)
        && (p.x - r.left()).abs() > HEADER_EDGE_GRAB
        && (p.x - r.right()).abs() > HEADER_EDGE_GRAB
}

/// What one frame of pointer input means for the game-list header.
///
/// The three header gestures 1.8.2 offers through `QHeaderView` — drag the
/// separator to resize, drag the section to reorder, click the section to sort —
/// share one primary button, so ownership has to be settled by *what was
/// pressed on*, and then kept until release. That is a small state machine, and
/// it lives here (pure, no egui) so the transitions can be tested: every bug
/// that has hit this header so far was a wrong transition, not a wrong pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderStep {
    /// nothing in flight and nothing to start
    Idle,
    /// press landed on a column separator: begin a width drag on that column
    BeginResize(usize),
    /// press landed inside a column (outside the grab strips): begin a reorder
    BeginDrag(usize),
    /// button still down and a width drag is running — caller tracks the width
    UpdateResize,
    /// button still down and a reorder is running — caller records the target
    UpdateDragTo(usize),
    /// released mid width drag: never falls through to sort
    EndResize,
    /// released mid reorder: caller applies the move, or sorts if nothing moved
    EndDrag,
}

/// Resolve one frame of the header state machine.
///
/// `resize_active` / `drag_active` are the two gestures the app may have in
/// flight; exactly one of them is ever set. The order below *is* the fix: the
/// release (`!down`) handling has to come before a second `down` arm would be
/// considered, and mixing the two into a flat `else if` chain over `down` made
/// the reorder branch unreachable.
pub fn header_step(
    pressed: bool,
    down: bool,
    resize_active: bool,
    drag_active: bool,
    // `sep`: column separator under the pointer, if any
    sep: Option<usize>,
    // `slot`: column slot under the pointer, if any
    slot: Option<usize>,
) -> HeaderStep {
    if pressed {
        if let Some(c) = sep {
            return HeaderStep::BeginResize(c);
        }
        return match slot {
            Some(s) => HeaderStep::BeginDrag(s),
            None => HeaderStep::Idle,
        };
    }
    if !down {
        if resize_active {
            return HeaderStep::EndResize;
        }
        if drag_active {
            return HeaderStep::EndDrag;
        }
        return HeaderStep::Idle;
    }
    // still held
    if resize_active {
        return HeaderStep::UpdateResize;
    }
    if drag_active {
        return match slot {
            Some(s) => HeaderStep::UpdateDragTo(s),
            None => HeaderStep::Idle,
        };
    }
    HeaderStep::Idle
}

/// Whose icon a row may borrow: a clone falls back to its parent set (up the
/// whole clone chain, like `getScreenshot` walking `cloneof`), a softlist entry
/// to the machine it runs on (README §6.1, layer ③).
fn icon_fallbacks(g: &GameMeta, lib: &GameLibrary) -> Vec<String> {
    let mut out = crate::core::icons::icon_candidates(g);
    let mut next = g.cloneof.clone();
    while !next.is_empty() && out.len() < 8 {
        let parent = lib.get(&next).map(|p| p.cloneof.clone()).unwrap_or_default();
        if parent.is_empty() || out.contains(&parent) {
            break;
        }
        out.push(parent.clone());
        next = parent;
    }
    out
}

/// Shift the column sitting in slot `from` into slot `to`.
///
/// The pure half of [`MameApp::move_column`]: like `QHeaderView::moveSection`,
/// dragging a column past two others must let those two slide *towards* the
/// vacated slot — a swap would leave the header disagreeing with the columns the
/// pointer crossed.
pub fn shift_slot(order: &mut [usize], from: usize, to: usize) {
    if from >= order.len() || to >= order.len() || from == to {
        return;
    }
    let col = order[from];
    if from < to {
        for k in from..to {
            order[k] = order[k + 1];
        }
    } else {
        for k in (to + 1..=from).rev() {
            order[k] = order[k - 1];
        }
    }
    order[to] = col;
}

/// x positions of the draggable column separators, given the header cell rects.
///
/// Each entry is `(column, x)`, where `x` is the midpoint of the gap between that
/// column's right edge and the next one's left edge — the separator of 1.8.2's
/// `QHeaderView`, which is drawn in the gap rather than on a cell edge.
fn column_separators(rects: &[(usize, egui::Rect)]) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    for w in rects.windows(2) {
        let (i, a) = w[0];
        let (_, b) = w[1];
        // a zero-width cell (a hidden column) has no gap to speak of; skip it so
        // the handle does not pile up on the hidden column's neighbour
        if b.left() - a.right() <= 0.0 {
            continue;
        }
        out.push((i, (a.right() + b.left()) * 0.5));
    }
    out
}

/// One left-aligned, ellipsis-truncated table cell.
///
/// `SelectableLabel` (the previous implementation) centres its galley inside the
/// rect it is given, so `add_sized(ui.available_size(), …)` pushed every cell's
/// text into the middle of its column — the description ended up half a column
/// away from the status icon and read as if it belonged to the next column.
/// A [`egui::Label`] with `TextWrapMode::Truncate` keeps the text flush against
/// the left edge of the cell and clips long strings with an ellipsis.
fn cell_text(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    // `selectable(false)` matters: egui's `interaction.selectable_labels` defaults
    // to true, which would make every cell sense click *and drag* for text
    // selection — dragging across the list would highlight label text instead of
    // behaving like a plain list.
    ui.add(
        egui::Label::new(egui::RichText::new(text).color(color))
            .truncate()
            .selectable(false)
            // a `Label` defaults to `Sense::hover()`, and it sits on top of the
            // cell's own hit rect: it would take the hover away from it, and the
            // row's context menu (`Response::context_menu`, which needs
            // `hovered()`) never opened
            .sense(egui::Sense {
                click: false,
                drag: false,
                focusable: false,
            }),
    );
}

fn wildcard_match(hay: &str, pattern: &str) -> bool {
    if !pattern.contains('*') {
        return hay.contains(pattern);
    }
    let parts: Vec<&str> = pattern.split('*').filter(|s| !s.is_empty()).collect();
    let mut pos = 0usize;
    for p in parts {
        match hay[pos..].find(p) {
            Some(i) => pos += i + p.len(),
            None => return false,
        }
    }
    true
}

pub fn dock_key(dock: usize, game: &str) -> usize {    // combine dock and game into a request key (hash-based)
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    dock.hash(&mut h);
    game.hash(&mut h);
    h.finish() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dragging a column header across another one used to do nothing at all:
    /// the pointer was still down, so the arm that should have recorded the
    /// drop target was shadowed by an earlier `else if down`.
    #[test]
    fn held_drag_records_its_drop_target() {
        // press on slot 0 (no separator under the pointer)
        assert_eq!(
            header_step(true, true, false, false, None, Some(0)),
            HeaderStep::BeginDrag(0)
        );
        // still down, now over slot 2 — this is the frame that used to vanish
        assert_eq!(
            header_step(false, true, false, true, None, Some(2)),
            HeaderStep::UpdateDragTo(2)
        );
        // release
        assert_eq!(
            header_step(false, false, false, true, None, Some(2)),
            HeaderStep::EndDrag
        );
    }

    /// A width drag owns the pointer for its whole duration; it must never be
    /// mistaken for a reorder that merely happens while a resize is running.
    #[test]
    fn width_drag_never_degrades_into_reorder() {
        assert_eq!(
            header_step(true, true, false, false, Some(1), Some(1)),
            HeaderStep::BeginResize(1)
        );
        assert_eq!(
            header_step(false, true, true, false, None, Some(3)),
            HeaderStep::UpdateResize
        );
        assert_eq!(
            header_step(false, false, true, false, None, Some(3)),
            HeaderStep::EndResize
        );
    }

    #[test]
    fn idle_pointer_is_idle() {
        assert_eq!(header_step(false, false, false, false, None, None), HeaderStep::Idle);
        // held with nothing in flight: e.g. a press that started outside the header
        assert_eq!(header_step(false, true, false, false, None, Some(0)), HeaderStep::Idle);
        // dragged off the header entirely — the target stays where it was
        assert_eq!(header_step(false, true, false, true, None, None), HeaderStep::Idle);
    }

    /// `QHeaderView::moveSection` semantics: the crossed columns slide over,
    /// they are not swapped with the dragged one.
    #[test]
    fn shift_slot_squeezes_the_crossed_columns() {
        let mut order = [0usize, 1, 2, 3, 4, 5, 6];
        shift_slot(&mut order, 0, 2);
        assert_eq!(order, [1, 2, 0, 3, 4, 5, 6]);
        // and back the other way
        shift_slot(&mut order, 2, 0);
        assert_eq!(order, [0, 1, 2, 3, 4, 5, 6]);
        // no-op cases
        shift_slot(&mut order, 3, 3);
        assert_eq!(order, [0, 1, 2, 3, 4, 5, 6]);
        shift_slot(&mut order, 0, 99);
        assert_eq!(order, [0, 1, 2, 3, 4, 5, 6]);
    }
}
