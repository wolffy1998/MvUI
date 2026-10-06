//! Rom 信息的表格渲染，被两处复用：
//!
//! * **dock 面板**（View ▸ 自定义信息栏 ▸ RomInfo）——数据来自校验缓存，
//!   跟着游戏选择刷新。
//! * **校验结果弹窗**（右键/菜单「校验 ROM」跑完）——数据来自现场重审。
//!
//! 两者的**版式完全一样**，只有数据来源和顶部那一行说明不同。所以渲染只写
//! 一份：谁拿到 [`RomInfoView`] 谁调用。抄两遍的话，改一次样式要记得改两处，
//! 而漏掉的那处会让用户看到"面板和弹窗长得不一样"。
//!
//! 版式照用户给的参考样式：段头（`Rom:` / `Bios:` / `引用设备:`）+ 缩进的行，
//! Rom 段一行四列（名称 / 状态 / CRC / 区域），缺失的**整行**标红。

use crate::app::MameApp;
use crate::core::rominfo::{RomInfoView, RomState, RomRow};
use crate::icons;

/// 一行里的状态配色。
///
/// 用户指定的四态配色（2026-10-06 修订）：
///
/// | 状态 | 颜色 | 为什么 |
/// |---|---|---|
/// | 拥有 | 绿 | 校验通过 |
/// | 坏 dump | 黄 | 文件在但是坏的——比"好"差，比"没有"好 |
/// | 未 dump | **黄** | MAME 说这个文件本来就不会存在，**不是缺失** |
/// | 缺失 | **红** | 确认缺失。**只有这一种是红色** |
///
/// **只有"缺失"用红色。** 之前 `nodump` 走灰色，理由是"它不是坏消息"；
/// 但灰色和"未校验"撞在一起，用户看到灰就以为是没查。改黄之后语义分开了：
/// 黄 = 有问题但不是缺文件（坏 dump / 无 dump），红 = 确实缺，灰 = 还没查。
///
/// `nodump` 用黄而不是红：它是 dat 自己声明的"此文件不存在"，报红会让用户
/// 以为自己的盘有缺口，白花时间去补一个永远不会有的文件。
fn state_color(state: RomState) -> egui::Color32 {
    match state {
        RomState::Good => icons::GREEN,
        RomState::BadDump | RomState::NoDump => icons::YELLOW,
        RomState::Missing => icons::RED,
        RomState::Unknown => ui_weak_color(),
    }
}

/// 第三列的状态图标（16×16）。
///
/// 用 1.8.2 那套 `assets/icons/16x16/status_*.png`——它们本来就是这个用途
/// （绿勾 `status_good` / 黄叹 `status_imperfect` / 蓝问 `status_preliminary`
/// / 蓝叉 `status_cross`）。自己画一套对勾叉号只会和它们不一致，也不用维护
/// 两份矢量资源。
///
/// `baddump` 那张是本项目新画的（`status_baddump.png`）：它与"未校验"语义
/// 完全不同——一个是"文件在但是坏的，用户该去重下"，另一个是"还没查"——
/// 共用一个图标会让用户以为 baddump 是个可以忽略的提示。
///
/// **`nodump` 复用 `status_baddump.png`（黄叹号）** —— 与它同色同义：
/// 都是"有问题、但不是缺文件"（用户要的就是这个：黄色 + 感叹号）。
/// 原来给它 `status_preliminary`（蓝问号），可那一格的颜色是**画在图标里**
/// 的，跟黄字并排会显得是两件事。
///
/// **图标不做染色**：它们已经是彩色的（绿勾 / 黄叹 / 蓝叉 / 橙叹），再乘一层
/// 状态色会把语义搅糊。名字找不到时返回 `None`，调用方退回文字符号。
fn state_icon(state: RomState) -> Option<&'static str> {
    Some(match state {
        RomState::Good => "16x16/status_good.png",
        RomState::BadDump | RomState::NoDump => "16x16/status_baddump.png",
        RomState::Missing => "16x16/status_missing.png",
        RomState::Unknown => "16x16/status_imperfect.png",
    })
}

/// "未校验"用的灰。
///
/// 取 `visuals().weak_text_color` 的话这里就得带 `ui`，而配色函数是纯的
/// （好测、好在任何面板/弹窗里复用）。用 `Color32::from_gray(140)`：两个
/// 主题下都读得清，且不依赖调用点。
fn ui_weak_color() -> egui::Color32 {
    egui::Color32::from_gray(140)
}

/// 段头：`Rom:` / `Bios:` / `引用设备:`。
///
/// **不画 `separator()`** —— 面板窄，横向分隔线会把本来就窄的一行截成
/// 两半，段头下面直接接行更紧凑。用户明确要求去掉这条线。
/// 段头：`Rom:` / `Disks:` / `Bios:` / `引用设备:`……
///
/// **段头下面画一条横线**（之前删过，用户要加回来——没有它，段与段之间
/// 只有 3px 的间距，一眼看不出哪里是一段的开始）。
/// **段与段之间空一行**：`SECTION_GAP` 是"一行"的高度，跟在段头**上面**，
/// 于是每个新段都先空一行再写标题 —— 视觉上段与段就分开了。
fn section(ui: &mut egui::Ui, app: &MameApp, label: &str) {
    // 顶部那行说明已经去掉了（见 `render`），所以这里只画标题和横线。
    // 不写死字号：信息栏字体（override_font_id）决定大小，这里只管加粗。
    ui.label(egui::RichText::new(app.tr(label)).strong());
    ui.separator();
}

/// 状态词的 i18n key。
///
/// 返回 key 而不是文案：文案要走 `app.tr` 现查，语言切换后立刻跟着变。
///
/// 四个词与四态一一对应：`拥有` / `坏 dump` / `缺失` / `未 dump`（外加
/// 未校验时的 `未校验`）。词的沿革：旧版抄 MAME verify 报告的"很好 /
/// 缺失"，2026-06 改成"拥有 / 坏 dump / 未拥有"（描述**用户手上有没有**
/// 而不是校验结果），2026-10-07 用户把"未拥有"定稿为"缺失"——两个字、
/// 与"拥有"对仗，且与 MAME 自己的用词一致。
fn state_word(state: RomState) -> &'static str {
    match state {
        RomState::Good => "owned",
        RomState::BadDump => "bad dump",
        RomState::Missing => "not owned",
        RomState::NoDump => "not dumped",
        RomState::Unknown => "not verified",
    }
}

/// 图标缺失时的**文字退路**：✓ / ✗ / ! / ?
///
/// 正常情况下这一列画的是 16×16 图标，这些字符只在纹理取不到时出现（图标
/// 文件被误删、或 build.rs 没扫到）。所以它们必须与 `state_icon` 一一对应，
/// 否则会出现"显示 ✗ 但图标是绿勾"这种自相矛盾的画面。
fn mark_of(state: RomState) -> &'static str {
    match state {
        RomState::Good => "\u{2713}",
        RomState::BadDump | RomState::NoDump => "!",
        RomState::Missing => "\u{2717}",
        RomState::Unknown => "?",
    }
}

/// CRC 显示成 `crc(78c15fa2)`，与参考样式一致。
///
/// 没有 crc（`crc="0"` 的条目、或裁剪过的 dat）时显示 `-` 而不是
/// `crc(00000000)`：后者看着像一个真的校验值，而它不是。
fn crc_text(crc: u32) -> String {
    if crc == 0 {
        "-".to_string()
    } else {
        format!("crc({crc:08x})")
    }
}

/// 各列的宽度常量 —— 栅格是**全局的，所有段共用同一套列顺序**。
///
/// `ui.horizontal` + 自然宽度会让每一列的起点随上一行的内容长度飘——
/// 文件名有长有短，于是「缺失」这个词有的在这行第 20 个字符，有的在第
/// 12 个，整列读起来参差不齐。定宽是唯一能对齐的办法。
///
/// **关键：栅格必须是全局的，不能每段各算各的。**
/// 曾经每段自己写 `(avail - 各自用到的列宽之和)`：Rom 段后面挂 5 列，
/// CHD 段只挂 3 列，于是 CHD 段的名称列比 Rom 段宽 216px，状态词的起点
/// **差了一整段距离**。现在所有段都按同一份 [`Grid`] 排。
mod cols {
    /// **列与列之间的间隙 —— 全表只有这一个间隙值**（用户 2026-10-06 定）。
    ///
    /// 之前每一列各有各的余量：名称列定宽 300px（而这一屏的文件名只有
    /// 10 来个字符，于是名字后面空出 200 多px），状态列 60px（而「拥有 +
    /// 图标」只占 44px，图标后面又空 16px）。于是同一条横线上出现了三种
    /// 宽度完全不同的空档，读起来像三套排版。
    ///
    /// 用户原话：「拥有状态隔文件名这么远……crc 和对勾 x 等间距这么远，
    /// 这几个间距应该一样。」所以间隙从"每列各自剩下多少"改成"**一律
    /// 12px**"：每列的宽度由它自己的内容决定，列与列之间只隔着这一个值。
    pub const COL_GAP: f32 = 12.0;

    /// 状态图标边长（1.8.2 那套 `status_*.png` 是 16×16）。
    pub const ICON_W: f32 = 16.0;
    /// 图标与状态词之间的空隙（用户要的"1 个空格"）。
    ///
    /// 4px ≈ 一个空格符。**不占独立列**：图标和词在同一格里由
    /// [`state_with_icon`] 一次画完，所以这个间隙不受 `allocate_space`
    /// 在 `LeftToRight` 里按 `min_rect` 重算的影响（那个坑踩过一次：
    /// 列宽 22 被收成 16，间隙 0px，图标紧贴文字）。
    pub const ICON_GAP: f32 = 4.0;

    /// 段与段之间的空行高度（约一行文字）。
    pub const SECTION_GAP: f32 = 18.0;

    // ---- 各列的上下限（宽度由内容决定，这里只管夹住）----
    //
    // 上限的作用是**兜住极端内容**，不是"标准宽度"：
    // - 名称列上界 300px ≈ 41 个等宽字符。真实 MAME 0.284 里最长的 rom
    //   文件名有 81 字符（`m2500p-vt09-epson,20091222ver05,...`），不设上界
    //   的话一台机器就能把整张表推到面板外面去。
    // - 状态列上界按最宽的状态词（`坏 dump` / `未 dump`，三个汉字 + 空格 +
    //   四个西文字母）再加间隙和图标。
    pub const NAME_MIN: f32 = 60.0;
    pub const NAME_MAX: f32 = 300.0;
    pub const STATE_MIN: f32 = 40.0;
    pub const STATE_MAX: f32 = 96.0;
    pub const CRC_MIN: f32 = 40.0;
    pub const CRC_MAX: f32 = 104.0;
    pub const REGION_MIN: f32 = 40.0;
    pub const REGION_MAX: f32 = 160.0;
}

/// 一次量出来的整幅栅格：**列宽由这次要画的内容决定**，不随面板宽度变。
///
/// 2026-10-06 用户报的问题正是「宽度不是由内容决定的」：名称列被钉死在
/// 300px，而一屏文件名只有 `242-p1.p1`（10 字符 ≈ 72px），于是名字到
/// 「拥有」之间空出 228px；而状态列里「拥有 + ✓」只占 44px，图标到 `crc`
/// 又空 16px。同一条横线上三种空档，宽的那几个把整张表撑散了。
///
/// 现在的规则：
///
/// - **列宽 = 这一列里最长内容的实测宽度**（用真实字体量，不是估算字符数），
///   再夹在 `*_MIN` / `*_MAX` 之间；
/// - **列与列之间一律 [`cols::COL_GAP`]**；
/// - 面板比整表还窄时，只压缩**名称列**（下界 `NAME_MIN`），其余列保持
///   绝对位置 —— 宁可在名称列里折行，也不能让状态/crc 互相压。
///
/// 关键点：**状态列的宽度只由"这一屏真正出现过的状态"决定**，不是五种
/// 状态里最宽的那个。第一屏全是「拥有」时它是 `24 + 4 + 16 = 44px`；
/// 若按「缺失」算就会留出 52px 的空档 —— 那正是用户嫌"图标离 crc 太远"
/// 的来源。
#[derive(Clone, Copy, Debug)]
struct Grid {
    /// 名称列宽。
    name: f32,
    /// 状态列宽（状态词 + [`cols::ICON_GAP`] + 图标）。
    state: f32,
    /// CRC 列宽。
    crc: f32,
    /// 区域列宽（`region:tag`）。
    region: f32,
}

/// 用真实字体量一段文字的**单行宽度**。
///
/// 必须走 `layout_job` + `append_to`，才能把 `.monospace()` 之类样式带上
/// ——`RichText` 的字段全是私有的，`append_to` 是唯一带样式的公开入口。
/// `max_width` 给 `f32::MAX` 表示"不许换行"，我们要的是这一行有多宽。
fn text_w(ui: &egui::Ui, text: egui::RichText) -> f32 {
    let style = ui.style().clone();
    let mut job = egui::text::LayoutJob::default();
    text.append_to(
        &mut job,
        &style,
        egui::FontSelection::default(),
        egui::Align::Min,
    );
    job.wrap.max_width = f32::MAX;
    ui.fonts(|f| f.layout_job(job)).size().x
}

/// `RomRow` 的区域文本：`region` 或 `region:tag`。
///
/// [`measure_grid`] 与 [`rom_line`] 必须用**同一个**函数，否则量出来的
/// 宽度和实际画出来的不一致（列宽按短的算、实际画长的，末尾会撞下一列）。
fn region_text(r: &RomRow) -> String {
    match (&r.region, &r.tag) {
        (reg, Some(tag)) => format!("{reg}:{tag}"),
        (reg, None) => reg.clone(),
    }
}

/// `RomState` 在"这一屏出现了哪些状态"表里的下标。
fn state_index(s: RomState) -> usize {
    match s {
        RomState::Good => 0,
        RomState::BadDump => 1,
        RomState::Missing => 2,
        RomState::NoDump => 3,
        RomState::Unknown => 4,
    }
}

/// 把 `s` 的宽度并进 `max`。
fn note(max: &mut f32, ui: &egui::Ui, s: &str) {
    *max = (*max).max(text_w(ui, egui::RichText::new(s).monospace()));
}

/// 把一个 [`RomRow`] 的三列宽度并进累加器。
fn note_row(
    name: &mut f32,
    region: &mut f32,
    crc: &mut f32,
    seen: &mut [bool; 5],
    ui: &egui::Ui,
    r: &RomRow,
) {
    note(name, ui, &r.name);
    note(region, ui, &region_text(r));
    note(crc, ui, &crc_text(r.crc));
    seen[state_index(r.state)] = true;
}

/// 把「量出来的原始宽度」换算成最终的 [`Grid`]。
///
/// **从 [`measure_grid`] 里拆出来是为了能测。** 原来这段逻辑埋在
/// `measure_grid` 末尾，而那个函数要 `&MameApp`（测试里造不出来：它要
/// 读配置、解MAME 路径）+ 要活的 `egui::Ui`（要字体度量）。于是"名称列
/// 到底由内容决定还是被写死"这条最要紧的性质**没法测**——试着把
/// `NAME_MAX` 抬到极大来模拟"回到写死 300px"，全套测试照样全绿。
///
/// 拆出来之后它是纯函数：`f32进、Grid 出`，上面那条变异能被抓住。
///
/// `name_w` / `crc_w` / `region_w` 是**实测**出来的最长内容宽，
/// `word_w` 是这一屏出现过的最宽状态词。
fn fit_grid(name_w: f32, word_w: f32, crc_w: f32, region_w: f32, avail: f32) -> Grid {
    use cols::{CRC_MAX, CRC_MIN, NAME_MAX, NAME_MIN, REGION_MAX, REGION_MIN, STATE_MAX, STATE_MIN};

    let state = (word_w + cols::ICON_GAP + cols::ICON_W).clamp(STATE_MIN, STATE_MAX);
    let crc = crc_w.clamp(CRC_MIN, CRC_MAX);
    let region = region_w.clamp(REGION_MIN, REGION_MAX);

    // 名称列由**内容**决定（实测宽），再夹在上界内；面板装不下时只压它，
    // 且不许压到 NAME_MIN 以下——负宽度会把后面的列往回挤（错位）。
    let fixed = cols::COL_GAP * 3.0 + state + crc + region;
    let name = name_w
        .clamp(NAME_MIN, NAME_MAX)
        .min((avail - fixed - cols::COL_GAP).max(NAME_MIN));

    Grid {
        name,
        state,
        crc,
        region,
    }
}

/// 量出这一屏的整幅栅格。
///
/// **必须在 `render` 里量一次**，然后传给每一行 —— 不是每行各量一次。
/// 每行各量的话，"这一行最长的是谁"就变了，列起点会逐行飘（这正是
/// [`tight_horizontal`] 注释里记的那个坑的同族）。
fn measure_grid(ui: &egui::Ui, app: &MameApp, view: &RomInfoView, avail: f32) -> Grid {
    let mut name = 0.0_f32;
    let mut region = 0.0_f32;
    let mut crc = 0.0_f32;
    // 出现过哪些状态 —— 只为状态列量宽，见 `Grid` 的注释
    let mut seen = [false; 5];

    // 名称列量的是**所有段的第一列**，因为名称列是全局的一列：Rom 段的
    // 文件名与引用设备段的机种名共用同一个栅格起点。
    for r in &view.roms {
        note_row(&mut name, &mut region, &mut crc, &mut seen, ui, r);
    }
    for d in &view.disks {
        note(&mut name, ui, &d.file_name);
        note(
            &mut crc,
            ui,
            &format!("sha1({})", &d.sha1[..8.min(d.sha1.len())]),
        );
        seen[state_index(d.state)] = true;
    }
    for b in &view.bios {
        note(&mut name, ui, &b.name);
        seen[state_index(b.state)] = true;
        for r in &b.roms {
            note_row(&mut name, &mut region, &mut crc, &mut seen, ui, r);
        }
    }
    for d in &view.devices {
        note(&mut name, ui, &d.name);
        seen[state_index(d.state)] = true;
    }
    for r in &view.device_roms {
        note_row(&mut name, &mut region, &mut crc, &mut seen, ui, r);
    }
    for s in &view.samples {
        note(&mut name, ui, &s.name);
        seen[state_index(s.state)] = true;
    }

    // 状态词宽度：只算**这一屏真的出现过**的状态。
    let mut word = 0.0_f32;
    for (i, on) in seen.iter().enumerate() {
        if *on {
            let st = [
                RomState::Good,
                RomState::BadDump,
                RomState::Missing,
                RomState::NoDump,
                RomState::Unknown,
            ][i];
            word = word.max(text_w(ui, egui::RichText::new(app.tr(state_word(st)))));
        }
    }

    fit_grid(name, word, crc, region, avail)
}

/// 状态词 + 图标**画在同一格里**：词在左，图标在右，中间 4px。
///
/// **顺序是「词 + 图标」**（2026-06 用户定的），而**格宽由参数 `w` 给定**
/// （`Grid::state`），不再是写死的 60px。
///
/// 写死 60 的坏处2026-10-06 被用户指出：那一屏全是「拥有」（约 24px），
/// 格宽 60 就在图标后面白留 20px，看起来就是"crc 离对勾那么远"。现在
/// `measure_grid` 按**这一屏真实出现过的状态**量宽——全是「拥有」时格宽
/// 恰好 `24 + 4 + 16`，图标贴格尾，格尾到 crc 只隔统一的 [`cols::COL_GAP`]。
///
/// **整格死占 `w`**，所以跨段的状态起点仍落在同一条竖线上——这条比图标在
/// 前还是在后重要得多。
///
/// 图标 16px（[`cols::ICON_W`]）画在 `词宽 + 4` 处，不占独立列。/// 图标**不染色**（1.8.2 那套 png 自带颜色），但**缺失用的是新画的
/// `status_missing.png`（红底白叉）** —— 原来的 `status_cross` 是**蓝底**
/// 白叉，深色主题下容易被读成灰色 = 「未审计」，而灰色是未审计的专属。
///
/// 退路：纹理未解码时在图标位置画文字符号。
fn state_with_icon(ui: &mut egui::Ui, w: f32, text: impl AsRef<str>, state: RomState) {
    let color = state_color(state);
    let style = ui.style().clone();
    // 1) 先排状态词量出宽度 —— 图标要贴在词尾，必须先知道词多宽。
    //    不换行：状态词最长三个字，溢出的应该是图标那一侧而不是词。
    let mut job = egui::text::LayoutJob::default();
    egui::RichText::new(text.as_ref())
        .color(color)
        .append_to(
            &mut job,
            &style,
            egui::FontSelection::default(),
            egui::Align::Min,
        );
    job.wrap.max_width = f32::MAX;
    let galley = ui.fonts(|f| f.layout_job(job));
    let word_w = galley.size().x;
    // 文字行框高（实测 14px；不是图标那 16px）。整格按它占高，文字就不溢出。
    let text_h = galley.size().y;
    let row_h = text_h.max(cols::ICON_W);
    // 2) 整格死占 w（列宽刚性由外部栅格保证，见 [`Grid`]）
    let (_id, rect) = ui.allocate_space(egui::vec2(w, row_h));
    ui.painter()
        .galley(rect.min, galley, ui.visuals().text_color());
    // 3) 图标贴在词尾 + 4px，**纵向对齐到文字的视觉中线**
    //
    // 这里对齐的是**文字行的中线**（`rect.min.y + text_h / 2`），不是整格
    // 的中线、也不是图标自身 16px 的中心。差1px 就看得出来（用户
    // 2026-10-06：「这个 logo 要和文字上下对齐」）。
    //
    // 为什么不能居中于格子：文字行框实测 **14px**、图标 **16px**，格子取
    // 两者较大值 16px。居中于格子 → 图标中心在 8px，而文字中线在 7px
    // → 图标比文字低 1px（探针 `examples/col_probe.rs` 实测）。
    //
    // 也不能顶对齐（原先那样）：图标中心 8px、文字 7px，图标比文字高 1px。
    //
    // **光学补偿 1px**（用户 2026-10-07：「图标有点点下沉」）：上面的公式
    // 对齐的是行框的**几何**中线，而汉字字面几乎不占用行框的 descent 区
    // —— 汉字的视觉重心在几何中线之上。把一个 16px 的有色实心块对到
    // 几何中线，肉眼读出来就是图标下沉。`ICON_OPTIC_NUDGE` 把图标再上移
    // 1px，让图标的中心落在汉字**字面**的视觉中线上。
    const ICON_OPTIC_NUDGE: f32 = 1.0;
    let ctx = ui.ctx().clone();
    let icon_y =
        rect.min.y + text_h / 2.0 - cols::ICON_W / 2.0 - ICON_OPTIC_NUDGE;
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + word_w + cols::ICON_GAP, icon_y),
        egui::vec2(cols::ICON_W, cols::ICON_W),
    );
    // `icons::put` 内部是 `painter().image(...)`，**不碰游标** —— 这条很关键：
    // 它若走 `ui.put`（widget），游标会被额外推一次且推的量随词宽变化，
    // crc 与区域列的起点就逐行漂（见 `icons::put` 的注释与
    // `examples/col_probe.rs` 的实测）。
    let drawn = state_icon(state).is_some_and(|n| icons::put(ui, &ctx, n, icon_rect));
    if !drawn {
        // 纹理还没解码完（第一帧）或图标名写错了：退回文字符号
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            mark_of(state),
            egui::FontId::proportional(12.0),
            color,
        );
    }
}

/// 画一个**左对齐的定宽单元格**。
///
/// **不能用 `add_sized`** —— 它内部是 `Layout::centered_and_justified`，
/// 文字在给定宽度里**居中**：名称列拿到 500px、文件名只有 70px 宽时，
/// 文字落在 215..285 而不是 0..70，整列看起来缩进了一大截（实测名称起点
/// 飘到面板左边缘右侧 237px 处）。而且它对同一段的所有列都居中，短内容的
/// 列偏移各不相同，列与列之间也就对不齐了。
///
/// **`allocate_ui_with_layout` 同样不能用——它按内容撑开。** 它的文档写得很直白：
///
/// > Allocated the given space and then adds content to that space.
/// > **If the contents overflow, more space will be allocated.**
///
/// 实现最后是 `advance_after_rects(child_ui.min_rect(), …)`，而 `min_rect`
/// 会被里面的 `Label` 顶大。Rom 文件名短看不出来，设备名一长就现原形：
/// `gfxdecode`（9 字符）把名称列顶宽 30px，"拥有"就比 `z80` 那行右移 30px，
/// 整段状态词连成一条斜线（实测起点 437 / 399 / 463 / 425 / 477 / 437）。
///
/// **正确顺序：先 `allocate_space(w)` 死占 w 像素**（它只推进游标，
/// 绝不回看之后画了什么，所以列宽是刚性的），**再在占好的 rect 里自己排版
/// 画字**。内容超宽就在 `w` 内换行，绝不溢出到下一列。
fn cell(ui: &mut egui::Ui, w: f32, text: egui::RichText) {
    if w <= 0.0 {
        return;
    }
    // 1) 排版。`RichText` 的字段全是私有的，唯一能把样式带出来的公开
    //    入口是 `append_to`（`Label` 内部也这么用）：把文字追加到一个
    //    `LayoutJob` 上，字号 / 等宽 / 颜色 / 粗体都跟过去。
    //    `max_width` = **列宽**：超宽就在列内换行。这是必须的——给
    //    `f32::MAX`（不换行）时长文件名会直接压到状态列上，
    //    `gambs2m_01.05.u3` 和 `10239811.u86` 都顶掉了"拥有"两个字。
    let style = ui.style().clone();
    let mut job = egui::text::LayoutJob::default();
    text.append_to(
        &mut job,
        &style,
        egui::FontSelection::default(),
        egui::Align::Min,
    );
    job.wrap.max_width = w;
    let galley = ui.fonts(|f| f.layout_job(job));
    // 2) 死占 `vec2(w, 本单元格的实际高度)`。
    //    -宽度必须死占（否则 `gfxdecode` 这类长名字会把列顶开，后面全飘）。
    //    - 高度必须是**内容高度**：名称折行时是 2 行、3 行，不报上去就会
    //      压到下一行头上（实测 `10239811.u86` 折成两行正好糊在下一行）。
    //
    //    `allocate_space` 的契约是"你给多少就至少占多少"，所以这里把
    //    `galley.size().y` 原样报上去，行高就由最"高"的那个单元格决定。
    let (_id, rect) = ui.allocate_space(egui::vec2(w, galley.size().y));
    // 3) 画在占位处的左上角，位置与内容长度无关。
    ui.painter()
        .galley(egui::pos2(rect.left(), rect.top()), galley, ui.visuals().text_color());
}

/// 画一个**空占位**，宽度给定的列，用来在"这一段没有这一列"时保持栅格。
///
/// egui 的 `horizontal` 是按累加推进的：少画一列，后面的列就整体前移。
/// 所以 CHD 段（没有 region 列）必须用这个补上，否则它的 crc 会落在
/// 状态列的位置上，跨段就错位了。
fn gap(ui: &mut egui::Ui, w: f32) {
    if w > 0.0 {
        ui.add_space(w);
    }
}

/// 列与列之间的那个**统一间隙**（[`cols::COL_GAP`]）。
///
/// 单独一个函数而不是到处写 `gap(ui, cols::COL_GAP)`：这个值是"全表看起来
/// 像一套排版"的唯一保证，散着写早晚有一处漏掉或者写成别的数（用户
/// 2026-10-06 报的就是同一条横线上三种空档）。凡是要往下一列走，先过它。
fn col_gap(ui: &mut egui::Ui) {
    gap(ui, cols::COL_GAP);
}

/// 行内的 `horizontal` 布局：**上下贴紧**。
///
/// egui 的 `horizontal` 默认给每一行留 `item_spacing.y`（默认 6px）加字体
/// 行高，40 个 rom 就是 40 × 多余的十几像素 —— 面板窄的时候，一屏能看的行数
/// 被行距吃掉一半。这里把交叉轴对齐改成 `Min`，让行高由内容（16px 图标）
/// 决定而不是由间距决定。
///
/// **注意这里不再传"行宽"给闭包**（2026-10-06 起）。以前每行要自己算
/// `cols::name_width(avail)`，而行内是个 `LeftToRight` 子 ui，它的
/// `cursor()` 带着**上一行遗留的 x 偏移**，量出来的可用宽度逐行变化，状态词
/// 从第1 行斜到第 8 行。现在栅格在 `render` 里量一次就定型，逐行原样传。
fn tight_horizontal(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let layout = egui::Layout {
        main_dir: egui::Direction::LeftToRight,
        main_wrap: false,
        main_align: egui::Align::Min,
        main_justify: false,
        cross_align: egui::Align::Min,
        cross_justify: false,
    };
    // `max_rect` 必须是**这一行自己的可用矩形**（默认行为），不能用
    // `ui.max_rect()`：后者是整个面板的矩形，行高就不再由内容决定，实测
    // 所有行塌成同一条横线。
    ui.scope_builder(egui::UiBuilder::new().layout(layout), |ui| {
        // **横向间距也必须归零。** 栅格的每一步都已经算死了（列宽 + 统一的
        // [`cols::COL_GAP`]），再叠一层 `item_spacing.x`（默认 8px）就等于
        // 每行凭空多出 6 项 × 8px = 48px 的偏移。行布局把它算进子 ui 的
        // `min_rect`，父 ui 游标跟着右移，于是状态词从第 1 行斜到第 9 行
        // （实测右移约 250px）——斜率恒定正是这个的特征。
        ui.spacing_mut().item_spacing.x = 0.0;
        // 竖向同理：紧凑是这里的目的，行高由 16px 图标决定。
        ui.spacing_mut().item_spacing.y = 0.0;
        add_contents(ui);
    });
}

/// Rom 段的一行：名称 / 状态+图标 / CRC / 区域(带 tag)。
///
/// **所有列宽由 [`Grid`] 定死且左对齐**，理由见 [`Grid`] 与 [`cell`]。
///
/// **只有状态列上色**（用户要求）：名称 / CRC / 区域一律走默认前景色。
/// 早先把整行都染成状态色，一屏几十行全是绿字，看着像报错；而且"缺一个
/// 文件"和"这台机器有 40 个文件全是好的"用同一种满屏绿色表达，信息量是零。
/// 状态词 + 图标已经足够定位，颜色只服务这两列。
fn rom_line(ui: &mut egui::Ui, app: &MameApp, g: &Grid, r: &RomRow) {
    tight_horizontal(ui, |ui| {
        cell(ui, g.name, egui::RichText::new(&r.name).monospace());
        col_gap(ui);
        // 状态词 + 图标：同一格，图标紧跟词尾（用户 2026-06 要求）
        state_with_icon(ui, g.state, app.tr(state_word(r.state)), r.state);
        col_gap(ui);
        cell(ui, g.crc, egui::RichText::new(crc_text(r.crc)).monospace());
        col_gap(ui);
        // 区域 + tag（`igs023:sprcol`）
        cell(ui, g.region, egui::RichText::new(region_text(r)).monospace());
        // 原来这里还有一列 `from`，显示继承来源的 `(pgm)` / `(m4acechs)`。
        // **2026-06 按用户要求删除**："最后一列括号不需要显示"。
        //
        // 两条理由：① 那是 listxml 的 `merge=` 属性，对"这盘游戏能不能跑"
        // 没有增量信息；② 它在 REGION 之后留出 84px，右侧整块空着，视觉上
        // 像多了一列不存在的东西。
        //
        // `RomRow::from` 字段仍在数据层（条目的真实来源，别的功能可能要用），
        // 只是不渲染了。
    });
}

/// 渲染整个视图。
///
/// `header_note` 是顶部那行说明（例如"来自校验缓存"或"刚刚重新校验"），
/// `None` 时不渲染那一行。
///
/// **只有数据，没有游戏名/描述/缺失计数。** 用户明确要求去掉顶部那一块
/// （游戏名 + 描述 + 说明 + "全部齐全"）——面板挂在游戏列表旁边，选中哪台
/// 一眼就看得见，重复一遍只是噪音；缺失与否在 Rom 段里每行都写着。
///
/// **行与行之间一律不缩进。** 曾经这里每行都套一层 `ui.indent("rom_rows", …)`，而 `ui.indent` 是**按 id 存状态的**：同一个 id 在循环里反复调用，缩进会逐行累加（第二行起每行往右挪一点）。更糟的是缩进会吃掉 `available_width()`，于是列宽逐行变小、状态词起点逐行左移 —— 用户看到的"每到下一行就额外缩进、根本没对齐"就是这个。对齐由 [`Grid`] 栅格保证，缩进只会碍事。
pub fn render(ui: &mut egui::Ui, app: &mut MameApp, view: &RomInfoView) {
    // **顶部不要那行"数据来源于校验缓存"。**
    // 旧版写的是"数据来源"这类元信息，用户不要：面板里每一行的状态词已经
    // 把结论说完了，顶部再写一遍"这数据是哪来的"是纯噪音。

    if view.is_empty() {
        ui.weak(app.tr("This game has no roms or disks."));
        return;
    }

    // **栅格在这里量一次**，然后逐行原样传下去。
    //
    // 必须在**行外**量：行内是个 `LeftToRight` 子 ui，它的 `cursor()` 带着
    // 上一行遗留的 x 偏移，`available_width()` 逐行不同 → 列宽逐行变 →
    // 状态词连成一条斜线（这个坑踩过，见 `tight_horizontal` 的注释）。
    let g = measure_grid(ui, app, view, ui.available_width());

    // Rom 段
    if !view.roms.is_empty() {
        section(ui, app, "Rom:");
        for r in &view.roms {
            rom_line(ui, app, &g, r);
        }
    }

    // CHD 段：文件名 / 状态 / sha1。它只画到 crc 列，**剩下区域列必须补空
    // 占位**，否则这段的状态词会比 Rom 段靠右，整面板的"拥有"对不齐。
    if !view.disks.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        section(ui, app, "Disks:");
        for d in &view.disks {
            let color = state_color(d.state);
            tight_horizontal(ui, |ui| {
                cell(ui, g.name, egui::RichText::new(&d.file_name).monospace());
                col_gap(ui);
                state_with_icon(ui, g.state, app.tr(state_word(d.state)), d.state);
                col_gap(ui);
                let short = if d.sha1.len() > 8 {
                    &d.sha1[..8]
                } else {
                    &d.sha1[..]
                };
                cell(
                    ui,
                    g.crc,
                    egui::RichText::new(format!("sha1({short})"))
                        .monospace()
                        .color(color),
                );
                // 补齐栅格最后一列
                col_gap(ui);
                gap(ui, g.region);
            });
        }
    }

    // Bios 段：集名 + 状态+图标 + 描述，**下面列这一套实际的文件**。
    // 文件用 `rom_line`（同一份 `Grid`），所以整段的 crc/ 区域也对齐。
    // 集标题行也走栅格——用 `ui.label` 自然宽度的话，集名一长就把状态词
    // 推到右边，看起来又是错位的。
    if !view.bios.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        section(ui, app, "Bios:");
        for b in &view.bios {
            let color = state_color(b.state);
            tight_horizontal(ui, |ui| {
                cell(
                    ui,
                    g.name,
                    egui::RichText::new(&b.name).monospace().strong().color(color),
                );
                col_gap(ui);
                // 状态词 + 图标，与 Rom 段同一套
                state_with_icon(ui, g.state, app.tr(state_word(b.state)), b.state);
                col_gap(ui);
                // 描述占 crc + 间隙 + region 的总宽（描述比 crc 长得多）。
                // 同样**不加 `.small()`** —— 字号跟其他列一致，靠灰色弱化。
                cell(
                    ui,
                    g.crc + cols::COL_GAP + g.region,
                    egui::RichText::new(&b.description)
                        .monospace()
                        .color(ui_weak_color()),
                );
            });
            for r in &b.roms {
                rom_line(ui, app, &g, r);
            }
        }
    }

    // 引用设备段：先列设备机种 + 状态 + 图标，再列它们的 rom 文件。
    // rom 文件走 `rom_line`，所以设备的文件明细与 Rom 段列宽完全一致。
    if !view.devices.is_empty() || !view.device_roms.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        section(ui, app, "Referenced devices:");
        for d in &view.devices {
            tight_horizontal(ui, |ui| {
                // 第一列：设备机种名（`m68000` / `igs036` / `z80`）。
                cell(ui, g.name, egui::RichText::new(&d.name).monospace());
                col_gap(ui);
                // 第二列：状态词 + 图标（与其他段同一套），不画描述/tag ——
                // tag（`:maincpu`）是内部引用名，描述在这台机器的语境下是废话。
                state_with_icon(ui, g.state, app.tr(state_word(d.state)), d.state);
                // 后面这些列一律留空，但**必须 gap 占住**——少一列，
                // 下一行的设备 rom 就会整体前移（见 `gap` 的注释）。
                col_gap(ui);
                gap(ui, g.crc);
                col_gap(ui);
                gap(ui, g.region);
            });
            // 设备自己的 rom：按设备机种名匹配回去（`device_roms` 的
            // `from` 就是设备机种名）
            for r in view.device_roms.iter().filter(|r| r.from.as_deref() == Some(d.name.as_str())) {
                rom_line(ui, app, &g, r);
            }
        }
        // 有 rom 但设备机种不在库里（裁剪过的 dat）——仍要把文件列出来
        for r in view
            .device_roms
            .iter()
            .filter(|r| !view.devices.iter().any(|d| Some(d.name.as_str()) == r.from.as_deref()))
        {
            rom_line(ui, app, &g, r);
        }
    }

    // 设备/ 槽位段已在 2026-10-06 **整段删除**（用户要求）。
    // 理由：`<device>` / `<slot>` 描述的是"模拟器自带的硬件能力"，
    // 不是"用户手上有没有这个文件"——`floppydisk` / `fdc:0` / `printer`
    // 这些行对"这盘游戏能不能跑"没有增量信息。Rom 段与引用设备段已经
    // 覆盖了真正影响可运行性的东西。
    //
    // 数据层仍在（`RomInfoView::slots` / `slot_decls`），只是不渲染了——
    // 它们是 MAME 硬件模型的完整描述，别的功能（命令行生成）可能要用。

    // Samples 段：**只有样本集名 + 状态两列**（2026-10-06 用户要求）。
    //
    // 去掉的是：状态图标、`18/18` 数量。判据从"包里逐个比对文件"改成
    // **"这个样本集包在不在"** —— 样本集是共享包（`genpin` 被 1438 台
    // pinball 游戏引用），`18/18` 的分子是整包的文件数、分母是这台机器
    // 需要的数量，两个不同口径的数并排显示并不说明任何事。
    //
    // **CRC 校验本来也做不了**：`<sample>` 是 EMPTY 元素，DTD 里只有 name，
    // 没有 size/crc/sha1。所以"包在不在"已经是能拿到的最强结论。
    if !view.samples.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        section(ui, app, "Samples:");
        for s in &view.samples {
            let color = state_color(s.state);
            tight_horizontal(ui, |ui| {
                cell(ui, g.name, egui::RichText::new(&s.name).monospace());
                col_gap(ui);
                cell(
                    ui,
                    g.state,
                    egui::RichText::new(app.tr(state_word(s.state))).color(color),
                );
                // 后面的列一律留空，但**必须 gap 占住**——少一列，这一段
                // 的列起点就与 Rom 段不一致（见 [`Grid`]）。
                col_gap(ui);
                gap(ui, g.crc);
                col_gap(ui);
                gap(ui, g.region);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// crc 为 0 的条目不能显示成一个像真值的 `crc(00000000)`。
    #[test]
    fn a_zero_crc_reads_as_a_dash() {
        assert_eq!(crc_text(0), "-");
        assert_eq!(crc_text(0x78c1_5fa2), "crc(78c15fa2)");
    }

    /// 用户指定的四态配色（2026-06 修订）：绿 / 黄 / 黄 / 红。
    ///
    /// **只有"缺失"是红色。** `nodump` 从灰色改成黄色（与坏 dump 同色，
    /// 图标也是同一个感叹号）——原先它和"未校验"共用灰色，用户看到灰就
    /// 以为是没查。灰色现在只属于"未校验"，独占。
    #[test]
    fn the_four_user_facing_colours_are_distinct() {
        assert_eq!(state_color(RomState::Good), icons::GREEN, "拥有=绿");
        assert_eq!(state_color(RomState::BadDump), icons::YELLOW, "坏 dump=黄");
        assert_eq!(state_color(RomState::NoDump), icons::YELLOW, "未 dump=黄");
        assert_eq!(state_color(RomState::Missing), icons::RED, "缺失=红");
        // 红色是"确实缺"的专属，不能被任何别的状态借用
        assert_ne!(
            state_color(RomState::Unknown),
            icons::RED,
            "未校验不能是红色 —— 那会让人以为自己的盘空了"
        );
        assert_ne!(
            state_color(RomState::Unknown),
            icons::YELLOW,
            "未校验不能是黄色 —— 黄色已经表示有问题了"
        );
    }

    /// 每个状态都要有图标名。
    ///
    /// **`nodump` 与 `baddump` 共用黄叹号是故意的**（2026-06）：用户要求
    /// "未 dump 用黄色加个感叹号"，而这两者同色同义 —— 都是"有问题，但不是
    /// 缺文件"。原先 `nodump` 用蓝问号（`status_preliminary`），那格图标
    /// 自带蓝色，跟旁边的黄字并排显得是两件事。
    ///
    /// 真正要防的是**绿勾 / 红叉 / 黄叹 / 蓝问这四个语义图标**被两个状态
    /// 共用：拥有/坏 dump/缺失/未校验必须各自不同。
    #[test]
    fn the_meaningful_icons_are_not_shared() {
        let unique = [
            RomState::Good,
            RomState::BadDump,
            RomState::Missing,
            RomState::Unknown,
        ];
        let mut names: Vec<&str> = unique
            .iter()
            .map(|s| state_icon(*s).expect("每个状态都要有图标"))
            .collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "图标不能重复：{names:?}");

        // nodump 明确跟baddump 共用那张黄叹号
        assert_eq!(
            state_icon(RomState::NoDump),
            state_icon(RomState::BadDump),
            "未 dump 与坏 dump 同为黄色感叹号"
        );
    }

    /// **缺失必须是红叉**，而且不能是 1.8.2 那张 `status_cross`。
    ///
    /// 用户 2026-06 指出：缺失显示的是**蓝色**叉（`status_cross.png` 是蓝底
    /// 白叉），而配色约定里蓝色在深色主题下容易被读成灰色 = 「未审计」——
    /// 那正是这个约定要避免的（灰色是「没查」的专属）。
    ///
    /// 所以新画了 `status_missing.png`（红底白叉），与绿勾同一套底色风格。
    /// 这条钉住"别哪天又换回蓝的那张"。
    #[test]
    fn not_owned_uses_the_red_cross() {
        let missing = state_icon(RomState::Missing).expect("缺失要有图标");
        assert!(
            missing.contains("missing"),
            "缺失该用新画的 status_missing（红底白叉），现在却是 {missing}"
        );
        assert_ne!(
            missing,
            "16x16/status_cross.png",
            "status_cross 是**蓝底**白叉，深色主题下会被读成灰色 = 未审计"
        );
        // 图标文件必须真的在资产表里（build.rs 按文件名生成 ICONS，名字写错
        // 只会在运行期"图标不显示"，编译期零报错 —— 这个坑与 `full_uv` 那次
        // 同源）。
        assert!(
            icons::ICONS.iter().any(|(n, _)| *n == missing),
            "assets里没有 {missing} —— 图标会在运行期静默不显示"
        );
    }

    /// **列间距全表只有一个值，且不随面板宽度变化。**
    ///
    /// 用户 2026-10-06 原话：「为什么拥有状态隔文件名这么远，这个间距应该
    /// 是固定的，不随窗口扩大而扩大，crc 和对勾 x 等间距这么远，这几个间距
    /// 应该一样。」
    ///
    /// 这条钉住两件事：
    ///
    /// 1. **间隙是常量**（[`cols::COL_GAP`]），不是"每列各自剩下多少"。
    ///    之前名称列钉死 300px（而这一屏文件名只有 `242-p1.p1`，10 字符
    ///    ≈ 72px → 空 228px），状态列钉死 60px（而「拥有 + ✓」只占 44px
    ///    → 图标后面空 16px）。同一条横线上三种空档。
    /// 2. **面板变宽时列宽不许变**：名称列不再吃剩余空间，所以右侧留白。
    ///    这条是2026-06 就定过的（"拉宽的话右侧就空出来即可"），当时用
    ///    定值实现，现在用"按内容定量+ 上限"实现，行为一致。
    #[test]
    fn the_column_gap_is_one_constant_and_does_not_track_the_panel_width() {
        assert_eq!(
            cols::COL_GAP, 12.0,
            "间隙是全表唯一的间距来源，改它等于改整张表的排版"
        );
        // 一个够宽的面板 + 一堆短名字：栅格总宽必须远小于面板（右侧留白），
        // 也就是"名字到拥有"的距离由名字本身决定，不由面板决定。
        let g = Grid {
            // `242-p1.p1` 十个等宽字符
            name: 72.0,
            // 「拥有」+ 4 + 16
            state: 44.0,
            // `crc(8893df89)` 十三个等宽字符
            crc: 94.0,
            // `cslot1:audiocpu` 十五个等宽字符
            region: 108.0,
        };
        let total = g.name + g.state + g.crc + g.region + cols::COL_GAP * 3.0;
        for panel in [400.0_f32, 900.0, 1400.0, 3000.0] {
            assert!(
                total < panel,
                "面板 {panel} 宽时表只占 {total}，剩下必须留白而不是把列撑开"
            );
        }
    }

    /// **名称列的宽度由内容决定 —— 这条是纯逻辑，所以能测。**
    ///
    /// 用户 2026-10-06 报的整件事就是这条被破坏了：名称列被钉死 300px，
    /// 而那一屏的文件名是 `242-p1.p1`（10 字符 ≈ 72px），于是名字到
    /// 「拥有」之间空出 228px。
    ///
    /// 这条测试的来历值得记下来：先前只测了 `COL_GAP` 常量，把
    /// `clamp(NAME_MIN, NAME_MAX)` 改成 `clamp(NAME_MIN, 100000.0)`
    /// （语义上就是"退回写死 300px"）**全套测试照样全绿**。原因是这段
    /// 逻辑埋在 `measure_grid` 里，而那个函数要 `&MameApp`（测试里造不
    /// 出来）+ 活的 `Ui`（要字体度量），于是最要紧的性质没法测。拆出
    /// [`fit_grid`] 之后它成了纯函数，这条才立得住。
    #[test]
    fn the_name_column_follows_the_content_not_a_fixed_value() {
        // 一屏短名字（实测约 72px）
        let short = fit_grid(72.0, 24.0, 94.0, 108.0, 1400.0);
        // 一屏长名字（实测约 290px）
        let long = fit_grid(290.0, 24.0, 94.0, 108.0, 1400.0);

        assert!(
            short.name < 120.0,
            "短文件名的一屏里名称列只有 72px，却占了 {} —— 又变回写死了",
            short.name
        );
        assert!(
            long.name > short.name + 150.0,
            "名字变长时名称列必须跟着变宽：{} -> {}",
            short.name,
            long.name
        );
        // 其余三列与名称列无关：名字长短不影响状态/crc/区域
        assert_eq!(short.state, long.state);
        assert_eq!(short.crc, long.crc);
        assert_eq!(short.region, long.region);
    }

    /// **面板变宽，列宽不许变**（右侧留白）。
    ///
    /// 用户 2026-06 原话：「拉宽的话右侧就空出来即可。」当时用"定值"
    /// 实现，现在用"按内容定量"，行为必须一样——名字短的一屏，面板从
    /// 1400 拉到 3000，表不许被撑开。
    #[test]
    fn widening_the_panel_does_not_stretch_the_columns() {
        let narrow = fit_grid(72.0, 24.0, 94.0, 108.0, 1400.0);
        for panel in [2000.0_f32, 3000.0, 5000.0] {
            let wide = fit_grid(72.0, 24.0, 94.0, 108.0, panel);
            assert_eq!(
                wide.name, narrow.name,
                "面板拉到 {panel}，名称列不许从 {} 变成 {}",
                narrow.name, wide.name
            );
            assert_eq!(wide.state, narrow.state);
            assert_eq!(wide.crc, narrow.crc);
            assert_eq!(wide.region, narrow.region);
        }
    }

    /// **面板装不下时，只压名称列，且压不穿下限。**
    ///
    /// 宁可在名称列里折行，也不能让状态/crc 互相压。负宽度会把后面的列
    /// 往回挤（错位）或者根本不渲染，比窄更糟。
    #[test]
    fn a_narrow_panel_squeezes_only_the_name_column_and_never_below_the_floor() {
        let roomy = fit_grid(72.0, 36.0, 94.0, 108.0, 1400.0);
        // 面板只剩 260px：名称列必须缩，且不低于 NAME_MIN
        let tight = fit_grid(72.0, 36.0, 94.0, 108.0, 260.0);
        assert!(
            tight.name < roomy.name,
            "面板变窄时名称列必须让位：{} -> {}",
            roomy.name,
            tight.name
        );
        // 荒谬地窄：仍然不许为负 / 不许小于下限
        for panel in [0.0_f32, 10.0, 100.0] {
            let g = fit_grid(72.0, 36.0, 94.0, 108.0, panel);
            assert!(
                g.name >= cols::NAME_MIN,
                "面板只有 {panel} 宽，名称列却只有 {} —— 负宽度会把后面的列往回挤",
                g.name
            );
            // 其余三列一个都不许动
            assert_eq!(g.state, roomy.state);
            assert_eq!(g.crc, roomy.crc);
            assert_eq!(g.region, roomy.region);
        }
    }

    /// **状态列的宽度只由"这一屏出现过的状态"决定。**
    ///
    /// 这是 2026-10-06 那个"crc 离对勾太远"的直接修法：写死 60px 时，
    /// 一屏全是「拥有」（24px）就在图标后面白留 20px。现在按真实出现的
    /// 状态量宽，所以**图标永远贴着格尾**，格尾到 crc 只隔 COL_GAP。
    ///
    /// 关键在"这一屏出现过"，不是"所有状态里最宽的"—— 后者会让全是
    /// 「拥有」的机种也按「未校验」三字的宽度留出空档（「缺失」改词后
    /// 与「拥有」同为两字，最宽的中文状态词是三字的「未校验」）。
    #[test]
    fn the_state_column_width_follows_the_states_actually_present() {
        // 「拥有」两字 + 间隙 + 图标：格子刚好装下，不留白
        let only_good = 24.0 + cols::ICON_GAP + cols::ICON_W;
        // 「未校验」三字：更宽
        let with_wider_word = 36.0 + cols::ICON_GAP + cols::ICON_W;
        assert!(
            with_wider_word - only_good >= 12.0,
            "三字状态词必须比两字词宽一整个汉字的量"
        );
        // 两者都在合法区间内
        for w in [only_good, with_wider_word] {
            assert!(
                (cols::STATE_MIN..=cols::STATE_MAX).contains(&w),
                "状态列宽 {w} 超出[{}, {}]",
                cols::STATE_MIN,
                cols::STATE_MAX
            );
        }
    }

    /// 状态图标与状态词之间留 1 个空格，且图标边长是 16（1.8.2 那套 png）。
    #[test]
    fn the_icon_sits_one_space_after_the_state_word() {
        assert_eq!(cols::ICON_W, 16.0, "1.8.2 的 status_*.png 是 16×16");
        assert!(
            (3.0..=12.0).contains(&cols::ICON_GAP),
            "图标与状态词之间留 1 个空格（约 4px），不能是 0 也不能太大：{}",
            cols::ICON_GAP
        );
    }

    /// **图标必须与状态词上下对齐**（用户 2026-10-06：「这个 logo 要和
    /// 文字上下对齐」；2026-10-07：「有点点下沉」→ 加 1px 光学补偿）。
    ///
    /// 差 1px 就看得出来，因为图标是**有色实心块**：文字之间的错位靠留白
    /// 吸收，而一个 16×16 的绿圆点偏上 1px 立刻读成"浮在字上面"。
    ///
    /// 对齐的基准是**文字行框的中线减 1px 光学补偿**，不是整格的中线、
    /// 也不是图标自身 16px 的中心：
    ///
    /// | 基准 | 图标中心 y | 文字行框中线 y | 差 |
    /// |---|---|---|---|
    /// |顶对齐（原先）| 8.0 | 7.0 | 图标高 1px |
    /// | 居中于整格（试过）| 8.0 | 7.0 | 图标低 1px |
    /// | **行框中线 - 光学补偿** | **6.0** | 7.0（字面视觉中线 ≈ 6.0） | **视觉 0** |
    ///
    /// 光学补偿的来由：汉字字面几乎不占行框的 descent 区，视觉重心在几何
    /// 中线之上 ~1px；实心图标对到几何中线会被读成"下沉"（用户
    /// 2026-10-07）。`ICON_OPTIC_NUDGE = 1.0` 把图标中心抬到汉字字面的
    /// 视觉中线上。
    ///
    /// **这条测试为什么查源码而不复刻公式**：第一版把公式抄进测试里自己算，
    /// 结果"改回顶对齐"与"改成居中于整格"两个变异**都全绿** —— 测试算的是
    /// 自己那份副本，真实代码改了它不知道（同义反复）。所以改成**断言源码
    /// 里那一行的字面形状**。真实字体下的数值验证交给探针
    /// `examples/font_metrics.rs`（行框 14px）与 `examples/col_probe.rs`。
    #[test]
    fn the_icon_is_vertically_centred_on_the_state_word() {
        let body = fn_body("fn state_with_icon(");
        // 必须是「文字中线 - 图标半高 - 光学补偿」，且必须用 text_h（文字行框高）
        let ok = "rect.min.y + text_h / 2.0 - cols::ICON_W / 2.0 - ICON_OPTIC_NUDGE";
        assert!(
            body.contains(ok),
            "图标必须对齐**文字视觉中线**（{ok}）—— 顶对齐偏高 1px、居中于整格偏低 1px、\
             对齐行框几何中线偏下沉 1px：\n{body}"
        );
        // 补偿量钉在 1px：0 就是回到了"下沉"，超过 1px 会读成偏高
        assert!(
            body.contains("const ICON_OPTIC_NUDGE: f32 = 1.0;"),
            "光学补偿必须是常量 1.0：\n{body}"
        );
        // 不许用整格高 row_h 来算居中（那正是 1px 错位的来源）
        assert!(
            !body.contains("(row_h - cols::ICON_W) / 2.0"),
            "不能用整格高居中：格子 16px 而文字行框 14px，那样图标低 1px：\n{body}"
        );
        // 文字行框高必须单独取出来（`galley.size().y`），不能被 row_h 顶掉
        assert!(
            body.contains("let text_h = galley.size().y;"),
            "要单独留文字行框高 text_h，居中公式才用得对：\n{body}"
        );
        assert!(
            body.contains("let row_h = text_h.max(cols::ICON_W);"),
            "整格高取文字与图标较大者：\n{body}"
        );
    }

    /// **图标比文字行框高 2px，所以对齐后必然上下各溢出 1px —— 这是允许的。**
    ///
    /// 真实字体下状态词行框 **14px**、图标 **16px**（`examples/font_metrics.rs`
    /// 实测）。要让两者中线重合，16px 的图标在 14px 的行里必然一头出 1px。
    ///
    /// 关键在于**溢出是对称的**（上下各 1px）：对称的溢出读起来仍像"这一行的
    /// 图标"，而单边溢出（只往上/ 只往下）会被读成错位。这条钉住"对称"这个
    /// 性质，并说明**为什么允许越界** —— 补的是上面那条查公式测试的**前提**。
    #[test]
    fn the_icon_overflows_the_text_row_symmetrically_by_one_pixel() {
        const TEXT_H: f32 = 14.0; // 探针实测
        let over_top = (TEXT_H - cols::ICON_W) / 2.0;
        let over_bottom = (TEXT_H - cols::ICON_W) / 2.0;
        // 对称
        assert_eq!(
            over_top, over_bottom,
            "溢出必须上下对称，单边溢出会被读成错位"
        );
        // 且各只有 1px（允许的前提是行间距为 0，见下面那条测试）
        assert!(
            over_top.abs() <= 1.01,
            "溢出应各不超过 1px，实际 {over_top}"
        );
        assert!(
            TEXT_H < cols::ICON_W,
            "这条测试的前提是「图标比文字行框高」；若字体变了行框更高，\
             就不该再有溢出，请重新算公式"
        );
    }

    /// **图标上下各溢出 1px 是安全的——因为行间距是0。**
    ///
    /// 上面那条对齐测试让图标对齐文字中线，于是它比文字行框上下各多出 1px。
    /// 这条钉住那个前提：**行布局的 `item_spacing.y` 必须归零**，否则这 1px
    /// 会去侵邻居行的字（图标是有色实心块，压到字上非常明显）。
    ///
    /// `item_spacing.y` 归零还有另一个理由（更贵）：不归零时每行之间多出
    /// 十几像素，一屏能看的行数被吃掉一半（用户要"每行间距紧凑一些"）。
    #[test]
    fn rows_leave_no_vertical_gap_for_the_icon_to_bump_into() {
        let body = fn_body("fn tight_horizontal(");
        assert!(
            body.contains("item_spacing.y = 0.0"),
            "行间距必须归零 —— 图标对齐文字中线后会上下各溢出 1px，\
             有间距时就会压到相邻行的字：\n{body}"
        );
    }

    /// **每一列的宽度都不许超过自己的上限。**
    ///
    /// 真实 MAME 0.284 里最长的 rom 文件名有 81 字符
    /// （`m2500p-vt09-epson,20091222ver05,...`），不夹上限的话一台机器
    /// 就能把整张表顶出面板。超长的在列内换行（见 [`cell`]）。
    #[test]
    fn every_column_is_clamped_to_its_upper_bound() {
        // name 列最坏情况：把测量结果换成天文数字也不许超过 NAME_MAX
        assert_eq!(
            100_000.0_f32.clamp(cols::NAME_MIN, cols::NAME_MAX),
            cols::NAME_MAX
        );
        assert!(
            cols::NAME_MAX < 400.0,
            "名称列上界要能装下绝大多数文件名，又不许把表撑散"
        );
        // 下限必须为正：0 或负宽度会让后面的列往回挤（错位）或根本不渲染
        for (name, min) in [
            ("name", cols::NAME_MIN),
            ("state", cols::STATE_MIN),
            ("crc", cols::CRC_MIN),
            ("region", cols::REGION_MIN),
        ] {
            assert!(min > 0.0, "{name} 列下限必须是正数：{min}");
        }
    }

    /// **同一段内，每两列之间都必须隔一个 `col_gap`。**
    ///
    /// 用户 2026-10-06 报的现象是"几个间距不一样"：名字到拥有是一段，
    /// 对勾到 crc 是另一段。根因是每一列各自留自己的余量，而不是留同一个
    /// [`cols::COL_GAP`]。
    ///
    /// 这条用源码钉住：`rom_line` 里四列后面必须都跟着 `col_gap(ui)`。
    /// 写成别的间隙值（或者干脆不写）都会让同一条横线参差不齐。
    #[test]
    fn every_column_is_followed_by_the_one_shared_gap() {
        let body = fn_body("fn rom_line(");
        // **三道**，不是四道：名称 / 状态 / crc 后面各一道，区域是最后一列、
        // 后面没有东西了，补一道纯粹浪费横向空间。
        assert_eq!(
            body.matches("col_gap(ui);").count(),
            3,
            "名称/状态/crc 三列后面各要一个 col_gap(ui)，现在不是 3 个：\n{body}"
        );
        // 不许绕过 col_gap 直接写死别的间隙
        assert!(
            !body.contains("add_space("),
            "列间距只能走 col_gap()，不许直接 add_space：\n{body}"
        );
    }

    /// **栅格必须在 `render` 里量一次**，逐行原样传下去。
    ///
    /// 每行各量一次就完了：行内是个 `LeftToRight` 子 ui，它的 `cursor()`
    /// 带着上一行遗留的 x 偏移，`available_width()` 逐行不同 → 列宽逐行变
    /// → 状态词连成一条斜线（实测右移约 250px）。
    #[test]
    fn the_grid_is_measured_once_outside_the_row_loops() {
        let body = fn_body("pub fn render(");
        assert_eq!(
            body.matches("measure_grid(").count(),
            1,
            "measure_grid 只能在 render 里调一次：\n{body}"
        );
        // `available_width()` **只允许出现在 measure_grid 那一次调用里**——
        // 它必须被量在所有行循环之外（行内子ui 的 cursor 带上一行的偏移）。
        // 只数**代码行**：注释里提到这个词是常事，计进去就永远不等于 1。
        let code: String = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            code.matches("available_width()").count(),
            1,
            "available_width() 只许在量栅格时出现一次，行循环里再量就是那个斜线 bug：\n{body}"
        );
        // 而且那一次必须与 measure_grid 同行（不是散落在别处）
        assert!(
            code.lines().any(|l| l.contains("measure_grid(") && l.contains("available_width()")),
            "量栅格的那一行必须同时取 available_width：\n{body}"
        );
    }

    /// 取出从 `marker` 起、到**第一个顶格 `}`** 为止的函数体。
    ///
    /// 不能用 `split("\n}\n")`：源码在 Windows 上是 CRLF，那个分隔符
    /// 永远匹配不上，`nth(1)` 之后拿到的就是"从该函数到文件末尾"——
    /// 测试照样跑，只是范围大到把 `render` 也吃进去，断言跟着失真
    /// （这个坑踩过一次：两条grep 测试同时失败， looked 像逻辑错，
    /// 其实是切分没生效）。
    fn fn_body(marker: &str) -> String {
        let src = include_str!("rompanel.rs");
        let after = src
            .split(marker)
            .nth(1)
            .unwrap_or_else(|| panic!("源码里找不到 {marker}"));
        let mut out = String::new();
        for (i, line) in after.lines().enumerate() {
            if i > 0 && line == "}" {
                break;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// **行与行之间不允许有缩进调用。**
    ///
    /// 缩进的量是**按 id 存在 `Memory.indentation` 里的有状态值**，同一个 id
    /// 在循环里每调用一次就累加一层。所以 `for r in &view.roms { … }` 循环体
    /// 里套一层缩进的第二行起会整体右移，而且缩进吃掉 `available_width()`
    /// 后 `cols::name_width()` 会逐行变小、状态词起点逐行左移——正是用户报的
    /// "每到下一行就额外缩进，根本没对齐"。
    ///
    /// 这种回归肉眼很难在改动里看出来（缩进本来就该存在），所以用测试钉住：
    /// **行布局里 `item_spacing` 两个轴都必须归零。**
    ///
    /// 这条最贵：症状是"状态词从第 1 行斜到第 9 行"（实测右移约 250px，
    /// 斜率恒定 ≈ 28px/行），而名称列起点、图标列起点看起来都正常，
    /// 很容易误判成"名称列宽度算错了"。
    ///
    /// 机制：`LeftToRight` 行布局把 `item_spacing.x`（默认 8px）加在
    /// **每一项之间**。一行有名称 + 状态 + 图标 + crc + 区域 + 来源 6 项，
    /// 就是 5 个间隙 × 8px = 40px 的额外宽度。它被算进子 ui 的
    /// `min_rect`，`allocate_new_ui` 再用它推进父 ui 的游标，于是
    /// **每行都比上一行右移一整个行的间距**。
    ///
    /// 栅格已经把每一步都算死了（[`Grid`] 的列宽 + [`cols::COL_GAP`]），再
    /// 叠一层 spacing 就是双重计费。竖向同理，`item_spacing.y` 归零才能让行高
    /// 由 16px 图标决定而不是由间距决定（用户要"每行间距紧凑一些"）。
    #[test]
    fn rows_have_no_item_spacing_at_all() {
        let body = fn_body("fn tight_horizontal(");
        assert!(
            body.contains("item_spacing.x = 0.0"),
            "行布局必须把 item_spacing.x 归零，否则每行递增右移"
        );
        assert!(
            body.contains("item_spacing.y = 0.0"),
            "行布局必须把 item_spacing.y 归零，否则行高被间距撑开"
        );
    }

    /// `render` 的函数体里出现缩进调用就失败。
    #[test]
    fn rows_are_never_indented() {
        let body = fn_body("pub fn render(");
        assert!(
            !body.contains("ui.indent("),
            "行循环里不许用 ui.indent（缩进按 id 累加，会逐行右移）：\n{}",
            body.lines().filter(|l| l.contains("ui.indent(")).collect::<Vec<_>>().join("\n")
        );
    }
}
