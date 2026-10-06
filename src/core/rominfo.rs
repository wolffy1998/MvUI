//! Rom 信息面板的数据聚合：把「一个游戏 + 它依赖的一切」摊平成可展示的分段。
//!
//! **这个模块不读磁盘、不做审计。** 它只回答一个问题：给定一个游戏库和
//! 一个游戏名，Rom 信息面板该显示哪些行、每行是什么状态。数据全部来自
//! `-listxml` 解析出来的 [`GameMeta`]，加上审计已经写在每条
//! [`RomInfo::available`] / [`DiskInfo::available`] 上的结果。
//!
//! 这也是它和现有五个文本 dock 的根本区别：History / MAMEInfo /
//! DriverInfo / Story / Command 都走 `dat_file_option` 指向的**外部 dat
//! 文件**（`core/dat.rs`），而 Rom 信息**不依赖任何外部文档**——它要的是
//! 审计结果，而审计结果就躺在游戏库里，跟着 `gamelist.cache` 一起落盘。
//! 所以这个模块只从 `&GameLibrary` 取数，一个文件都不开。
//!
//! 依赖的边界（用户口径）：此 ROM + 依赖的主 ROM 文件（`romof` 父集，
//! 再上一级祖父集）+ BIOS + 引用设备 + Samples + CHD。
//!
//! 分段顺序与用户给的参考样式一致：
//!
//! ```text
//! Rom:
//!   pgm_p02s.u20      很好    ✓    CRC(78c15fa2)    maincpu
//! Bios:
//!   v2                PGM BIOS V2
//! 引用设备:
//!   m68000            全部获得
//! ```
//!
//! [`RomRow::state`] 用**枚举**而不是 i18n key：这一层不许出现面向用户的
//! 文案（翻译是 UI 的事），所以「很好 / 缺失 / 未审计」是三个可辨识的状态
//! 变体，UI 各自映射成词条或颜色。

use crate::core::library::GameLibrary;
use crate::core::model::{GameMeta, RomInfo};

/// 一行 Rom / CHD 的展示状态。
///
/// 刻意区分「审计过且缺失」和「根本没审计过」：前者是红色的坏消息，后者是
/// 灰色的未知。把两者混成一个"缺失"会让刚装好 MAME、还没跑过审计的用户
/// 看到一屏红色，而那不是缺失，是**还没查**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomState {
    /// 审计通过。
    Good,
    /// 审计过，确实没有。
    Missing,
    /// 尚未审计（`audited=false` 的冷启动、或该条目来自未走审计的路径）。
    Unknown,
    /// `nodump`：MAME 明确说这个条目没有 dump，**算作拥有**。
    ///
    /// 单独成一个变体而不是并进 `Good`，是因为它值得在界面上说清楚：
    /// 一个"永远不会有"的条目显示成绿色对勾是对的，但用户看到它时应当
    /// 知道原因。UI 决定用哪一种绿色。
    NoDump,
    /// `baddump`：条目**有**，但内容是坏的（MAME 自己都标了坏 dump）。
    ///
    /// 必须和 `NoDump` 分开：一个是"永远不会有"（MAME 说得很清楚），一个是
    /// "有但不能用"（真的坏文件）。用户拿到一份 baddump 的 rom 时该去重新
    /// 下载，而不是等着它自己变好——所以它不能显示成绿色。
    BadDump,
}

impl RomState {
    /// `available` 布尔 + `status` 字符串 → 展示状态。
    ///
    /// MAME 的 `status` 属性只有三个合法值（`-listxml` 的 DTD 写死了
    /// `(baddump|nodump|good)`），所以这里穷举而不是猜子串。
    ///
    /// `nodump` / `baddump` 的判定都在**前**：审计把 nodump 一律置成
    /// `available = true`（`core/audit.rs` 的重置循环），所以只看
    /// `available` 会把它并进 `Good`，而它其实压根不会被校验。
    pub fn of(rom: &RomInfo) -> Self {
        if rom.is_nodump() {
            return RomState::NoDump;
        }
        if rom.is_baddump() {
            return RomState::BadDump;
        }
        if rom.available {
            RomState::Good
        } else {
            RomState::Missing
        }
    }

    /// 这个状态算不算「这套文件齐了」。
    ///
    /// nodump 与 baddump 都算**有**（文件在盘上，只是 MAME 对它的评价不好），
    /// 缺失和未审计才算不齐。BIOS 段 / 设备段 / 样本段的整体判定都走这里，
    /// 免得三处各写一遍 `all()` 而在某处忘了排除 baddump。
    pub fn counts_as_present(&self) -> bool {
        matches!(self, RomState::Good | RomState::NoDump | RomState::BadDump)
    }
}

/// 一行 Rom 文件。
#[derive(Debug, Clone)]
pub struct RomRow {
    /// 归档内的条目名（`RomInfo::name`），如 `pgm_p02s.u20`。
    pub name: String,
    /// 实际应该找的文件名：`merge` 非空时是它，否则就是 `name`。
    ///
    /// 展示用 `name`（用户在 MAME 文档里认的是它），但"到底找哪个文件"
    /// 是 `effective_name` 的事——一个 `merge` 出去的条目在自己的包里叫
    /// `A1200.11`，在 `pgm` 包里叫 `A1200.11` 却由 `pgm_a1200.11` 提供。
    pub file_name: String,
    /// 该行来自哪个机种：本体是 `None`，依赖项是父集 / BIOS / 设备的名字。
    pub from: Option<String>,
    /// 设备引用带 tag（`igs023:sprcol`），普通条目为 `None`。
    pub tag: Option<String>,
    pub crc: u32,
    pub size: u64,
    pub region: String,
    pub merge: String,
    pub state: RomState,
}

/// 一行 CHD。
#[derive(Debug, Clone)]
pub struct DiskRow {
    pub name: String,
    /// `.chd` 后缀是 MAME 的约定，参考样式里也是 `xxx.chd`，这里补上。
    pub file_name: String,
    pub from: Option<String>,
    pub sha1: String,
    pub region: String,
    pub index: u8,
    pub state: RomState,
}

impl RomState {
    /// CHD 走同一套状态判定，只是入参不同（`DiskInfo` 没有 `RomInfo`
    /// 那套 `nodump` 之外的字段）。抽出来免得两处各写一遍 `available`。
    fn of_disk(disk: &crate::core::model::DiskInfo) -> Self {
        if disk.is_nodump() {
            return RomState::NoDump;
        }
        if disk.is_baddump() {
            return RomState::BadDump;
        }
        if disk.available {
            RomState::Good
        } else {
            RomState::Missing
        }
    }
}

/// 一条 BIOS 记录：`v2` → `PGM BIOS V2`。
#[derive(Debug, Clone)]
pub struct BiosRow {
    /// 该 BIOS 集的名字（`GameMeta::name`，如 `v2`）。
    pub name: String,
    /// 人可读描述（`GameMeta::description`，如 `PGM BIOS V2`）。
    pub description: String,
    /// 是否是游戏默认使用的那套（`bios_sets[].is_default`）。
    pub is_default: bool,
    /// 该 BIOS 集自身的可用性：它自己的 roms 是否齐全。
    ///
    /// 从**这个 BIOS 自己的 `GameMeta`** 取，而不是从引用它的游戏取——一个
    /// BIOS 集在库里是独立一条记录（`is_bios`），审计已经单独标记过它。
    pub state: RomState,
    /// 这一套底下的**实际 rom 文件**。
    ///
    /// 之前 BIOS 段只有"集名 + 描述"一行，用户看不到 BIOS 到底要哪些文件，
    /// 而这些文件又因为带 `bios=` 属性被排除在 Rom 段之外——等于凭空
    /// 消失。BIOS 集不是独立 machine（见 [`default_bios_names`]），它的
    /// 文件就躺在这台机种自己的 `roms` 里，所以从这里筛出来。
    pub roms: Vec<RomRow>,
}

/// 一行"引用设备"（`<device_ref name="..." tag="..."/>`，一引用一行）。
///
/// 参考样式里这一段**不去重**：同一设备被多个槽引用就出现多行
/// （`pgm2_memcard` 四个槽四行），顺序与引用顺序一致。状态取设备机种
/// 自己的 roms——无 rom 的纯设备（screen / palette / nvram…）直接算
/// "全部获得"。
#[derive(Debug, Clone)]
pub struct DeviceRow {
    /// 设备机种名（`igs036` / `timer` / `pgm2_memcard`）。
    pub name: String,
    /// 设备机种自己的描述（`IGS036` / `Z80 CPU`）。
    ///
    /// 从库里那条设备机种记录取（`is_device` 的独立 `GameMeta`），所以设备
    /// 不在库里时这里是空串——但那一行仍要显示，否则用户以为没引用设备。
    pub description: String,
    /// 引用它的设备标签（`maincpu` / `igs023:sprcol`）。
    pub tag: String,
    /// 该设备 rom 的整体状态：全齐 / 缺东西 / 没审计。
    pub state: RomState,
}

/// 一行"设备"（`<device>` 槽位：可挂载的设备实例）。
///
/// 参考样式三列：设备类型（`memcard`）/ 实例名（`memcard1`）/ 扩展名
/// （`pg2,bin,mem`，逗号连接）。
#[derive(Debug, Clone)]
pub struct DeviceSlotRow {
    /// 设备类型（`<device type="...">`）。
    pub kind: String,
    /// 实例名（`<instance name="...">`，命令行 `-<instance>` 用的就是它）。
    pub instance: String,
    /// 安装路径（`<device tag="...">`，如 `upd765:0:525hd`）。
    ///
    /// **去掉前导冒号**——MAME 在属性里写成`:upd765:0:525hd`，那个冒号是
    /// "机种内引用"的意思，命令行里没有，用户也不该看到。
    pub tag: String,
    /// 扩展名，逗号连接（`pg2,bin,mem`）。
    pub extensions: String,
}

/// `<slot>` 的一行：槽位名 + 可选设备数。
///
/// 与 `DeviceSlotRow` 的分工：那条是 `<device>`（本机自带的槽位设备），
/// 这条是 `<slot>`（MAME 的槽位声明 + `<slotoption>` 可选设备列表）。
/// 两者在同一台机器上并存 —— 实测 `nes` 有 9 个 `<device>` 与 12 个 `<slot>`。
#[derive(Debug, Clone)]
pub struct SlotRow {
    /// 槽位名（`ctrl1`、`nes_slot`…）。
    pub name: String,
    /// 可选设备数（`<slotoption>` 条数）。
    pub option_count: usize,
    /// 逗号连接的选项名，空槽位则为空串。
    pub options: String,
}

/// 一段样本音频（`sampleof` 指向的那个机种）。
#[derive(Debug, Clone)]
pub struct SampleRow {
    /// 样本机种名。
    pub name: String,
    /// 缺失的样本文件数 / 总数。
    pub have: usize,
    pub total: usize,
    pub state: RomState,
}

/// Rom 信息面板要显示的全部内容。
///
/// 一次性算完、整份交给 UI：dock 是在**每帧**里被调用的（egui 的
/// display-refresh-rate 渲染），在里面现查库会是 5 万台游戏的一次线性扫描
/// ×60 次/秒。UI 侧按 `(game, audited)` 缓存这个结果。
#[derive(Debug, Clone, Default)]
pub struct RomInfoView {
    /// 被查看的游戏名，面板标题用。
    pub game: String,
    /// 描述（面板顶部），有本地化优先用本地化（`lc_desc`）。
    pub description: String,

    /// 本机种的 rom 条目。
    pub roms: Vec<RomRow>,
    /// 本机种的 CHD。
    pub disks: Vec<DiskRow>,
    /// 依赖的 BIOS 集。
    pub bios: Vec<BiosRow>,
    /// 引用设备（`<device_ref>`，一引用一行、不去重）。
    pub devices: Vec<DeviceRow>,
    /// 被引用设备的 rom 明细。
    ///
    /// 单独于 `devices` 是因为版式：段头列设备机种名，段内的行是**设备自己
    /// 的 rom 文件**（`mc68000.bin` / `igs023.rom`…），tag 跟在行尾
    /// （`igs023:sprcol`）。两段在 UI 上拼起来读才顺，拆成两个数组比在
    /// 渲染时重新遍历设备机种更省事。
    pub device_roms: Vec<RomRow>,
    /// 可挂载槽位（`<device>`：类型 / 实例 / 扩展名）。
    pub slots: Vec<DeviceSlotRow>,
    /// MAME 槽位（`<slot>`：槽位名 / 可选设备数）。
    pub slot_decls: Vec<SlotRow>,
    /// 样本音频。
    pub samples: Vec<SampleRow>,
    /// 依赖的主 ROM 文件（`romof` 父集 / 祖父集）里那些**本机种没有**的
    /// 条目。父集自己的 roms 已经在审计时回填进本机种的 `available` 了，
    /// 但**条目本身**不会出现在本机种的 `roms` 里——参考样式要看到
    /// "这个文件其实来自 pgm（父集）"，就得把父集的条目也列出来。
    ///
    /// 私有：它在构建结束前就被并进 `roms`（`from` 字段标出��源）。对外只
    /// 暴露一个合并后的列表，因为"这盘游戏要哪些文件"对用户是**一个**问题，
    /// 拆成"本体 / 继承"两个列表只会让调用方自己再拼一次。
    inherited: Vec<RomRow>,

    /// 全库是否审计过。为 false 时所有缺失都只是"未知"。
    pub audited: bool,
}

impl Default for RomState {
    fn default() -> Self {
        RomState::Unknown
    }
}

impl RomInfoView {
    /// 本机种 roms（含从父集继承的）里缺失的条数。
    ///
    /// 面板标题旁的徽标要这个数。继承来的条目也算进去：那些文件同样是这盘
    /// 游戏要跑的，缺了照样起不来。
    pub fn missing_count(&self) -> usize {
        self.roms.iter().filter(|r| r.state == RomState::Missing).count()
    }

    /// 面板是否该显示"去审计"的提示：没审计过，或者确实缺东西。
    pub fn needs_audit_hint(&self) -> bool {
        !self.audited || self.missing_count() > 0
    }

    /// 全空的判断：连一段内容都没有（既无 rom 也无 chd）。
    ///
    /// 旧版对空游戏是直接 `return` 的（`gamelist.cpp` 的
    /// `convertMameInfo`：`roms.isEmpty() && disks.isEmpty()`），面板该
    /// 保持空白而不是显示五个空标题。
    pub fn is_empty(&self) -> bool {
        self.roms.is_empty()
            && self.disks.is_empty()
            && self.bios.is_empty()
            && self.devices.is_empty()
            && self.slots.is_empty()
            && self.samples.is_empty()
            && self.inherited.is_empty()
    }
}

/// 该机种依赖的主 ROM 文件（父集）的名字，按近到远。
///
/// MAME 的 `romof` 是一条链：`a` romof `b`，`b` romof `c`。`a` 跑不起来时
/// 缺的文件可能来自 `b` 也可能来自 `c`，所以要一路走到底。
///
/// **防环**：`visited` 必须在**入栈前**判定，否则 `a → b → a` 这种坏数据
/// （或用户手改过的 dat）会在这里死循环把界面卡死。坏数据是外部输入，
/// 按不可信处理。
fn parent_chain<'a>(lib: &'a GameLibrary, game: &'a GameMeta) -> Vec<&'a GameMeta> {
    let mut out: Vec<&GameMeta> = Vec::new();
    let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
    visited.insert(game.name.as_str());
    let mut cursor = game.romof.as_str();
    // 链长上限：真实 dat 最深 3 层，这个上限只为兜住坏数据
    let mut hops = 0;
    while !cursor.is_empty() && hops < 8 {
        let Some(gi) = lib.get_idx(cursor) else { break };
        let p = &lib.games[gi];
        if !visited.insert(p.name.as_str()) {
            break;
        }
        out.push(p);
        cursor = p.romof.as_str();
        hops += 1;
    }
    out
}

/// 该机种默认使用的 BIOS 集的名字。
///
/// **BIOS 集不是独立的 machine。** 查真实的 `-listxml` 输出（`mame pgm
/// -listxml`）：BIOS 集是同一个 `<machine name="pgm" isbios="yes">` 上的
/// `<biosset name="v2" description="PGM BIOS V2"/>` 标签，而它的 rom **混在
/// 同一个 `<rom>` 列表里**，用 `bios="v2"` 属性区分：
///
/// ```xml
/// <biosset name="v2" description="PGM BIOS V2"/>
/// <rom name="pgm_p02s.u20" bios="v2" crc="78c15fa2" region="maincpu"/>
/// ```
///
/// 所以"BIOS 段"的行是从**本机种的 roms 里按 `bios` 属性筛出来的**，不是
/// 去库里查另一条 machine 记录（`lib.get_idx("v2")` 永远是 `None`）。这个
/// 假设错起来很安静：段位是空的，但不报错。
fn default_bios_names(game: &GameMeta) -> Vec<String> {
    if game.bios_sets.is_empty() {
        return Vec::new();
    }
    let def = game.bios_sets.iter().find(|b| b.is_default);
    match def {
        Some(b) => vec![b.name.clone()],
        None => game
            .bios_sets
            .first()
            .map(|b| vec![b.name.clone()])
            .unwrap_or_default(),
    }
}

/// 把 `game.devices` 拆成"引用设备"与"可挂载槽位"两摊。
///
/// 这个列表里混着两种来源的条目，靠 `DeviceInfo::is_ref` 区分：
///
/// * `<device_ref name="igs036" tag=":maincpu"/>` —— "引用设备"段的行
///   （一引用一行，**按机种名去重**：实测 `kov3` 的 `palette` 被 `sp_palette`/
///   `tx_palette`/`bg_palette` 引用 3 次，不去重会刷出三行一样的）。
/// * `<device type="memcard" tag="memcard_p1"><instance name="memcard1">`
///   —— "设备"段的槽位行。
///
/// **别再用 `kind == instance` 判别**：解析器给 `device_ref` 同时填了
/// `kind` 和 `instance`，而 `<device>` 的 `instance` 名字常常**恰好**等于
/// `type`（`nes` 的 9 个 `<device>` 全是 `type="cartridge"` +
/// `instance name="cartridge"`），判别式会把这些槽位全当成引用设备 ——
/// 实测结果：引用设备段混进假设备，设备段全空。
fn split_devices(
    g: &GameMeta,
) -> (
    Vec<&crate::core::model::DeviceInfo>,
    Vec<&crate::core::model::DeviceInfo>,
) {
    let mut refs = Vec::new();
    let mut slots = Vec::new();
    for d in &g.devices {
        if d.is_ref {
            refs.push(d);
        } else {
            slots.push(d);
        }
    }
    (refs, slots)
}

/// 引用设备行里要显示的**设备机种名**。
///
/// `device_ref` 有 `name` 与 `instance` 两个属性，`split_devices` 判定
/// "引用"用的就是 `kind == instance`；这里取 `kind`（它才是设备机种名），
/// 兜底才用 `instance`，再兜底用 tag 的第一段。
fn device_name_of(d: &crate::core::model::DeviceInfo) -> String {
    if !d.kind.is_empty() {
        d.kind.clone()
    } else if !d.instance.is_empty() {
        d.instance.clone()
    } else {
        d.tag.split(':').next().unwrap_or(&d.tag).trim().to_string()
    }
}

/// 引用设备行的状态：设备机种自己的 roms 全齐（或本来就没有 rom）即
/// "全部获得"。
fn device_state(dev: Option<&GameMeta>, audited: bool) -> RomState {
    let Some(d) = dev else {
        // 设备机种不在库里（裁剪过的 dat）——仍要列出来，标成未知
        return RomState::Unknown;
    };
    if !audited {
        RomState::Unknown
    } else if d.roms.is_empty() || d.roms.iter().all(|r| RomState::of(r).counts_as_present()) {
        // 没有 rom 的设备（纯外部设备）不算缺失
        RomState::Good
    } else {
        RomState::Missing
    }
}

/// 引用设备段里，**设备自己那些 rom 文件**的行。
///
/// 段头列设备机种名（`m68000` / `igs036`），段内的行是设备机的真实 rom
/// （`mc68000.bin`…）——用户要看到"这台游戏还要哪些设备的文件"，光有设备名
/// 是不够的。
///
/// 按设备机种名分组：每行 `from` 放设备机种名，UI 拿它 match 回段头。
fn device_rom_rows(
    lib: &GameLibrary,
    names: &[(String, String)],
    audited: bool,
) -> Vec<RomRow> {
    let mut out: Vec<RomRow> = Vec::new();
    for (dev_name, tag) in names {
        let Some(gi) = lib.get_idx(dev_name) else {
            continue;
        };
        for r in &lib.games[gi].roms {
            let mut row = row_of(r, Some(dev_name.clone()), Some(tag.clone()));
            if !audited {
                row.state = RomState::Unknown;
            }
            out.push(row);
        }
    }
    out
}

fn row_of(rom: &RomInfo, from: Option<String>, tag: Option<String>) -> RomRow {
    RomRow {
        name: rom.name.clone(),
        file_name: rom.effective_name().to_string(),
        from,
        tag,
        crc: rom.crc,
        size: rom.size,
        region: rom.region.clone(),
        merge: rom.merge.clone(),
        state: RomState::of(rom),
    }
}

/// 算出一台机种的完整 Rom 信息视图。
///
/// `audited` 是**全库**是否审计过（`gamelist.cache` 的 `audited` 标志，
/// 挂在 `cache::CacheData` 上而不在 `GameLibrary` 上，所以由调用方传进来）。
/// 为 false 时把每个缺失都降级成 [`RomState::Unknown`]：冷启动后审计还没跑，
/// 此时 `available` 全是默认值，一律显示"缺失"会让用户以为自己的盘是空的。
pub fn view_of(lib: &GameLibrary, game: &str, audited: bool) -> RomInfoView {
    let mut view = RomInfoView {
        game: game.to_string(),
        audited,
        ..Default::default()
    };
    let Some(gi) = lib.get_idx(game) else { return view };
    let g = &lib.games[gi];
    view.description = if g.lc_desc.is_empty() {
        g.description.clone()
    } else {
        g.lc_desc.clone()
    };

    // 1) 本机种 CHD
    for d in &g.disks {
        let mut state = RomState::of_disk(d);
        if !audited {
            state = RomState::Unknown;
        }
        view.disks.push(DiskRow {
            name: d.name.clone(),
            file_name: format!("{}.chd", d.name),
            from: None,
            sha1: d.sha1.clone(),
            region: d.region.clone(),
            index: d.index,
            state,
        });
    }

    // 2) 依赖的主 ROM 文件：父集/祖父集里本机种**没有**的条目。
    //    审计已经把父集里"有"的回填进本机种的 `available` 了（romof 回填
    //    那一段），所以这里只补"条目本身"——否则一个克隆集在面板里会
    //    看不到自己其实依赖了父集的文件。
    //
    //    `own_crcs` 从**本体 rom**（`bios` 为空）起算：带 `bios=` 的条目归
    //    BIOS 段，拿它们当"已拥有"会让父集里同名的 BIOS 条目被当成继承项，
    //    在 Rom 段里冒出一行重复的 BIOS 文件。
    let mut own_crcs: std::collections::HashSet<u32> = g
        .roms
        .iter()
        .filter(|r| r.bios.is_empty())
        .map(|r| r.crc)
        .collect();
    for parent in parent_chain(lib, g) {
        if parent.is_bios {
            // BIOS 集由下面的 Bios 段负责，不混进 Rom 段
            continue;
        }
        for r in &parent.roms {
            if !r.bios.is_empty() {
                continue;
            }
            if !own_crcs.insert(r.crc) {
                continue;
            }
            let mut row = row_of(r, Some(parent.name.clone()), None);
            if !audited {
                row.state = RomState::Unknown;
            }
            view.inherited.push(row);
        }
    }

    // 4) BIOS。BIOS 集不是独立 machine（见 `default_bios_names` 的注释），
    //    它的 rom 就躺在这台机种自己的 `roms` 里、用 `bios="v2"` 标着。
    //    `bios_sets` 只提供"哪一套是默认 + 它的描述"。
    let bios_names = default_bios_names(g);
    for bname in &bios_names {
        let Some(desc) = g.bios_sets.iter().find(|b| &b.name == bname) else {
            continue;
        };
        // 这一套底下的 rom：该套要的文件**全部**齐了才算好。
        // 注意是"该套的"，不是"全部 rom"——一台机器的 rom 列表里同时躺着
        // 三套 BIOS 的文件，只看其中一套。
        let members: Vec<&RomInfo> = g.roms.iter().filter(|r| &r.bios == bname).collect();
        let state = if !audited {
            RomState::Unknown
        } else if members.is_empty() {
            // 声明了这一套却没有属于它的 rom（裁剪过的 dat）
            RomState::Unknown
        } else if members.iter().all(|r| RomState::of(r).counts_as_present()) {
            RomState::Good
        } else {
            RomState::Missing
        };
        view.bios.push(BiosRow {
            name: bname.clone(),
            description: desc.description.clone(),
            is_default: true,
            state,
            // 这一套实际要的文件。列出来是为了"BIOS 段和 CHD / Samples /
            // 设备一个待遇"——用户能在同一个版式里看到每一段的明细，
            // 而不是只看到一个集名就猜它要什么。
            roms: members
                .iter()
                .map(|r| {
                    let mut row = row_of(r, Some(g.name.clone()), Some(bname.clone()));
                    if !audited {
                        row.state = RomState::Unknown;
                    }
                    row
                })
                .collect(),
        });
    }
    // BIOS 的文件本身不该在 Rom 段里重复出现：`bios="v2"` 的条目归 BIOS 段。
    // 本体 rom 是 `bios` 属性为空的那些，重排一次把 inherited 接在后面。
    view.roms = {
        let mut own: Vec<RomRow> = g
            .roms
            .iter()
            .filter(|r| r.bios.is_empty())
            .map(|r| {
                let mut row = row_of(r, None, None);
                if !audited {
                    row.state = RomState::Unknown;
                }
                row
            })
            .collect();
        own.append(&mut view.inherited);
        own
    };

    // 5) 引用设备 + 设备槽位。两种来源见 `split_devices` 的注释。
    let (dev_refs, dev_slots) = split_devices(g);
    // 设备机种名去重（`palette` 在 kov3 里被引用三次），设备 rom 明细按它分组
    let mut dev_names: Vec<(String, String)> = Vec::new();
    let mut seen_dev: std::collections::HashSet<String> = std::collections::HashSet::new();
    for d in &dev_refs {
        let dev_name = device_name_of(d);
        if dev_name.is_empty() || !seen_dev.insert(dev_name.clone()) {
            continue;
        }
        let state = device_state(lib.get(&dev_name), audited);
        view.devices.push(DeviceRow {
            name: dev_name.clone(),
            description: lib
                .get(&dev_name)
                .map(|g| g.description.clone())
                .unwrap_or_default(),
            tag: d.tag.clone(),
            state,
        });
        dev_names.push((dev_name, d.tag.clone()));
    }
    for d in dev_slots {
        view.slots.push(DeviceSlotRow {
            kind: d.kind.clone(),
            instance: d.instance.clone(),
            tag: d.tag.trim_start_matches(':').to_string(),
            extensions: d.extensions.join(","),
        });
    }
    // `<slot>` 是独立于 `<device>` 的另一种表达，只在真有槽位声明时出行。
    for s in &g.slots {
        view.slot_decls.push(SlotRow {
            name: s.name.clone(),
            option_count: s.options.len(),
            options: s
                .options
                .iter()
                .map(|o| o.name.clone())
                .collect::<Vec<_>>()
                .join(","),
        });
    }
    view.device_roms = device_rom_rows(lib, &dev_names, audited);

    // 6) 样本。**这里原来靠 `lib.get_idx(&g.sampleof)` 反查"样本集机种"** ——
    // 而 MAME 根本不把样本集输出成 `<machine>`（全量 listxml 里 `genpin`
    // 出现 0 次），所以 1574/1898 恒None，Samples 段对绝大多数游戏永远空。
    // 现在改用 `core::samples`：拿本机`<sample>` 名去 `samplepath` 的
    // `{sampleof}.zip` 里比对条目名。
    //
    // `sample_dirs()` 返回 `Vec`（内部是 `RwLock`，不能借出 `&'static`），
    // 先绑到局部再借引用，别把临时值的引用传下去。
    let sdirs = crate::core::samples::sample_dirs();
    if let Some(row) = crate::core::samples::audit_game_sample(g, &sdirs, audited) {
        view.samples.push(row);
    }

    view
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{BiosSet, DeviceInfo, DiskInfo, GameMeta};

    fn meta(name: &str) -> GameMeta {
        GameMeta {
            name: name.into(),
            description: format!("{name} desc"),
            ..Default::default()
        }
    }

    fn rom(name: &str, crc: u32, region: &str) -> RomInfo {
        RomInfo {
            name: name.into(),
            crc,
            region: region.into(),
            size: 1024,
            ..Default::default()
        }
    }

    fn lib_with(games: Vec<GameMeta>) -> GameLibrary {
        let mut lib = GameLibrary::new("test".into());
        lib.games = games;
        lib.rebuild_indexes();
        lib
    }

    /// 状态判定：`nodump` 必须排在 `available` 前面。审计把 nodump 一律置成
    /// `available = true`，只按 `available` 判会把它并进 `Good`。
    #[test]
    fn nodump_outranks_available() {
        let mut r = rom("a.rom", 1, "maincpu");
        r.status = "nodump".into();
        r.available = true;
        assert_eq!(RomState::of(&r), RomState::NoDump);

        r.available = false;
        assert_eq!(RomState::of(&r), RomState::NoDump, "nodump 不看 available");

        r.status = String::new();
        assert_eq!(RomState::of(&r), RomState::Missing);
        r.available = true;
        assert_eq!(RomState::of(&r), RomState::Good);
    }

    /// `baddump` 必须单独成一个状态，不能被并进 `Good`。
    ///
    /// MAME 的 `status` 只有 `good|baddump|nodump` 三值（`-listxml` 的 DTD
    /// 写死的）。baddump 的文件**在盘上但内容是坏的**——审计会把它算成
    /// `available`，所以如果只按 `available` 判，它会显示成绿色对勾，用户
    /// 以为没问题，而实际跑起来是花的。必须显示成黄色「坏 dump」。
    #[test]
    fn a_baddump_is_not_reported_as_owned() {
        let mut r = rom("bad.rom", 1, "maincpu");
        r.status = "baddump".into();
        // 审计把 baddump 算成"文件在"（available = true）
        r.available = true;
        assert_eq!(
            RomState::of(&r),
            RomState::BadDump,
            "baddump 哪怕 available 也不能显示成拥有"
        );
        assert_ne!(
            RomState::of(&r),
            RomState::Good,
            "并进 Good 会让用户以为坏文件能用"
        );
        // 但它**算文件在**——整体判定（BIOS 段 / 设备段）不能因此报缺失
        assert!(
            RomState::BadDump.counts_as_present(),
            "baddump 的文件在盘上，整体判定要算齐"
        );
        assert!(!RomState::Missing.counts_as_present());
        assert!(!RomState::Unknown.counts_as_present());
    }

    /// 没审计过时，缺失必须降级成 Unknown——否则冷启动后一屏红色。
    #[test]
    fn unaudited_library_reports_unknown_not_missing() {
        let mut g = meta("pacman");
        let mut r = rom("pacman.6e", 0xaaa, "maincpu");
        r.available = false;
        g.roms.push(r);
        let lib = lib_with(vec![g]);

        let v = view_of(&lib, "pacman", false);
        assert!(!v.audited);
        assert_eq!(v.roms[0].state, RomState::Unknown);
        assert_eq!(v.missing_count(), 0, "没审计不该报缺失");
        assert!(v.needs_audit_hint(), "但要提示去审计");
    }

    /// 依赖的父集条目要能列出来，且不与本机种重复。
    #[test]
    fn parent_roms_appear_as_inherited() {
        let mut child = meta("puckman");
        child.romof = "pacman".into();
        child.roms.push({
            let mut r = rom("puckman.6e", 0xbbb, "maincpu");
            r.available = true;
            r
        });
        let mut parent = meta("pacman");
        // 父集里有一条子集也声明了的（crc 相同）——必须去重
        parent.roms.push(rom("pacman.6e", 0xbbb, "maincpu"));
        parent.roms.push({
            let mut r = rom("pacman.6f", 0xccc, "maincpu");
            r.available = true;
            r
        });

        let lib = lib_with(vec![child, parent]);
        let v = view_of(&lib, "puckman", true);

        // 继承来的条目被**并进** Rom 段（`from` 标出来源），因为对用户来说
        // "这盘游戏要哪些文件" 是一个列表，不是两个。父集那条同 crc 的必须
        // 去重，不能出现两行 `pacman.6e`。
        assert_eq!(v.roms.len(), 2, "本体一条 + 父集独有的一条");
        assert_eq!(v.roms[0].name, "puckman.6e");
        assert_eq!(v.roms[0].from, None, "本体条目没有来源标记");
        assert_eq!(v.roms[1].name, "pacman.6f");
        assert_eq!(v.roms[1].from.as_deref(), Some("pacman"));
    }

    /// `romof` 成环时必须停下来，不能死循环。
    ///
    /// 坏数据是外部输入：用户手改过的 dat 完全可能写出 `a → b → a`。
    #[test]
    fn a_cyclic_romof_chain_terminates() {
        let mut a = meta("a");
        a.romof = "b".into();
        a.roms.push(rom("a.rom", 1, "maincpu"));
        let mut b = meta("b");
        b.romof = "a".into();
        b.roms.push(rom("b.rom", 2, "maincpu"));
        let lib = lib_with(vec![a, b]);

        let v = view_of(&lib, "a", true);
        // a 的本体 + 从 b 继承来的 = 2 行。关键是**没有第三行**：环被
        // `parent_chain` 的 visited 挡住了，a 不会通过 b 又把自己列一遍。
        assert_eq!(v.roms.len(), 2, "环不能让 parent_chain 无限走");
        assert_eq!(v.roms[0].name, "a.rom");
        assert_eq!(v.roms[1].name, "b.rom");
        assert_eq!(v.roms[1].from.as_deref(), Some("b"));
    }

    /// 设备名从 `kind` 取（`device_ref` 的 `name` 属性），`tag` 是父机种里
    /// 的标签全名（`igs023:sprcol`）。拿 tag 去查库永远查不到。
    #[test]
    fn device_name_comes_from_the_kind_not_the_tag() {
        let mut g = meta("pgm");
        g.devices = vec![
            DeviceInfo {
                kind: "m68000".into(),
                instance: "m68000".into(),
                tag: "maincpu".into(),
                is_ref: true,
                ..Default::default()
            },
            DeviceInfo {
                kind: "igs023".into(),
                instance: "igs023".into(),
                tag: "igs023:sprcol".into(),
                is_ref: true,
                ..Default::default()
            },
            // 同一设备被引用两次（maincpu + aux）——只列一次
            DeviceInfo {
                kind: "m68000".into(),
                instance: "m68000".into(),
                tag: "aux".into(),
                is_ref: true,
                ..Default::default()
            },
        ];
        let mut dev = meta("m68000");
        dev.is_device = true;
        dev.roms.push({
            let mut r = rom("mc68000.bin", 0xddd, "maincpu");
            r.available = true;
            r
        });
        let lib = lib_with(vec![g, dev]);

        let v = view_of(&lib, "pgm", true);
        assert_eq!(v.devices.len(), 2, "去重后两个设备");
        assert_eq!(v.devices[0].name, "m68000");
        assert_eq!(v.devices[1].name, "igs023");
        assert_eq!(v.devices[1].tag, "igs023:sprcol", "tag 本身留着");
        assert_eq!(v.device_roms.len(), 1, "设备 rom 明细只一条");
        assert_eq!(v.device_roms[0].name, "mc68000.bin");
    }

    /// `<device>` 的 `instance` 名字**常常恰好等于** `type`，不能因此被当成
    /// 引用设备。
    ///
    /// 真实样本（`mame nes -listxml`）：
    /// ```xml
    /// <device type="cartridge" tag="nes_slot" mandatory="1">
    ///   <instance name="cartridge" briefname="cart"/>
    /// </device>
    /// ```
    /// 判别式若写成 `kind == instance`，这条会被归进引用设备段 —— 实测 `nes`
    /// 的 9 个 `<device>` **全部**命中，结果是引用设备段混进 cartridge /
    /// floppydisk / midiin 这些假设备，而真正的设备段永远是空的。
    #[test]
    fn a_device_whose_instance_matches_its_type_is_not_a_device_ref() {
        let mut g = meta("nes");
        g.devices = vec![
            // 真引用设备
            DeviceInfo {
                kind: "rp2a03g".into(),
                instance: "rp2a03g".into(),
                tag: "maincpu".into(),
                is_ref: true,
                ..Default::default()
            },
            // 可挂载槽位：type 与 instance 同名，正是判别式的坑
            DeviceInfo {
                kind: "cartridge".into(),
                instance: "cartridge".into(),
                tag: "nes_slot".into(),
                extensions: vec!["nes".into(), "unf".into()],
                ..Default::default()
            },
            DeviceInfo {
                kind: "floppydisk".into(),
                instance: "floppydisk".into(),
                tag: "floppy0".into(),
                extensions: vec!["fds".into()],
                ..Default::default()
            },
        ];
        let mut dev = meta("rp2a03g");
        dev.is_device = true;
        let lib = lib_with(vec![g, dev]);

        let v = view_of(&lib, "nes", true);
        assert_eq!(v.devices.len(), 1, "只有 rp2a03g 是引用设备");
        assert_eq!(v.devices[0].name, "rp2a03g");
        assert_eq!(v.slots.len(), 2, "两个 <device> 都要落到设备段");
        assert_eq!(v.slots[0].kind, "cartridge");
        assert_eq!(v.slots[0].extensions, "nes,unf");
        assert_eq!(v.slots[1].kind, "floppydisk");
    }

    /// `<slot>` / `<slotoption>` 要能进面板，空槽位也保留一行。
    #[test]
    fn slot_declarations_carry_their_options() {
        let mut g = meta("nes");
        g.slots = vec![
            crate::core::model::SlotInfo {
                name: "ctrl1".into(),
                options: vec![
                    crate::core::model::SlotOption {
                        name: "vboy".into(),
                        devname: "nes_vboyctrl".into(),
                        default: false,
                    },
                    crate::core::model::SlotOption {
                        name: "powerpad".into(),
                        devname: "nes_powerpad".into(),
                        default: true,
                    },
                ],
            },
            // 空槽位：`<slot name="nes_slot"/>`，自闭合
            crate::core::model::SlotInfo {
                name: "nes_slot".into(),
                options: Vec::new(),
            },
        ];
        let lib = lib_with(vec![g]);

        let v = view_of(&lib, "nes", true);
        assert_eq!(v.slot_decls.len(), 2);
        assert_eq!(v.slot_decls[0].name, "ctrl1");
        assert_eq!(v.slot_decls[0].option_count, 2);
        assert_eq!(v.slot_decls[0].options, "vboy,powerpad");
        assert_eq!(v.slot_decls[1].name, "nes_slot");
        assert_eq!(v.slot_decls[1].option_count, 0);
        assert!(v.slot_decls[1].options.is_empty());
    }

    /// 默认 BIOS 从 `bios_sets[].is_default` 取，缺标记时退回第一套。
    #[test]
    fn default_bios_comes_from_the_flag_else_the_first() {
        let mut g = meta("pgm");
        g.bios_sets = vec![
            BiosSet {
                name: "v1".into(),
                description: "PGM BIOS V1".into(),
                is_default: false,
            },
            BiosSet {
                name: "v2".into(),
                description: "PGM BIOS V2".into(),
                is_default: true,
            },
        ];
        let lib = lib_with(vec![g]);
        let v = view_of(&lib, "pgm", true);
        assert_eq!(v.bios.len(), 1);
        assert_eq!(v.bios[0].name, "v2", "认 is_default，不认列表顺序");

        let mut g2 = meta("pgm2");
        g2.bios_sets = vec![BiosSet {
            name: "only".into(),
            description: "Only".into(),
            is_default: false,
        }];
        let lib2 = lib_with(vec![g2]);
        assert_eq!(view_of(&lib2, "pgm2", true).bios[0].name, "only");
    }

    /// BIOS 集**不是独立 machine**：它的 rom 就在本机种的 `roms` 里、用
    /// `bios="v2"` 标着（真实 `-listxml` 的形状）。所以段内文件的成败看
    /// 那些条目的 `available`，而不是去库里查另一条记录。
    #[test]
    fn an_incomplete_bios_reads_as_missing() {
        let mut g = meta("pgm");
        g.bios_sets = vec![BiosSet {
            name: "v2".into(),
            description: "PGM BIOS V2".into(),
            is_default: true,
        }];
        g.roms.push({
            let mut r = rom("b.rom", 0x999, "maincpu");
            r.bios = "v2".into();
            r // available 默认 false
        });
        // 一条本体 rom，确认它没被算进 BIOS 段
        g.roms.push(rom("own.rom", 0x111, "maincpu"));
        let lib = lib_with(vec![g.clone()]);
        let v = view_of(&lib, "pgm", true);
        assert_eq!(v.bios[0].state, RomState::Missing);
        assert_eq!(v.bios[0].description, "PGM BIOS V2");
        assert_eq!(v.roms.len(), 1, "BIOS 的文件不进 Rom 段");
        assert_eq!(v.roms[0].name, "own.rom");

        // 补上就变好
        let mut g2 = g;
        g2.roms[0].available = true;
        let lib2 = lib_with(vec![g2]);
        assert_eq!(view_of(&lib2, "pgm", true).bios[0].state, RomState::Good);
    }

    /// CHD 文件名补 `.chd`，与参考样式一致。
    #[test]
    fn disk_rows_get_the_chd_suffix() {
        let mut g = meta("dgame");
        g.disks = vec![DiskInfo {
            name: "dgame".into(),
            sha1: "abc".into(),
            index: 0,
            ..Default::default()
        }];
        let lib = lib_with(vec![g]);
        let v = view_of(&lib, "dgame", true);
        assert_eq!(v.disks[0].file_name, "dgame.chd");
        assert_eq!(v.disks[0].state, RomState::Missing);
    }

    /// 样本行的 `拥有/总数` 来自「本机`<sample>` 名」vs「样本集包内条目名」。
    ///
    /// 这个测试原来假设"样本集是另一台机种，它的 `roms` 就是采样文件"——
    /// **那个假设是错的**（MAME 从不把样本集输出成 `<machine>`，`genpin` 在
    /// 全量 listxml 里出现 0 次）。真实数据源是 `GameMeta::samples`（本机
    /// 要的 `<sample>` 名）比对 `{sampleof}.zip` 的包内条目，所以测试改成
    /// 真造一个 zip 出来。`have` 的算法细节在 `core::samples` 里有更细的
    /// 测试（含缺文件、全齐、散目录三种）。
    #[test]
    fn sample_rows_report_have_over_total() {
        // 真造一个样本集包：本机要 a / b / zz，包里只有 a / b
        let dir = std::env::temp_dir().join("mvui_rominfo_samples");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        {
            use std::io::Write;
            let f = std::fs::File::create(dir.join("ssample.zip")).expect("建 zip");
            let mut zw = zip::ZipWriter::new(f);
            let opts: zip::write::FileOptions<'_, ()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for n in ["a.wav", "b.wav"] {
                zw.start_file(n, opts).expect("写条目");
                zw.write_all(b"x").expect("写内容");
            }
            zw.finish().expect("收尾");
        }
        crate::core::samples::set_sample_dirs(vec![dir.clone()]);

        let mut g = meta("game");
        g.sampleof = "ssample".into();
        g.samples = vec!["a".into(), "b".into(), "zz".into()];
        let lib = lib_with(vec![g]);
        let v = view_of(&lib, "game", true);
        assert_eq!(v.samples.len(), 1, "有 sampleof 与 samples 才出行");
        assert_eq!(v.samples[0].name, "ssample", "显示的是样本集名，不是本机名");
        assert_eq!(v.samples[0].have, 2, "a / b 在包里，zz 不在");
        assert_eq!(v.samples[0].total, 3);
        assert_eq!(v.samples[0].state, RomState::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 本地化描述优先。
    #[test]
    fn localized_description_wins() {
        let mut g = meta("pacman");
        g.lc_desc = "吃豆人".into();
        let lib = lib_with(vec![g]);
        assert_eq!(view_of(&lib, "pacman", true).description, "吃豆人");
    }

    /// 库里的游戏名不存在时返回空视图，不能panic。
    #[test]
    fn an_unknown_game_yields_an_empty_view() {
        let lib = lib_with(vec![meta("pacman")]);
        let v = view_of(&lib, "nope", true);
        assert!(v.is_empty());
        assert_eq!(v.game, "nope");
    }

    /// 本机种与父集都没东西时不该显示五个空段。
    #[test]
    fn a_bare_game_reads_as_empty() {
        let lib = lib_with(vec![meta("bare")]);
        assert!(view_of(&lib, "bare", true).is_empty());
    }
}
