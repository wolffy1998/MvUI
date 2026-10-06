//! 量真实字体：行框高、字形视觉上下边界、基线。
fn main() {
    let ctx = egui::Context::default();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            for (label, t) in [("拥有", "拥有"), ("未拥有", "未拥有"), ("未 dump", "未 dump"), ("gnw_smb.svg", "gnw")] {
                let style = ui.style().clone();
                let mut job = egui::text::LayoutJob::default();
                egui::RichText::new(t).append_to(&mut job, &style, egui::FontSelection::default(), egui::Align::Min);
                job.wrap.max_width = f32::MAX;
                let g = ui.fonts(|f| f.layout_job(job));
                let row = &g.rows[0];
                let r = row.rect;
                // 字形相对行框的上下留白
                let top_pad = r.min.y - g.rect.min.y;
                let bot_pad = g.rect.max.y - r.max.y;
                println!("{label:12} 行框 h={:6.2} (y {:6.2}..{:6.2})  galley h={:6.2}  上留白={:5.2} 下留白={:5.2} 视觉中心={:6.2} 行框中心={:6.2} 差={:5.2}",
                    r.height(), r.min.y, r.max.y, g.size().y, top_pad, bot_pad,
                    (r.min.y + r.max.y) / 2.0, g.size().y / 2.0,
                    ((r.min.y + r.max.y) / 2.0 - g.size().y / 2.0).abs());
            }
        });
    });
}
