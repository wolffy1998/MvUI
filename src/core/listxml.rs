//! Streaming parser for `mame -listxml` (origin: prototype.cpp XmlDatHandler).

use crate::core::library::GameLibrary;
use crate::core::model::*;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::BufRead;

pub type ProgressFn<'a> = &'a mut dyn FnMut(usize);

pub fn parse_from_reader<R: BufRead>(
    mut r: R,
    is_mess: bool,
    progress: ProgressFn,
) -> Result<GameLibrary, String> {
    let mut reader = Reader::from_reader(&mut r);
    reader.config_mut().trim_text(true);

    let mut lib = GameLibrary::new(String::new());
    let mut cur: Option<GameMeta> = None;
    // deviceInfo persists across machines in the original (no stack); we scope
    // it per machine but keep the "instance requires preceding device" rule
    let mut cur_device: Option<DeviceInfo> = None;
    let mut buf = Vec::with_capacity(8192);
    let mut count = 0usize;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"machine" | b"game" => {
                    let mut m = GameMeta {
                        is_horz: true,
                        ..Default::default()
                    };
                    for a in e.attributes() {
                        let a = a.map_err(|er| er.to_string())?;
                        match a.key.as_ref() {
                            b"name" => m.name = attr_str(&a.value),
                            b"sourcefile" => m.sourcefile = attr_str(&a.value),
                            b"isbios" => m.is_bios = a.value.as_ref() == b"yes",
                            b"isdevice" => m.is_device = a.value.as_ref() == b"yes",
                            b"ismechanical" => m.is_mechanical = a.value.as_ref() == b"yes",
                            b"cloneof" => m.cloneof = attr_str(&a.value),
                            b"romof" => m.romof = attr_str(&a.value),
                            b"sampleof" => m.sampleof = attr_str(&a.value),
                            _ => {}
                        }
                    }
                    cur_device = None;
                    cur = Some(m);
                }
                b"device" => {
                    let mut d = DeviceInfo::default();
                    for a in e.attributes() {
                        let a = a.map_err(|er| er.to_string())?;
                        match a.key.as_ref() {
                            b"type" => d.kind = attr_str(&a.value),
                            b"tag" => d.tag = attr_str(&a.value),
                            b"mandatory" => d.mandatory = a.value.as_ref() == b"1",
                            _ => {}
                        }
                    }
                    cur_device = Some(d);
                }
                b"instance" => {
                    if let Some(d) = cur_device.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            if a.key.as_ref() == b"name" {
                                d.instance = attr_str(&a.value);
                            }
                        }
                    }
                    skip_subtree(&mut reader)?;
                }
                b"extension" => {
                    if let Some(d) = cur_device.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            if a.key.as_ref() == b"name" {
                                d.extensions.push(attr_str(&a.value));
                            }
                        }
                    }
                    skip_subtree(&mut reader)?;
                }
                b"description" => {
                    if let Some(m) = cur.as_mut() {
                        m.description = read_text(&mut reader)?;
                    }
                }
                b"year" => {
                    if let Some(m) = cur.as_mut() {
                        m.year = read_text(&mut reader)?;
                    }
                }
                b"manufacturer" => {
                    if let Some(m) = cur.as_mut() {
                        m.manufacturer = read_text(&mut reader)?;
                    }
                }
                b"ramoption" => {
                    // `<ramoption default="1">2</ramoption>` carries the size as a
                    // text node, so it always arrives as a *Start* event; reading
                    // `default` only in the Empty branch meant it was never seen
                    // and `default_ram_option` stayed 0 for every machine.
                    let is_default = e
                        .try_get_attribute("default")
                        .ok()
                        .flatten()
                        .map(|v| v.value.as_ref() == b"1")
                        .unwrap_or(false);
                    if let Some(m) = cur.as_mut() {
                        let t = read_text(&mut reader)?;
                        if let Ok(n) = t.trim().parse::<u32>() {
                            m.ram_options.push(n);
                            if is_default {
                                m.default_ram_option = n;
                            }
                        }
                    }
                }
                b"version" => {
                    read_text(&mut reader)?; // header, ignored (version from -help)
                }
                b"feature" | b"configuration" | b"dipswitch" | b"port" | b"device_ref" => {
                    skip_subtree(&mut reader)?;
                }
                _ => {}
            },
            Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                b"rom" => {
                    if let Some(m) = cur.as_mut() {
                        m.roms.push(rom_from_attrs(e)?);
                    }
                }
                b"disk" => {
                    if let Some(m) = cur.as_mut() {
                        let mut d = DiskInfo::default();
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            match a.key.as_ref() {
                                b"name" => d.name = attr_str(&a.value),
                                b"sha1" => d.sha1 = attr_str(&a.value),
                                b"merge" => d.merge = attr_str(&a.value),
                                b"region" => d.region = attr_str(&a.value),
                                b"index" => d.index = attr_str(&a.value).parse().unwrap_or(0),
                                b"status" => d.status = attr_str(&a.value),
                                _ => {}
                            }
                        }
                        m.disks.push(d);
                    }
                }
                b"biosset" => {
                    // original: only parsed when isMESS || isBios
                    let keep = is_mess || cur.as_ref().map(|m| m.is_bios).unwrap_or(false);
                    if keep {
                        if let Some(m) = cur.as_mut() {
                            let mut b = BiosSet::default();
                            for a in e.attributes() {
                                let a = a.map_err(|er| er.to_string())?;
                                match a.key.as_ref() {
                                    b"name" => b.name = attr_str(&a.value),
                                    b"description" => b.description = attr_str(&a.value),
                                    b"default" => b.is_default = a.value.as_ref() == b"yes",
                                    _ => {}
                                }
                            }
                            m.bios_sets.push(b);
                        }
                    }
                }
                b"sample" => {
                    if let Some(m) = cur.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            if a.key.as_ref() == b"name" {
                                m.samples.push(attr_str(&a.value));
                            }
                        }
                    }
                }
                b"chip" => {
                    if let Some(m) = cur.as_mut() {
                        let mut c = ChipInfo::default();
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            match a.key.as_ref() {
                                b"name" => c.name = attr_str(&a.value),
                                b"tag" => c.tag = attr_str(&a.value),
                                b"type" => c.kind = attr_str(&a.value),
                                b"clock" => c.clock = attr_str(&a.value).parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                        m.chips.push(c);
                    }
                }
                b"softwarelist" => {
                    if let Some(m) = cur.as_mut() {
                        let mut s = SoftwareListRef::default();
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            match a.key.as_ref() {
                                b"name" => s.name = attr_str(&a.value),
                                b"status" => s.status = attr_str(&a.value),
                                b"filter" => s.filter = attr_str(&a.value),
                                _ => {}
                            }
                        }
                        m.softwarelists.push(s);
                    }
                }
                b"display" => {
                    if let Some(m) = cur.as_mut() {
                        let mut d = DisplayInfo::default();
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            match a.key.as_ref() {
                                b"type" => d.kind = attr_str(&a.value),
                                b"rotate" => d.rotate = attr_str(&a.value),
                                b"flipx" => d.flipx = a.value.as_ref() == b"yes",
                                b"width" => d.width = attr_str(&a.value).parse().unwrap_or(0),
                                b"height" => d.height = attr_str(&a.value).parse().unwrap_or(0),
                                b"refresh" => d.refresh = attr_str(&a.value),
                                _ => {}
                            }
                        }
                        m.displays.push(d);
                    }
                }
                b"sound" => {
                    if let Some(m) = cur.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            if a.key.as_ref() == b"channels" {
                                m.channels = attr_str(&a.value).parse().unwrap_or(0);
                            }
                        }
                    }
                }
                b"input" => {
                    if let Some(m) = cur.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            match a.key.as_ref() {
                                b"service" => m.service = a.value.as_ref() == b"yes",
                                b"tilt" => m.tilt = a.value.as_ref() == b"yes",
                                b"players" => m.players = attr_str(&a.value).parse().unwrap_or(0),
                                b"buttons" => m.buttons = attr_str(&a.value).parse().unwrap_or(0),
                                b"coins" => m.coins = attr_str(&a.value).parse().unwrap_or(0),
                                _ => {}
                            }
                        }
                    }
                }
                b"control" => {
                    if let Some(m) = cur.as_mut() {
                        let mut c = ControlInfo::default();
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            match a.key.as_ref() {
                                b"type" => c.kind = attr_str(&a.value),
                                b"minimum" => c.min = attr_str(&a.value).parse().unwrap_or(0),
                                b"maximum" => c.max = attr_str(&a.value).parse().unwrap_or(0),
                                b"sensitivity" => {
                                    c.sensitivity = attr_str(&a.value).parse().unwrap_or(0)
                                }
                                b"keydelta" => c.keydelta = attr_str(&a.value).parse().unwrap_or(0),
                                b"reverse" => c.reverse = a.value.as_ref() == b"yes",
                                _ => {}
                            }
                        }
                        m.controls.push(c);
                    }
                }
                b"driver" => {
                    if let Some(m) = cur.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            let grade = grade_of(&attr_str(&a.value));
                            match a.key.as_ref() {
                                b"status" => m.driver.status = grade,
                                b"emulation" => m.driver.emulation = grade,
                                b"color" => m.driver.color = grade,
                                b"sound" => m.driver.sound = grade,
                                b"graphic" => m.driver.graphic = grade,
                                b"cocktail" => m.driver.cocktail = grade,
                                b"protection" => m.driver.protection = grade,
                                b"savestate" => m.driver.savestate = grade,
                                _ => {}
                            }
                        }
                    }
                }
                b"palette" => {
                    if let Some(m) = cur.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            if a.key.as_ref() == b"size" {
                                m.palettesize = attr_str(&a.value).parse().unwrap_or(0);
                            }
                        }
                    }
                }
                b"ramoption" => {
                    // `<ramoption default="1">2</ramoption>` always arrives as a
                    // Start event (it carries the size as text), so the attribute
                    // is read there — this Empty arm could never fire and kept a
                    // stale `default_ram` around.
                }
                _ => {}
            },
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"machine" | b"game" => {
                    if let Some(mut m) = cur.take() {
                        if let Some(d) = cur_device.take() {
                            if !d.instance.is_empty() {
                                m.devices.push(d);
                            }
                        }
                        lib.push(m);
                        count += 1;
                        if count % 200 == 0 {
                            progress(count);
                        }
                    }
                }
                b"device" => {
                    if let (Some(d), Some(m)) = (cur_device.take(), cur.as_mut()) {
                        if !d.instance.is_empty() {
                            m.devices.push(d);
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(err) => {
                return Err(format!(
                    "listxml parse error at offset {}: {err}",
                    reader.buffer_position()
                ))
            }
        }
        buf.clear();
    }

    lib.rebuild_indexes();
    progress(count);
    Ok(lib)
}

fn rom_from_attrs(e: &quick_xml::events::BytesStart) -> Result<RomInfo, String> {
    let mut r = RomInfo {
        available: false,
        ..Default::default()
    };
    for a in e.attributes() {
        let a = a.map_err(|err| err.to_string())?;
        match a.key.as_ref() {
            b"name" => r.name = attr_str(&a.value),
            b"bios" => r.bios = attr_str(&a.value),
            b"size" => r.size = attr_str(&a.value).parse().unwrap_or(0),
            b"crc" => {
                r.crc = u32::from_str_radix(attr_str(&a.value).trim_start_matches("0x"), 16)
                    .unwrap_or(0)
            }
            b"merge" => r.merge = attr_str(&a.value),
            b"region" => r.region = attr_str(&a.value),
            b"status" => r.status = attr_str(&a.value),
            _ => {}
        }
    }
    Ok(r)
}

fn attr_str(value: &[u8]) -> String {
    xml_unescape(&String::from_utf8_lossy(value))
}

/// Minimal XML entity decoder.
pub fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'&' {
            if let Some(end) = s[i..].find(';') {
                let ent = &s[i + 1..i + end];
                let mapped = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => {
                        let cp = if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                            u32::from_str_radix(hex, 16).ok()
                        } else {
                            ent.strip_prefix('#').and_then(|d| d.parse::<u32>().ok())
                        };
                        cp.and_then(char::from_u32)
                    }
                };
                match mapped {
                    Some(ch) => {
                        out.push(ch);
                        i += end + 1;
                        continue;
                    }
                    None => out.push_str(&s[i..i + end + 1]),
                }
                i += end + 1;
                continue;
            }
        }
        let ch = s[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn read_text<R: BufRead>(reader: &mut Reader<R>) -> Result<String, String> {
    let mut out = String::new();
    let mut buf = Vec::with_capacity(1024);
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(t)) => {
                // quick-xml >=0.41 dropped `unescape()`; `decode()` is the
                // replacement that resolves the predefined XML entities and
                // numeric character references (what `unescape` did).
                if let Ok(s) = t.decode() {
                    out.push_str(&s);
                }
            }
            Ok(Event::End(_)) | Ok(Event::Eof) => return Ok(out.trim().to_string()),
            Ok(Event::Start(_)) | Ok(Event::Empty(_)) => skip_subtree(reader)?,
            Err(err) => return Err(err.to_string()),
            _ => {}
        }
        buf.clear();
    }
}

fn skip_subtree<R: BufRead>(reader: &mut Reader<R>) -> Result<(), String> {
    skip_subtree_buf(reader, &mut Vec::with_capacity(4096))
}

fn skip_subtree_buf<R: BufRead>(reader: &mut Reader<R>, buf: &mut Vec<u8>) -> Result<(), String> {
    let mut depth = 1usize;
    while depth > 0 {
        match reader.read_event_into(buf) {
            Ok(Event::Start(_)) => depth += 1,
            Ok(Event::End(_)) => depth -= 1,
            Ok(Event::Eof) => return Ok(()),
            Err(err) => return Err(err.to_string()),
            _ => {}
        }
        buf.clear();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_listxml() {
        let xml = r##"<?xml version="1.0"?>
<mame build="0.261">
<version>0.261</version>
<machine name="pacmana" sourcefile="pacman.c">
<description>Pac-Man (Namco)</description>
<year>1980</year>
<manufacturer>Namco</manufacturer>
<display type="raster" rotate="0" width="224" height="288" refresh="60.606060"/>
<rom name="pacman.6e" size="4096" crc="95877ba1" region="maincpu"/>
<driver status="good" emulation="good" color="good" sound="good" graphic="good" savestate="supported"/>
</machine>
<machine name="pacman" sourcefile="pacman.c" cloneof="pacmana" romof="pacmanb">
<description>Pac-Man (Midway)</description>
<year>1980</year>
<manufacturer>Midway</manufacturer>
<display type="raster" rotate="90" width="224" height="288" refresh="60.606060"/>
<rom name="pacman.6e" size="4096" crc="95877ba1" region="maincpu"/>
<driver status="good" emulation="good" color="good" sound="good" graphic="good" savestate="supported"/>
</machine>
</mame>"##;
        let mut lib = parse_from_reader(xml.as_bytes(), false, &mut |_| {}).unwrap();
        assert_eq!(lib.len(), 2);
        lib.complete_data();
        let g = lib.get("pacman").unwrap();
        assert_eq!(g.description, "Pac-Man (Midway)");
        assert_eq!(g.roms[0].crc, 0x95877ba1);
        assert_eq!(g.driver.status, STATUS_GOOD);
        assert_eq!(g.driver.savestate, STATUS_GOOD);
        assert_eq!(g.displays[0].rotate, "90");
        assert!(!g.is_horz);
        let pi = lib.get_idx("pacmana").unwrap();
        assert!(lib.games[pi].clones.contains("pacman"));
    }
}
