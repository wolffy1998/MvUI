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

/// 各列的固定宽度。
///
/// `ui.horizontal` + 自然宽度会让每一列的起点随上一行的内容长度飘——
/// 文件名有长有短，于是「未拥有」这个词有的在这行第 20 个字符，有的在第
/// 12 个，整列读起来参差不齐。定宽是唯一能对齐的办法：名称列吃掉剩余
/// 空间，其余列按最长内容取上界。
mod cols {
    /// 状态词列（`未审计` / `未拥有` / `坏 dump` / `无 dump`）——中文最宽的
    /// "未审计" 三个字，加点余量。
    pub const STATE: f32 = 60.0;
    /// CRC 列：`crc(78c15fa2)` 是定宽的等宽字体串。
    pub const CRC: f32 = 104.0;
    /// 区域 + tag 列（`maincpu` / `igs023:sprcol`）。
    pub const REGION: f32 = 132.0;
    /// 继承来源标记 `(pgm)` 的宽度上限。
    pub const FROM: f32 = 84.0;
}

/// 状态图标列的宽度：16×16 的图 + 一点余量。
const ICON_COL: f32 = 22.0;

/// 画第三列的 16×16 状态图标，返回这次分配占的 `Response`。
///
/// 抽出来是因为 Rom 段和 CHD 段都要画，而**必须走同一套逻辑**：取不到纹理时
/// 的退路（文字符号）也要一致，否则两段在同一个面板里会一个显示图标、一个
/// 显示方框。
///
/// 返回 `Response` 是 `add_sized` 的要求（闭包要实现 `Widget`），不是有用途。
fn icon_cell(ui: &mut egui::Ui, state: RomState) -> egui::Response {
    let ctx = ui.ctx().clone();
    let font = egui::FontId::proportional(12.0);
    let color = state_color(state);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    let drawn = state_icon(state).is_some_and(|n| icons::put(ui, &ctx, n, rect));
    if !drawn {
        // 纹理还没解码完（第一帧）或图标名写错了：退回文字符号。列宽是定死
        // 的，这里的文字画在 rect 中心，不会挤歪后面的列。
        ui.painter()
            .text(rect.center(), egui::Align2::CENTER_CENTER, mark_of(state), font, color);
    }
    resp
}

/// Rom 段的一行：名称 / 状态 / 图标 / CRC / 区域(带 tag)。
///
/// **所有列定宽**，理由见 [`cols`]：自然宽度下每行的列起点都不同，
/// 参差不齐。名称列吃掉剩余空间（`add_sized` 给的是 max_width，egui 会按
/// 内容收缩也不会超过它），所以长文件名不会把后面的列挤走。
fn rom_line(ui: &mut egui::Ui, app: &MameApp, r: &RomRow) {
    let color = state_color(r.state);
    ui.horizontal(|ui| {
        // 名称：缺失时整行标红，所以名称本身也吃这个颜色
        let fixed = cols::STATE + ICON_COL + cols::CRC + cols::REGION + cols::FROM;
        ui.add_sized(
            [ui.available_width() - fixed, 0.0],
            egui::Label::new(egui::RichText::new(&r.name).monospace().color(color)),
        );
        // 状态词
        ui.add_sized(
            [cols::STATE, 0.0],
            egui::Label::new(egui::RichText::new(app.tr(state_word(r.state))).color(color)),
        );
        // 16×16 状态图标
        ui.add_sized([ICON_COL, 0.0], |ui: &mut egui::Ui| icon_cell(ui, r.state));
        // CRC
        ui.add_sized(
            [cols::CRC, 0.0],
            egui::Label::new(egui::RichText::new(crc_text(r.crc)).monospace().color(color)),
        );
        // 区域 + tag（`igs023:sprcol`）
        let region = match (&r.region, &r.tag) {
            (reg, Some(tag)) => format!("{reg}:{tag}"),
            (reg, None) => reg.clone(),
        };
        ui.add_sized(
            [cols::REGION, 0.0],
            egui::Label::new(egui::RichText::new(region).monospace().color(color)),
        );
        // 继承来的条目标一下来源，否则用户会以为这是本机种自己的文件
        let from = r.from.clone().map(|f| format!("({f})")).unwrap_or_default();
        ui.add_sized(
            [cols::FROM, 0.0],
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

    // CHD 段：文件名 / 状态 / sha1。列宽与 Rom 段共用，所以两段的
    // 状态词、✓✗ 落在同一条竖线上。
    if !view.disks.is_empty() {
        section(ui, app, "Disks:");
        for d in &view.disks {
            let color = state_color(d.state);
            ui.indent("disk_rows", |ui| {
                ui.horizontal(|ui| {
                    let avail = ui.available_width();
                    ui.add_sized(
                        [avail - (cols::STATE + ICON_COL + cols::CRC), 0.0],
                        egui::Label::new(
                            egui::RichText::new(&d.file_name).monospace().color(color),
                        ),
                    );
                    ui.add_sized(
                        [cols::STATE, 0.0],
                        egui::Label::new(
                            egui::RichText::new(app.tr(state_word(d.state))).color(color),
                        ),
                    );
                    ui.add_sized([ICON_COL, 0.0], |ui: &mut egui::Ui| icon_cell(ui, d.state));
                    let short = if d.sha1.len() > 8 {
                        d.sha1[..8].to_string()
                    } else {
                        d.sha1.clone()
                    };
                    ui.add_sized(
                        [cols::CRC, 0.0],
                        egui::Label::new(
                            egui::RichText::new(format!("sha1({short})"))
                                .monospace()
                                .color(color),
                        ),
                    );
                });
            });
        }
    }

    // Bios 段：集名 + 描述 + 状态，**下面缩进列这一套实际的文件**。
    // 文件用 `rom_line`（与 Rom 段同一套列宽），所以整段的 ✓✗ / crc 也对齐。
    if !view.bios.is_empty() {
        section(ui, app, "Bios:");
        for b in &view.bios {
            let color = state_color(b.state);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&b.name).monospace().strong().color(color));
                if !b.description.is_empty() {
                    ui.label(egui::RichText::new(&b.description).color(color));
                }
                ui.label(egui::RichText::new(app.tr(state_word(b.state))).color(color));
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
                let avail = ui.available_width();
                ui.add_sized(
                    [avail - (cols::STATE + ICON_COL + cols::REGION), 0.0],
                    egui::Label::new(egui::RichText::new(&d.name).monospace().color(color)),
                );
                ui.add_sized(
                    [cols::STATE, 0.0],
                    egui::Label::new(
                        egui::RichText::new(app.tr(state_word(d.state))).color(color),
                    ),
                );
                ui.add_sized([ICON_COL, 0.0], |ui: &mut egui::Ui| icon_cell(ui, d.state));
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
                ui.add_sized(
                    [cols::REGION, 0.0],
                    egui::Label::new(egui::RichText::new(tail).color(color)),
                );
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
    // 类型 / 实例 / 扩展名。
    if !view.slots.is_empty() {
        section(ui, app, "Device slots:");
        for s in &view.slots {
            ui.indent("slot_rows", |ui| {
                ui.horizontal(|ui| {
                    let avail = ui.available_width();
                    // 类型：槽位的身份（`memcard`）
                    ui.add_sized(
                        [avail * 0.3, 0.0],
                        egui::Label::new(
                            egui::RichText::new(&s.kind).monospace().color(ui_weak_color()),
                        ),
                    );
                    // 实例：命令行 `-memcard1` 用的就是它
                    ui.add_sized(
                        [avail * 0.3, 0.0],
                        egui::Label::new(
                            egui::RichText::new(&s.instance).monospace().color(ui_weak_color()),
                        ),
                    );
                    // 扩展名：逗号连接
                    ui.add_sized(
                        [avail * 0.4, 0.0],
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
                    let avail = ui.available_width();
                    // 槽位名：命令行 `nes:ctrl1=<dev>` 用的就是它
                    ui.add_sized(
                        [cols::FROM, 0.0],
                        egui::Label::new(
                            egui::RichText::new(&s.name).monospace().color(ui_weak_color()),
                        ),
                    );
                    // 可选设备数；空槽位（`nes_slot`）显示 0，不留空
                    let count = egui::RichText::new(format!("{}", s.option_count))
                        .monospace()
                        .color(ui_weak_color());
                    ui.add_sized([40.0, 0.0], egui::Label::new(count));
                    // 选项名清单，空槽位给个明确的"—"而不是空白
                    let text = if s.options.is_empty() {
                        "-".to_string()
                    } else {
                        s.options.clone()
                    };
                    ui.add_sized(
                        [avail - cols::FROM - 40.0, 0.0],
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
                    let avail = ui.available_width();
                    ui.add_sized(
                        [avail - (cols::STATE + ICON_COL + cols::CRC), 0.0],
                        egui::Label::new(egui::RichText::new(&s.name).monospace().color(color)),
                    );
                    ui.add_sized(
                        [cols::STATE, 0.0],
                        egui::Label::new(
                            egui::RichText::new(app.tr(state_word(s.state))).color(color),
                        ),
                    );
                    ui.add_sized([ICON_COL, 0.0], |ui: &mut egui::Ui| icon_cell(ui, s.state));
                    ui.add_sized(
                        [cols::CRC, 0.0],
                        egui::Label::new(
                            egui::RichText::new(format!("{}/{}", s.have, s.total))
                                .monospace()
                                .color(color),
                        ),
                    );
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
}
