//! Squarified treemap layout: turns a folder into nested boxes whose areas match their sizes.

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::epaint::Vertex;
use eframe::egui::{Color32, Mesh, Pos2, Rect, TextureId, pos2, vec2};

use crate::format;
use crate::scan::{NO_PARENT, Tree};

const BORDER: Color32 = Color32::from_rgb(10, 16, 31);
const BODY: Color32 = Color32::from_rgb(19, 29, 53);
const HEADER: Color32 = Color32::from_rgb(36, 54, 94);
const FOLDER_BLOCK: Color32 = Color32::from_rgb(74, 92, 132);
const HEADER_TEXT: Color32 = Color32::from_rgb(246, 236, 212);

const HEADER_H: f32 = 19.0;
/// The paper grain repeats every this many points.
const GRAIN: f32 = 256.0;
/// Anything smaller than this many square points is not drawn at all.
const MIN_AREA: f64 = 0.6;

pub struct Tile {
    pub rect: Rect,
    pub node: u32,
}

pub struct Label {
    pub clip: Rect,
    pub pos: Pos2,
    pub text: String,
    pub color: Color32,
    pub size: f32,
}

pub struct Layout {
    /// Parents come before their children, so the last tile under the pointer is the deepest.
    pub tiles: Vec<Tile>,
    pub labels: Vec<Label>,
    /// Where each folder header's little folder icon goes.
    pub icons: Vec<Rect>,
    pub mesh: Arc<Mesh>,
    node_tile: HashMap<u32, usize>,
}

impl Layout {
    pub fn hit(&self, p: Pos2) -> Option<usize> {
        self.tiles.iter().rposition(|t| t.rect.contains(p))
    }

    /// The tile that shows `node`, or the nearest drawn folder that contains it.
    pub fn tile_for(&self, tree: &Tree, node: u32) -> Option<usize> {
        let mut i = node;
        while i != NO_PARENT {
            if let Some(&t) = self.node_tile.get(&i) {
                return Some(t);
            }
            i = tree.node(i).parent;
        }
        None
    }
}

/// `grain` is a near-white paper texture that every box is cut from.
pub fn build(tree: &Tree, root: u32, rect: Rect, pixels_per_point: f32, grain: TextureId) -> Layout {
    let mut b = Builder {
        tree,
        ppp: pixels_per_point,
        mesh: Mesh::with_texture(grain),
        tiles: Vec::new(),
        labels: Vec::new(),
        icons: Vec::new(),
        node_tile: HashMap::new(),
    };
    b.quad(rect, [BODY; 4]);
    b.children(root, rect.shrink(2.0));
    Layout { tiles: b.tiles, labels: b.labels, icons: b.icons, mesh: Arc::new(b.mesh), node_tile: b.node_tile }
}

struct Builder<'a> {
    tree: &'a Tree,
    ppp: f32,
    mesh: Mesh,
    tiles: Vec<Tile>,
    labels: Vec<Label>,
    icons: Vec<Rect>,
    node_tile: HashMap<u32, usize>,
}

impl Builder<'_> {
    fn children(&mut self, idx: u32, inner: Rect) {
        let size = self.tree.node(idx).size;
        if size == 0 || inner.width() < 1.0 || inner.height() < 1.0 {
            return;
        }
        let scale = inner.area() as f64 / size as f64;
        let mut ids = Vec::new();
        let mut areas = Vec::new();
        // Children are sorted largest first, so everything after the first tiny one is tiny too.
        for child in self.tree.children(idx) {
            let area = self.tree.node(child).size as f64 * scale;
            if area < MIN_AREA {
                break;
            }
            ids.push(child);
            areas.push(area);
        }
        for (child, r) in ids.into_iter().zip(squarify(&areas, inner)) {
            let r = self.snap(r);
            self.item(child, r);
        }
    }

    fn item(&mut self, idx: u32, rect: Rect) {
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let node = self.tree.node(idx);
        self.tile(rect, idx);
        if node.is_dir && node.child_count > 0 && rect.width() >= 8.0 && rect.height() >= 8.0 {
            // A folder big enough to show what is inside it.
            self.quad(rect, [BORDER; 4]);
            self.quad(rect.shrink(1.0), [BODY; 4]);
            let mut inner = rect.shrink(2.0);
            if rect.width() >= 44.0 && rect.height() >= 38.0 {
                let header = Rect::from_min_size(rect.min + vec2(1.0, 1.0), vec2(rect.width() - 2.0, HEADER_H));
                self.quad(header, vertical_shade(HEADER));
                let with_icon = header.width() >= 70.0;
                let text_x = if with_icon { 24.0 } else { 5.0 };
                if with_icon {
                    self.icons.push(Rect::from_min_size(header.min + vec2(5.0, 3.0), vec2(15.0, 13.0)));
                }
                let name = &*node.name;
                let with_size = format!("{name}   {}", format::bytes(node.size));
                let text =
                    if fits(&with_size, 13.0, header.width() - text_x - 4.0) { with_size } else { name.to_string() };
                self.labels.push(Label {
                    clip: header.shrink2(vec2(3.0, 0.0)),
                    pos: header.min + vec2(text_x, 1.5),
                    text,
                    color: HEADER_TEXT,
                    size: 13.5,
                });
                inner.min.y = header.max.y + 1.0;
            }
            self.children(idx, inner);
        } else if node.is_dir {
            // Too small to open up, so it's drawn as a single block.
            let r = self.gap(rect);
            self.quad(r, cushion(FOLDER_BLOCK));
            self.file_label(idx, r, FOLDER_BLOCK);
        } else {
            let color = self.tree.color_of(idx);
            let r = self.gap(rect);
            self.quad(r, cushion(color));
            self.file_label(idx, r, color);
        }
    }

    fn file_label(&mut self, idx: u32, r: Rect, bg: Color32) {
        if r.width() < 36.0 || r.height() < 16.0 {
            return;
        }
        let node = self.tree.node(idx);
        let big = r.width() >= 160.0 && r.height() >= 64.0;
        let (name_size, size_size) = if big { (17.0, 14.5) } else { (13.0, 12.0) };
        let color = text_color_on(bg);
        let clip = r.shrink2(vec2(3.0, 1.0));
        let top = r.min + vec2(5.0, 3.0);
        self.labels.push(Label { clip, pos: top, text: node.name.to_string(), color, size: name_size });
        if r.height() >= name_size * 2.0 + 10.0 {
            self.labels.push(Label {
                clip,
                pos: top + vec2(0.0, name_size + 3.0),
                text: format::bytes(node.size),
                color: color.gamma_multiply(0.75),
                size: size_size,
            });
        }
    }

    fn tile(&mut self, rect: Rect, node: u32) {
        self.node_tile.insert(node, self.tiles.len());
        self.tiles.push(Tile { rect, node });
    }

    /// Leaves a one-pixel line on the right and bottom so neighbouring boxes stay separate.
    fn gap(&self, r: Rect) -> Rect {
        let px = 1.0 / self.ppp;
        if r.width() > 3.0 * px && r.height() > 3.0 * px {
            Rect::from_min_max(r.min, r.max - vec2(px, px))
        } else {
            r
        }
    }

    /// Rounds to whole screen pixels so edges stay crisp.
    fn snap(&self, r: Rect) -> Rect {
        let s = |v: f32| (v * self.ppp).round() / self.ppp;
        Rect::from_min_max(pos2(s(r.min.x), s(r.min.y)), pos2(s(r.max.x), s(r.max.y)))
    }

    /// Corner colours go top-left, top-right, bottom-right, bottom-left. The paper grain is
    /// mapped by screen position so neighbouring boxes look cut from the same sheet.
    fn quad(&mut self, r: Rect, colors: [Color32; 4]) {
        let base = self.mesh.vertices.len() as u32;
        let corners = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()];
        for (pos, color) in corners.into_iter().zip(colors) {
            self.mesh.vertices.push(Vertex { pos, uv: pos2(pos.x / GRAIN, pos.y / GRAIN), color });
        }
        self.mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Lit from the top-left, so each box reads as a slightly raised piece of card.
fn cushion(c: Color32) -> [Color32; 4] {
    [mix(c, Color32::WHITE, 0.22), mix(c, Color32::WHITE, 0.05), mix(c, Color32::BLACK, 0.30), mix(c, Color32::BLACK, 0.05)]
}

fn vertical_shade(c: Color32) -> [Color32; 4] {
    let top = mix(c, Color32::WHITE, 0.10);
    let bottom = mix(c, Color32::BLACK, 0.18);
    [top, top, bottom, bottom]
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn text_color_on(bg: Color32) -> Color32 {
    let luma = 0.299 * bg.r() as f32 + 0.587 * bg.g() as f32 + 0.114 * bg.b() as f32;
    if luma > 150.0 { Color32::from_rgb(40, 28, 20) } else { Color32::from_rgb(250, 244, 230) }
}

/// Rough width check so labels don't get cut off mid-word when a shorter one would fit.
fn fits(text: &str, font_size: f32, width: f32) -> bool {
    text.chars().count() as f32 * font_size * 0.56 <= width
}

/// Squarified treemap (Bruls, Huizing & van Wijk). `areas` must be sorted largest first.
/// Returns one rect per area, in the same order.
pub fn squarify(areas: &[f64], rect: Rect) -> Vec<Rect> {
    let mut out = Vec::with_capacity(areas.len());
    let (mut x0, mut y0) = (rect.min.x as f64, rect.min.y as f64);
    let (x1, y1) = (rect.max.x as f64, rect.max.y as f64);
    let mut i = 0;
    while i < areas.len() {
        let (w, h) = (x1 - x0, y1 - y0);
        if w <= 0.0 || h <= 0.0 {
            out.resize(areas.len(), Rect::NOTHING);
            break;
        }
        let side = w.min(h);
        let mut sum = areas[i];
        let mut end = i + 1;
        let mut worst = worst_ratio(areas[i], areas[i], sum, side);
        while end < areas.len() {
            let next_sum = sum + areas[end];
            let next_worst = worst_ratio(areas[i], areas[end], next_sum, side);
            if next_worst > worst {
                break;
            }
            worst = next_worst;
            sum = next_sum;
            end += 1;
        }
        let r = |a: f64, b: f64, c: f64, d: f64| Rect::from_min_max(pos2(a as f32, b as f32), pos2(c as f32, d as f32));
        if w >= h {
            // A column down the left side.
            let thick = (sum / side).min(w);
            let mut y = y0;
            for &a in &areas[i..end] {
                let next = y + a / thick;
                out.push(r(x0, y, x0 + thick, next));
                y = next;
            }
            x0 += thick;
        } else {
            // A row along the top.
            let thick = (sum / side).min(h);
            let mut x = x0;
            for &a in &areas[i..end] {
                let next = x + a / thick;
                out.push(r(x, y0, next, y0 + thick));
                x = next;
            }
            y0 += thick;
        }
        i = end;
    }
    out
}

fn worst_ratio(largest: f64, smallest: f64, sum: f64, side: f64) -> f64 {
    let (s2, w2) = (sum * sum, side * side);
    (w2 * largest / s2).max(s2 / (w2 * smallest))
}

#[cfg(test)]
mod scan_tests {
    use crate::scan::{Progress, scan};

    /// Scans a real folder and prints the totals: `SCAN_PATH=C:\ cargo test --release real_scan -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_scan() {
        let path = std::env::var("SCAN_PATH").unwrap_or_else(|_| ".".into());
        let tree = scan(std::path::Path::new(&path), &Progress::default());
        let root = tree.node(0);
        println!(
            "{path}: {:.1} GB in {} files, {} folders, {} unreadable, {:.1} s",
            root.size as f64 / 1e9,
            root.files,
            root.dirs,
            tree.errors,
            tree.elapsed.as_secs_f32()
        );
        for &i in tree.largest_files(0, 5).iter() {
            println!("  {:>8.1} GB  {}", tree.node(i).size as f64 / 1e9, tree.path_of(i).display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squarify_fills_the_rect_with_matching_areas() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0));
        let sizes = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let total: f64 = sizes.iter().sum();
        let areas: Vec<f64> = sizes.iter().map(|s| s / total * 240_000.0).collect();
        let rects = squarify(&areas, rect);
        assert_eq!(rects.len(), areas.len());
        for (r, a) in rects.iter().zip(&areas) {
            assert!(rect.expand(0.01).contains_rect(*r), "{r:?} escapes the canvas");
            assert!((r.area() as f64 - a).abs() < 1.0, "area {} should be {a}", r.area());
        }
        let covered: f32 = rects.iter().map(|r| r.area()).sum();
        assert!((covered - 240_000.0).abs() < 2.0);
    }
}
