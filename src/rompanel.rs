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
/// `nodump` 用**黄**而不是绿：它不是"校验通过"，是"MAME 说这个文件压根不会
/// 有"。染成绿色会让人以为程序真的检查过它——而那是个会被用户当成 bug 的
/// 谎。灰色（未审计）与红色（确认缺失）必须分得开，同理。
fn state_color(state: RomState) -> egui::Color32 {
    match state {
        RomState::Good => icons::GREEN,
        // nodump：与"未审计"都是"不算坏但也不是好消息"，用黄提醒它有来由
        RomState::NoDump => icons::YELLOW,
        RomState::Missing => icons::RED,
        RomState::Unknown => ui_weak_color(),
    }
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

/// 状态词的 i18n key：`很好` / `缺失` / `未审计` / `无 dump`。
///
/// 返回 key 而不是文案：文案要走 `app.tr` 现查，语言切换后立刻跟着变。
fn state_word(state: RomState) -> &'static str {
    match state {
        RomState::Good => "very good",
        RomState::Missing => "missing",
        RomState::NoDump => "no dump",
        RomState::Unknown => "not audited",
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

/// Rom 段的一行：名称 / 状态 / CRC / 区域(带 tag)。
///
/// 状态那一列后面还跟一个 ✓/✗ 符号，对应参考样式里那列勾。egui 没有
/// 内建的对勾字体，所以用最朴素的字符——它们在**任何**字体下都在，而
/// 图标字体里没有对勾字形（`assets/icons/` 里没有这一项）。
fn rom_line(ui: &mut egui::Ui, app: &MameApp, r: &RomRow) {
    let color = state_color(r.state);
    ui.horizontal(|ui| {
        // 名称：缺失时整行标红，所以名称本身也吃这个颜色
        ui.label(egui::RichText::new(&r.name).monospace().color(color));
        // 状态词
        ui.label(
            egui::RichText::new(app.tr(state_word(r.state))).color(color),
        );
        // ✓ / ✗
        let mark = if r.state == RomState::Good || r.state == RomState::NoDump {
            "\u{2713}"
        } else if r.state == RomState::Missing {
            "\u{2717}"
        } else {
            "?"
        };
        ui.label(egui::RichText::new(mark).color(color));
        // CRC
        ui.label(egui::RichText::new(crc_text(r.crc)).monospace().color(color));
        // 区域 + tag（`igs023:sprcol`）
        let region = match (&r.region, &r.tag) {
            (reg, Some(tag)) => format!("{reg}:{tag}"),
            (reg, None) => reg.clone(),
        };
        if !region.is_empty() {
            ui.label(egui::RichText::new(region).monospace().color(color));
        }
        // 继承来的条目标一下来源，否则用户会以为这是本机种自己的文件
        if let Some(from) = &r.from {
            ui.label(egui::RichText::new(format!("({from})")).small().color(ui_weak_color()));
        }
    });
}

/// 渲染整个视图。
///
/// `header_note` 是顶部那行说明（例如"来自审计缓存"或"刚刚重新审计"），
/// `None` 时不渲染那一行。
pub fn render(ui: &mut egui::Ui, app: &mut MameApp, view: &RomInfoView, header_note: Option<String>) {
    // 顶部：游戏名 + 说明 + 缺失计数
    ui.label(egui::RichText::new(&view.game).strong().size(16.0));
    if !view.description.is_empty() {
        ui.label(egui::RichText::new(&view.description).color(ui_weak_color()));
    }
    if let Some(note) = header_note {
        ui.label(egui::RichText::new(note).small().color(ui_weak_color()));
    }
    // 缺失计数：0 时是绿色"全部齐全"，否则红色"N 个缺失"。这是用户扫一眼
    // 就想知道的第一件事，所以给它自己的颜色，不用弱化色。
    let missing = view.missing_count();
    if view.audited {
        if missing == 0 {
            ui.label(egui::RichText::new(app.tr("all roms present")).color(icons::GREEN));
        } else {
            // `tr` 不接受参数（`i18n::tr` 只做整条替换），所以组合而不是
            // 走 `{}` 占位。中文是"缺失 3 项"，语序不需要变。
            let text = format!("{} {missing}", app.tr("missing"));
            ui.label(egui::RichText::new(text).color(icons::RED));
        }
    } else {
        ui.label(egui::RichText::new(app.tr("not audited yet")).color(ui_weak_color()));
    }
    ui.separator();

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

    // CHD 段
    if !view.disks.is_empty() {
        section(ui, app, "Disks:");
        for d in &view.disks {
            let color = state_color(d.state);
            ui.indent("disk_rows", |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&d.file_name).monospace().color(color));
                    ui.label(
                        egui::RichText::new(app.tr(state_word(d.state))).color(color),
                    );
                    if !d.sha1.is_empty() {
                        let short = if d.sha1.len() > 8 { &d.sha1[..8] } else { &d.sha1 };
                        ui.label(
                            egui::RichText::new(format!("sha1({short})")).monospace().color(color),
                        );
                    }
                });
            });
        }
    }

    // Bios 段：参考样式是 `v2` + 描述（`PGM BIOS V2`）
    if !view.bios.is_empty() {
        section(ui, app, "Bios:");
        for b in &view.bios {
            let color = state_color(b.state);
            ui.indent("bios_rows", |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&b.name).monospace().color(color));
                    if !b.description.is_empty() {
                        ui.label(egui::RichText::new(&b.description).color(color));
                    }
                });
            });
        }
    }

    // 引用设备段：先列设备机种，再缩进列它们的 rom 文件
    if !view.devices.is_empty() || !view.device_roms.is_empty() {
        section(ui, app, "Referenced devices:");
        // 设备机种按它第一个 rom 的 tag 分组太麻烦，直接逐个设备下面列它自己
        // 的 rom——`device_roms` 里每行都带 `tag`，按 tag 匹配回去。
        for d in &view.devices {
            let color = state_color(d.state);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&d.name).monospace().color(color));
                if !d.description.is_empty() {
                    ui.label(egui::RichText::new(&d.description).color(color));
                }
                ui.label(
                    egui::RichText::new(app.tr(state_word(d.state))).color(color),
                );
            });
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

    // Samples 段
    if !view.samples.is_empty() {
        section(ui, app, "Samples:");
        for s in &view.samples {
            let color = state_color(s.state);
            ui.indent("sample_rows", |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&s.name).monospace().color(color));
                    ui.label(
                        egui::RichText::new(format!("{}/{}", s.have, s.total))
                            .monospace()
                            .color(color),
                    );
                    ui.label(
                        egui::RichText::new(app.tr(state_word(s.state))).color(color),
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

    /// 四种状态必须给出四个**不同**的颜色，否则"缺失"和"未审计"在界面上一
    /// 样，用户会以为自己的盘空了。
    #[test]
    fn every_state_has_its_own_colour() {
        let good = state_color(RomState::Good);
        let missing = state_color(RomState::Missing);
        let unknown = state_color(RomState::Unknown);
        let nodump = state_color(RomState::NoDump);
        assert_ne!(good, missing);
        assert_ne!(good, unknown);
        assert_ne!(good, nodump);
        assert_ne!(missing, unknown, "缺失和未审计不能同色");
        assert_ne!(missing, nodump, "缺失和 nodump 不能同色");
    }
}
