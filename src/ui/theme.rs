//! Colours and spacing.
//!
//! One accent colour, a neutral background and a single rounding value. The
//! window should look calm enough that the screenshots in it stand out.

use egui::{Color32, CornerRadius, Stroke};

use crate::config::{Accent, Theme};

pub struct Palette {
    pub background: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub danger: Color32,
}

pub fn palette(theme: Theme, accent: Accent) -> Palette {
    let [r, g, b] = accent.rgb();
    let accent = Color32::from_rgb(r, g, b);
    match theme {
        Theme::Dark => Palette {
            background: Color32::from_rgb(0x15, 0x17, 0x1b),
            panel: Color32::from_rgb(0x1b, 0x1e, 0x23),
            raised: Color32::from_rgb(0x23, 0x27, 0x2d),
            hover: Color32::from_rgb(0x2c, 0x31, 0x38),
            text: Color32::from_rgb(0xe4, 0xe6, 0xea),
            muted: Color32::from_rgb(0x8b, 0x92, 0x9d),
            line: Color32::from_rgb(0x2e, 0x33, 0x3a),
            accent,
            danger: Color32::from_rgb(0xd0, 0x5f, 0x5f),
        },
        // Pure black background so an OLED panel can switch those pixels off.
        // Everything else stays a shade darker than the normal dark theme.
        Theme::Black => Palette {
            background: Color32::BLACK,
            panel: Color32::from_rgb(0x0b, 0x0b, 0x0d),
            raised: Color32::from_rgb(0x16, 0x17, 0x1a),
            hover: Color32::from_rgb(0x21, 0x23, 0x27),
            text: Color32::from_rgb(0xe4, 0xe6, 0xea),
            muted: Color32::from_rgb(0x80, 0x86, 0x90),
            line: Color32::from_rgb(0x22, 0x24, 0x28),
            accent,
            danger: Color32::from_rgb(0xd0, 0x5f, 0x5f),
        },
        Theme::Light => Palette {
            background: Color32::from_rgb(0xf6, 0xf7, 0xf9),
            panel: Color32::from_rgb(0xff, 0xff, 0xff),
            raised: Color32::from_rgb(0xed, 0xef, 0xf2),
            hover: Color32::from_rgb(0xe2, 0xe5, 0xea),
            text: Color32::from_rgb(0x1b, 0x1e, 0x23),
            muted: Color32::from_rgb(0x6b, 0x72, 0x7d),
            line: Color32::from_rgb(0xdc, 0xe0, 0xe6),
            accent,
            danger: Color32::from_rgb(0xc0, 0x39, 0x39),
        },
    }
}

pub fn apply(ctx: &egui::Context, theme: Theme, accent: Accent) {
    let p = palette(theme, accent);
    let mut visuals = match theme {
        Theme::Dark | Theme::Black => egui::Visuals::dark(),
        Theme::Light => egui::Visuals::light(),
    };

    visuals.panel_fill = p.background;
    visuals.window_fill = p.panel;
    visuals.extreme_bg_color = p.raised;
    visuals.faint_bg_color = p.raised;
    visuals.override_text_color = Some(p.text);
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    visuals.window_stroke = Stroke::new(1.0, p.line);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(90),
    };
    visuals.window_shadow = visuals.popup_shadow;

    // Small widgets such as checkboxes are only a dozen pixels across, so a
    // larger radius turns them into circles.
    let radius = CornerRadius::same(4);
    visuals.widgets.noninteractive.bg_fill = p.panel;
    visuals.widgets.noninteractive.weak_bg_fill = p.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    visuals.widgets.noninteractive.corner_radius = radius;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.muted);

    visuals.widgets.inactive.bg_fill = p.raised;
    visuals.widgets.inactive.weak_bg_fill = p.raised;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);

    visuals.widgets.hovered.bg_fill = p.hover;
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.line);
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);

    visuals.widgets.active.bg_fill = p.accent.gamma_multiply(0.45);
    visuals.widgets.active.weak_bg_fill = p.accent.gamma_multiply(0.45);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.text);

    visuals.widgets.open.bg_fill = p.hover;
    visuals.widgets.open.weak_bg_fill = p.hover;
    visuals.widgets.open.corner_radius = radius;

    let mut style = egui::Style {
        visuals,
        ..Default::default()
    };
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 9.0;
    ctx.set_global_style(style);
}
