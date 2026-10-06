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

/// `is_mess` **已不再使用**（2026-10-06），保留参数是为了不改动全部调用点。
///
/// 它当年只有一个用处：门控 `<biosset>` 的解析（`is_mess || is_bios`，
/// 抄 1.8.2 的 `prototype.cpp:204`）。那道门在真实 MAME 0.284 上几乎全关
/// —— `<biosset>` 分布在 3655 个 machine 上，而 `isbios="yes"` 只有 79 处、
/// `isbios="no"` 零处，于是只解析出 42 个机种的 BIOS 声明。参数就此失去用途。
///
/// 将来若有 MESS 专属的解析分支，再把它用起来。
pub fn parse_from_reader<R: BufRead>(
    mut r: R,
    _is_mess: bool,
    progress: ProgressFn,
) -> Result<GameLibrary, String> {
    let mut reader = Reader::from_reader(&mut r);
    reader.config_mut().trim_text(true);

    let mut lib = GameLibrary::new(String::new());
    let mut cur: Option<GameMeta> = None;
    // deviceInfo persists across machines in the original (no stack); we scope
    // it per machine but keep the "instance requires preceding device" rule
    let mut cur_device: Option<DeviceInfo> = None;
    // 当前正在解析的 `<slot>`。`<slotoption>` 是它的子元素，得靠这个跨事件
    // 挂回去；`<slot>` 本身是 START/END 成对的，END 时收尾。
    let mut cur_slot: Option<SlotInfo> = None;
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
                    cur_slot = None;
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
                // 注意：`<instance>` / `<extension>` 在 DTD 里都是 EMPTY
                // （`<!ELEMENT instance EMPTY>`），**只**会走 Empty 事件。
                // 这里原来有同名的 Start 分支，是死代码，导致 `instance` 永远
                // 读不到 → End 分支的 `!d.instance.is_empty()` 守卫把整个
                // `<device>` 丢掉（实测全库 `<device>` 收不到一条）。已挪到
                // Empty 分支，别挪回来。
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
                b"slot" => {
                    // `<slot name="ctrl1"> ... <slotoption .../> ... </slot>`
                    //
                    // 槽位本身只是个名字，真正有信息量的是里面的 option
                    // （可选设备）。这里只**开**槽位，收到 `</slot>` 时才挂回
                    // 机种（见 End 分支）；自闭合的 `<slot/>` 走 Empty 分支。
                    //
                    // 注意 DTD 里 `<slotoption>` 是 EMPTY（`<!ELEMENT slotoption
                    // EMPTY>`），所以它**永远**走 Empty 事件，Start 分支里那个
                    // 同名分支是死代码 —— 而且它调`skip_subtree` 会一路吃到下一
                    // 个 End，把后面的 slotoption 吞掉。别加回来。
                    cur_slot = Some(read_slot(e)?);
                }
                b"feature" | b"configuration" | b"dipswitch" | b"port" => {
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
                    // **门控已去掉（2026-10-06）。** 原来是
                    // `is_mess || is_bios`（照抄 1.8.2 的 `prototype.cpp:204`
                    // `"else if ((isMESS || gameInfo->isBios) && qName == "biosset")"`），
                    // 但实测 MAME 0.284 全量 listxml（320MB）：
                    //   - `<biosset>` 出现 40435 次，分布在 **3655 个 machine**
                    //   - `isbios="yes"` 只有 **79 处**，`isbios="no"` **0 处**
                    //     （绝大多数 machine 干脆不写这个属性 → 默认 false）
                    // 于是这道门只放行了极少数：**3655 个机种只解析出 42 个**，
                    // 丢 99.9%。
                    //
                    // 后果不是"少显示点东西"，而是**BIOS 文件凭空消失**：
                    // Rom 段按 `r.bios.is_empty()` 把带 `bios=` 的条目排除掉，
                    // 而 Bios 段因为拿不到 `bios_sets` 也一行都渲染不出来。
                    // 用户看到的现象正是"BIOS 文件没从 Rom 里独立显示"——
                    // 它既不在 Rom 段，也不在 Bios 段（实测 64576 个 rom 如此）。
                    //
                    // `<biosset>` 只是个描述性的标签元素，多解析它不会引入错误
                    // 数据；真正判定用哪一套的是 `default` 属性与 `view_of`
                    // 的筛选逻辑。
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
                b"device_ref" => {
                    // `<device_ref tag=":maincpu" name="m68000"/>`
                    //
                    // 1.8.2 也把 device_ref 归进忽略分支（`utils.cpp`），所以
                    // 这里没有旧版可抄；但"引用设备"面板要显示设备 rom，而设备
                    // 引用**只**在这里出现过——不解析它，`GameMeta::devices`
                    // 永远是空的（`verify.rs` 的 MESS 主机扫描也依赖它来认出一
                    // 台主机）。反向依赖：console 扫描靠 `!devices.is_empty()`
                    // 挑出主机机种。
                    //
                    // 2. `tag` 形如 `":maincpu"`（**前导冒号**），是标签在父机种
                    // 里的全名；`name` 才是设备机种名。冒号前缀留着会让
                    // "按 tag 查设备"永远查不到，所以在这里剥掉，多级标签
                    // （`"igs023:sprcol"`）保留后段。
                    if let Some(m) = cur.as_mut() {
                        let mut d = DeviceInfo::default();
                        // 标记这是 `<device_ref>` 而非 `<device>`。判别依据是
                        // XML 元素本身，不能靠 `kind == instance` 猜（见
                        // `DeviceInfo::is_ref` 的注释：nes 的 9 个 `<device>`
                        // 全会被猜错）。
                        d.is_ref = true;
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            match a.key.as_ref() {
                                b"tag" => {
                                    let t = attr_str(&a.value);
                                    d.tag = t.strip_prefix(':').unwrap_or(&t).to_string();
                                }
                                b"name" => {
                                    let n = attr_str(&a.value);
                                    // 设备机种名放在 `kind`：它是唯一能拿去查库
                                    // 的键（`get_idx(name)`），而 `instance` 是
                                    // 标签侧的旧版 map key，语义不同。
                                    d.kind = n.clone();
                                    d.instance = n;
                                }
                                _ => {}
                            }
                        }
                        m.devices.push(d);
                    }
                }
                b"slot" => {
                    // 自闭合的空槽位：`<slot name="nes_slot"/>`。它走 Empty
                    // 事件而不是 Start+End，**只**在 Start 分支收尾会被静默
                    // 丢掉 —— 实测 `nes.xml` 的 12 个 slot 里就有这种形态。
                    if let Some(m) = cur.as_mut() {
                        m.slots.push(read_slot(e)?);
                    }
                }
                b"slotoption" => {
                    // 理论上不会发生在 Empty 上（`slotoption` 总在 `<slot>` 里），
                    // 但真出现了也不该 panic 或丢数据。
                    if let Some(s) = cur_slot.as_mut() {
                        s.options.push(read_slotoption(e)?);
                    }
                }
                b"instance" => {
                    // `<instance name="cartridge" briefname="cart"/>` —— DTD 标
                    // EMPTY，所以只在 Empty 分支处理（见 Start 分支的注释）。
                    if let Some(d) = cur_device.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            if a.key.as_ref() == b"name" {
                                d.instance = attr_str(&a.value);
                            }
                        }
                    }
                }
                b"extension" => {
                    // `<extension name="nes"/>`，同样EMPTY（见上）。
                    if let Some(d) = cur_device.as_mut() {
                        for a in e.attributes() {
                            let a = a.map_err(|er| er.to_string())?;
                            if a.key.as_ref() == b"name" {
                                d.extensions.push(attr_str(&a.value));
                            }
                        }
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
                b"slot" => {
                    // `</slot>`：收尾挂回机种。空槽位走的是 Empty 事件（见下），
                    // 这里只处理 START/END 成对的那种。
                    if let (Some(s), Some(m)) = (cur_slot.take(), cur.as_mut()) {
                        m.slots.push(s);
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

/// 从 `<slot name="...">` 的属性里读出槽位。
///
/// 单独抽出来是因为这个元素有两种事件形态（带 `slotoption` 时是 Start+End，
/// 空槽位时是自闭合的 Empty），两处都要用同一份读取逻辑，否则改一处忘另一处
/// 就会出现"带选项的槽位有名字、空槽位没名字"这种半截数据。
fn read_slot(e: &quick_xml::events::BytesStart) -> Result<SlotInfo, String> {
    let mut s = SlotInfo::default();
    for a in e.attributes() {
        let a = a.map_err(|er| er.to_string())?;
        if a.key.as_ref() == b"name" {
            s.name = attr_str(&a.value);
        }
    }
    Ok(s)
}

/// 从 `<slotoption name="..." devname="..." default="yes"/>` 读出一个选项。
fn read_slotoption(e: &quick_xml::events::BytesStart) -> Result<SlotOption, String> {
    let mut o = SlotOption::default();
    for a in e.attributes() {
        let a = a.map_err(|er| er.to_string())?;
        match a.key.as_ref() {
            b"name" => o.name = attr_str(&a.value),
            b"devname" => o.devname = attr_str(&a.value),
            b"default" => o.default = a.value.as_ref() == b"yes",
            _ => {}
        }
    }
    Ok(o)
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

    /// **`<biosset>` 必须无条件解析**——不能被 `is_mess || is_bios` 门控。
    ///
    /// 这条测试是2026-10-06 那个BIOS「整段消失」BUG 的回归钉子。它看起来
    /// 琐碎，但那个门控是**照抄旧版 1.8.2**（`prototype.cpp:204`）的，而旧版
    /// 在真实 MAME 上也是错的：
    ///
    /// - `<biosset>` 分布在全库 **3655 个 machine** 上（实测 MAME 0.284）
    /// - 而 `isbios="yes"` 只有 **79 处**，`isbios="no"` **0 处**
    ///   （绝大多数 machine 干脆不写这个属性 → `is_bios` 默认 false）
    ///
    /// 于是门控只放行了 42 个机种，**丢 99.9%**。后果不是"少显示"，而是
    /// Rom 段按 `bios.is_empty()` 排除掉64576 个 BIOS rom、Bios 段又没数据
    /// 可渲染 → **文件凭空消失，零报错**。
    ///
    /// 下面的xml刻意**不带** `isbios` 属性（`kovplus` 的真实形态），且两套
    /// BIOS 都没写 `default="yes"`（实测大量机种如此）——正是当年被门控
    /// 挡掉的那一类。
    #[test]
    fn biosset_is_parsed_even_without_the_isbios_attribute() {
        let xml = br#"<mame build="0.1">
<machine name="kovplus" sourcefile="igs/pgm.cpp" romof="pgm">
  <description>Kovplus</description>
  <biosset name="v2" description="PGM BIOS V2"/>
  <biosset name="v1" description="PGM BIOS V1"/>
  <rom name="pgm_p0603_v119.u1" size="4194304" crc="e4b0875d" region="maincpu"/>
  <rom name="pgm_p02s.u20" bios="v2" size="131072" crc="78c15fa2" region="maincpu"/>
  <rom name="pgm_p01s.u20" bios="v1" size="131072" crc="e42b166e" region="maincpu"/>
</machine>
</mame>"#;
        let lib = parse_from_reader(&xml[..], false, &mut |_| {}).unwrap();
        let g = lib.get("kovplus").expect("kovplus 在库里");
        assert_eq!(
            g.bios_sets.len(),
            2,
            "没有 isbios 属性、也没写 default 的机种，biosset 照样要解析出来"
        );
        assert_eq!(g.bios_sets[0].name, "v2");
        assert_eq!(g.bios_sets[0].description, "PGM BIOS V2");
        assert_eq!(g.bios_sets[1].name, "v1");
        assert!(
            g.bios_sets.iter().all(|b| !b.is_default),
            "这两套本来就没有 default 属性，别凭空造一个"
        );
        // 带bios= 的 rom 本身也要在（Rom 段排除它们是靠这个字段）
        let tagged: Vec<&str> = g
            .roms
            .iter()
            .filter(|r| !r.bios.is_empty())
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(tagged, vec!["pgm_p02s.u20", "pgm_p01s.u20"]);
    }

    /// `<biosset>` 的 `default="yes"` 必须如实记录，且**不能**因为解析放开
    /// 就给所有机种都塞一个默认值。
    #[test]
    fn biosset_default_flag_is_read_from_the_attribute() {
        let xml = br#"<mame build="0.1">
<machine name="pgm" isbios="yes">
  <biosset name="v1" description="V1"/>
  <biosset name="v2" description="V2" default="yes"/>
</machine>
</mame>"#;
        let lib = parse_from_reader(&xml[..], false, &mut |_| {}).unwrap();
        let g = lib.get("pgm").expect("pgm 在库里");
        assert_eq!(g.bios_sets.len(), 2);
        assert!(!g.bios_sets[0].is_default, "v1 没写 default");
        assert!(g.bios_sets[1].is_default, "v2 写了 default=yes");
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

    /// `<slot>` 的两种写法都要解析：带 `<slotoption>` 的（START…END）和
    /// 自闭合的空槽位（`<slot name="nes_slot"/>`，走 Empty 事件）。
    ///
    /// 片段逐字取自 `mame nes -listxml`。
    #[test]
    fn slots_are_parsed_in_both_the_paired_and_the_self_closing_form() {
        let xml = br#"<mame build="x">
<machine name="nes" sourcefile="nes.cpp">
<description>Nintendo Entertainment System</description>
<device_ref tag=":maincpu" name="rp2a03g"/>
<device type="cartridge" tag="nes_slot" mandatory="1" interface="nes_cart">
<instance name="cartridge" briefname="cart"/>
<extension name="nes"/>
<extension name="unf"/>
</device>
<slot name="ctrl1">
<slotoption name="vboy" devname="nes_vboyctrl"/>
<slotoption name="powerpad" devname="nes_powerpad" default="yes"/>
</slot>
<slot name="nes_slot">
</slot>
<slot name="empty_slot"/>
</machine>
</mame>"#;
        let mut lib = parse_from_reader(&xml[..], false, &mut |_| {}).unwrap();
        lib.complete_data();
        let g = lib.get("nes").unwrap();

        assert_eq!(g.slots.len(), 3, "带 option 的 + 成对空槽位 + 自闭合槽位");
        assert_eq!(g.slots[0].name, "ctrl1");
        assert_eq!(g.slots[0].options.len(), 2);
        assert_eq!(g.slots[0].options[0].name, "vboy");
        assert_eq!(g.slots[0].options[0].devname, "nes_vboyctrl");
        assert!(!g.slots[0].options[0].default);
        assert_eq!(g.slots[0].options[1].name, "powerpad");
        assert!(g.slots[0].options[1].default, "default=\"yes\"");
        assert_eq!(g.slots[1].name, "nes_slot");
        assert!(g.slots[1].options.is_empty());
        assert_eq!(g.slots[2].name, "empty_slot", "自闭合的也要留下");

        // device_ref 与 device 必须能区分开：前者 is_ref，后者不是
        assert_eq!(g.devices.len(), 2);
        let r = g.devices.iter().find(|d| d.is_ref).expect("device_ref");
        assert_eq!(r.kind, "rp2a03g");
        let d = g
            .devices
            .iter()
            .find(|d| !d.is_ref)
            .expect("<device> 不是 device_ref");
        assert_eq!(d.kind, "cartridge");
        assert_eq!(d.instance, "cartridge", "instance 名恰好等于 type");
        assert_eq!(d.extensions, vec!["nes".to_string(), "unf".to_string()]);
    }
}
