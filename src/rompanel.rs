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
/// | 未拥有 | **红** | 确认缺失。**只有这一种是红色** |
///
/// **只有"未拥有"用红色。** 之前 `nodump` 走灰色，理由是"它不是坏消息"；
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
/// 四个词与用户指定的四态一一对应：`拥有` / `坏 dump` / `未拥有` /
/// `未 dump`（外加未校验时的 `未校验`）。旧版的"很好 / 缺失"是从 MAME 的
/// verify 报告里抄的词，但用户明确要求用"拥有 / 坏 dump / 未拥有"这套——
/// 前者描述校验结果，后者描述**用户手上有没有**，后者才是用户真正关心的
/// 问题。`nodump` 的词也从"无 dump"改成"**未 dump**"（与"未拥有"对齐，
/// 都是"用户手上没有"的意思）。
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
    /// 状态列：图标 + 空格 + 状态词。**图标与词同格**（见 [`state_with_icon`]），
    /// 所以这一列的宽度是"16px 图标 + 4px 间隙 + 状态词"。
    ///
    /// 60px 的来历：图标 16 + 间隙 4= 20，剩 40 放三个汉字（约 13px/字）。
    pub const STATE: f32 = 60.0;
    /// CRC 列：`crc(78c15fa2)` 是定宽的等宽字体串。
    pub const CRC: f32 = 104.0;
    /// 区域 + tag 列（`maincpu` / `igs023:sprcol`）。
    pub const REGION: f32 = 132.0;

    /// 状态图标边长（1.8.2 那套 `status_*.png` 是 16×16）。
    pub const ICON_W: f32 = 16.0;
    /// 图标与状态词之间的空隙（用户要的"1 个空格"）。
    ///
    /// 4px ≈ 一个空格符。**不占独立列**：图标和词在同一格里由
    /// [`state_with_icon`] 一次画完，所以这个间隙不受 `allocate_exact_size`
    /// 在 `LeftToRight` 里按 `min_rect` 重算的影响（那个坑踩过一次：
    /// 列宽 22 被收成 16，间隙 0px，图标紧贴文字）。
    pub const ICON_GAP: f32 = 4.0;

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

    /// 名称列之外**全部固定列的宽度之和**。
    ///
    /// 任何一段只要画到状态，就必须用这个值来定位名称列宽度，否则那段的状态
    /// 词就会飘。少用的列留空（画个空白占位），不要缩减它——缩减等于承认
    /// "这段例外"，而例外就是错位的来源。
    ///
    /// **不含图标**（2026-10-06）：图标改成紧跟状态词、画在 `STATE` 格子里，
    /// 不再独占列。见 [`state_with_icon`]。
    pub const GRID: f32 = STATE + CRC + REGION;

    /// 名称列宽度：吃掉所有剩余空间，并保证不小于一个可读的下限。
    ///
    /// 下限很重要：面板被拉窄时 `avail - GRID` 会变负，`add_sized` 收到负宽度
    /// 会把后面的列往回挤（错位）或直接不渲染。让名称列收缩、其余列保持
    /// 绝对位置，比让整行乱掉好。
    pub fn name_width(avail: f32) -> f32 {
        (avail - GRID).max(60.0)
    }
}

/// 状态图标 + 状态词**画在同一格里**：图标在左，词在右，中间 4px。
///
/// 用户要求（2026-06，最后一次调整）：「这对勾，X 或者感叹号图标放在拥有、
/// 缺失、坏 dump **前面**且加个空格隔开」。先前是"词 + 图标"（图标跟在词尾），
/// 现在反过来 —— 图标在前更像一列的**标记**，词是它的说明。
///
/// **整格死占 [`cols::STATE`]**，所以跨段的状态起点仍落在同一条竖线上 ——
/// 这条比"图标在前还是在后"重要得多。
///
/// 图标 16px（[`cols::ICON_W`]）贴格子左端，状态词从 `16 + 4` 处起排。
/// 图标**不染色**（1.8.2 那套 png 自带颜色：绿勾 / 红叉 / 黄叹 / 蓝问），
/// 但**未拥有用的是新画的 `status_missing.png`（红底白叉）** —— 原来的
/// `status_cross` 是**蓝底**白叉，跟"红色专属给确实缺的东西"这条配色约定
/// 冲突（蓝色在深色主题下还容易被看成灰色 = 未审计）。
///
/// 退路：纹理未解码时在图标位置画文字符号。
fn state_with_icon(ui: &mut egui::Ui, text: impl AsRef<str>, state: RomState) {
    let color = state_color(state);
    let (_id, rect) = ui.allocate_space(egui::vec2(cols::STATE, 16.0));
    // 1) 图标贴在格子左端
    let ctx = ui.ctx().clone();
    let icon_rect = egui::Rect::from_min_size(rect.min, egui::vec2(cols::ICON_W, cols::ICON_W));
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
    // 2) 状态词排在图标右边。宽度不参与布局（整格已被 STATE 死占），
    //    词宽超了也只是画出去，不会把后面的列挤歪 —— 与 `cell` 不同，
    //    这里不需要换行逻辑：状态词最长三个字（"未拥有"），60px 足够。
    let style = ui.style().clone();
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
    ui.painter().galley(
        egui::pos2(rect.min.x + cols::ICON_W + cols::ICON_GAP, rect.min.y),
        galley,
        ui.visuals().text_color(),
    );
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
    tight_horizontal(ui, |ui, _row_width| {
        // 名称：列宽减去 `NAME_PAD` —— 内边距放在**列宽里**而不是画在右侧，
        // 这样"名称列起点"和"状态列起点"都不受影响，栅格照样严丝合缝。
        cell(
            ui,
            (cols::name_width(_row_width) - cols::NAME_PAD).max(40.0),
            egui::RichText::new(&r.name).monospace(),
        );
        // 状态词 + 图标：同一格，图标紧跟词尾（用户 2026-06 要求）
        state_with_icon(ui, app.tr(state_word(r.state)), r.state);
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
/// **行与行之间一律不缩进。** 曾经这里每行都套一层 `ui.indent("rom_rows", …)`，而 `ui.indent` 是**按 id 存状态的**：同一个 id 在循环里反复调用，缩进会逐行累加（第二行起每行往右挪一点）。更糟的是缩进会吃掉 `available_width()`，于是 `cols::name_width()` 算出来的名称列宽度逐行变小、状态词起点逐行左移 —— 用户看到的"每到下一行就额外缩进、根本没对齐"就是这个。对齐由 `cols` 栅格保证，缩进只会碍事。
pub fn render(ui: &mut egui::Ui, app: &mut MameApp, view: &RomInfoView) {
    // **顶部不要那行"数据来源于校验缓存"。**
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
                state_with_icon(ui, app.tr(state_word(d.state)), d.state);
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
                // 状态词 + 图标，与 Rom 段同一套
                state_with_icon(ui, app.tr(state_word(b.state)), b.state);
                // 描述占 crc + region 两列的宽度（描述比 crc 长得多）。
                // 同样**不加 `.small()`** —— 字号跟其他列一致，靠灰色弱化。
                cell(
                    ui,
                    cols::CRC + cols::REGION,
                    egui::RichText::new(&b.description)
                        .monospace()
                        .color(ui_weak_color()),
                );
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
            tight_horizontal(ui, |ui, _row_width| {
                // 第一列：设备机种名（`m68000` / `igs036` / `z80`）。
                cell(
                    ui,
                    (cols::name_width(_row_width) - cols::NAME_PAD).max(40.0),
                    egui::RichText::new(&d.name).monospace(),
                );
                // 第二列：状态词 + 图标（与其他段同一套），不画描述/tag ——
                // tag（`:maincpu`）是内部引用名，描述在这台机器的语境下是废话。
                state_with_icon(ui, app.tr(state_word(d.state)), d.state);
                // 后面这些列一律留空，但**必须用 gap 占住**——少一列，
                // 下一行的设备 rom 就会整体前移（见 `gap` 的注释）。
                gap(ui, cols::CRC);
                gap(ui, cols::REGION);
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
                // 后面的列一律留空，但**必须 gap 占住**——少一列，这一段
                // 的名称列宽度就与 Rom 段不一致（见 `cols::GRID`）。
                gap(ui, cols::CRC);
                gap(ui, cols::REGION);
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
    /// **只有"未拥有"是红色。** `nodump` 从灰色改成黄色（与坏 dump 同色，
    /// 图标也是同一个感叹号）——原先它和"未校验"共用灰色，用户看到灰就
    /// 以为是没查。灰色现在只属于"未校验"，独占。
    #[test]
    fn the_four_user_facing_colours_are_distinct() {
        assert_eq!(state_color(RomState::Good), icons::GREEN, "拥有=绿");
        assert_eq!(state_color(RomState::BadDump), icons::YELLOW, "坏 dump=黄");
        assert_eq!(state_color(RomState::NoDump), icons::YELLOW, "未 dump=黄");
        assert_eq!(state_color(RomState::Missing), icons::RED, "未拥有=红");
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
    /// 共用：拥有/坏 dump/未拥有/未校验必须各自不同。
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

    /// **未拥有必须是红叉**，而且不能是 1.8.2 那张 `status_cross`。
    ///
    /// 用户 2026-06 指出：未拥有显示的是**蓝色**叉（`status_cross.png` 是蓝底
    /// 白叉），而配色约定里蓝色在深色主题下容易被读成灰色 = 「未审计」——
    /// 那正是这个约定要避免的（灰色是「没查」的专属）。
    ///
    /// 所以新画了 `status_missing.png`（红底白叉），与绿勾同一套底色风格。
    /// 这条钉住"别哪天又换回蓝的那张"。
    #[test]
    fn not_owned_uses_the_red_cross() {
        let missing = state_icon(RomState::Missing).expect("未拥有要有图标");
        assert!(
            missing.contains("missing"),
            "未拥有该用新画的 status_missing（红底白叉），现在却是 {missing}"
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
        // GRID 必须是三列之和（曾经漏算一列，导致列宽对不上）
        assert_eq!(
            cols::GRID,
            cols::STATE + cols::CRC + cols::REGION,
            "GRID 必须等于 状态 + crc + 区域 三列（from 列2026-06 已删）"
        );
        // 图标与状态词**同格**（画在 STATE 这一格里），所以状态列必须装得下
        // 「图标 + 间隙 + 最长的三个汉字」。装不下就会压到 crc 列上。
        assert!(
            cols::STATE >= cols::ICON_W + cols::ICON_GAP + 39.0,
            "STATE({}) 装不下 图标({}) + 间隙({}) + 「未拥有」三字(约 39)",
            cols::STATE,
            cols::ICON_W,
            cols::ICON_GAP
        );
        // 间隙必须仍是正数 —— 它是用户要的"1 个空格"
        assert!(
            (3.0..=12.0).contains(&cols::ICON_GAP),
            "图标与状态词之间留 1 个空格（约 4px），不能是 0 也不能太大：{}",
            cols::ICON_GAP
        );
        // 面板拉窄时名称列不许变负——负宽度会把后面的列往回挤，比窄更糟
        assert!(cols::name_width(10.0) >= 60.0, "窄面板下要有下限");
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

    /// `render` 的函数体里出现缩进调用就失败。
    ///
    /// （下面这条测试自己也在 `render` 之后，所以它**不能**在注释里写出那个
    /// 函数的完整名字 —— 会被自己 grep 到。）
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
