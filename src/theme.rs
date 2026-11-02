//! The paper-cut look: colours, fonts, and helpers for drawing cut paper, buttons and icons.

use std::sync::Arc;

use eframe::egui::epaint::{Mesh, Vertex};
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect, Response,
    Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, Vec2, pos2, vec2,
};

use crate::assets::Sprite;

include!(concat!(env!("OUT_DIR"), "/fonts.rs"));

pub const NAVY_DEEP: Color32 = Color32::from_rgb(12, 19, 36);
pub const NAVY: Color32 = Color32::from_rgb(21, 32, 58);
pub const NAVY_MID: Color32 = Color32::from_rgb(31, 47, 82);
pub const NAVY_LIGHT: Color32 = Color32::from_rgb(47, 68, 112);
pub const GOLD: Color32 = Color32::from_rgb(214, 172, 88);
pub const GOLD_BRIGHT: Color32 = Color32::from_rgb(250, 214, 122);
pub const GOLD_DARK: Color32 = Color32::from_rgb(138, 102, 44);
pub const CREAM: Color32 = Color32::from_rgb(246, 236, 212);
pub const PAPER: Color32 = Color32::from_rgb(240, 228, 200);
pub const INK: Color32 = Color32::from_rgb(52, 36, 24);
pub const INK_SOFT: Color32 = Color32::from_rgb(116, 90, 64);
pub const AMBER: Color32 = Color32::from_rgb(246, 178, 72);
pub const RED_INK: Color32 = Color32::from_rgb(168, 52, 40);

fn family(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

pub fn ui_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}
pub fn medium(size: f32) -> FontId {
    FontId::new(size, family("medium"))
}
pub fn bold(size: f32) -> FontId {
    FontId::new(size, family("bold"))
}
pub fn heading(size: f32) -> FontId {
    FontId::new(size, family("heading"))
}
pub fn title(size: f32) -> FontId {
    FontId::new(size, family("title"))
}
pub fn hand(size: f32) -> FontId {
    FontId::new(size, family("hand"))
}

/// Embedded fonts by file stem, falling back to egui's own font when one isn't bundled.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    for (stem, bytes) in FONT_FILES {
        fonts.font_data.insert(stem.to_string(), Arc::new(FontData::from_static(bytes)));
    }
    let mut set = |fam: FontFamily, stem: &str| {
        let mut list = fallback.clone();
        if FONT_FILES.iter().any(|(s, _)| *s == stem) {
            list.insert(0, stem.to_string());
        }
        fonts.families.insert(fam, list);
    };
    set(FontFamily::Proportional, "AlegreyaSans-Regular");
    set(family("medium"), "AlegreyaSans-Medium");
    set(family("bold"), "AlegreyaSans-Bold");
    set(family("heading"), "AlegreyaSC-Bold");
    set(family("title"), "AlegreyaSC-Black");
    set(family("hand"), "PatrickHand-Regular");
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut(|s| {
        s.text_styles.insert(TextStyle::Body, ui_font(16.0));
        s.text_styles.insert(TextStyle::Button, medium(16.0));
        s.text_styles.insert(TextStyle::Heading, heading(22.0));
        s.text_styles.insert(TextStyle::Small, ui_font(13.0));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(14.0));
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(10.0, 5.0);
        s.spacing.interact_size.y = 28.0;

        let v = &mut s.visuals;
        v.override_text_color = Some(CREAM);
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = NAVY;
        v.window_stroke = Stroke::new(1.0_f32, GOLD_DARK);
        v.extreme_bg_color = NAVY_DEEP;
        v.faint_bg_color = NAVY_MID;
        v.selection.bg_fill = Color32::from_rgb(120, 92, 36);
        v.selection.stroke = Stroke::new(1.5_f32, GOLD_BRIGHT);
        v.hyperlink_color = GOLD_BRIGHT;
        let radius = CornerRadius::same(6);
        for (w, fill, stroke) in [
            (&mut v.widgets.noninteractive, NAVY, Stroke::new(1.0_f32, GOLD_DARK)),
            (&mut v.widgets.inactive, NAVY_MID, Stroke::new(1.0_f32, GOLD_DARK)),
            (&mut v.widgets.hovered, NAVY_LIGHT, Stroke::new(1.5_f32, GOLD)),
            (&mut v.widgets.active, NAVY_LIGHT, Stroke::new(2.0_f32, GOLD_BRIGHT)),
            (&mut v.widgets.open, NAVY_LIGHT, Stroke::new(1.5_f32, GOLD)),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.bg_stroke = stroke;
            w.fg_stroke = Stroke::new(1.5_f32, CREAM);
            w.corner_radius = radius;
        }
    });
}

/// A rectangle filled with a repeating texture, tinted.
pub fn tiled(painter: &Painter, rect: Rect, tex: &Sprite, tint: Color32) {
    let mut mesh = Mesh::with_texture(tex.id());
    let uv = |p: Pos2| pos2(p.x / tex.size.x, p.y / tex.size.y);
    for p in [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()] {
        mesh.vertices.push(Vertex { pos: p, uv: uv(p), color: tint });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}

/// A sprite placed by its centre, scaled to `height` and rotated by `angle` radians.
pub fn sprite_rotated(painter: &Painter, center: Pos2, height: f32, angle: f32, s: &Sprite, tint: Color32) {
    let scale = height / s.size.y;
    let half = s.size * scale * 0.5;
    let (sin, cos) = angle.sin_cos();
    let rot = |v: Vec2| center + vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
    let mut mesh = Mesh::with_texture(s.id());
    for (corner, uv) in [
        (vec2(-half.x, -half.y), pos2(0.0, 0.0)),
        (vec2(half.x, -half.y), pos2(1.0, 0.0)),
        (vec2(half.x, half.y), pos2(1.0, 1.0)),
        (vec2(-half.x, half.y), pos2(0.0, 1.0)),
    ] {
        mesh.vertices.push(Vertex { pos: rot(corner), uv, color: tint });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}

/// Small deterministic noise so each piece of paper keeps the same torn edge every frame.
fn hash(seed: u32, i: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ i.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// Outline of a piece of paper with deckled (slightly torn) edges, clockwise from the top-left.
pub fn deckled(rect: Rect, seed: u32, roughness: f32) -> Vec<Pos2> {
    let mut points = Vec::new();
    let mut i = 0;
    let mut edge = |from: Pos2, to: Pos2, normal: Vec2, points: &mut Vec<Pos2>| {
        let len = from.distance(to);
        let steps = (len / 7.0).max(2.0) as u32;
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let jitter = (hash(seed, i) - 0.5) * 2.0 * roughness;
            i += 1;
            points.push(from + (to - from) * t + normal * jitter);
        }
    };
    edge(rect.left_top(), rect.right_top(), vec2(0.0, -1.0), &mut points);
    edge(rect.right_top(), rect.right_bottom(), vec2(1.0, 0.0), &mut points);
    edge(rect.right_bottom(), rect.left_bottom(), vec2(0.0, 1.0), &mut points);
    edge(rect.left_bottom(), rect.left_top(), vec2(-1.0, 0.0), &mut points);
    points
}

/// A piece of cut paper: soft shadow, textured fill, and a faint light edge where it was cut.
pub fn paper(painter: &Painter, outline: &[Pos2], tex: &Sprite, tint: Color32, lift: f32) {
    if outline.len() < 3 {
        return;
    }
    for (offset, alpha) in [(vec2(lift * 0.5, lift * 1.4), 46u8), (vec2(lift * 0.25, lift * 0.7), 60)] {
        let shadow: Vec<Pos2> = outline.iter().map(|p| *p + offset).collect();
        painter.add(Shape::convex_polygon(shadow, Color32::from_black_alpha(alpha), Stroke::NONE));
    }
    let centre = outline.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / outline.len() as f32;
    let mut mesh = Mesh::with_texture(tex.id());
    let uv = |p: Pos2| pos2(p.x / tex.size.x, p.y / tex.size.y);
    mesh.vertices.push(Vertex { pos: centre.to_pos2(), uv: uv(centre.to_pos2()), color: tint });
    for p in outline {
        mesh.vertices.push(Vertex { pos: *p, uv: uv(*p), color: tint });
    }
    let n = outline.len() as u32;
    for k in 0..n {
        mesh.indices.extend_from_slice(&[0, 1 + k, 1 + (k + 1) % n]);
    }
    painter.add(Shape::mesh(mesh));
    painter.add(Shape::closed_line(outline.to_vec(), Stroke::new(1.0_f32, Color32::from_white_alpha(46))));
}

/// A parchment card with deckled edges.
pub fn card(painter: &Painter, rect: Rect, parchment: &Sprite, seed: u32) {
    paper(painter, &deckled(rect, seed, 1.6), parchment, Color32::WHITE, 5.0);
}

/// A round pushpin seen from above.
pub fn pin(painter: &Painter, at: Pos2, color: Color32) {
    painter.circle_filled(at + vec2(2.5, 3.5), 7.0, Color32::from_black_alpha(80));
    painter.circle_filled(at, 7.0, color);
    painter.circle_filled(at + vec2(-1.0, -1.0), 5.0, color.gamma_multiply(1.25));
    painter.circle_filled(at + vec2(-2.4, -2.6), 2.0, Color32::from_white_alpha(170));
}

/// Manila folder icon.
pub fn folder_icon(painter: &Painter, rect: Rect, fill: Color32) {
    let w = rect.width();
    let h = rect.height();
    let tab = vec![
        rect.min + vec2(0.0, h * 0.12),
        rect.min + vec2(w * 0.38, h * 0.12),
        rect.min + vec2(w * 0.48, h * 0.30),
        rect.min + vec2(0.0, h * 0.30),
    ];
    painter.add(Shape::convex_polygon(tab, fill.gamma_multiply(0.78), Stroke::NONE));
    let body = Rect::from_min_max(rect.min + vec2(0.0, h * 0.26), rect.max);
    painter.rect_filled(body, CornerRadius::same(2), fill);
    painter.line_segment(
        [body.left_top() + vec2(1.0, 1.5), body.right_top() + vec2(-1.0, 1.5)],
        Stroke::new(1.0_f32, Color32::from_white_alpha(70)),
    );
}

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Folder,
    Play,
    Stop,
    Up,
    Glass,
    SpeakerOn,
    SpeakerOff,
}

pub fn icon(painter: &Painter, rect: Rect, which: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.height();
    match which {
        Icon::Folder => folder_icon(painter, rect.shrink2(vec2(0.0, s * 0.08)), Color32::from_rgb(232, 182, 84)),
        Icon::Play => {
            let pts = vec![c + vec2(-s * 0.32, -s * 0.4), c + vec2(s * 0.42, 0.0), c + vec2(-s * 0.32, s * 0.4)];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Stop => {
            painter.rect_filled(Rect::from_center_size(c, Vec2::splat(s * 0.62)), CornerRadius::same(2), color);
        }
        Icon::Up => {
            let pts = vec![c + vec2(0.0, -s * 0.42), c + vec2(s * 0.4, s * 0.02), c + vec2(-s * 0.4, s * 0.02)];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
            painter.rect_filled(
                Rect::from_center_size(c + vec2(0.0, s * 0.2), vec2(s * 0.24, s * 0.4)),
                CornerRadius::ZERO,
                color,
            );
        }
        Icon::Glass => {
            let lens = c + vec2(-s * 0.1, -s * 0.1);
            painter.circle_stroke(lens, s * 0.28, Stroke::new(s * 0.1, color));
            painter.line_segment(
                [lens + vec2(s * 0.2, s * 0.2), c + vec2(s * 0.42, s * 0.42)],
                Stroke::new(s * 0.13, color),
            );
        }
        Icon::SpeakerOn | Icon::SpeakerOff => {
            let body = vec![
                c + vec2(-s * 0.42, -s * 0.14),
                c + vec2(-s * 0.2, -s * 0.14),
                c + vec2(s * 0.05, -s * 0.38),
                c + vec2(s * 0.05, s * 0.38),
                c + vec2(-s * 0.2, s * 0.14),
                c + vec2(-s * 0.42, s * 0.14),
            ];
            painter.add(Shape::convex_polygon(body, color, Stroke::NONE));
            if which == Icon::SpeakerOn {
                for r in [0.22, 0.38] {
                    let arc: Vec<Pos2> = (0..=8)
                        .map(|i| {
                            let a = -0.8 + 1.6 * i as f32 / 8.0;
                            c + vec2(s * 0.05 + a.cos() * s * r, a.sin() * s * r)
                        })
                        .collect();
                    painter.add(Shape::line(arc, Stroke::new(s * 0.08, color)));
                }
            } else {
                let x = c + vec2(s * 0.3, 0.0);
                let d = s * 0.14;
                painter.line_segment([x + vec2(-d, -d), x + vec2(d, d)], Stroke::new(s * 0.08, color));
                painter.line_segment([x + vec2(-d, d), x + vec2(d, -d)], Stroke::new(s * 0.08, color));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ButtonStyle {
    /// Navy with a gold edge.
    Normal,
    /// Larger, brighter edge: the main action.
    Primary,
    /// Gold-filled, for the current choice.
    Chosen,
}

/// A navy, gold-trimmed button with an optional icon.
pub fn button(ui: &mut Ui, text: &str, icon_kind: Option<Icon>, style: ButtonStyle, enabled: bool) -> Response {
    let (font, height, pad) = match style {
        ButtonStyle::Primary => (medium(20.0), 40.0, 16.0),
        _ => (medium(16.0), 32.0, 11.0),
    };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, CREAM);
    let icon_w = if icon_kind.is_some() { height * 0.55 + 8.0 } else { 0.0 };
    let width = pad * 2.0 + icon_w + galley.size().x;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let (fill, edge, text_color) = match style {
        ButtonStyle::Chosen => (GOLD, GOLD_BRIGHT, NAVY_DEEP),
        _ if !enabled => (NAVY, GOLD_DARK.gamma_multiply(0.6), CREAM.gamma_multiply(0.45)),
        _ if pressed => (NAVY, GOLD_BRIGHT, CREAM),
        _ if hovered => (NAVY_LIGHT, GOLD_BRIGHT, Color32::WHITE),
        _ => (NAVY_MID, if style == ButtonStyle::Primary { GOLD } else { GOLD_DARK }, CREAM),
    };
    let r = CornerRadius::same(7);
    painter.rect_filled(rect.translate(vec2(0.0, 2.5)), r, Color32::from_black_alpha(90));
    painter.rect_filled(rect, r, fill);
    // A lighter top half reads as a slightly raised paper button.
    painter.rect_filled(
        Rect::from_min_max(rect.min, pos2(rect.max.x, rect.center().y)),
        CornerRadius { nw: 7, ne: 7, sw: 0, se: 0 },
        Color32::from_white_alpha(if style == ButtonStyle::Chosen { 30 } else { 9 }),
    );
    let width: f32 = if style == ButtonStyle::Primary || hovered { 2.0 } else { 1.5 };
    painter.rect_stroke(rect, r, Stroke::new(width, edge), StrokeKind::Inside);
    let mut x = rect.min.x + pad;
    if let Some(kind) = icon_kind {
        let s = height * 0.55;
        let icon_color = if style == ButtonStyle::Chosen { NAVY_DEEP } else { GOLD_BRIGHT };
        icon(painter, Rect::from_min_size(pos2(x, rect.center().y - s / 2.0), Vec2::splat(s)), kind, icon_color);
        x += s + 8.0;
    }
    painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, text_color);
    response
}

/// A folder-tab shaped breadcrumb.
pub fn tab(ui: &mut Ui, text: &str, icon_kind: Option<Icon>, current: bool) -> Response {
    let font = if current { bold(16.0) } else { medium(16.0) };
    let color = if current { GOLD_BRIGHT } else { CREAM };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let icon_w = if icon_kind.is_some() { 22.0 } else { 0.0 };
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + icon_w + 30.0, 32.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let slant = 9.0;
        let shape = vec![
            rect.left_bottom(),
            rect.left_top() + vec2(slant, 0.0),
            rect.right_top() - vec2(slant, 0.0),
            rect.right_bottom(),
        ];
        let fill = if current {
            NAVY_LIGHT
        } else if response.hovered() {
            NAVY_MID.gamma_multiply(1.3)
        } else {
            NAVY_MID
        };
        painter.add(Shape::convex_polygon(shape.clone(), fill, Stroke::NONE));
        let edge = if current || response.hovered() { GOLD } else { GOLD_DARK };
        painter.add(Shape::line(shape, Stroke::new(1.5_f32, edge)));
        let mut x = rect.min.x + 15.0;
        if let Some(kind) = icon_kind {
            icon(painter, Rect::from_min_size(pos2(x, rect.center().y - 8.0), Vec2::splat(16.0)), kind, GOLD_BRIGHT);
            x += icon_w;
        }
        painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, color);
    }
    response
}

