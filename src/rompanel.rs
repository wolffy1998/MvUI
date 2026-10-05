//! Rom 信息的表格渲染，被两处复用：
//!
//! * **dock 面板**（View ▸ 自定义信息栏 ▸ RomInfo）——数据来自审计缓存，
//!   跟着游戏选择刷新。
//! * **审计结果弹窗**（右键/菜单「审计 ROM」跑完）——数据来自现场重审。
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
/// 用户指定的四态配色：
///
/// | 状态 | 颜色 | 为什么 |
/// |---|---|---|
/// | 拥有 | 绿 | 校验通过 |
/// | 坏 dump | 黄 | 文件在但是坏的——比"好"差，比"没有"好 |
/// | 未拥有 | 红 | 确认缺失 |
/// | 无 dump | 灰 | MAME 说这个文件永远不会有，等于**不用管** |
///
/// `nodump` 用灰而不是黄/红：它压根不是坏消息，"没找到"才是。用户看到一屏
/// 红色会以为自己的盘有问题，而 nodump 是 dat 自己声明的例外。
/// 未审计也用灰——同样不是缺失，是**还没查**，两者靠文字区分而不是靠颜色。
fn state_color(state: RomState) -> egui::Color32 {
    match state {
        RomState::Good => icons::GREEN,
        RomState::BadDump => icons::YELLOW,
        RomState::Missing => icons::RED,
        RomState::NoDump | RomState::Unknown => ui_weak_color(),
    }
}

/// 第三列的状态图标（16×16）。
///
/// 用 1.8.2 那套 `assets/icons/16x16/status_*.png`——它们本来就是这个用途
/// （绿勾 `status_good` / 黄叹 `status_imperfect` / 蓝问 `status_preliminary`
/// / 蓝叉 `status_cross`）。自己画一套对勾叉号只会和它们不一致，也不用维护
/// 两份矢量资源。
///
/// `baddump` 那张是本项目新画的（`status_baddump.png`）：它与"未审计"语义
/// 完全不同——一个是"文件在但是坏的，用户该去重下"，另一个是"还没查"——
/// 共用一个图标会让用户以为 baddump 是个可以忽略的提示。
///
/// **图标不做染色**：它们已经是彩色的（绿勾 / 黄叹 / 蓝叉 / 橙叹），再乘一层
/// 状态色会把语义搅糊。名字找不到时返回 `None`，调用方退回文字符号。
fn state_icon(state: RomState) -> Option<&'static str> {
    Some(match state {
        RomState::Good => "16x16/status_good.png",
        RomState::BadDump => "16x16/status_baddump.png",
        RomState::Missing => "16x16/status_cross.png",
        RomState::NoDump => "16x16/status_preliminary.png",
        RomState::Unknown => "16x16/status_imperfect.png",
    })
}

/// "未审计"用的灰。
///
/// 取 `visuals().weak_text_color` 的话这里就得带 `ui`，而配色函数是纯的
/// （好测、好在任何面板/弹窗里复用）。用 `Color32::from_gray(140)`：两个
/// 主题下都读得清，且不依赖调用点。
fn ui_weak_color() -> egui::Color32 {
    egui::Color32::from_gray(140)
}

/// 段头：`Rom:` / `Bios:` / `引用设备:`。
fn section(ui: &mut egui::Ui, app: &MameApp, label: &str) {
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new(app.tr(label))
            .strong()
            .size(15.0),
    );
    ui.separator();
}

/// 状态词的 i18n key。
///
/// 返回 key 而不是文案：文案要走 `app.tr` 现查，语言切换后立刻跟着变。
///
/// 四个词与用户指定的四态一一对应：`拥有` / `坏 dump` / `未拥有` / `无 dump`
/// （外加未审计时的 `未审计`）。旧版的"很好 / 缺失"是从 MAME 的 audit 报告
/// 里抄的词，但用户明确要求用"拥有 / 坏 dump / 未拥有"这套——前者描述校验
/// 结果，后者描述**用户手上有没有**，后者才是用户真正关心的问题。
fn state_word(state: RomState) -> &'static str {
    match state {
        RomState::Good => "owned",
        RomState::BadDump => "bad dump",
        RomState::Missing => "not owned",
        RomState::NoDump => "no dump",
        RomState::Unknown => "not audited",
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
        RomState::BadDump => "!",
        RomState::Missing => "\u{2717}",
        RomState::NoDump | RomState::Unknown => "?",
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

/// 各列的固定宽度 —— **全局栅格，所有段共用同一套列顺序**。
///
/// `ui.horizontal` + 自然宽度会让每一列的起点随上一行的内容长度飘——
/// 文件名有长有短，于是「未拥有」这个词有的在这行第 20 个字符，有的在第
/// 12 个，整列读起来参差不齐。定宽是唯一能对齐的办法：名称列吃掉剩余
/// 空间，其余列按最长内容取上界。
///
/// **关键：栅格必须是全局的，不能每段各算各的剩余宽度。**
/// 曾经每段自己写`(avail - 各自用到的列宽之和)`：Rom 段后面挂 5 列
/// （state/icon/crc/region/from，402px），CHD 段只挂 3 列（186px），
/// 于是 CHD 段的名称列比Rom 段宽 216px，状态词的起点就**差了一整段距离**——
/// 面板上表现为"Rom 段的拥有在x=470，CHD 段的拥有在 x=686"。
/// 现在所有段都按 `GRID` 排，名称列统一吃 `avail - GRID`，
/// 后面的列自然全部落在同一条竖线上。
mod cols {
    /// 状态词列（`未审计` / `未拥有` / `坏 dump` / `无 dump`）——中文最宽的
    /// "未审计" 三个字，加点余量。
    pub const STATE: f32 = 60.0;
    /// CRC 列：`crc(78c15fa2)` 是定宽的等宽字体串。
    pub const CRC: f32 = 104.0;
    /// 区域 + tag 列（`maincpu` / `igs023:sprcol` / 描述 + `(tag)`）。
    pub const REGION: f32 = 132.0;
    /// 继承来源标记 `(pgm)` 的宽度上限。
    pub const FROM: f32 = 84.0;
    /// 状态图标列：16×16 的图 + 一点余量。
    pub const ICON: f32 = 22.0;

    /// 名称列右侧的内边距。
    ///
    /// 没有它，长文件名会紧贴"拥有"（截图里 `p060-ep1  拥有` 像一个词）。
    /// 12px 是"能看出是两列"又不浪费横向空间的量。
    pub const NAME_PAD: f32 = 12.0;

    /// 名称列之外**全部固定列的宽度之和**。
    ///
    /// 任何一段只要画到状态 / 图标，就必须用这个值来定位名称列宽度，
    /// 否则那段的状态词就会飘。少用的列留空（画个空白占位），不要缩减它——
    /// 缩减等于承认"这段例外"，而例外就是错位的来源。
    pub const GRID: f32 = STATE + ICON + CRC + REGION + FROM;

    /// 名称列宽度：吃掉所有剩余空间，并保证不小于一个可读的下限。
    ///
    /// 下限很重要：面板被拉窄时 `avail - GRID` 会变负，`add_sized` 收到负宽度
    /// 会把后面的列往回挤（错位）或直接不渲染。让名称列收缩、其余列保持
    /// 绝对位置，比让整行乱掉好。
    pub fn name_width(avail: f32) -> f32 {
        (avail - GRID).max(60.0)
    }
}

/// 画第三列的 16×16 状态图标，返回这次分配占的 `Response`。
///
/// 抽出来是因为 Rom / CHD / 引用设备 / Samples 四段都要画，而**必须走同一套
/// 逻辑**：取不到纹理时的退路（文字符号）也要一致，否则两段在同一个面板里
/// 会一个显示图标、一个显示方框。
///
/// 列宽 `cols::ICON` 里含 6px 余量，图标本身 16px。这里**不用 `add_sized`**
/// （它会居中，见 [`cell`]），而是先吃掉列宽的空白、再按 16px 左对齐放置，
/// 这样图标列的起点与其他列的起点严格对齐。
///
/// 返回 `Response` 是调用方忽略结果但签名需要，不是别的原因。
fn icon_cell(ui: &mut egui::Ui, state: RomState) -> egui::Response {
    let ctx = ui.ctx().clone();
    let font = egui::FontId::proportional(12.0);
    let color = state_color(state);
    // 先占满整列宽，再用它自己的 layout 把 16px 图标放到列首
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(cols::ICON, 16.0), egui::Sense::hover());
    let icon_rect = egui::Rect::from_min_size(rect.min, egui::vec2(16.0, 16.0));
    let drawn = state_icon(state).is_some_and(|n| icons::put(ui, &ctx, n, icon_rect));
    if !drawn {
        // 纹理还没解码完（第一帧）或图标名写错了：退回文字符号。列宽是定死
        // 的，这里的文字画在图标格中心，不会挤歪后面的列。
        ui.painter().text(
            icon_rect.center(),
            egui::Align2::CENTER_CENTER,
            mark_of(state),
            font,
            color,
        );
    }
    resp
}

/// 画一个**左对齐**的定宽单元格。
///
/// **不能用 `add_sized`——它会把内容居中。** 源码（egui 0.29 `ui.rs`）：
/// ```ignore
/// let layout = Layout::centered_and_justified(self.layout().main_dir());
/// self.allocate_ui_with_layout(max_size.into(), layout, |ui| ui.add(widget))
/// ```
/// `centered_and_justified` 意味着文字在给定宽度里**居中**。于是名称列拿到
/// 500px、文件名只有 70px 宽时，文字落在 215..285 而不是 0..70 —— 整列
/// 看起来缩进了一大截（实测名称起点在面板左边缘右侧 237px 处）。
/// 而且它对**同一段的所有列**都居中，短内容的列偏移各不相同，列与列之间
/// 也就对不齐了。
///
/// `allocate_ui_with_layout` + `Layout::left_to_right(Horizontal)` 才是
/// "占住w 像素、内容从左边开始"的语义。
fn cell(ui: &mut egui::Ui, w: f32, label: egui::Label) {
    if w <= 0.0 {
        return;
    }
    // `Layout::left_to_right(Align)` 的 `main_align` 是 **`Align::Center`**，
    // 主轴照样居中；参数只管交叉轴。所以这里直接改字段：主轴 `Min`（贴左）、
    // 交叉轴 `Min`（贴顶）。这才是"占住 w 像素、内容从左上角开始"。
    let layout = egui::Layout {
        main_dir: egui::Direction::LeftToRight,
        main_wrap: false,
        main_align: egui::Align::Min,
        main_justify: false,
        cross_align: egui::Align::Min,
        cross_justify: false,
    };
    ui.allocate_ui_with_layout(egui::vec2(w, 0.0), layout, |ui| ui.add(label));
}

/// 画一个**空占位**，宽度给定的列，用来在"这一段没有这一列"时保持栅格。
///
/// egui 的 `horizontal` 是按累加推进的：少画一列，后面的列就整体前移。
/// 所以 CHD 段（没有 region / from 列）必须用这个补上，否则它的 crc 会落在
/// 状态列的位置上，跨段就错位了。
fn gap(ui: &mut egui::Ui, w: f32) {
    if w > 0.0 {
        ui.add_space(w);
    }
}

/// Rom 段的一行：名称 / 状态 / 图标 / CRC / 区域(带 tag)。
///
/// **所有列定宽且左对齐**，理由见 [`cols`] 与 [`cell`]。
fn rom_line(ui: &mut egui::Ui, app: &MameApp, r: &RomRow) {
    let color = state_color(r.state);
    ui.horizontal(|ui| {
        // 名称：缺失时整行标红，所以名称本身也吃这个颜色。
        // 列宽减去 `NAME_PAD` —— 内边距放在**列宽里**而不是画在右侧，
        // 这样"名称列起点"和"状态列起点"都不受影响，栅格照样严丝合缝。
        cell(
            ui,
            (cols::name_width(ui.available_width()) - cols::NAME_PAD).max(40.0),
            egui::Label::new(egui::RichText::new(&r.name).monospace().color(color)),
        );
        // 状态词
        cell(
            ui,
            cols::STATE,
            egui::Label::new(egui::RichText::new(app.tr(state_word(r.state))).color(color)),
        );
        // 16×16 状态图标
        let _ = icon_cell(ui, r.state);
        // CRC
        cell(
            ui,
            cols::CRC,
            egui::Label::new(egui::RichText::new(crc_text(r.crc)).monospace().color(color)),
        );
        // 区域 + tag（`igs023:sprcol`）
        let region = match (&r.region, &r.tag) {
            (reg, Some(tag)) => format!("{reg}:{tag}"),
            (reg, None) => reg.clone(),
        };
        cell(
            ui,
            cols::REGION,
            egui::Label::new(egui::RichText::new(region).monospace().color(color)),
        );
        // 继承来的条目标一下来源，否则用户会以为这是本机种自己的文件
        let from = r.from.clone().map(|f| format!("({f})")).unwrap_or_default();
        cell(
            ui,
            cols::FROM,
            egui::Label::new(egui::RichText::new(from).small().color(ui_weak_color())),
        );
    });
}

/// 渲染整个视图。
///
/// `header_note` 是顶部那行说明（例如"来自审计缓存"或"刚刚重新审计"），
/// `None` 时不渲染那一行。
///
/// **只有数据，没有游戏名/描述/缺失计数。** 用户明确要求去掉顶部那一块
/// （游戏名 + 描述 + 说明 + "全部齐全"）——面板挂在游戏列表旁边，选中哪台
/// 一眼就看得见，重复一遍只是噪音；缺失与否在 Rom 段里每行都写着。
pub fn render(ui: &mut egui::Ui, app: &mut MameApp, view: &RomInfoView, header_note: Option<String>) {
    // 顶部只剩一行淡灰的说明（面板 / 弹窗的数据来源），其余全删。
    if let Some(note) = header_note {
        ui.label(egui::RichText::new(note).small().color(ui_weak_color()));
        ui.separator();
    }

    if view.is_empty() {
        ui.weak(app.tr("This game has no roms or disks."));
        return;
    }

    // Rom 段
    if !view.roms.is_empty() {
        section(ui, app, "Rom:");
        for r in &view.roms {
            ui.indent("rom_rows", |ui| rom_line(ui, app, r));
        }
    }

    // CHD 段：文件名 / 状态 / sha1。它只画到 crc 列，**剩下两列必须补空占位**，
    // 否则这段的状态词会比Rom 段靠右，整面板的"拥有"对不齐（历史 bug）。
    if !view.disks.is_empty() {
        section(ui, app, "Disks:");
        for d in &view.disks {
            let color = state_color(d.state);
            ui.indent("disk_rows", |ui| {
                ui.horizontal(|ui| {
                    cell(
                        ui,
                        (cols::name_width(ui.available_width()) - cols::NAME_PAD).max(40.0),
                        egui::Label::new(
                            egui::RichText::new(&d.file_name).monospace().color(color),
                        ),
                    );
                    cell(
                        ui,
                        cols::STATE,
                        egui::Label::new(
                            egui::RichText::new(app.tr(state_word(d.state))).color(color),
                        ),
                    );
                    let _ = icon_cell(ui, d.state);
                    let short = if d.sha1.len() > 8 {
                        d.sha1[..8].to_string()
                    } else {
                        d.sha1.clone()
                    };
                    cell(
                        ui,
                        cols::CRC,
                        egui::Label::new(
                            egui::RichText::new(format!("sha1({short})"))
                                .monospace()
                                .color(color),
                        ),
                    );
                    // 补齐栅格后两列
                    gap(ui, cols::REGION);
                    gap(ui, cols::FROM);
                });
            });
        }
    }

    // Bios 段：集名 + 描述 + 状态，**下面缩进列这一套实际的文件**。
    // 文件用 `rom_line`（与 Rom 段同一套列宽），所以整段的图标/ crc 也对齐。
    // 集标题行也走栅格——用 `ui.label` 自然宽度的话，集名一长就把状态词
    // 推到右边，看起来又是错位的。
    if !view.bios.is_empty() {
        section(ui, app, "Bios:");
        for b in &view.bios {
            let color = state_color(b.state);
            ui.horizontal(|ui| {
                cell(
                    ui,
                    (cols::name_width(ui.available_width()) - cols::NAME_PAD).max(40.0),
                    egui::Label::new(egui::RichText::new(&b.name).monospace().strong().color(color)),
                );
                cell(
                    ui,
                    cols::STATE,
                    egui::Label::new(egui::RichText::new(app.tr(state_word(b.state))).color(color)),
                );
                // 描述占 crc + region 两列的宽度（描述比 crc 长得多）
                cell(
                    ui,
                    cols::CRC + cols::REGION,
                    egui::Label::new(
                        egui::RichText::new(&b.description)
                            .small()
                            .color(ui_weak_color()),
                    ),
                );
                gap(ui, cols::ICON);
                gap(ui, cols::FROM);
            });
            for r in &b.roms {
                ui.indent("bios_rom_rows", |ui| rom_line(ui, app, r));
            }
        }
    }

    // 引用设备段：先列设备机种 + 状态 + 图标，再缩进列它们的 rom 文件。
    // rom 文件走 `rom_line`，所以设备的文件明细与 Rom 段列宽完全一致。
    if !view.devices.is_empty() || !view.device_roms.is_empty() {
        section(ui, app, "Referenced devices:");
        for d in &view.devices {
            let color = state_color(d.state);
            ui.horizontal(|ui| {
                cell(
                    ui,
                    (cols::name_width(ui.available_width()) - cols::NAME_PAD).max(40.0),
                    egui::Label::new(egui::RichText::new(&d.name).monospace().color(color)),
                );
                cell(
                    ui,
                    cols::STATE,
                    egui::Label::new(
                        egui::RichText::new(app.tr(state_word(d.state))).color(color),
                    ),
                );
                let _ = icon_cell(ui, d.state);
                // 描述 + 引用它的 tag（`:maincpu` / `igs023:sprcol`）。tag
                // 是**这台机器里的引用名**，去掉前导冒号才是用户认的写法。
                let mut tail = String::new();
                if !d.description.is_empty() {
                    tail.push_str(&d.description);
                }
                let tag = d.tag.trim_start_matches(':');
                if !tag.is_empty() {
                    if !tail.is_empty() {
                        tail.push_str("  ");
                    }
                    tail.push_str(&format!("({tag})"));
                }
                // 前导空格：这一列紧跟在16px 图标后面，不留的话描述会贴着
                // 图标看成一团
                let tail = format!(" {tail}");
                cell(
                    ui,
                    cols::CRC + cols::REGION,
                    egui::Label::new(
                        egui::RichText::new(tail)
                            .small()
                            .color(ui_weak_color()),
                    ),
                );
                gap(ui, cols::FROM);
            });
            // 设备自己的 rom：按设备机种名匹配回去（`device_roms` 的
            // `from` 就是设备机种名）
            for r in view.device_roms.iter().filter(|r| r.from.as_deref() == Some(d.name.as_str())) {
                ui.indent("device_rom_rows", |ui| rom_line(ui, app, r));
            }
        }
        // 有 rom 但设备机种不在库里（裁剪过的 dat）——仍要把文件列出来
        for r in view
            .device_roms
            .iter()
            .filter(|r| !view.devices.iter().any(|d| Some(d.name.as_str()) == r.from.as_deref()))
        {
            ui.indent("device_rom_orphan", |ui| rom_line(ui, app, r));
        }
    }

    // 设备段：`<device type="memcard" tag="memcard_p1">` 这种**可挂载**的
    // 设备，跟上面的引用设备不是一回事——它没有 rom 要校验，用户关心的是
    // "这游戏支持插什么卡、插什么文件"。所以这一段没有状态列，只有
    // 类型 / 实例 / 扩展名；**前面补一段空占位**，让名称列的起点与上面
    // 那些段落在同一条竖线上（缩进已经保证了左边距，再补空列就成了双重
    // 缩进，所以这里补的是状态+图标那两列的宽度）。
    if !view.slots.is_empty() {
        section(ui, app, "Device slots:");
        for s in &view.slots {
            ui.indent("slot_rows", |ui| {
                ui.horizontal(|ui| {
                    // 让"类型"列的起点 = 名称列起点：先吃掉状态列的宽度
                    gap(ui, cols::STATE);
                    let avail = ui.available_width();
                    // 类型：槽位的身份（`memcard`）
                    cell(
                        ui,
                        avail * 0.3,
                        egui::Label::new(
                            egui::RichText::new(&s.kind).monospace().color(ui_weak_color()),
                        ),
                    );
                    // 实例：命令行 `-memcard1` 用的就是它
                    cell(
                        ui,
                        avail * 0.3,
                        egui::Label::new(
                            egui::RichText::new(&s.instance).monospace().color(ui_weak_color()),
                        ),
                    );
                    // 扩展名：逗号连接
                    cell(
                        ui,
                        avail * 0.4,
                        egui::Label::new(
                            egui::RichText::new(&s.extensions).monospace().color(ui_weak_color()),
                        ),
                    );
                });
            });
        }
    }

    //槽位段：`<slot name="ctrl1">` + `<slotoption>`，MAME 的槽位声明。
    //与上面那段并存 —— 实测 `nes` 同时有 9 个 `<device>` 和 12 个 `<slot>`，
    //前者说"这台机器带什么设备"，后者说"这里能插什么"。同样没有状态列。
    if !view.slot_decls.is_empty() {
        section(ui, app, "Slots:");
        for s in &view.slot_decls {
            ui.indent("slot_decl_rows", |ui| {
                ui.horizontal(|ui| {
                    // 与设备段一致：补状态列的宽度，让槽位名落在名称列起点上
                    gap(ui, cols::STATE);
                    let avail = ui.available_width();
                    // 槽位名：命令行 `nes:ctrl1=<dev>` 用的就是它
                    cell(
                        ui,
                        cols::FROM,
                        egui::Label::new(
                            egui::RichText::new(&s.name).monospace().color(ui_weak_color()),
                        ),
                    );
                    // 可选设备数；空槽位（`nes_slot`）显示 0，不留空
                    let count = egui::RichText::new(format!("{}", s.option_count))
                        .monospace()
                        .color(ui_weak_color());
                    cell(ui, 40.0, egui::Label::new(count));
                    // 选项名清单，空槽位给个明确的"—"而不是空白
                    let text = if s.options.is_empty() {
                        "-".to_string()
                    } else {
                        s.options.clone()
                    };
                    cell(
                        ui,
                        (avail - cols::FROM - 40.0).max(40.0),
                        egui::Label::new(
                            egui::RichText::new(text)
                                .monospace()
                                .color(ui_weak_color()),
                        ),
                    );
                });
            });
        }
    }

    // Samples 段：样本机种名 + 拥有数 / 总数 + 状态 + 图标。
    // 列宽与 Rom 段共用，让整面板的状态词、图标落在同一条竖线上。
    if !view.samples.is_empty() {
        section(ui, app, "Samples:");
        for s in &view.samples {
            let color = state_color(s.state);
            ui.indent("sample_rows", |ui| {
                ui.horizontal(|ui| {
                    cell(
                        ui,
                        (cols::name_width(ui.available_width()) - cols::NAME_PAD).max(40.0),
                        egui::Label::new(egui::RichText::new(&s.name).monospace().color(color)),
                    );
                    cell(
                        ui,
                        cols::STATE,
                        egui::Label::new(
                            egui::RichText::new(app.tr(state_word(s.state))).color(color),
                        ),
                    );
                    let _ = icon_cell(ui, s.state);
                    cell(
                        ui,
                        cols::CRC,
                        egui::Label::new(
                            egui::RichText::new(format!("{}/{}", s.have, s.total))
                                .monospace()
                                .color(color),
                        ),
                    );
                    // 补齐栅格后两列
                    gap(ui, cols::REGION);
                    gap(ui, cols::FROM);
                });
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

    /// 用户指定的四态配色必须真的不同，否则"未拥有"和"未审计"在界面上一样，
    /// 用户会以为自己的盘空了。
    ///
    /// 灰色是 `nodump` 与 `unknown` **共用**的：两者都不是坏消息，靠文字
    /// 区分（"无 dump" / "未审计"），颜色上再分成两档只会让人以为"无 dump"
    /// 也是要处理的问题。
    #[test]
    fn the_four_user_facing_colours_are_distinct() {
        let owned = state_color(RomState::Good);
        let baddump = state_color(RomState::BadDump);
        let not_owned = state_color(RomState::Missing);
        let grey = state_color(RomState::NoDump);
        assert_eq!(owned, icons::GREEN, "拥有=绿");
        assert_eq!(baddump, icons::YELLOW, "坏 dump=黄");
        assert_eq!(not_owned, icons::RED, "未拥有=红");
        assert_ne!(owned, not_owned);
        assert_ne!(owned, baddump);
        assert_ne!(baddump, not_owned);
        assert_ne!(not_owned, grey, "未拥有和 nodump 不能同色");
        assert_eq!(
            state_color(RomState::Unknown),
            grey,
            "未审计与 nodump 共用灰色"
        );
    }

    /// 五种状态都必须有图标名，且**互不相同**——共用图标等于让用户分不清
    /// 那一列到底在说什么。
    #[test]
    fn every_state_has_its_own_icon() {
        let mut names: Vec<&str> = [
            RomState::Good,
            RomState::BadDump,
            RomState::Missing,
            RomState::NoDump,
            RomState::Unknown,
        ]
        .iter()
        .map(|s| state_icon(*s).expect("每个状态都要有图标"))
        .collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "图标不能重复：{names:?}");
    }

    /// **所有段的状态列必须落在同一条竖线上。**
    ///
    /// 这条钉的是一个已经真刀真枪发生过的错位：Rom 段后面挂 5 列、CHD 段只
    /// 挂 3 列，两段各自用 `(avail - 自己用到的列宽之和)` 当名称列宽度，
    /// 于是 CHD 段的状态词比 Rom 段右移了 216px —— 面板上两段"拥有"上下
    /// 错开，肉眼一看就是"没对齐"。
    ///
    /// 现在名称列统一吃 `avail - GRID`，缺的列用 `gap()` 补占位，所以
    /// 任何一段的状态列起点都等于 `name_width + 其缩进`。
    #[test]
    fn the_name_column_is_identical_across_every_section() {
        // 面板宽度无关紧要：重要的是**各段用的是同一个函数**，而不是各自
        // 算一遍。列宽从哪来、写在哪，都得是同一处。
        for avail in [400.0_f32, 600.0, 900.0, 1400.0] {
            let w = cols::name_width(avail);
            assert_eq!(
                w,
                cols::name_width(avail),
                "同样宽度下必须给出同样结果：{avail}"
            );
            // 名称列不能吃掉固定列的位置，否则后面的列被推走
            assert!(
                w + cols::GRID <= avail.max(cols::GRID + 60.0),
                "名称列 + 固定列不能超出可用宽度：{avail} -> {w}"
            );
        }
        // GRID 必须是五列之和（曾经漏算一列，导致列宽对不上）
        assert_eq!(cols::GRID, cols::STATE + cols::ICON + cols::CRC + cols::REGION + cols::FROM);
        // 面板拉窄时名称列不许变负——负宽度会把后面的列往回挤，比窄更糟
        assert!(cols::name_width(10.0) >= 60.0, "窄面板下要有下限");
    }
}
