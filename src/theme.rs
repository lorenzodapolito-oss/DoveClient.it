//! Palette azzurro cielo (chiara e scura) e stile dell'interfaccia: mai bianco puro.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

#[derive(Clone, Copy)]
pub struct Palette {
    pub toolbar: Color32,
    pub toolbar_text: Color32,
    pub toolbar_sub: Color32,
    pub window_bg: Color32,
    pub sidebar: Color32,
    pub selected: Color32,
    pub list_bg: Color32,
    pub stripe: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_light: Color32,
    pub error: Color32,
    pub pec: Color32,
    pub star: Color32,
    pub unread: Color32,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const LIGHT: Palette = Palette {
    toolbar: c(0x0B, 0x5C, 0x8C),
    toolbar_text: c(0xD2, 0xEE, 0xFC),
    toolbar_sub: c(0xBE, 0xE3, 0xF5),
    window_bg: c(0xE4, 0xF4, 0xFC),
    sidebar: c(0xD3, 0xEC, 0xF9),
    selected: c(0x9C, 0xD3, 0xF0),
    list_bg: c(0xDF, 0xF1, 0xFB),
    stripe: c(0xD6, 0xEB, 0xF8),
    border: c(0xA9, 0xD9, 0xF2),
    text: c(0x08, 0x3A, 0x57),
    text_secondary: c(0x1E, 0x6E, 0x97),
    text_muted: c(0x54, 0x98, 0xBE),
    accent: c(0x00, 0x98, 0xD8),
    accent_light: c(0xB6, 0xE4, 0xF7),
    error: c(0xB0, 0x3A, 0x5B),
    pec: c(0x0B, 0x4C, 0x82),
    star: c(0xE0, 0xA8, 0x00),
    unread: c(0x0E, 0x8F, 0xD0),
};

pub const DARK: Palette = Palette {
    toolbar: c(0x03, 0x1F, 0x33),
    toolbar_text: c(0xCD, 0xEB, 0xFA),
    toolbar_sub: c(0x7F, 0xC0, 0xE0),
    window_bg: c(0x08, 0x2C, 0x47),
    sidebar: c(0x0B, 0x3D, 0x5F),
    selected: c(0x16, 0x6B, 0x99),
    list_bg: c(0x0B, 0x3D, 0x5F),
    stripe: c(0x0A, 0x36, 0x56),
    border: c(0x14, 0x5A, 0x85),
    text: c(0xCD, 0xEB, 0xFA),
    text_secondary: c(0x7F, 0xC0, 0xE0),
    text_muted: c(0x4E, 0x8C, 0xAE),
    accent: c(0x22, 0xB8, 0xF0),
    accent_light: c(0x0F, 0x4C, 0x74),
    error: c(0xFF, 0x8F, 0xA8),
    pec: c(0x4F, 0xCB, 0xFA),
    star: c(0xFF, 0xC8, 0x3D),
    unread: c(0x3F, 0xC6, 0xFA),
};

pub fn palette(dark: bool) -> Palette {
    if dark {
        DARK
    } else {
        LIGHT
    }
}

pub fn visuals(dark: bool) -> egui::Visuals {
    let p = palette(dark);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.window_bg;
    v.window_fill = p.sidebar;
    v.extreme_bg_color = p.list_bg;
    v.faint_bg_color = p.stripe;
    v.code_bg_color = p.list_bg;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.selected;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;
    let r = CornerRadius::same(7);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_fill = p.sidebar;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.bg_fill = p.accent_light;
    v.widgets.inactive.weak_bg_fill = p.accent_light;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.hovered.bg_fill = p.selected;
    v.widgets.hovered.weak_bg_fill = p.selected;
    v.widgets.hovered.bg_stroke = Stroke::new(1.5, p.accent);
    v.widgets.active.bg_fill = p.accent;
    v.widgets.active.weak_bg_fill = p.accent;
    v.widgets.open.bg_fill = p.accent_light;
    v.widgets.open.weak_bg_fill = p.accent_light;
    colorize(&mut v, c(0xB6, 0xE4, 0xF7), c(0x08, 0x3A, 0x57));
    v
}

pub fn apply(ctx: &egui::Context, dark: bool) {
    let v = visuals(dark);
    // Il tema è sempre quello scelto nel programma, non quello di Windows.
    ctx.set_theme(if dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
    ctx.set_visuals_of(egui::Theme::Light, v.clone());
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}

/// Bianco e grigio non devono esistere: ogni colore neutro (anche quelli che egui mette di
/// default, come il testo dei pulsanti, le ombre o i bordi) prende una tinta.
/// I chiari vanno verso `light`, gli scuri verso `dark`.
pub fn colorize(v: &mut egui::Visuals, light: Color32, dark: Color32) {
    fn fix(c: &mut Color32, light: Color32, dark: Color32) {
        if !is_neutral(*c) {
            return;
        }
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        let lum = (u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b)) / 10;
        let (tone, t) = if lum >= 128 {
            (light, 0.85)
        } else {
            (dark, 0.9)
        };
        let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
        let [tr, tg, tb, _] = tone.to_srgba_unmultiplied();
        *c = Color32::from_rgba_unmultiplied(mix(r, tr), mix(g, tg), mix(b, tb), a);
    }
    for c in [
        &mut v.panel_fill,
        &mut v.window_fill,
        &mut v.extreme_bg_color,
        &mut v.faint_bg_color,
        &mut v.code_bg_color,
        &mut v.window_stroke.color,
        &mut v.window_shadow.color,
        &mut v.popup_shadow.color,
        &mut v.selection.bg_fill,
        &mut v.selection.stroke.color,
        &mut v.hyperlink_color,
        &mut v.text_cursor.stroke.color,
        &mut v.warn_fg_color,
        &mut v.error_fg_color,
    ] {
        fix(c, light, dark);
    }
    for c in [
        &mut v.override_text_color,
        &mut v.weak_text_color,
        &mut v.text_edit_bg_color,
    ]
    .into_iter()
    .flatten()
    {
        fix(c, light, dark);
    }
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        fix(&mut w.bg_fill, light, dark);
        fix(&mut w.weak_bg_fill, light, dark);
        fix(&mut w.bg_stroke.color, light, dark);
        fix(&mut w.fg_stroke.color, light, dark);
    }
}

/// Neutro = bianco, grigio o quasi (i canali RGB differiscono di meno di 24).
pub fn is_neutral(c: Color32) -> bool {
    let [r, g, b, a] = c.to_srgba_unmultiplied().map(i32::from);
    // Un colore del tutto trasparente non si vede: non è né bianco né grigio.
    a > 0 && r.max(g).max(b) - r.min(g).min(b) < 24
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_white_or_gray_anywhere_in_the_visuals() {
        for dark in [false, true] {
            let v = visuals(dark);
            let mut named: Vec<(&str, Color32)> = vec![
                ("panel", v.panel_fill),
                ("window", v.window_fill),
                ("faint", v.faint_bg_color),
                ("extreme", v.extreme_bg_color),
                ("code", v.code_bg_color),
                ("window_stroke", v.window_stroke.color),
                ("window_shadow", v.window_shadow.color),
                ("popup_shadow", v.popup_shadow.color),
                ("selection", v.selection.bg_fill),
                ("selection_stroke", v.selection.stroke.color),
                ("hyperlink", v.hyperlink_color),
                ("cursor", v.text_cursor.stroke.color),
                ("warn", v.warn_fg_color),
                ("error", v.error_fg_color),
            ];
            for (n, c) in [
                ("text", v.override_text_color),
                ("weak_text", v.weak_text_color),
                ("text_edit", v.text_edit_bg_color),
            ] {
                if let Some(c) = c {
                    named.push((n, c));
                }
            }
            for (n, w) in [
                ("noninteractive", v.widgets.noninteractive),
                ("inactive", v.widgets.inactive),
                ("hovered", v.widgets.hovered),
                ("active", v.widgets.active),
                ("open", v.widgets.open),
            ] {
                named.push((n, w.bg_fill));
                named.push((n, w.weak_bg_fill));
                named.push((n, w.bg_stroke.color));
                named.push((n, w.fg_stroke.color));
            }
            for (n, c) in named {
                assert!(!is_neutral(c), "dark={dark} {n}: {c:?} è bianco o grigio");
            }
        }
    }

    #[test]
    fn no_pure_white_backgrounds() {
        for p in [LIGHT, DARK] {
            for col in [p.window_bg, p.sidebar, p.list_bg, p.stripe, p.toolbar] {
                assert!(!is_neutral(col), "{col:?} è bianco o grigio");
            }
        }
        assert_eq!(palette(true).window_bg, DARK.window_bg);
    }
}
