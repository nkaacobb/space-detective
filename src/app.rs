use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Id, Key, LayerId, Layout as EguiLayout, Order, Rect, RichText, Sense,
    Stroke, StrokeKind, TextStyle, pos2, vec2,
};

use crate::format;
use crate::scan::{self, NO_PARENT, Progress, RANKED_COLORS, Tree};
use crate::treemap::{self, Layout};

const LAST_PATH_KEY: &str = "last_path";
const SELECTED: Color32 = Color32::from_rgb(255, 214, 10);
const WARN: Color32 = Color32::from_rgb(240, 180, 90);

#[cfg(windows)]
const REVEAL_LABEL: &str = "📂 Show in Explorer";
#[cfg(target_os = "macos")]
const REVEAL_LABEL: &str = "📂 Show in Finder";
#[cfg(not(any(windows, target_os = "macos")))]
const REVEAL_LABEL: &str = "📂 Show in file manager";

#[cfg(windows)]
const DRIVE_WORD: &str = "drive";
#[cfg(windows)]
const DRIVES_TITLE: &str = "Drives";
#[cfg(not(windows))]
const DRIVE_WORD: &str = "disk";
#[cfg(not(windows))]
const DRIVES_TITLE: &str = "Disks";

#[cfg(target_os = "macos")]
const UNREADABLE_HINT: &str = "macOS protects some folders. To include them, give Space Detective Full Disk Access \
                               in System Settings › Privacy & Security, then scan again.";
#[cfg(not(target_os = "macos"))]
const UNREADABLE_HINT: &str = "Usually system folders that need administrator rights. Their sizes aren't counted.";

struct Drive {
    label: String,
    path: String,
}

struct ScanJob {
    path: PathBuf,
    progress: Arc<Progress>,
    rx: Receiver<Tree>,
    started: Instant,
}

#[derive(PartialEq)]
struct LayoutKey {
    view: u32,
    rect: Rect,
    ppp: f32,
}

/// Things the panels ask for; applied once drawing is done so the tree isn't borrowed.
enum Action {
    Browse,
    Scan(String),
    Cancel,
    Zoom(u32),
    Up,
    Select(Option<u32>),
    Reveal(u32),
    CopyPath(u32),
}

pub struct App {
    ctx: egui::Context,
    path_input: String,
    drives: Vec<Drive>,
    job: Option<ScanJob>,
    tree: Option<Tree>,
    /// The folder currently filling the map.
    view: u32,
    selected: Option<u32>,
    hovered: Option<u32>,
    menu_node: Option<u32>,
    layout: Option<(LayoutKey, Layout)>,
    largest: Option<(u32, Vec<u32>)>,
    error: Option<String>,
    first_frame: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start_path: Option<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        ctx.set_theme(egui::Theme::Dark);
        ctx.style_mut(|s| {
            s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
            s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
            s.text_styles.insert(TextStyle::Heading, FontId::proportional(20.0));
            s.text_styles.insert(TextStyle::Small, FontId::proportional(11.5));
            s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
            s.spacing.item_spacing = vec2(8.0, 6.0);
            s.spacing.button_padding = vec2(9.0, 4.0);
        });

        let remembered = cc.storage.and_then(|s| eframe::get_value::<String>(s, LAST_PATH_KEY));
        let path_input = remembered
            .or_else(|| std::env::var("USERPROFILE").ok())
            .or_else(|| std::env::var("HOME").ok())
            .unwrap_or_default();

        let mut app = Self {
            ctx,
            path_input,
            drives: list_drives(),
            job: None,
            tree: None,
            view: 0,
            selected: None,
            hovered: None,
            menu_node: None,
            layout: None,
            largest: None,
            error: None,
            first_frame: true,
        };
        if let Some(path) = start_path {
            app.start_scan(path.display().to_string());
        }
        app
    }

    fn start_scan(&mut self, input: String) {
        let path = normalize_path(&input);
        if !path.is_dir() {
            self.error = Some(format!("“{}” isn't a folder I can open.", input.trim()));
            return;
        }
        if let Some(old) = self.job.take() {
            old.progress.cancel.store(true, Relaxed);
        }
        self.error = None;
        self.path_input = path.display().to_string();

        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let (worker_progress, worker_path, ctx) = (progress.clone(), path.clone(), self.ctx.clone());
        std::thread::Builder::new()
            .name("scanner".into())
            .spawn(move || {
                let tree = scan::scan(&worker_path, &worker_progress);
                // Nobody is listening if the scan was cancelled; that's fine.
                let _ = tx.send(tree);
                ctx.request_repaint();
            })
            .expect("spawn scanner thread");
        self.job = Some(ScanJob { path, progress, rx, started: Instant::now() });
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else { return };
        match job.rx.try_recv() {
            Ok(tree) => {
                self.job = None;
                self.tree = Some(tree);
                self.view = 0;
                self.selected = None;
                self.hovered = None;
                self.layout = None;
                self.largest = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                self.error = Some("The scan stopped unexpectedly.".into());
            }
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Browse => {
                let mut dialog = rfd::FileDialog::new().set_title("Choose a folder to scan");
                let current = normalize_path(&self.path_input);
                if current.is_dir() {
                    dialog = dialog.set_directory(current);
                }
                if let Some(folder) = dialog.pick_folder() {
                    self.start_scan(folder.display().to_string());
                }
            }
            Action::Scan(path) => self.start_scan(path),
            Action::Cancel => {
                if let Some(job) = self.job.take() {
                    job.progress.cancel.store(true, Relaxed);
                }
            }
            Action::Select(node) => self.selected = node,
            Action::Up => {
                if let Some(tree) = &self.tree
                    && self.view != 0
                {
                    self.view = tree.node(self.view).parent;
                }
            }
            Action::Zoom(node) => {
                if let Some(tree) = &self.tree {
                    let target = if tree.node(node).is_dir { node } else { tree.node(node).parent };
                    if target != NO_PARENT {
                        self.view = target;
                    }
                }
            }
            Action::Reveal(node) => {
                if let Some(tree) = &self.tree {
                    reveal(&tree.path_of(node));
                }
            }
            Action::CopyPath(node) => {
                if let Some(tree) = &self.tree {
                    self.ctx.copy_text(tree.path_of(node).display().to_string());
                }
            }
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(12, 9));
        egui::TopBottomPanel::top("top").frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🔍 Space Detective").size(19.0).strong());
                ui.add_space(16.0);
                let scanning = self.job.is_some();
                if ui.add_enabled(!scanning, egui::Button::new("📁 Browse…")).clicked() {
                    actions.push(Action::Browse);
                }
                let edit = ui.add_enabled(
                    !scanning,
                    egui::TextEdit::singleline(&mut self.path_input)
                        .desired_width(620.0)
                        .hint_text("Folder to scan, e.g. C:\\Users"),
                );
                let entered = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if scanning {
                    if ui.button("⏹ Cancel").clicked() {
                        actions.push(Action::Cancel);
                    }
                } else if ui.button("▶ Scan").clicked() || entered {
                    actions.push(Action::Scan(self.path_input.clone()));
                }
                ui.with_layout(EguiLayout::right_to_left(Align::Center), |ui| {
                    for drive in self.drives.iter().rev() {
                        let button = ui.add_enabled(!scanning, egui::Button::new(&drive.label));
                        if button.on_hover_text(format!("Scan the whole {} {DRIVE_WORD}", drive.label)).clicked() {
                            actions.push(Action::Scan(drive.path.clone()));
                        }
                    }
                    if !self.drives.is_empty() {
                        ui.label(RichText::new(DRIVES_TITLE).weak());
                    }
                });
            });
        });

        let (Some(tree), None) = (&self.tree, &self.job) else { return };
        let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(12, 6));
        egui::TopBottomPanel::top("breadcrumbs").frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let up = ui.add_enabled(self.view != 0, egui::Button::new("⬆ Up"));
                if up.on_hover_text("Zoom out one level (Backspace)").clicked() {
                    actions.push(Action::Up);
                }
                ui.add_space(6.0);
                for (i, node) in tree.ancestry(self.view).into_iter().enumerate() {
                    if i > 0 {
                        ui.label(RichText::new("›").weak());
                    }
                    let current = node == self.view;
                    let crumb = RichText::new(tree.display_name(node));
                    if ui.selectable_label(current, if current { crumb.strong() } else { crumb }).clicked() && !current {
                        actions.push(Action::Zoom(node));
                    }
                }
                ui.with_layout(EguiLayout::right_to_left(Align::Center), |ui| {
                    let v = tree.node(self.view);
                    ui.label(format!(
                        "{}  ·  {} files  ·  {} folders",
                        format::bytes(v.size),
                        format::count(v.files),
                        format::count(v.dirs)
                    ));
                });
            });
        });
    }

    fn status_bar(&self, ctx: &egui::Context) {
        let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::symmetric(12, 5));
        egui::TopBottomPanel::bottom("status").frame(frame).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let Some(tree) = &self.tree else {
                    ui.label(RichText::new("No folder scanned yet").weak());
                    return;
                };
                let root = tree.node(0);
                ui.label(format!(
                    "{} in {} files and {} folders  ·  scanned in {:.1} s",
                    format::bytes(root.size),
                    format::count(root.files),
                    format::count(root.dirs),
                    tree.elapsed.as_secs_f32()
                ));
                if tree.errors > 0 {
                    ui.label(RichText::new(format!("·  {} couldn't be read", format::count(tree.errors))).color(WARN))
                        .on_hover_text(UNREADABLE_HINT);
                }
                if let Some(node) = self.hovered {
                    ui.with_layout(EguiLayout::right_to_left(Align::Center), |ui| {
                        let text = format!("{}   {}", tree.path_of(node).display(), format::bytes(tree.node(node).size));
                        ui.add(egui::Label::new(RichText::new(text).weak()).truncate());
                    });
                }
            });
        });
    }

    fn side_panel(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(tree) = &self.tree else { return };
        if self.largest.as_ref().is_none_or(|(view, _)| *view != self.view) {
            self.largest = Some((self.view, tree.largest_files(self.view, 40)));
        }
        let largest = &self.largest.as_ref().unwrap().1;
        let (view, selected) = (self.view, self.selected);

        egui::SidePanel::right("details")
            .resizable(true)
            .default_width(390.0)
            .min_width(290.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.add_space(8.0);
                    section_title(ui, "Selected");
                    match selected {
                        Some(node) => details(ui, tree, node, view, actions),
                        None => {
                            ui.label(
                                RichText::new(
                                    "Click any box in the map to see what it is.\n\
                                     Double-click a box to zoom into its folder.\n\
                                     Right-click for more options.",
                                )
                                .weak(),
                            );
                        }
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    section_title(ui, "Largest files here");
                    if largest.is_empty() {
                        ui.label(RichText::new("No files in this folder.").weak());
                    }
                    for &file in largest {
                        let n = tree.node(file);
                        let row = item_row(ui, tree.color_of(file), &n.name, &format::bytes(n.size), selected == Some(file));
                        if row.clicked() {
                            actions.push(Action::Select(Some(file)));
                        }
                        if row.double_clicked() {
                            actions.push(Action::Zoom(file));
                        }
                        row.on_hover_text(tree.path_of(file).display().to_string());
                    }

                    ui.add_space(12.0);
                    ui.separator();
                    section_title(ui, "File types (whole scan)");
                    let total = tree.node(0).size;
                    for &id in tree.ext_rank.iter().take(RANKED_COLORS) {
                        let ext = &tree.exts[id as usize];
                        if ext.bytes == 0 {
                            continue;
                        }
                        let value = format!("{}   {}", format::bytes(ext.bytes), format::percent(ext.bytes, total));
                        item_row(ui, ext.color, &ext.label, &value, false)
                            .on_hover_text(format!("{} files", format::count(ext.count)));
                    }
                    let rest: u64 = tree.ext_rank.iter().skip(RANKED_COLORS).map(|&id| tree.exts[id as usize].bytes).sum();
                    if rest > 0 {
                        let value = format!("{}   {}", format::bytes(rest), format::percent(rest, total));
                        item_row(ui, Color32::from_gray(120), "Everything else", &value, false);
                    }
                    ui.add_space(12.0);
                });
            });
    }

    fn central(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let frame = egui::Frame::new().fill(treemap::CANVAS).inner_margin(egui::Margin::same(8));
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            if let Some(job) = &self.job {
                scanning_screen(ui, job, actions);
            } else if self.tree.is_some() {
                self.treemap_ui(ui, actions);
            } else {
                welcome_screen(ui, &self.drives, self.error.as_deref(), actions);
            }
        });
    }

    fn treemap_ui(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let Some(tree) = &self.tree else { return };
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
        if tree.node(self.view).size == 0 {
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "This folder is empty",
                FontId::proportional(20.0),
                Color32::GRAY,
            );
            return;
        }

        let ppp = ui.ctx().pixels_per_point();
        let key = LayoutKey { view: self.view, rect, ppp };
        if self.layout.as_ref().is_none_or(|(k, _)| *k != key) {
            self.layout = Some((key, treemap::build(tree, self.view, rect, ppp)));
        }
        let layout = &self.layout.as_ref().unwrap().1;

        let painter = ui.painter_at(rect);
        painter.add(egui::Shape::Mesh(layout.mesh.clone()));
        for label in &layout.labels {
            painter.with_clip_rect(label.clip).text(
                label.pos,
                Align2::LEFT_TOP,
                &label.text,
                FontId::proportional(label.size),
                label.color,
            );
        }

        let hovered_tile = response.hover_pos().and_then(|p| layout.hit(p));
        self.hovered = hovered_tile.map(|t| layout.tiles[t].node);

        if let Some(t) = hovered_tile {
            let r = layout.tiles[t].rect;
            painter.rect_filled(r, 0.0, Color32::from_white_alpha(26));
            painter.rect_stroke(r, 0.0, Stroke::new(1.5_f32, Color32::from_white_alpha(210)), StrokeKind::Inside);
        }
        if let Some(sel) = self.selected
            && let Some(t) = layout.tile_for(tree, sel)
        {
            painter.rect_stroke(layout.tiles[t].rect, 0.0, Stroke::new(2.5_f32, SELECTED), StrokeKind::Inside);
        }

        if response.clicked() {
            actions.push(Action::Select(self.hovered));
        }
        if response.double_clicked()
            && let Some(node) = self.hovered
        {
            actions.push(Action::Zoom(node));
        }
        if response.secondary_clicked() {
            self.menu_node = self.hovered;
            if self.hovered.is_some() {
                actions.push(Action::Select(self.hovered));
            }
        }
        let (menu_node, view) = (self.menu_node, self.view);
        response.context_menu(|ui| context_menu(ui, tree, menu_node, view, actions));

        if let (Some(node), Some(pointer)) = (self.hovered, response.hover_pos())
            && !egui::Popup::is_any_open(ui.ctx())
        {
            tooltip(ui.ctx(), tree, node, self.view, pointer);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.first_frame {
            // On Windows the window is created hidden and loses `with_maximized`, so ask again once it's up.
            self.first_frame = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        self.poll_scan(ctx);
        let mut actions = Vec::new();

        let (back, escape) = ctx.input(|i| {
            (
                i.pointer.button_pressed(egui::PointerButton::Extra1) || i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Escape),
            )
        });
        if !ctx.wants_keyboard_input() {
            if back {
                actions.push(Action::Up);
            }
            if escape {
                actions.push(Action::Select(None));
            }
        }

        self.top_bar(ctx, &mut actions);
        self.status_bar(ctx);
        if self.job.is_none() {
            self.side_panel(ctx, &mut actions);
        }
        self.central(ctx, &mut actions);

        for action in actions {
            self.apply(action);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAST_PATH_KEY, &self.path_input);
    }
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(15.0).strong());
    ui.add_space(2.0);
}

fn details(ui: &mut egui::Ui, tree: &Tree, node: u32, view: u32, actions: &mut Vec<Action>) {
    let n = tree.node(node);
    ui.horizontal(|ui| {
        swatch(ui, tree.color_of(node));
        ui.add(egui::Label::new(RichText::new(tree.display_name(node)).size(17.0).strong()).wrap());
    });
    ui.add(egui::Label::new(RichText::new(tree.path_of(node).display().to_string()).size(12.0).weak()).wrap());
    ui.add_space(6.0);
    egui::Grid::new("details_grid").num_columns(2).spacing(vec2(16.0, 5.0)).show(ui, |ui| {
        let key = |ui: &mut egui::Ui, text: &str| ui.label(RichText::new(text).weak());
        key(ui, "Size");
        ui.label(RichText::new(format::bytes(n.size)).strong());
        ui.end_row();
        key(ui, "Exact size");
        ui.label(format!("{} bytes", format::count(n.size)));
        ui.end_row();
        if node != view && tree.is_inside(node, view) {
            key(ui, "Share of this view");
            ui.label(format::percent(n.size, tree.node(view).size));
            ui.end_row();
        }
        key(ui, "Share of whole scan");
        ui.label(format::percent(n.size, tree.node(0).size));
        ui.end_row();
        if n.is_dir {
            key(ui, "Contains");
            ui.label(format!("{} files, {} folders", format::count(n.files), format::count(n.dirs)));
        } else {
            key(ui, "Type");
            ui.label(&tree.exts[n.ext as usize].label);
        }
        ui.end_row();
    });
    ui.add_space(8.0);
    ui.horizontal_wrapped(|ui| {
        if n.is_dir && node != view && ui.button("🔍 Zoom in").clicked() {
            actions.push(Action::Zoom(node));
        }
        if ui.button(REVEAL_LABEL).clicked() {
            actions.push(Action::Reveal(node));
        }
        if ui.button("📋 Copy path").clicked() {
            actions.push(Action::CopyPath(node));
        }
    });
}

fn context_menu(ui: &mut egui::Ui, tree: &Tree, node: Option<u32>, view: u32, actions: &mut Vec<Action>) {
    ui.set_min_width(220.0);
    let Some(node) = node else {
        if view != 0 && ui.button("⬆ Zoom out").clicked() {
            actions.push(Action::Up);
            ui.close();
        }
        return;
    };
    let n = tree.node(node);
    ui.label(RichText::new(tree.display_name(node)).strong());
    ui.label(RichText::new(format::bytes(n.size)).weak());
    ui.separator();
    let target = if n.is_dir { node } else { n.parent };
    if target != view {
        let label = if n.is_dir { "🔍 Zoom into this folder" } else { "🔍 Zoom into its folder" };
        if ui.button(label).clicked() {
            actions.push(Action::Zoom(node));
            ui.close();
        }
    }
    if view != 0 && ui.button("⬆ Zoom out").clicked() {
        actions.push(Action::Up);
        ui.close();
    }
    if ui.button(REVEAL_LABEL).clicked() {
        actions.push(Action::Reveal(node));
        ui.close();
    }
    if ui.button("📋 Copy path").clicked() {
        actions.push(Action::CopyPath(node));
        ui.close();
    }
}

/// Drawn by hand so it follows the pointer straight away instead of waiting for the mouse to rest.
fn tooltip(ctx: &egui::Context, tree: &Tree, node: u32, view: u32, pointer: egui::Pos2) {
    let n = tree.node(node);
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("treemap_tooltip")));
    let wrap = 480.0;
    let name = painter.layout(tree.display_name(node), FontId::proportional(15.0), Color32::WHITE, wrap);
    let mut lines = vec![
        painter.layout(tree.path_of(node).display().to_string(), FontId::proportional(12.0), Color32::from_gray(150), wrap),
        painter.layout(
            format!("{}   ·   {} of this view", format::bytes(n.size), format::percent(n.size, tree.node(view).size)),
            FontId::proportional(13.5),
            Color32::from_gray(225),
            wrap,
        ),
    ];
    let kind = if n.is_dir {
        format!("Folder with {} files, {} folders", format::count(n.files), format::count(n.dirs))
    } else {
        format!("{} file", tree.exts[n.ext as usize].label)
    };
    lines.push(painter.layout(kind, FontId::proportional(12.5), Color32::from_gray(180), wrap));

    let pad = vec2(12.0, 10.0);
    let swatch_w = 18.0;
    let width = lines.iter().map(|g| g.size().x).fold(name.size().x + swatch_w, f32::max);
    let height = name.size().y + lines.iter().map(|g| g.size().y + 4.0).sum::<f32>();
    let size = vec2(width, height) + pad * 2.0;

    let screen = ctx.screen_rect();
    let mut origin = pointer + vec2(18.0, 20.0);
    if origin.x + size.x > screen.max.x - 6.0 {
        origin.x = pointer.x - 14.0 - size.x;
    }
    if origin.y + size.y > screen.max.y - 6.0 {
        origin.y = pointer.y - 14.0 - size.y;
    }
    let rect = Rect::from_min_size(origin, size);
    painter.rect_filled(rect, 7.0, Color32::from_rgba_unmultiplied(16, 17, 21, 245));
    painter.rect_stroke(rect, 7.0, Stroke::new(1.0_f32, Color32::from_gray(70)), StrokeKind::Inside);

    let mut y = rect.min.y + pad.y;
    let swatch_rect = Rect::from_min_size(pos2(rect.min.x + pad.x, y + name.size().y / 2.0 - 6.0), vec2(12.0, 12.0));
    painter.rect_filled(swatch_rect, 2.0, tree.color_of(node));
    painter.galley(pos2(rect.min.x + pad.x + swatch_w, y), name.clone(), Color32::WHITE);
    y += name.size().y + 4.0;
    for line in lines {
        let h = line.size().y;
        painter.galley(pos2(rect.min.x + pad.x, y), line, Color32::WHITE);
        y += h + 4.0;
    }
}

fn scanning_screen(ui: &mut egui::Ui, job: &ScanJob, actions: &mut Vec<Action>) {
    let p = &job.progress;
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.28);
        ui.add(egui::Spinner::new().size(52.0));
        ui.add_space(16.0);
        ui.label(RichText::new(format!("Scanning {}", job.path.display())).size(22.0).strong());
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!(
                "{} files  ·  {} folders  ·  {}",
                format::count(p.files.load(Relaxed)),
                format::count(p.dirs.load(Relaxed)),
                format::bytes(p.bytes.load(Relaxed))
            ))
            .size(16.0),
        );
        ui.label(RichText::new(format!("{:.1} s", job.started.elapsed().as_secs_f32())).weak());
        ui.add_space(10.0);
        let current = p.current.lock().map(|c| c.clone()).unwrap_or_default();
        ui.add(egui::Label::new(RichText::new(current).size(12.0).weak()).truncate());
        ui.add_space(18.0);
        if ui.add(egui::Button::new("⏹  Cancel").min_size(vec2(120.0, 32.0))).clicked() {
            actions.push(Action::Cancel);
        }
    });
}

fn welcome_screen(ui: &mut egui::Ui, drives: &[Drive], error: Option<&str>, actions: &mut Vec<Action>) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.2);
        ui.label(RichText::new("🔍").size(80.0));
        ui.add_space(4.0);
        ui.label(RichText::new("Where did all the space go?").size(30.0).strong());
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Pick a folder or a drive. Every file becomes a box sized by how much space it takes,\n\
                 and every folder becomes a box holding its contents.",
            )
            .size(15.0)
            .weak(),
        );
        ui.add_space(26.0);
        let browse = egui::Button::new(RichText::new("📁  Choose a folder…").size(18.0)).min_size(vec2(250.0, 48.0));
        if ui.add(browse).clicked() {
            actions.push(Action::Browse);
        }
        if !drives.is_empty() {
            ui.add_space(18.0);
            ui.label(RichText::new(format!("or scan a whole {DRIVE_WORD}")).weak());
            ui.add_space(4.0);
            // Rough widths, only used to centre the row of buttons.
            let gap = 8.0;
            let button_w = |label: &str| (label.chars().count() as f32 * 9.5 + 30.0).max(70.0);
            let width = drives.iter().map(|d| button_w(&d.label) + gap).sum::<f32>() - gap;
            ui.allocate_ui(vec2(width, 36.0), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for drive in drives {
                        let label = RichText::new(&drive.label).size(16.0);
                        let button = egui::Button::new(label).min_size(vec2(button_w(&drive.label), 34.0));
                        if ui.add(button).clicked() {
                            actions.push(Action::Scan(drive.path.clone()));
                        }
                    }
                });
            });
        }
        if let Some(error) = error {
            ui.add_space(18.0);
            ui.label(RichText::new(error).color(WARN).size(15.0));
        }
    });
}

fn swatch(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, color);
}

/// A clickable list row: colour swatch, a name that shortens with "…" if needed, and a value on the right.
fn item_row(ui: &mut egui::Ui, color: Color32, name: &str, value: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.visuals();
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, 3.0, visuals.selection.bg_fill);
    } else if response.hovered() {
        painter.rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let swatch = Rect::from_center_size(pos2(rect.min.x + 11.0, rect.center().y), vec2(11.0, 11.0));
    painter.rect_filled(swatch, 2.0, color);

    let value = painter.layout_no_wrap(value.to_string(), FontId::proportional(13.0), visuals.weak_text_color());
    let value_pos = pos2(rect.max.x - 6.0 - value.size().x, rect.center().y - value.size().y / 2.0);
    let name_left = rect.min.x + 24.0;
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), FontId::proportional(13.5), visuals.text_color());
    job.wrap = egui::text::TextWrapping::truncate_at_width((value_pos.x - name_left - 12.0).max(10.0));
    let name = painter.layout_job(job);
    painter.galley(pos2(name_left, rect.center().y - name.size().y / 2.0), name, visuals.text_color());
    painter.galley(value_pos, value, visuals.weak_text_color());
    response
}

fn normalize_path(input: &str) -> PathBuf {
    let trimmed = input.trim().trim_matches('"');
    // "C:" on its own means "current folder on drive C", which is never what's wanted here.
    if trimmed.len() == 2 && trimmed.ends_with(':') {
        return PathBuf::from(format!("{trimmed}\\"));
    }
    PathBuf::from(trimmed)
}

#[cfg(windows)]
fn list_drives() -> Vec<Drive> {
    (b'A'..=b'Z')
        .map(|c| format!("{}:\\", c as char))
        .filter(|path| Path::new(path).exists())
        .map(|path| Drive { label: path.trim_end_matches('\\').to_string(), path })
        .collect()
}

#[cfg(target_os = "macos")]
fn list_drives() -> Vec<Drive> {
    // Every mounted disk appears in /Volumes. The startup disk is there as a link back to "/".
    let mut startup = Drive { label: "Startup disk".into(), path: "/".into() };
    let mut others = Vec::new();
    for entry in std::fs::read_dir("/Volumes").into_iter().flatten().flatten() {
        let label = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_symlink() {
            if std::fs::read_link(entry.path()).is_ok_and(|target| target == Path::new("/")) {
                startup.label = label;
            }
        } else if file_type.is_dir() {
            others.push(Drive { label, path: entry.path().display().to_string() });
        }
    }
    others.sort_by(|a, b| a.label.cmp(&b.label));
    std::iter::once(startup).chain(others).collect()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn list_drives() -> Vec<Drive> {
    vec![Drive { label: "/".into(), path: "/".into() }]
}

fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
    }
}
