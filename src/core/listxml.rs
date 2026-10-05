//! `mame -listxml` 的解析。
//!
//! 分两个阶段走，见 [`buffer_and_count`]：先把子进程的整份输出收进内存缓冲并
//! 顺手数出机种总数，再从缓冲里解析——这样解析阶段第一帧就有真分母。

use crate::core::library::GameLibrary;
use crate::core::model::*;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::{self, BufRead, Read, Write};

pub type ProgressFn<'a> = &'a mut dyn FnMut(usize);

/// 机种元素的两种写法：现代 MAME 是 `<machine>`，很老的版本是 `<game>`。
///
/// 必须和 [`parse_from_reader`] 里 `b"machine" | b"game"` 那个分支**完全同口径**，
/// 否则数出来的总数会小于实际解析出的条数，百分比会冲过 100%。
const TAG_MACHINE: &[u8] = b"<machine";
const TAG_GAME: &[u8] = b"<game";
/// 跨块重叠的字节数 = 最长 needle 减一。
const OVERLAP: usize = TAG_MACHINE.len() - 1;

/// 把 `-listxml` 的输出收进 `writer`，同时数出一共有多少台机种。
///
/// 为什么先攒起来再解析：机种总数只有收完整份输出才知道，而解析阶段需要它
/// 当分母。直接把子进程 stdout 喂给解析器能省掉这份缓冲，代价是**总数永远
/// 无从得知**，百分比只能拿常量瞎估。
///
/// `on_count` 会周期性收到当前的机种计数。收输出这一步给不出百分比——总数
/// 还没收完——只能报台数。
///
/// 返回机种总数，也就是第二阶段 [`parse_from_reader`] 的分母。
pub fn buffer_and_count<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    on_count: &mut dyn FnMut(usize),
) -> io::Result<usize> {
    let mut chunk = vec![0u8; 1 << 20];
    // carry = 上一块末尾的 OVERLAP 字节；hay = carry + 本块，这样跨块边界的
    // needle 不会被切断
    let mut carry: Vec<u8> = Vec::new();
    let mut hay: Vec<u8> = Vec::with_capacity((1 << 20) + OVERLAP);
    let mut machines = 0usize;
    let mut last_reported = 0usize;

    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        writer.write_all(&chunk[..n])?;

        hay.clear();
        hay.extend_from_slice(&carry);
        hay.extend_from_slice(&chunk[..n]);
        machines += count_tags(&hay, carry.len());

        carry.clear();
        let keep = OVERLAP.min(hay.len());
        carry.extend_from_slice(&hay[hay.len() - keep..]);

        // 每 200 台汇报一次，和解析阶段的粒度保持一致
        if machines >= last_reported + 200 {
            last_reported = machines;
            on_count(machines);
        }
    }
    writer.flush()?;
    if machines != last_reported {
        on_count(machines);
    }
    Ok(machines)
}

/// 数 `hay` 里的机种起始标签。
///
/// `carry_len` 是 `hay` 开头那段的字节数——它上一轮已经数过了。**只计入
/// `i + needle.len() > carry_len` 的匹配**：完整落在 carry 里的那些上一轮必然
/// 已经计过，再算一次就重复了；而起始位置虽然落在 carry 里、但需要借本块
/// 字节才凑得齐的，上一轮不可能命中，正是重叠存在的意义。
fn count_tags(hay: &[u8], carry_len: usize) -> usize {
    let mut n = 0;
    for tag in [TAG_MACHINE, TAG_GAME] {
        let len = tag.len();
        for (i, w) in hay.windows(len).enumerate() {
            if w == tag && i + len > carry_len {
                n += 1;
            }
        }
    }
    n
}

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

    /// 每次最多吐 `n` 字节的 Reader。
    ///
    /// 缓冲是 1MB 分块的，测试里造不出 1MB 的输入，只能靠这种"滴水式"
    /// reader 把标签强行切断在块边界上——跨块重叠逻辑正是为这种情况存在的。
    struct Chunked<'a> {
        data: &'a [u8],
        pos: usize,
        n: usize,
    }

    impl Read for Chunked<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let n = self.n.min(buf.len()).min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    #[test]
    fn count_tags_matches_both_spellings() {
        // 现代 MAME 是 <machine>，老版本是 <game>；两者都要算，否则总数会
        // 小于实际解析出的条数，百分比会冲过 100%
        assert_eq!(count_tags(br#"<machine name="a">"#, 0), 1);
        assert_eq!(count_tags(br#"<game name="b">"#, 0), 1);
        // 不是机种元素的标签不能误算
        assert_eq!(count_tags(br#"<software name="c">"#, 0), 0);
        assert_eq!(count_tags(br#"<device_ref name="d"/>"#, 0), 0);
    }

    #[test]
    fn count_tags_skips_what_carry_already_counted() {
        // hay = carry ++ 本块。完整落在 carry 里的匹配上一轮已经数过。
        let mut hay = Vec::new();
        hay.extend_from_slice(b"<machin"); // 7 字节 carry
        hay.extend_from_slice(b"e>");
        // 起始在 carry 里、但借了本块字节才凑齐 → 该算
        assert_eq!(count_tags(&hay, 7), 1);

        // 完整落在 carry 里 → 不该再算一次
        let mut hay2 = Vec::new();
        hay2.extend_from_slice(b"<machine"); // 8 字节全在 carry 段里
        hay2.extend_from_slice(b">");
        assert_eq!(count_tags(&hay2, 8), 0);
    }

    #[test]
    fn buffer_passes_bytes_through_and_counts() {
        let xml = br#"junk<machine name="a"/><machine name="b"/><game name="c"/>tail"#;
        let mut out: Vec<u8> = Vec::new();
        let mut ticks: Vec<usize> = Vec::new();
        let total = buffer_and_count(
            Chunked { data: xml, pos: 0, n: 3 },
            &mut out,
            &mut |n| ticks.push(n),
        )
        .unwrap();
        // 写出去的必须和读进来的一模一样，否则第二阶段解析的就是坏数据
        assert_eq!(out.as_slice(), xml.as_slice());
        assert_eq!(total, 3);
        // 最后一次回调必须给出最终总数
        assert_eq!(ticks.last().copied(), Some(3));
    }

    #[test]
    fn buffer_counts_once_across_every_chunk_boundary() {
        // 逐字节喂：每个标签都会被切断好几次，但必须只数一次
        let xml = br#"<machine name="a"/><game name="b"/><machine name="c"/>"#;
        for n in [1usize, 2, 3, 5, 7, 8, 9, 13] {
            let mut out: Vec<u8> = Vec::new();
            let total =
                buffer_and_count(Chunked { data: xml, pos: 0, n }, &mut out, &mut |_| {}).unwrap();
            assert_eq!(total, 3, "chunk size {n}");
            assert_eq!(out.as_slice(), xml.as_slice(), "chunk size {n}");
        }
    }

    /// 两阶段的核心不变量：**数出来的总数 == 解析阶段实际产出的条数**。
    ///
    /// 少算了百分比会冲过 100%，多算了永远到不了 100%。两者一旦口径漂移，
    /// 这里就会红。
    #[test]
    fn counted_total_matches_the_parsed_machine_count() {
        let xml = br#"<mame build="0.1">
<machine name="a"><description>A</description></machine>
<machine name="b"><description>B</description></machine>
<game name="c"><description>C</description></game>
<machine name="d"><description>D</description><device type="t" tag=":"><instance name="i"/></device></machine>
</mame>"#;
        let mut buf: Vec<u8> = Vec::new();
        let total = buffer_and_count(Chunked { data: xml, pos: 0, n: 5 }, &mut buf, &mut |_| {})
            .unwrap();
        let lib = parse_from_reader(&buf[..], false, &mut |_| {}).unwrap();
        assert_eq!(total, 4);
        assert_eq!(lib.len(), 4);
    }

    #[test]
    fn buffer_on_empty_input() {
        let mut out: Vec<u8> = Vec::new();
        assert_eq!(buffer_and_count(&b""[..], &mut out, &mut |_| {}).unwrap(), 0);
    }

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
