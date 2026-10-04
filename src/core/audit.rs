//! ROM audit, 1:1 port of audit.cpp RomAuditor (internal audit + console scan
//! + Logiqx fixdat export with the 4 export methods).
//!
//! Availability lives in the model (per-rom/disk `available` + game `available`),
//! matching the original GameInfo fields.

use crate::core::archive::{self, is_7z, is_zip};
use crate::core::library::GameLibrary;
use crate::core::model::*;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// AUDIT_ONLY=0, AUDIT_EXPORT_COMPLETE, AUDIT_EXPORT_ALL, AUDIT_EXPORT_INCOMPLETE, AUDIT_EXPORT_MISSING
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
    /// set once the run is over, so progress forwarders can exit even when
    /// nothing was scanned (total == 0)
    pub finished: Arc<AtomicBool>,
    progress: Arc<Mutex<(usize, usize, String)>>,
}

impl AuditHandle {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
            progress: Arc::new(Mutex::new((0, 0, String::new()))),
        }
    }
    pub fn snapshot(&self) -> (usize, usize, String) {
        self.progress.lock().unwrap().clone()
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
    fn set_progress(&self, done: usize, total: usize, current: &str) {
        *self.progress.lock().unwrap() = (done, total, current.to_string());
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
    // Whatever happens from here on — normal return, early exit or a panic in
    // rayon/utf8/etc. — the handle must end up finished, otherwise the 200 ms
    // progress forwarder never terminates and keeps flooding the channel
    // (README P2-19).
    let _finish_guard = FinishOnDrop(handle.finished.clone());
    // 1) reset: nodump → available
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

    let mut units: Vec<(PathBuf, usize)> = Vec::new();
    for dir in rom_paths {
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

    handle.set_progress(0, units.len(), "");

    let results: Mutex<Vec<Vec<Mark>>> = Mutex::new(Vec::new());
    units.par_iter().enumerate().for_each(|(ui, (path, gi))| {
        if handle.cancelled() {
            return;
        }
        let cur = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        handle.set_progress(ui, units.len(), &format!("{cur}/"));
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
        } else if let Ok(entries) = archive::list_archive(path) {
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
            for e in entries {
                let Some(crc) = e.crc else { continue };
                for (gj, ri) in table.get(&crc).map(|v| v.as_slice()).unwrap_or(&[]) {
                    marks.push(Mark::Rom(*gj, *ri, true));
                }
            }
        }
        results.lock().unwrap().push(marks);
    });

    // apply marks; disk marks propagate to clones sharing the sha1
    let mut disk_marks: Vec<(usize, usize)> = Vec::new();
    for marks in results.into_inner().unwrap() {
        for m in marks {
            match m {
                Mark::Rom(gi, ri, ok) => {
                    lib.games[gi].roms[ri].available = ok || lib.games[gi].roms[ri].available;
                }
                Mark::Disk(gi, di) => disk_marks.push((gi, di)),
            }
        }
    }
    // a CHD is shared by the whole clone family — mark every game that
    // references the same sha1 (origin: the clone loop in RomAuditor::run)
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

    // 3) finalize per game (origin step 6)
    let mut parent_maps: HashMap<usize, HashMap<u32, usize>> = HashMap::new();
    for i in 0..lib.games.len() {
        if lib.games[i].is_ext_rom {
            lib.games[i].available = GAME_COMPLETE;
            continue;
        }
        let romof = lib.games[i].romof.clone();
        // crc → rom slot of the parent set, memoized: the fallback used to
        // linearly scan the parent's roms for every missing rom
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
        // origin: audit.cpp:468-514 — the level has to be judged from the *final*
        // rom / disk state. 1.8.2 read `allinParent` after the clone-set scan had
        // already propagated the parent's ROMs into the clone, so a clone whose
        // ROMs all come from the parent set counts as complete; recomputing here
        // is equivalent, and it also covers the loose-file / directory path,
        // where the scan propagates nothing and only this backfill can fill the
        // last missing ROMs.
        let complete = lib.games[i].roms.iter().all(|r| r.available)
            && lib.games[i].disks.iter().all(|d| d.available);
        lib.games[i].available = if complete { GAME_COMPLETE } else { GAME_MISSING };
    }

    // 4) console (MESS) audit — creates ext roms
    let console_names: Vec<String> = lib
        .games
        .iter()
        .filter(|g| !g.devices.is_empty() && !g.is_ext_rom)
        .map(|g| g.name.clone())
        .collect();
    let total_consoles = console_names.len();
    for (ci, console) in console_names.iter().enumerate() {
        let Some(dirpath) = extra_software.get(console) else {
            continue;
        };
        if dirpath.is_empty() || !Path::new(dirpath).exists() {
            continue;
        }
        handle.set_progress(ci, total_consoles, console);
        audit_console(lib, console, dirpath);
    }
    handle.set_progress(0, 0, "");
    handle.finish();

    lib.complete_data();
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
