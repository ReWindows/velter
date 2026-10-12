use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(110, 200, 255);
pub const ACCENT2: Color32 = Color32::from_rgb(190, 140, 255);
pub const GOOD: Color32 = Color32::from_rgb(120, 230, 160);
pub const WARN: Color32 = Color32::from_rgb(255, 120, 130);
pub const DIM: Color32 = Color32::from_rgb(150, 158, 175);

pub fn glass_fill(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(16, 19, 28, alpha)
}

pub fn edge() -> Stroke {
    Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 34))
}

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.window_fill = glass_fill(185);
    v.panel_fill = Color32::TRANSPARENT;
    v.extreme_bg_color = Color32::from_rgba_unmultiplied(0, 0, 0, 90);
    v.faint_bg_color = Color32::from_rgba_unmultiplied(255, 255, 255, 8);
    v.window_stroke = edge();
    v.window_corner_radius = CornerRadius::same(14);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    v.popup_shadow = v.window_shadow;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;

    let w = &mut v.widgets;
    for (wv, a) in [
        (&mut w.noninteractive, 0u8),
        (&mut w.inactive, 14),
        (&mut w.hovered, 30),
        (&mut w.active, 46),
        (&mut w.open, 30),
    ] {
        wv.corner_radius = CornerRadius::same(8);
        wv.bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, a);
        wv.weak_bg_fill = wv.bg_fill;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.noninteractive.fg_stroke.color = Color32::from_rgb(214, 220, 232);
    w.inactive.fg_stroke.color = Color32::from_rgb(225, 230, 240);

    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.window_margin = egui::Margin::same(14);
    });
}
