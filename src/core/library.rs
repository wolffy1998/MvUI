//! 游戏集合 + 二级索引（按名查、按克隆关系查）。

use crate::core::model::GameMeta;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct GameLibrary {
    pub mame_version: String,
    /// raw `mame -showconfig -noreadconfig` text (template for ini write-back,
    /// persisted in the original cache too)
    #[serde(default)]
    pub default_ini: String,
    pub games: Vec<GameMeta>,
    #[serde(skip)]
    index: HashMap<String, usize>,
}

impl GameLibrary {
    pub fn new(mame_version: String) -> Self {
        Self {
            mame_version,
            ..Default::default()
        }
    }

    /// name → slot. The crc index was dropped: the verify matches a zip only
    /// against its own game + clones (origin behaviour), so a global crc table
    /// cost a full pass at every boot and bought nothing.
    pub fn rebuild_indexes(&mut self) {
        self.index.clear();
        for (gi, g) in self.games.iter().enumerate() {
            self.index.insert(g.name.to_lowercase(), gi);
        }
    }

    pub fn get(&self, name: &str) -> Option<&GameMeta> {
        self.index.get(&name.to_lowercase()).map(|&i| &self.games[i])
    }

    pub fn get_idx(&self, name: &str) -> Option<usize> {
        self.index.get(&name.to_lowercase()).copied()
    }

    pub fn len(&self) -> usize {
        self.games.len()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    pub fn push(&mut self, meta: GameMeta) {
        let i = self.games.len();
        let key = meta.name.to_lowercase();
        self.games.push(meta);
        self.index.insert(key, i);
    }

    /// origin: MameDat::completeData()
    pub fn complete_data(&mut self) {
        for i in 0..self.games.len() {
            let mut fixed_source: Option<String> = None;
            let mut parent_exists = true;
            {
                let g = &self.games[i];
                if g.is_ext_rom {
                    if let Some(p) = self.get(&g.romof) {
                        fixed_source = Some(p.sourcefile.clone());
                    } else {
                        parent_exists = false;
                    }
                }
            }
            if !parent_exists {
                continue;
            }
            if let Some(src) = fixed_source {
                self.games[i].sourcefile = src;
            }
            // isHorz: reference = parent for ext roms (do before clone skip)
            let ref_name = {
                let g = &self.games[i];
                if g.is_ext_rom && !g.romof.is_empty() {
                    g.romof.clone()
                } else {
                    g.name.clone()
                }
            };
            let horz = match self.get(&ref_name) {
                Some(r) => match r.displays.first() {
                    Some(d) => d.rotate != "90" && d.rotate != "270",
                    None => true,
                },
                None => true,
            };
            self.games[i].is_horz = horz;
            // clone lists (original aborts the pass on a missing parent; we
            // skip only the broken link)
            let cloneof = self.games[i].cloneof.clone();
            if !cloneof.is_empty() {
                if let Some(pi) = self.get_idx(&cloneof) {
                    let name = self.games[i].name.clone();
                    self.games[pi].clones.insert(name);
                }
            }
        }
    }

    pub fn unique_sorted<F: Fn(&GameMeta) -> String>(&self, f: F) -> Vec<String> {
        let mut v: Vec<String> = self.games.iter().map(&f).filter(|s| !s.is_empty()).collect();
        v.sort();
        v.dedup();
        v
    }
}
