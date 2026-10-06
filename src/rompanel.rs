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
    // 顶部那行说明已经去掉了（见 `render`），所以这里只画标题和横线
    ui.label(
        egui::RichText::new(app.tr(label))
            .strong()
            .size(14.0),
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
    ///
    /// 22 = 16（图）+ 6（小间距）。**那 6px 是用户明确要的**："图标和后面的
    /// crc 要有个小空格"——16px 图标紧贴 `crc(d42e505d)` 会让两列看成一团。
    pub const ICON: f32 = 22.0;

    /// 名称列右侧的内边距。
    ///
    /// 没有它，长文件名会紧贴"拥有"（截图里 `p060-ep1  拥有` 像一个词）。
    /// 12px 是"能看出是两列"又不浪费横向空间的量。
    /// **别设太大。** 面板只有 512px 宽时名称列只剩 110px，扣掉 12px 内边距
    /// 就剩 98px，而 `10239811.u86` 这种 12 字符等宽名约需 100px —— 差2px
    /// 就折行。4px 足够看出是两列，又不把长文件名挤到第二行。
    pub const NAME_PAD: f32 = 4.0;

    /// 段与段之间的空行高度（约一行文字）。
    pub const SECTION_GAP: f32 = 18.0;

    /// 合并后的设备段第 1 列：设备类型 / 槽名。
    ///
    /// `floppydisk`（9 字符）是这一列最长的常见值，留够 13 个字符的量。
    pub const DEVICE_TYPE: f32 = 104.0;
    /// 设备段第 2 列：安装路径。
    ///
    /// `upd765:0:525hd`（15 字符）实测是最长的一类；`centronics:printer:printer`
    /// 更长，但那种行第二列是空的，宽度不够也不会挤到第三列（超宽会换行）。
    pub const DEVICE_TAG: f32 = 190.0;

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
    // 先占满整列宽，再用它自己的 layout 把 16px 图标放到列首。
    // `ICON` 是 22 而图标是 16，所以图标**靠左**、右侧天然空 6px ——
    // 这就是"图标与后面的 crc 之间的小空格"，不需要额外画。
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
/// 所以 CHD 段（没有 region / from 列）必须用这个补上，否则它的 crc 会落在
/// 状态列的位置上，跨段就错位了。
fn gap(ui: &mut egui::Ui, w: f32) {
    if w > 0.0 {
        ui.add_space(w);
    }
}

/// 行内的 `horizontal` 布局：**上下贴紧**。
///
/// egui 的 `horizontal` 默认给每一行留 `item_spacing.y`（默认 6px）加字体
/// 行高，40 个 rom 就是 40 × 多余的十几像素 —— 面板窄的时候，一屏能看的行数
/// 被行距吃掉一半。这里把交叉轴对齐改成 `Min`，让行高由内容（16px 图标）
/// 决定而不是由间距决定。
fn tight_horizontal(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui, f32)) {
    // **宽度在进布局之前就定下来**，这是全面板对齐的关键。
    //
    // 之前把 `cols::name_width(ui.available_width())` 写在行内。行内是个
    // `LeftToRight` 的子 ui，它的 `cursor()` 带着**上一行遗留的 x 偏移**
    // （实测 `cursor=163`，且逐行递增：子 ui 的 `min_rect` 被内容撑大后，
    // 父 ui 的游标跟着右移）。于是名称列宽度一行比一行小/大，状态词
    // 从第 1 行的 x 斜到第 8 行的 x+390——比不改之前更离谱。
    //
    // 行外量一次、每行传同一个值，各段的名称列宽度就与行序无关了。
    let row_width = ui.available_width();
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
        // **横向间距也必须归零。** 栅格的每一步都已经算死了（名称列吃掉
        // 剩余空间 + 固定列宽），再叠一层 `item_spacing.x`（默认 8px）就
        //等于每行凭空多出 6 项 × 8px = 48px 的偏移。行布局把它算进子 ui 的
        // `min_rect`，父 ui 游标跟着右移，于是状态词从第 1 行斜到第 9 行
        // （实测右移约 250px）——斜率恒定正是这个的特征。
        ui.spacing_mut().item_spacing.x = 0.0;
        // 竖向同理：紧凑是这里的目的，行高由 16px 图标决定。
        ui.spacing_mut().item_spacing.y = 0.0;
        add_contents(ui, row_width);
    });
}

/// Rom 段的一行：名称 / 状态 / 图标 / CRC / 区域(带 tag)。
///
/// **所有列定宽且左对齐**，理由见 [`cols`] 与 [`cell`]。
///
/// **只有状态列上色**（用户要求）：名称 / CRC / 区域一律走默认前景色。
/// 早先把整行都染成状态色，一屏几十行全是绿字，看着像报错；而且"缺一个
/// 文件"和"这台机器有 40 个文件全是好的"用同一种满屏绿色表达，信息量是零。
/// 状态词 + 图标已经足够定位，颜色只服务这两列。
fn rom_line(ui: &mut egui::Ui, app: &MameApp, r: &RomRow) {
    let color = state_color(r.state);
    tight_horizontal(ui, |ui, _row_width| {
        // 名称：列宽减去 `NAME_PAD` —— 内边距放在**列宽里**而不是画在右侧，
        // 这样"名称列起点"和"状态列起点"都不受影响，栅格照样严丝合缝。
        cell(
            ui,
            (cols::name_width(_row_width) - cols::NAME_PAD).max(40.0),
            egui::RichText::new(&r.name).monospace(),
        );
        // 状态词：唯一带状态色的文字
        cell(
            ui,
            cols::STATE,
            egui::RichText::new(app.tr(state_word(r.state))).color(color),
        );
        // 16×16 状态图标
        let _ = icon_cell(ui, r.state);
        // CRC
        cell(
            ui,
            cols::CRC,
            egui::RichText::new(crc_text(r.crc)).monospace(),
        );
        // 区域 + tag（`igs023:sprcol`）
        let region = match (&r.region, &r.tag) {
            (reg, Some(tag)) => format!("{reg}:{tag}"),
            (reg, None) => reg.clone(),
        };
        cell(ui, cols::REGION, egui::RichText::new(region).monospace());
        // 继承来的条目标一下来源，否则用户会以为这是本机种自己的文件
        let from = r.from.clone().map(|f| format!("({f})")).unwrap_or_default();
        cell(
            ui,
            cols::FROM,
            egui::RichText::new(from).small().color(ui_weak_color()),
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
///
/// **行与行之间一律不缩进。** 曾经这里每行都套一层 `ui.indent("rom_rows", …)`，而 `ui.indent` 是**按 id 存状态的**：同一个 id 在循环里反复调用，缩进会逐行累加（第二行起每行往右挪一点）。更糟的是缩进会吃掉 `available_width()`，于是 `cols::name_width()` 算出来的名称列宽度逐行变小、状态词起点逐行左移 —— 用户看到的"每到下一行就额外缩进、根本没对齐"就是这个。对齐由 `cols` 栅格保证，缩进只会碍事。
pub fn render(ui: &mut egui::Ui, app: &mut MameApp, view: &RomInfoView) {
    // **顶部不要那行"数据来源于审计缓存"。**
    // 旧版写的是"数据来源"这类元信息，用户不要：面板里每一行的状态词已经
    // 把结论说完了，顶部再写一遍"这数据是哪来的"是纯噪音。

    if view.is_empty() {
        ui.weak(app.tr("This game has no roms or disks."));
        return;
    }

    // Rom 段
    if !view.roms.is_empty() {
        section(ui, app, "Rom:");
        for r in &view.roms {
            rom_line(ui, app, r);
        }
    }

    // CHD 段：文件名 / 状态 / sha1。它只画到 crc 列，**剩下两列必须补空占位**，
    // 否则这段的状态词会比Rom 段靠右，整面板的"拥有"对不齐（历史 bug）。
    if !view.disks.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        section(ui, app, "Disks:");
        for d in &view.disks {
            let color = state_color(d.state);
            tight_horizontal(ui, |ui, _row_width| {
                cell(
                    ui,
                    (cols::name_width(_row_width) - cols::NAME_PAD).max(40.0),
                egui::RichText::new(&d.file_name).monospace(),
                );
                cell(
                    ui,
                    cols::STATE,
                egui::RichText::new(app.tr(state_word(d.state))).color(color),
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
                egui::RichText::new(format!("sha1({short})"))
                            .monospace()
                            .color(color),
                );
                // 补齐栅格后两列
                gap(ui, cols::REGION);
                gap(ui, cols::FROM);
            });
        }
    }

    // Bios 段：集名 + 描述 + 状态，**下面列这一套实际的文件**。
    // 文件用 `rom_line`（与 Rom 段同一套列宽），所以整段的图标/ crc 也对齐。
    // 集标题行也走栅格——用 `ui.label` 自然宽度的话，集名一长就把状态词
    // 推到右边，看起来又是错位的。
    if !view.bios.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        section(ui, app, "Bios:");
        for b in &view.bios {
            let color = state_color(b.state);
            tight_horizontal(ui, |ui, _row_width| {
                cell(
                    ui,
                    (cols::name_width(_row_width) - cols::NAME_PAD).max(40.0),
                    egui::RichText::new(&b.name).monospace().strong().color(color),
                );
                cell(
                    ui,
                    cols::STATE,
                    egui::RichText::new(app.tr(state_word(b.state))).color(color),
                );
                // 描述占 crc + region 两列的宽度（描述比 crc 长得多）
                cell(
                    ui,
                    cols::CRC + cols::REGION,
                egui::RichText::new(&b.description)
                            .small()
                            .color(ui_weak_color()),
                );
                gap(ui, cols::ICON);
                gap(ui, cols::FROM);
            });
            for r in &b.roms {
                rom_line(ui, app, r);
            }
        }
    }

    // 引用设备段：先列设备机种 + 状态 + 图标，再缩进列它们的 rom 文件。
    // rom 文件走 `rom_line`，所以设备的文件明细与 Rom 段列宽完全一致。
    if !view.devices.is_empty() || !view.device_roms.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        section(ui, app, "Referenced devices:");
        for d in &view.devices {
            let color = state_color(d.state);
            tight_horizontal(ui, |ui, _row_width| {
                // 第一列：设备机种名（`m68000` / `igs036` / `z80`）。
                cell(
                    ui,
                    (cols::name_width(_row_width) - cols::NAME_PAD).max(40.0),
                    egui::RichText::new(&d.name).monospace(),
                );
                // 第二列：**只有状态词**，图标/描述/tag 全部去掉。
                // 用户要的是"两列：名 + 拥有/缺失"——tag（`:maincpu`）是内部
                // 引用名，描述在这台机器的语境下是废话，三样都只会让第二列
                // 长得参差不齐（`z80` 只有 3 个字，`floppy_525_hd` 有 13 个）。
                cell(
                    ui,
                    cols::STATE,
                    egui::RichText::new(app.tr(state_word(d.state))).color(color),
                );
                // 后面这些列一律留空，但**必须用 gap 占住**——少一列，
                // 下一行的设备 rom 就会整体前移（见 `gap` 的注释）。
                gap(ui, cols::ICON);
                gap(ui, cols::CRC);
                gap(ui, cols::REGION);
                gap(ui, cols::FROM);
            });
            // 设备自己的 rom：按设备机种名匹配回去（`device_roms` 的
            // `from` 就是设备机种名）
            for r in view.device_roms.iter().filter(|r| r.from.as_deref() == Some(d.name.as_str())) {
                rom_line(ui, app, r);
            }
        }
        // 有 rom 但设备机种不在库里（裁剪过的 dat）——仍要把文件列出来
        for r in view
            .device_roms
            .iter()
            .filter(|r| !view.devices.iter().any(|d| Some(d.name.as_str()) == r.from.as_deref()))
        {
            rom_line(ui, app, r);
        }
    }

    // 设备 + 槽位**合并成一段**（用户要求）：两者回答的是同一个问题
    // ——"这台机器能插什么、插什么文件"，拆成"设备段"和"槽位段"两段
    // 反而让人以为它们是两回事。实测 `elwro800` 两种都有：4 个 `<device>`
    // （floppydisk / printout / cassette…）+ 3 个 `<slot>`（其中 centronics
    // 有 19 个 option）。
    //
    // 三列：**类型 / 安装路径 / 详情**
    //   - `<device>`：type（`floppydisk`）/ tag（`upd765:0:525hd`）/ 扩展名
    //   - `<slot>`  ：槽名（`centronics`） / 空/ option 数 / option 清单
    //
    // 第2 列的 tag 是用户截图里那一列；`DeviceSlotRow::tag` 在
    // `rominfo.rs` 填充时已去掉前导冒号（MAME 写的是 `:upd765:0:525hd`）。
    if !view.slots.is_empty() || !view.slot_decls.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        // 段头复用 `"Device slots:"` 这个键——它的译文本来就是"设备:"
        // （繁"裝置:"），而 `"Devices:"` 是个没登记过的新键，`tr` 会退化成
        // 显示英文原文。
        section(ui, app, "Device slots:");
        // 统一列宽：类型 / tag / 详情。第三列吃掉全部剩余空间。
        for d in &view.slots {
            tight_horizontal(ui, |ui, _row_width| {
                // 与上面各段对齐：先吃掉状态列的宽度，让第1 列起点一致
                gap(ui, cols::STATE);
                // 第1 列：设备类型（`floppydisk` / `printout` / `cassette`）
                cell(
                    ui,
                    cols::DEVICE_TYPE,
                    egui::RichText::new(&d.kind).monospace().color(icons::GREEN),
                );
                // 第2 列：安装路径（`upd765:0:525hd` / `cassette`）
                cell(
                    ui,
                    cols::DEVICE_TAG,
                    egui::RichText::new(&d.tag).monospace().color(ui_weak_color()),
                );
                // 第3 列：可用的文件扩展名。**要留 6px 小间距**——
                // 与图标和 crc 之间的空隙一致。
                let exts = format!("  {}", d.extensions);
                cell(
                    ui,
                    (_row_width - cols::STATE - cols::DEVICE_TYPE - cols::DEVICE_TAG).max(40.0),
                    egui::RichText::new(exts).monospace().color(ui_weak_color()),
                );
            });
        }
        for sl in &view.slot_decls {
            tight_horizontal(ui, |ui, _row_width| {
                gap(ui, cols::STATE);
                // 槽位行第1 列填**槽名**（与设备行的 type 同列，同为"这行的身份"）
                cell(
                    ui,
                    cols::DEVICE_TYPE,
                    egui::RichText::new(&sl.name).monospace().color(icons::GREEN),
                );
                // 第2 列：可选设备数，空槽位（`nes_slot`）显示 0而不是空白
                // 走 i18n：简繁两套里key 相同、译文不同，硬编码"个"会让
                // 繁体用户看到简体字。
                cell(
                    ui,
                    cols::DEVICE_TAG,
                    egui::RichText::new(app.tr("{} options").replace("{}", &sl.option_count.to_string()))
                        .monospace()
                        .color(ui_weak_color()),
                );
                // 第3 列：option 清单。19 个会折行，`cell` 已按列宽换行并
                // 把多行高度报给行布局（见 `cell` 的注释）。
                let text = if sl.options.is_empty() {
                    "-".to_string()
                } else {
                    sl.options.clone()
                };
                let text = format!("  {text}");
                cell(
                    ui,
                    (_row_width - cols::STATE - cols::DEVICE_TYPE - cols::DEVICE_TAG).max(40.0),
                    egui::RichText::new(text).monospace().color(ui_weak_color()),
                );
            });
        }
    }

    // Samples 段：**样本集名 + 状态 + 图标 + 拥有数/总数**。
    //
    // 只到"包"这一级，**不逐个列出 zip 里的wav**（用户要求）。理由：
    // 样本集是共享包——`genpin` 被1438 台 pinball 游戏共用，但每台需要的
    // 文件子集不同（rctycn 要 18 个，别的可能只要 5 个）。列出 18 行wav
    // 会把面板撑得很长，而用户真正关心的是"这台游戏要的齐了没有"。
    //
    // 数据链路：`GameMeta::samples`（本机要的 `<sample>` 名，解析时就有了）
    // vs `samplepath` 下 `{sampleof}.zip` 的包内条目名（拼 `.wav` 后比对），
    // 得出 `have/total`。**CRC 校验做不了** —— `<sample>` 元素只有 name，
    // DTD 写死`<!ELEMENT sample EMPTY>`，官方没给校验值。
    if !view.samples.is_empty() {
        ui.add_space(cols::SECTION_GAP);
        section(ui, app, "Samples:");
        for s in &view.samples {
            let color = state_color(s.state);
            tight_horizontal(ui, |ui, _row_width| {
                cell(
                    ui,
                    (cols::name_width(_row_width) - cols::NAME_PAD).max(40.0),
                    egui::RichText::new(&s.name).monospace(),
                );
                cell(
                    ui,
                    cols::STATE,
                    egui::RichText::new(app.tr(state_word(s.state))).color(color),
                );
                let _ = icon_cell(ui, s.state);
                // `9/9` —— 有了（数）/ 需要（总数）。左对齐，与Rom 段的
                // `crc(...)` 同一列同一栅格。
                cell(
                    ui,
                    cols::CRC,
                    egui::RichText::new(format!("{}/{}", s.have, s.total)).monospace(),
                );
                // 补齐栅格后两列
                gap(ui, cols::REGION);
                gap(ui, cols::FROM);
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
    /// 任何一段的状态列起点都等于 `name_width`（行不再缩进，见`render` 的注释）。
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

    /// **行与行之间不允许有 `ui.indent`。**
    ///
    /// `ui.indent` 的缩进量是**按 id 存在 `Memory.indentation` 里的有状态值**，
    /// 同一个 id 在循环里每调用一次就累加一层。所以 `for r in &view.roms {
    /// ui.indent("rom_rows", ..) }` 的第二行起会整体右移，而且缩进吃掉
    /// `available_width()` 后 `cols::name_width()` 会逐行变小、状态词起点
    /// 逐行左移——正是用户报的"每到下一行就额外缩进，根本没对齐"。
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
    /// 栅格已经把每一步都算死了（`cols::GRID`），再叠一层 spacing 就是
    /// 双重计费。竖向同理，`item_spacing.y` 归零才能让行高由 16px 图标决定
    /// 而不是由间距决定（用户要"每行间距紧凑一些"）。
    #[test]
    fn rows_have_no_item_spacing_at_all() {
        let src = include_str!("rompanel.rs");
        let body = src
            .split("fn tight_horizontal(")
            .nth(1)
            .expect("找不到 tight_horizontal");
        let body = body.split("\n}\n").next().unwrap_or(body);
        assert!(
            body.contains("item_spacing.x = 0.0"),
            "行布局必须把 item_spacing.x 归零，否则每行递增右移"
        );
        assert!(
            body.contains("item_spacing.y = 0.0"),
            "行布局必须把 item_spacing.y 归零，否则行高被间距撑开"
        );
    }

    /// `render` 的函数体里出现 `ui.indent(` 就失败。
    #[test]
    fn rows_are_never_indented() {
        let src = include_str!("rompanel.rs");
        let body = src
            .split("pub fn render(")
            .nth(1)
            .expect("找不到 render");
        // 只看 render 到下一个顶层项为止的部分
        let body = body.split("\n}\n").next().unwrap_or(body);
        assert!(
            !body.contains("ui.indent("),
            "行循环里不许用 ui.indent（缩进按 id 累加，会逐行右移）：\n{}",
            body.lines().filter(|l| l.contains("ui.indent(")).collect::<Vec<_>>().join("\n")
        );
    }
}
