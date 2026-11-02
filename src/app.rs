use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui::text::{LayoutJob, TextWrapping};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Id, Key, LayerId, Layout as EguiLayout, Margin, Order, Painter,
    Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, epaint::TextShape, pos2, vec2,
};

use crate::assets::Assets;
use crate::demo::{Demo, Step};
use crate::detective::{self, Act, Detective};
use crate::format;
use crate::scan::{self, NO_PARENT, Progress, RANKED_COLORS, Tree};
use crate::theme::{self, ButtonStyle, Icon};
use crate::treemap::{self, Layout};

const LAST_PATH_KEY: &str = "last_path";
const MUTED_KEY: &str = "muted";
const SIDE_WIDTH: f32 = 432.0;
const STAGE_HEIGHT: f32 = 300.0;
const HIGHLIGHT: Color32 = Color32::from_rgb(255, 214, 64);

#[cfg(windows)]
const REVEAL_LABEL: &str = "Show in Explorer";
#[cfg(target_os = "macos")]
const REVEAL_LABEL: &str = "Show in Finder";
#[cfg(not(any(windows, target_os = "macos")))]
const REVEAL_LABEL: &str = "Show in file manager";

#[cfg(windows)]
const DRIVE_WORD: &str = "drive";
#[cfg(windows)]
const DRIVES_TITLE: &str = "Drives:";
#[cfg(not(windows))]
const DRIVE_WORD: &str = "disk";
#[cfg(not(windows))]
const DRIVES_TITLE: &str = "Disks:";

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

#[derive(Clone, Copy, PartialEq)]
enum ListTab {
    Largest,
    Types,
}

/// Things the panels ask for; applied once drawing is done so the tree isn't borrowed.
enum Action {
    Browse,
    Scan(String),
    Cancel,
    Zoom(u32),
    Up,
    Select(Option<u32>),
    /// Send the detective's glass over to a box on the map.
    Inspect(u32, Rect),
    Reveal(u32),
    CopyPath(u32),
    Tab(ListTab),
    ToggleMagnifier,
    ToggleMute,
}

pub struct App {
    ctx: egui::Context,
    assets: Assets,
    detective: Detective,
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
    tab: ListTab,
    stage: Rect,
    map_rect: Option<Rect>,
    hole: Option<Pos2>,
    /// The pin on the Selected card, where the clue string ends.
    card_pin: Option<Pos2>,
    selected_rect: Option<Rect>,
    last_spoken: Option<u32>,
    spoke_empty_for: Option<u32>,
    roll: u32,
    demo: Option<Demo>,
    /// Stands in for the mouse during the demo walkthrough.
    demo_pointer: Option<Pos2>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start_path: Option<PathBuf>) -> Self {
        let ctx = cc.egui_ctx.clone();
        theme::install(&ctx);
        let assets = Assets::load(&ctx);

        let remembered = cc.storage.and_then(|s| eframe::get_value::<String>(s, LAST_PATH_KEY));
        let muted = cc.storage.and_then(|s| eframe::get_value::<bool>(s, MUTED_KEY)).unwrap_or(false);
        let path_input = remembered
            .or_else(|| std::env::var("USERPROFILE").ok())
            .or_else(|| std::env::var("HOME").ok())
            .unwrap_or_default();

        let detective = Detective::new(&assets, muted);
        let mut app = Self {
            ctx,
            assets,
            detective,
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
            tab: ListTab::Largest,
            stage: Rect::NOTHING,
            map_rect: None,
            hole: None,
            card_pin: None,
            selected_rect: None,
            last_spoken: None,
            spoke_empty_for: None,
            roll: 12345,
            demo: Demo::from_env(),
            demo_pointer: None,
        };
        match start_path {
            Some(path) => app.start_scan(path.display().to_string()),
            None => app.detective.say_later("welcome", 1.2),
        }
        app
    }

    fn roll(&mut self) -> f32 {
        self.roll ^= self.roll << 13;
        self.roll ^= self.roll >> 17;
        self.roll ^= self.roll << 5;
        (self.roll % 1000) as f32 / 1000.0
    }

    fn start_scan(&mut self, input: String) {
        let path = normalize_path(&input);
        if !path.is_dir() {
            self.error = Some(format!("“{}” isn't a folder I can open.", input.trim()));
            self.detective.say("bad_path");
            self.detective.react(Act::Worried, 3.0);
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
        let whole_drive = path.parent().is_none();
        self.job = Some(ScanJob { path, progress, rx, started: Instant::now() });
        self.detective.begin_scan(whole_drive);
    }

    fn poll_scan(&mut self) {
        let Some(job) = &self.job else { return };
        match job.rx.try_recv() {
            Ok(tree) => {
                self.job = None;
                let root = tree.node(0);
                let result = if root.size >= 500 << 30 {
                    "scan_done_huge"
                } else if root.size < 1 << 30 {
                    "scan_done_small"
                } else {
                    "scan_done"
                };
                let errors = tree.errors;
                self.tree = Some(tree);
                self.view = 0;
                self.selected = None;
                self.hovered = None;
                self.layout = None;
                self.largest = None;
                self.last_spoken = None;
                self.detective.end_scan(Some(result));
                if errors > 0 && self.roll() < 0.5 {
                    self.detective.say_later("unreadable", 0.5);
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.job = None;
                self.error = Some("The scan stopped unexpectedly.".into());
                self.detective.end_scan(Some("error"));
            }
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Browse => {
                let mut dialog = rfd::FileDialog::new().set_title("Choose a folder to investigate");
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
                    self.detective.end_scan(Some("scan_cancel"));
                }
            }
            Action::Select(node) => self.selected = node,
            Action::Inspect(node, rect) => {
                self.selected = Some(node);
                let roll = self.roll();
                if self.last_spoken != Some(node)
                    && let Some(tree) = &self.tree
                {
                    let n = tree.node(node);
                    let event = detective::event_for(&n.name, n.size, n.is_dir, tree.node(self.view).size, roll);
                    self.last_spoken = Some(node);
                    self.detective.investigate(rect);
                    self.detective.say(event);
                }
            }
            Action::Up => {
                if let Some(tree) = &self.tree
                    && self.view != 0
                {
                    self.view = tree.node(self.view).parent;
                    self.detective.say("zoom_out");
                    self.detective.hop();
                }
            }
            Action::Zoom(node) => {
                if let Some(tree) = &self.tree {
                    let target = if tree.node(node).is_dir { node } else { tree.node(node).parent };
                    if target != NO_PARENT && target != self.view {
                        self.view = target;
                        self.detective.say("zoom_in");
                        self.detective.react(Act::Suspicious, 2.0);
                    }
                }
            }
            Action::Reveal(node) => {
                if let Some(tree) = &self.tree {
                    reveal(&tree.path_of(node));
                    self.detective.say("reveal");
                    self.detective.hop();
                }
            }
            Action::CopyPath(node) => {
                if let Some(tree) = &self.tree {
                    self.ctx.copy_text(tree.path_of(node).display().to_string());
                    self.detective.say("copy_path");
                    self.detective.hop();
                }
            }
            Action::Tab(tab) => self.tab = tab,
            Action::ToggleMagnifier => {
                let on = !self.detective.magnifier_on();
                self.detective.set_magnifier(on);
            }
            Action::ToggleMute => {
                let muted = !self.detective.muted();
                self.detective.set_muted(muted);
            }
        }
    }

    // ------------------------------------------------------------------------------------------
    // Header

    fn header(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let frame = Frame::NONE.inner_margin(Margin { left: 18, right: 18, top: 10, bottom: 2 });
        egui::TopBottomPanel::top("header").frame(frame).show_separator_line(false).show(ctx, |ui| {
            let scanning = self.job.is_some();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                // Logo: his magnifying glass and the title in storybook capitals.
                let (logo, _) = ui.allocate_exact_size(vec2(352.0, 58.0), Sense::hover());
                theme::sprite_rotated(ui.painter(), logo.left_center() + vec2(24.0, 2.0), 56.0, -0.5, self.assets.part("glass"), Color32::WHITE);
                let title_at = logo.left_center() + vec2(58.0, 2.0);
                ui.painter().text(title_at + vec2(2.0, 3.0), Align2::LEFT_CENTER, "Space Detective", theme::title(40.0), Color32::from_black_alpha(150));
                ui.painter().text(title_at, Align2::LEFT_CENTER, "Space Detective", theme::title(40.0), theme::CREAM);

                if theme::button(ui, "Browse…", Some(Icon::Folder), ButtonStyle::Normal, !scanning).clicked() {
                    actions.push(Action::Browse);
                }
                let drives_w = 110.0 + self.drives.len() as f32 * 52.0;
                let field_w = (ui.available_width() - drives_w - 150.0).max(220.0);
                let edit = egui::TextEdit::singleline(&mut self.path_input)
                    .font(theme::medium(18.0))
                    .text_color(theme::CREAM)
                    .background_color(theme::NAVY_DEEP)
                    .margin(vec2(12.0, 7.0))
                    .desired_width(field_w)
                    .hint_text("Folder to investigate, e.g. C:\\Users");
                let response = ui.add_enabled(!scanning, edit);
                ui.painter().rect_stroke(response.rect, CornerRadius::same(6), Stroke::new(1.5_f32, theme::GOLD_DARK), StrokeKind::Outside);
                let entered = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if scanning {
                    if theme::button(ui, "Cancel", Some(Icon::Stop), ButtonStyle::Primary, true).clicked() {
                        actions.push(Action::Cancel);
                    }
                } else if theme::button(ui, "Scan", Some(Icon::Play), ButtonStyle::Primary, true).clicked() || entered {
                    actions.push(Action::Scan(self.path_input.clone()));
                }

                ui.with_layout(EguiLayout::right_to_left(Align::Center), |ui| {
                    let current = self.tree.as_ref().map(|t| t.root_path.display().to_string());
                    for drive in self.drives.iter().rev() {
                        let chosen = current.as_deref().is_some_and(|c| c.starts_with(drive.path.as_str()));
                        let style = if chosen { ButtonStyle::Chosen } else { ButtonStyle::Normal };
                        let response = theme::button(ui, &drive.label, None, style, !scanning);
                        if response.on_hover_text(format!("Investigate the whole {} {DRIVE_WORD}", drive.label)).clicked() {
                            actions.push(Action::Scan(drive.path.clone()));
                        }
                    }
                    if !self.drives.is_empty() {
                        ui.label(egui::RichText::new(DRIVES_TITLE).font(theme::medium(17.0)).color(theme::CREAM));
                    }
                });
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (Some(tree), false) = (&self.tree, scanning) else {
                    ui.add_space(32.0);
                    return;
                };
                if self.view != 0 && theme::tab(ui, "Up", Some(Icon::Up), false).on_hover_text("Zoom out one level (Backspace)").clicked() {
                    actions.push(Action::Up);
                }
                for (i, node) in tree.ancestry(self.view).into_iter().enumerate() {
                    let current = node == self.view;
                    let icon = if i == 0 { Some(Icon::Folder) } else { None };
                    if theme::tab(ui, &tree.display_name(node), icon, current).clicked() && !current {
                        actions.push(Action::Zoom(node));
                    }
                }
                ui.with_layout(EguiLayout::right_to_left(Align::Center), |ui| {
                    let v = tree.node(self.view);
                    let text = format!("{}   •   {} files   •   {} folders", format::bytes(v.size), format::count(v.files), format::count(v.dirs));
                    let galley = ui.painter().layout_no_wrap(text, theme::bold(18.0), theme::INK);
                    let (rect, _) = ui.allocate_exact_size(galley.size() + vec2(44.0, 12.0), Sense::hover());
                    theme::paper(ui.painter(), &theme::deckled(rect, 7, 2.2), &self.assets.parchment, Color32::WHITE, 4.0);
                    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, theme::INK);
                });
            });
        });
    }

    fn status_bar(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").exact_height(40.0).frame(Frame::NONE).show_separator_line(false).show(ctx, |ui| {
            let rect = ui.max_rect();
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::ZERO, theme::NAVY_DEEP.gamma_multiply(0.92));
            painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0_f32, theme::GOLD_DARK));
            theme::sprite_rotated(painter, rect.left_center() + vec2(26.0, 0.0), 34.0, 0.0, self.assets.prop("compass"), Color32::WHITE);
            let mut x = rect.min.x + 52.0;
            let y = rect.center().y;
            let text = match (&self.tree, &self.job) {
                (_, Some(job)) => format!("Investigating {}…", job.path.display()),
                (Some(tree), None) => {
                    let root = tree.node(0);
                    format!(
                        "{} in {} files and {} folders   •   scanned in {:.1} s",
                        format::bytes(root.size),
                        format::count(root.files),
                        format::count(root.dirs),
                        tree.elapsed.as_secs_f32()
                    )
                }
                (None, None) => "No case open yet. Pick a folder to investigate.".to_string(),
            };
            let r = painter.text(pos2(x, y), Align2::LEFT_CENTER, text, theme::medium(16.0), theme::CREAM);
            x = r.max.x + 14.0;
            if let (Some(tree), None) = (&self.tree, &self.job)
                && tree.errors > 0
            {
                let warn = format!("•   {} couldn't be read", format::count(tree.errors));
                let r = painter.text(pos2(x, y), Align2::LEFT_CENTER, warn, theme::medium(16.0), theme::AMBER);
                ui.interact(r, Id::new("unreadable_hint"), Sense::hover()).on_hover_text(UNREADABLE_HINT);
            }
            painter.text(pos2(rect.max.x - SIDE_WIDTH - 16.0, y), Align2::RIGHT_CENTER, "Every byte tells a story.", theme::hand(23.0), theme::CREAM.gamma_multiply(0.85));
        });
    }

    // ------------------------------------------------------------------------------------------
    // Side panel: the detective's corner and his case notes

    fn side_panel(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let frame = Frame::NONE.inner_margin(Margin { left: 4, right: 16, top: 2, bottom: 6 });
        egui::SidePanel::right("case_file").exact_width(SIDE_WIDTH).resizable(false).frame(frame).show_separator_line(false).show(ctx, |ui| {
            let full = ui.available_rect_before_wrap();
            self.stage = Rect::from_min_size(full.min, vec2(full.width(), STAGE_HEIGHT));
            self.stage_buttons(ui, actions);

            let selected_rect = Rect::from_min_size(pos2(full.min.x + 4.0, self.stage.max.y + 6.0), vec2(full.width() - 8.0, 238.0));
            self.selected_card(ui, selected_rect, actions);

            let list_top = selected_rect.max.y + 26.0;
            let list_rect = Rect::from_min_max(pos2(full.min.x + 4.0, list_top), pos2(full.max.x - 4.0, full.max.y - 64.0));
            self.evidence_card(ui, list_rect, actions);
        });
    }

    fn stage_buttons(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let base = self.stage.right_bottom() + vec2(-18.0, -18.0);
        let muted = self.detective.muted();
        let magnifier = self.detective.magnifier_on();
        let buttons = [
            (base, if muted { Icon::SpeakerOff } else { Icon::SpeakerOn }, false, if muted { "Let the detective speak" } else { "Mute the detective" }, Action::ToggleMute),
            (base + vec2(-42.0, 0.0), Icon::Glass, magnifier, "Borrow his magnifying glass (M)", Action::ToggleMagnifier),
        ];
        for (i, (center, icon, on, tip, action)) in buttons.into_iter().enumerate() {
            let rect = Rect::from_center_size(center, Vec2::splat(34.0));
            let response = ui.interact(rect, Id::new(("stage_button", i)), Sense::click());
            let painter = ui.painter();
            let fill = if on { theme::GOLD } else if response.hovered() { theme::NAVY_LIGHT } else { theme::NAVY_MID };
            painter.circle_filled(center + vec2(0.0, 2.0), 17.0, Color32::from_black_alpha(90));
            painter.circle_filled(center, 17.0, fill);
            painter.circle_stroke(center, 17.0, Stroke::new(1.5_f32, if response.hovered() { theme::GOLD_BRIGHT } else { theme::GOLD_DARK }));
            theme::icon(painter, Rect::from_center_size(center, Vec2::splat(18.0)), icon, if on { theme::NAVY_DEEP } else { theme::GOLD_BRIGHT });
            if response.on_hover_text(tip).clicked() {
                actions.push(action);
            }
        }
    }

    fn selected_card(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let painter = ui.painter().clone();
        theme::card(&painter, rect, &self.assets.parchment, 21);
        let pin = rect.left_top() + vec2(20.0, 16.0);
        self.card_pin = Some(pin);
        let inner = rect.shrink2(vec2(22.0, 14.0));
        painter.text(inner.min + vec2(18.0, 0.0), Align2::LEFT_TOP, "Selected", theme::heading(22.0), theme::INK);
        theme::sprite_rotated(&painter, rect.right_top() + vec2(-62.0, 104.0), 100.0, 0.25, self.assets.prop("fingerprint"), Color32::from_white_alpha(60));
        theme::sprite_rotated(&painter, rect.right_top() + vec2(-128.0, 12.0), 60.0, 0.18, self.assets.prop("paperclip"), Color32::WHITE);

        let (Some(tree), Some(node)) = (&self.tree, self.selected) else {
            let text = if self.job.is_some() {
                "The detective is busy rummaging.\nDo hold on a moment."
            } else {
                "Click any box to investigate it.\nDouble-click to go inside.\nRight-click for more."
            };
            painter.text(inner.min + vec2(0.0, 44.0), Align2::LEFT_TOP, text, theme::hand(21.0), theme::INK_SOFT);
            theme::pin(&painter, pin, theme::RED_INK);
            return;
        };
        let n = tree.node(node);
        let mut y = inner.min.y + 36.0;
        let icon_rect = Rect::from_min_size(pos2(inner.min.x, y + 2.0), vec2(26.0, 22.0));
        if n.is_dir {
            theme::folder_icon(&painter, icon_rect, Color32::from_rgb(232, 182, 84));
        } else {
            painter.rect_filled(icon_rect.shrink(3.0), CornerRadius::same(3), tree.color_of(node));
        }
        let name = painter.layout_job(single_line(tree.display_name(node), theme::bold(21.0), theme::INK, inner.width() - 34.0));
        painter.galley(pos2(inner.min.x + 34.0, y), name, theme::INK);
        y += 34.0;

        let label_w = 86.0;
        let row = |y: &mut f32, label: &str, value: String, lines: usize| {
            painter.text(pos2(inner.min.x, *y), Align2::LEFT_TOP, label, theme::medium(16.0), theme::INK_SOFT);
            let mut job = LayoutJob::simple(value, theme::ui_font(16.0), theme::INK, inner.width() - label_w);
            job.wrap = TextWrapping { max_rows: lines, break_anywhere: true, ..TextWrapping::wrap_at_width(inner.width() - label_w) };
            let galley = painter.layout_job(job);
            let h = galley.size().y;
            painter.galley(pos2(inner.min.x + label_w, *y), galley, theme::INK);
            *y += h + 3.0;
        };
        let location = tree.path_of(node).parent().map(|p| p.display().to_string()).unwrap_or_default();
        row(&mut y, "Location:", location, 2);
        row(&mut y, "Size:", format!("{}  ({} bytes)", format::bytes(n.size), format::count(n.size)), 1);
        if n.is_dir {
            row(&mut y, "Contains:", format!("{} files, {} folders", format::count(n.files), format::count(n.dirs)), 1);
        } else {
            row(&mut y, "Type:", tree.exts[n.ext as usize].label.clone(), 1);
        }
        row(&mut y, "Share:", format!("{} of this view, {} of the scan", format::percent(n.size, tree.node(self.view).size), format::percent(n.size, tree.node(0).size)), 1);

        let buttons = Rect::from_min_max(pos2(inner.min.x, rect.max.y - 48.0), pos2(inner.max.x, rect.max.y - 12.0));
        let is_dir = n.is_dir;
        ui.scope_builder(UiBuilder::new().max_rect(buttons), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if is_dir && node != self.view && theme::button(ui, "Go inside", None, ButtonStyle::Normal, true).clicked() {
                    actions.push(Action::Zoom(node));
                }
                if theme::button(ui, REVEAL_LABEL, None, ButtonStyle::Normal, true).clicked() {
                    actions.push(Action::Reveal(node));
                }
                if theme::button(ui, "Copy path", None, ButtonStyle::Normal, true).clicked() {
                    actions.push(Action::CopyPath(node));
                }
            });
        });
        theme::pin(&painter, pin, theme::RED_INK);
    }

    fn evidence_card(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        // Tabs poke up above the card like folder tabs.
        let tabs = [(ListTab::Largest, "Largest files here"), (ListTab::Types, "File types")];
        let mut x = rect.min.x + 14.0;
        for (tab, label) in tabs {
            let current = self.tab == tab;
            let galley = ui.painter().layout_no_wrap(label.to_string(), theme::heading(17.0), theme::INK);
            let tab_rect = Rect::from_min_size(pos2(x, rect.min.y - 22.0), vec2(galley.size().x + 28.0, 30.0));
            let response = ui.interact(tab_rect, Id::new(("evidence_tab", label)), Sense::click());
            let tint = if current { Color32::WHITE } else { Color32::from_gray(if response.hovered() { 215 } else { 190 }) };
            theme::paper(ui.painter(), &theme::deckled(tab_rect, x as u32, 1.0), &self.assets.parchment, tint, 2.0);
            ui.painter().galley(tab_rect.min + vec2(14.0, 5.0), galley, if current { theme::INK } else { theme::INK_SOFT });
            if response.clicked() {
                actions.push(Action::Tab(tab));
            }
            x = tab_rect.max.x + 6.0;
        }

        let painter = ui.painter().clone();
        theme::card(&painter, rect, &self.assets.parchment, 33);
        // Lined notebook paper.
        let row_h = 25.0;
        let list = Rect::from_min_max(rect.min + vec2(10.0, 14.0), rect.max - vec2(10.0, 10.0));
        let mut ly = list.min.y + row_h;
        while ly < list.max.y {
            painter.line_segment([pos2(rect.min.x + 4.0, ly), pos2(rect.max.x - 4.0, ly)], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(110, 150, 200, 70)));
            ly += row_h;
        }
        painter.line_segment([pos2(list.min.x + 30.0, rect.min.y + 2.0), pos2(list.min.x + 30.0, rect.max.y - 2.0)], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(200, 80, 70, 90)));

        let Some(tree) = &self.tree else {
            let text = if self.job.is_some() { "Gathering evidence…" } else { "No evidence yet." };
            painter.text(list.min + vec2(40.0, 2.0), Align2::LEFT_TOP, text, theme::hand(21.0), theme::INK_SOFT);
            return;
        };
        if self.largest.as_ref().is_none_or(|(view, _)| *view != self.view) {
            self.largest = Some((self.view, tree.largest_files(self.view, 40)));
        }
        let largest = &self.largest.as_ref().unwrap().1;

        // A little rubber stamp in the corner.
        let stamp = if self.tab == ListTab::Largest { format!("TOP {}", largest.len()) } else { format!("{} TYPES", tree.ext_rank.len().min(RANKED_COLORS)) };
        let galley = painter.layout_no_wrap(stamp, theme::heading(15.0), theme::RED_INK.gamma_multiply(0.85));
        let at = rect.right_top() + vec2(-galley.size().x - 30.0, -20.0);
        let box_rect = Rect::from_min_size(at - vec2(6.0, 3.0), galley.size() + vec2(12.0, 6.0));
        painter.add(Shape::Text(TextShape::new(at, galley, theme::RED_INK).with_angle(0.1)));
        let (s, c) = 0.1f32.sin_cos();
        let corners: Vec<Pos2> = [box_rect.left_top(), box_rect.right_top(), box_rect.right_bottom(), box_rect.left_bottom()]
            .iter()
            .map(|p| at + vec2((p.x - at.x) * c - (p.y - at.y) * s, (p.x - at.x) * s + (p.y - at.y) * c))
            .collect();
        painter.add(Shape::closed_line(corners, Stroke::new(1.5_f32, theme::RED_INK.gamma_multiply(0.8))));

        let selected = self.selected;
        let tab = self.tab;
        let layout = self.layout.as_ref().map(|(_, l)| l);
        ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                match tab {
                    ListTab::Largest => {
                        if largest.is_empty() {
                            ui.label(egui::RichText::new("No files in this folder.").font(theme::hand(20.0)).color(theme::INK_SOFT));
                        }
                        for &file in largest {
                            let n = tree.node(file);
                            let row = evidence_row(ui, tree.color_of(file), &n.name, &format::bytes(n.size), selected == Some(file), row_h);
                            if row.clicked() {
                                match layout.and_then(|l| l.tile_for(tree, file)) {
                                    Some(t) => actions.push(Action::Inspect(file, layout.unwrap().tiles[t].rect)),
                                    None => actions.push(Action::Select(Some(file))),
                                }
                            }
                            if row.double_clicked() {
                                actions.push(Action::Zoom(file));
                            }
                            row.on_hover_text(tree.path_of(file).display().to_string());
                        }
                    }
                    ListTab::Types => {
                        let total = tree.node(0).size;
                        for &id in tree.ext_rank.iter().take(RANKED_COLORS) {
                            let ext = &tree.exts[id as usize];
                            if ext.bytes == 0 {
                                continue;
                            }
                            let value = format!("{}   {}", format::bytes(ext.bytes), format::percent(ext.bytes, total));
                            evidence_row(ui, ext.color, &ext.label, &value, false, row_h).on_hover_text(format!("{} files", format::count(ext.count)));
                        }
                        let rest: u64 = tree.ext_rank.iter().skip(RANKED_COLORS).map(|&id| tree.exts[id as usize].bytes).sum();
                        if rest > 0 {
                            let value = format!("{}   {}", format::bytes(rest), format::percent(rest, total));
                            evidence_row(ui, Color32::from_gray(150), "Everything else", &value, false, row_h);
                        }
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------------------------------------
    // The map

    fn central(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let frame = Frame::NONE.inner_margin(Margin { left: 18, right: 10, top: 8, bottom: 10 });
        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            let painter = ui.painter();
            painter.rect_filled(rect.translate(vec2(0.0, 4.0)), CornerRadius::same(8), Color32::from_black_alpha(110));
            painter.rect_filled(rect, CornerRadius::same(8), theme::NAVY_DEEP);
            painter.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.5_f32, theme::GOLD_DARK), StrokeKind::Outside);
            let inner = rect.shrink(6.0);
            self.map_rect = Some(inner);
            self.hole = None;
            if self.job.is_some() {
                self.scanning_card(ui, inner, actions);
            } else if self.tree.is_some() {
                let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
                self.treemap_ui(&mut child, inner, actions);
            } else {
                self.welcome_card(ui, inner, actions);
            }
        });
    }

    fn scanning_card(&mut self, ui: &mut Ui, area: Rect, actions: &mut Vec<Action>) {
        let Some(job) = &self.job else { return };
        let rect = Rect::from_center_size(area.center(), vec2(760.0f32.min(area.width() - 40.0), 500.0f32.min(area.height() - 40.0)));
        let painter = ui.painter().clone();
        theme::card(&painter, rect, &self.assets.parchment, 51);
        theme::pin(&painter, rect.left_top() + vec2(22.0, 18.0), theme::RED_INK);
        theme::pin(&painter, rect.right_top() + vec2(-22.0, 18.0), theme::RED_INK);
        let p = &job.progress;
        let mut y = rect.min.y + 34.0;
        let cx = rect.center().x;
        painter.text(pos2(cx, y), Align2::CENTER_TOP, "Investigation in progress", theme::heading(32.0), theme::INK);
        y += 48.0;
        let path = painter.layout_job(single_line(job.path.display().to_string(), theme::medium(18.0), theme::INK_SOFT, rect.width() - 80.0));
        painter.galley(pos2(cx - path.size().x / 2.0, y), path, theme::INK_SOFT);
        y += 36.0;
        let counts = format!("{} files   •   {} folders   •   {}", format::count(p.files.load(Relaxed)), format::count(p.dirs.load(Relaxed)), format::bytes(p.bytes.load(Relaxed)));
        painter.text(pos2(cx, y), Align2::CENTER_TOP, counts, theme::bold(24.0), theme::INK);
        y += 38.0;
        let current = p.current.lock().map(|c| c.clone()).unwrap_or_default();
        let current = painter.layout_job(single_line(current, theme::ui_font(14.0), theme::INK_SOFT, rect.width() - 80.0));
        painter.galley(pos2(cx - current.size().x / 2.0, y), current, theme::INK_SOFT);
        painter.text(rect.left_bottom() + vec2(30.0, -24.0), Align2::LEFT_BOTTOM, format!("{:.0} s on the case", job.started.elapsed().as_secs_f32()), theme::hand(21.0), theme::INK_SOFT);

        self.hole = Some(pos2(cx, rect.max.y - 110.0));
        let cancel = Rect::from_min_size(rect.right_bottom() + vec2(-150.0, -64.0), vec2(126.0, 40.0));
        ui.scope_builder(UiBuilder::new().max_rect(cancel), |ui| {
            if theme::button(ui, "Cancel", Some(Icon::Stop), ButtonStyle::Normal, true).clicked() {
                actions.push(Action::Cancel);
            }
        });
    }

    fn welcome_card(&mut self, ui: &mut Ui, area: Rect, actions: &mut Vec<Action>) {
        let rect = Rect::from_center_size(area.center(), vec2(780.0f32.min(area.width() - 40.0), 440.0));
        let painter = ui.painter().clone();
        theme::card(&painter, rect, &self.assets.parchment, 61);
        theme::pin(&painter, rect.center_top() + vec2(0.0, 18.0), theme::RED_INK);
        let cx = rect.center().x;
        painter.text(pos2(cx, rect.min.y + 48.0), Align2::CENTER_TOP, "Where did all the space go?", theme::title(40.0), theme::INK);
        painter.text(
            pos2(cx, rect.min.y + 112.0),
            Align2::CENTER_TOP,
            "Point the detective at a folder or a whole drive. Every file becomes a box\nsized by how much space it takes, tucked inside boxes for its folders.",
            theme::ui_font(19.0),
            theme::INK_SOFT,
        );
        let button_area = Rect::from_center_size(pos2(cx, rect.min.y + 230.0), vec2(320.0, 48.0));
        ui.scope_builder(UiBuilder::new().max_rect(button_area).layout(EguiLayout::top_down(Align::Center)), |ui| {
            if theme::button(ui, "Choose a folder…", Some(Icon::Folder), ButtonStyle::Primary, true).clicked() {
                actions.push(Action::Browse);
            }
        });
        if !self.drives.is_empty() {
            painter.text(pos2(cx, rect.min.y + 290.0), Align2::CENTER_TOP, format!("or investigate a whole {DRIVE_WORD}"), theme::hand(22.0), theme::INK_SOFT);
            let width = self.drives.iter().map(|d| d.label.len() as f32 * 10.0 + 32.0 + 10.0).sum::<f32>();
            let row = Rect::from_center_size(pos2(cx, rect.min.y + 346.0), vec2(width, 36.0));
            ui.scope_builder(UiBuilder::new().max_rect(row), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    for drive in &self.drives {
                        if theme::button(ui, &drive.label, None, ButtonStyle::Normal, true).clicked() {
                            actions.push(Action::Scan(drive.path.clone()));
                        }
                    }
                });
            });
        }
        if let Some(error) = &self.error {
            painter.text(pos2(cx, rect.max.y - 30.0), Align2::CENTER_BOTTOM, error, theme::medium(17.0), theme::RED_INK);
        }
    }

    fn treemap_ui(&mut self, ui: &mut Ui, rect: Rect, actions: &mut Vec<Action>) {
        let Some(tree) = &self.tree else { return };
        let response = ui.interact(rect, Id::new("treemap"), Sense::click());
        if tree.node(self.view).size == 0 {
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "This folder is empty", theme::hand(30.0), theme::CREAM);
            if self.spoke_empty_for != Some(self.view) {
                self.spoke_empty_for = Some(self.view);
                self.detective.say("empty");
                self.detective.react(Act::Puzzled, 2.5);
            }
            return;
        }

        let ppp = ui.ctx().pixels_per_point();
        let key = LayoutKey { view: self.view, rect, ppp };
        if self.layout.as_ref().is_none_or(|(k, _)| *k != key) {
            self.layout = Some((key, treemap::build(tree, self.view, rect, ppp, self.assets.grain.id())));
        }
        let layout = &self.layout.as_ref().unwrap().1;

        let painter = ui.painter_at(rect);
        painter.add(Shape::Mesh(layout.mesh.clone()));
        for icon in &layout.icons {
            theme::folder_icon(&painter, *icon, Color32::from_rgb(232, 182, 84));
        }
        for label in &layout.labels {
            painter.with_clip_rect(label.clip).text(label.pos, Align2::LEFT_TOP, &label.text, theme::ui_font(label.size), label.color);
        }

        let hovered_tile = response.hover_pos().and_then(|p| layout.hit(p));
        self.hovered = hovered_tile.map(|t| layout.tiles[t].node);
        if let Some(t) = hovered_tile {
            let r = layout.tiles[t].rect;
            painter.rect_filled(r, 0.0, Color32::from_white_alpha(22));
            painter.rect_stroke(r, 0.0, Stroke::new(1.5_f32, theme::CREAM.gamma_multiply(0.85)), StrokeKind::Inside);
        }
        self.selected_rect = None;
        if let Some(sel) = self.selected
            && let Some(t) = layout.tile_for(tree, sel)
        {
            let r = layout.tiles[t].rect;
            self.selected_rect = Some(r);
            painter.rect_stroke(r.expand(1.5), CornerRadius::same(2), Stroke::new(5.0_f32, Color32::from_rgba_unmultiplied(255, 200, 40, 60)), StrokeKind::Outside);
            painter.rect_stroke(r, CornerRadius::same(2), Stroke::new(3.0_f32, HIGHLIGHT), StrokeKind::Inside);
            if r.width() > 90.0 && r.height() > 70.0 {
                let size = (r.height() * 0.5).min(r.width() * 0.4).min(120.0);
                theme::sprite_rotated(&painter, r.right_bottom() - Vec2::splat(size * 0.62), size, -0.3, self.assets.prop("fingerprint"), Color32::from_white_alpha(80));
            }
        }

        if response.clicked() {
            match (self.hovered, hovered_tile) {
                (Some(node), Some(t)) => actions.push(Action::Inspect(node, layout.tiles[t].rect)),
                _ => actions.push(Action::Select(None)),
            }
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
            && !self.detective.magnifier_on()
        {
            tooltip(ui.ctx(), &self.assets, tree, node, self.view, pointer);
        }
    }

    // ------------------------------------------------------------------------------------------
    // On top of everything: the clue string, the desk props and the detective himself

    fn overlay(&mut self, ctx: &egui::Context) {
        let front = ctx.layer_painter(LayerId::new(Order::Middle, Id::new("detective_front")));
        let back = ctx.layer_painter(LayerId::background()).with_clip_rect(self.stage.expand(40.0));
        let screen = ctx.screen_rect();

        if let (Some(tile), Some(pin), None) = (self.selected_rect, self.card_pin, &self.job) {
            clue_string(&front, tile.right_top() + vec2(-8.0, 8.0), pin);
        }
        self.desk_props(&front, screen);

        let pointer = self.demo_pointer.or(ctx.pointer_latest_pos());
        let over_map = pointer.filter(|p| self.map_rect.is_some_and(|r| r.contains(*p)));
        self.detective.magnifier_at(over_map);
        self.detective.set_hole(self.hole, self.map_rect);
        let (now, dt) = ctx.input(|i| (i.time, i.unstable_dt.min(0.1)));
        self.detective.paint(&self.assets, &back, &front, self.stage, pointer, now, dt);
        ctx.request_repaint_after(Duration::from_millis(if self.detective.busy() { 16 } else { 33 }));
    }

    fn run_demo(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(demo) = &mut self.demo else { return };
        demo.save(ctx);
        let now = ctx.input(|i| i.time);
        for step in demo.steps(now, self.job.is_some(), self.tree.is_some()) {
            match step {
                Step::Shot(name) => Demo::shoot(ctx, name),
                Step::InspectLargest => {
                    let file = self.largest.as_ref().and_then(|(_, l)| l.first().copied());
                    let tile = file.zip(self.layout.as_ref()).and_then(|(f, (_, l))| {
                        self.tree.as_ref().and_then(|t| l.tile_for(t, f)).map(|i| (f, l.tiles[i].rect))
                    });
                    if let Some((file, rect)) = tile {
                        actions.push(Action::Inspect(file, rect));
                    }
                }
                Step::Magnifier(on) => {
                    self.demo_pointer = if on { self.map_rect.map(|r| r.center() + vec2(-120.0, -60.0)) } else { None };
                    self.detective.set_magnifier(on);
                }
                Step::Peer => self.detective.peer_now(),
                Step::React(act) => self.detective.react(act, 2.0),
                Step::Doze => self.detective.doze(),
                Step::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }

    fn desk_props(&self, painter: &Painter, screen: Rect) {
        let floor = screen.max.y - 6.0;
        let right = screen.max.x;
        let props = [
            ("papers", pos2(right - 336.0, floor - 52.0), 96.0, -0.08),
            ("pen", pos2(right - 318.0, floor - 30.0), 44.0, -0.18),
            ("books", pos2(right - 74.0, floor - 40.0), 78.0, 0.0),
            ("teacup", pos2(right - 196.0, floor - 46.0), 90.0, 0.0),
        ];
        for (name, center, height, angle) in props {
            theme::sprite_rotated(painter, center + vec2(4.0, 6.0), height, angle, self.assets.prop(name), Color32::from_black_alpha(70));
            theme::sprite_rotated(painter, center, height, angle, self.assets.prop(name), Color32::WHITE);
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
        self.poll_scan();
        let mut actions = Vec::new();

        let (active, back, escape, magnifier) = ctx.input(|i| {
            (
                i.pointer.is_moving() || i.pointer.any_pressed() || !i.keys_down.is_empty() || i.raw_scroll_delta != Vec2::ZERO,
                i.pointer.button_pressed(egui::PointerButton::Extra1) || i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::M),
            )
        });
        if active {
            self.detective.activity();
        }
        if !ctx.wants_keyboard_input() {
            if back {
                actions.push(Action::Up);
            }
            if escape {
                actions.push(Action::Select(None));
            }
            if magnifier && self.job.is_none() {
                actions.push(Action::ToggleMagnifier);
            }
        }

        let screen = ctx.screen_rect();
        theme::tiled(&ctx.layer_painter(LayerId::background()), screen, &self.assets.navy, Color32::WHITE);

        self.header(ctx, &mut actions);
        self.status_bar(ctx);
        self.side_panel(ctx, &mut actions);
        self.central(ctx, &mut actions);
        self.overlay(ctx);
        self.run_demo(ctx, &mut actions);

        for action in actions {
            self.apply(action);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAST_PATH_KEY, &self.path_input);
        eframe::set_value(storage, MUTED_KEY, &self.detective.muted());
    }
}

fn single_line(text: String, font: egui::FontId, color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::simple_singleline(text, font, color);
    job.wrap = TextWrapping::truncate_at_width(width.max(10.0));
    job
}

/// A row on the lined evidence card: colour swatch, name and a value on the right.
fn evidence_row(ui: &mut Ui, color: Color32, name: &str, value: &str, selected: bool, height: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    if selected {
        // Highlighter pen.
        painter.rect_filled(rect.shrink2(vec2(26.0, 3.0)), CornerRadius::same(3), Color32::from_rgba_unmultiplied(255, 226, 80, 120));
    } else if response.hovered() {
        painter.rect_filled(rect.shrink2(vec2(26.0, 3.0)), CornerRadius::same(3), Color32::from_rgba_unmultiplied(120, 90, 50, 30));
    }
    let swatch = Rect::from_center_size(pos2(rect.min.x + 12.0, rect.center().y), vec2(13.0, 13.0));
    painter.rect_filled(swatch, CornerRadius::same(2), color);
    painter.rect_stroke(swatch, CornerRadius::same(2), Stroke::new(1.0_f32, Color32::from_black_alpha(60)), StrokeKind::Outside);
    let value = painter.layout_no_wrap(value.to_string(), theme::medium(15.0), theme::INK_SOFT);
    let value_pos = pos2(rect.max.x - 6.0 - value.size().x, rect.center().y - value.size().y / 2.0);
    let name_left = rect.min.x + 30.0;
    let name = painter.layout_job(single_line(name.to_string(), theme::ui_font(16.0), theme::INK, value_pos.x - name_left - 12.0));
    painter.galley(pos2(name_left, rect.center().y - name.size().y / 2.0), name, theme::INK);
    painter.galley(value_pos, value, theme::INK_SOFT);
    response
}

/// A dashed gold thread from the selected box to the pin on its case card, sagging a little.
fn clue_string(painter: &Painter, from: Pos2, to: Pos2) {
    let mid = from + (to - from) * 0.5 + vec2(0.0, (to - from).length() * 0.12);
    let bez = |t: f32| from + (mid - from) * 2.0 * t * (1.0 - t) + (to - from) * t * t;
    let steps = ((to - from).length() / 7.0).max(4.0) as usize;
    painter.circle_filled(from, 5.0, theme::GOLD_BRIGHT);
    for i in (0..steps).step_by(2) {
        let a = bez(i as f32 / steps as f32);
        let b = bez((i + 1) as f32 / steps as f32);
        painter.line_segment([a + vec2(1.0, 2.0), b + vec2(1.0, 2.0)], Stroke::new(2.5_f32, Color32::from_black_alpha(80)));
        painter.line_segment([a, b], Stroke::new(2.5_f32, theme::GOLD_BRIGHT));
    }
}

fn context_menu(ui: &mut Ui, tree: &Tree, node: Option<u32>, view: u32, actions: &mut Vec<Action>) {
    ui.set_min_width(230.0);
    let Some(node) = node else {
        if view != 0 && ui.button("Zoom out").clicked() {
            actions.push(Action::Up);
            ui.close();
        }
        return;
    };
    let n = tree.node(node);
    ui.label(egui::RichText::new(tree.display_name(node)).font(theme::bold(17.0)));
    ui.label(egui::RichText::new(format::bytes(n.size)).color(theme::GOLD));
    ui.separator();
    let target = if n.is_dir { node } else { n.parent };
    if target != view {
        let label = if n.is_dir { "Go inside this folder" } else { "Go inside its folder" };
        if ui.button(label).clicked() {
            actions.push(Action::Zoom(node));
            ui.close();
        }
    }
    if view != 0 && ui.button("Zoom out").clicked() {
        actions.push(Action::Up);
        ui.close();
    }
    if ui.button(REVEAL_LABEL).clicked() {
        actions.push(Action::Reveal(node));
        ui.close();
    }
    if ui.button("Copy path").clicked() {
        actions.push(Action::CopyPath(node));
        ui.close();
    }
}

/// A little parchment note that follows the pointer.
fn tooltip(ctx: &egui::Context, assets: &Assets, tree: &Tree, node: u32, view: u32, pointer: Pos2) {
    let n = tree.node(node);
    let painter = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("treemap_tooltip")));
    let wrap = 460.0;
    let name = painter.layout(tree.display_name(node), theme::bold(19.0), theme::INK, wrap);
    let mut lines = vec![
        painter.layout(tree.path_of(node).display().to_string(), theme::ui_font(14.0), theme::INK_SOFT, wrap),
        painter.layout(format!("{}   •   {} of this view", format::bytes(n.size), format::percent(n.size, tree.node(view).size)), theme::medium(16.0), theme::INK, wrap),
    ];
    let kind = if n.is_dir {
        format!("Folder with {} files, {} folders", format::count(n.files), format::count(n.dirs))
    } else {
        format!("{} file", tree.exts[n.ext as usize].label)
    };
    lines.push(painter.layout(kind, theme::ui_font(15.0), theme::INK_SOFT, wrap));

    let pad = vec2(16.0, 12.0);
    let swatch_w = 22.0;
    let width = lines.iter().map(|g| g.size().x).fold(name.size().x + swatch_w, f32::max);
    let height = name.size().y + lines.iter().map(|g| g.size().y + 4.0).sum::<f32>();
    let size = vec2(width, height) + pad * 2.0;
    let screen = ctx.screen_rect();
    let mut origin = pointer + vec2(22.0, 24.0);
    if origin.x + size.x > screen.max.x - 8.0 {
        origin.x = pointer.x - 16.0 - size.x;
    }
    if origin.y + size.y > screen.max.y - 8.0 {
        origin.y = pointer.y - 16.0 - size.y;
    }
    let rect = Rect::from_min_size(origin, size);
    theme::paper(&painter, &theme::deckled(rect, node, 1.2), &assets.parchment, Color32::WHITE, 6.0);

    let mut y = rect.min.y + pad.y;
    let swatch_rect = Rect::from_min_size(pos2(rect.min.x + pad.x, y + name.size().y / 2.0 - 7.0), vec2(14.0, 14.0));
    if n.is_dir {
        theme::folder_icon(&painter, swatch_rect.expand(1.0), Color32::from_rgb(232, 182, 84));
    } else {
        painter.rect_filled(swatch_rect, CornerRadius::same(2), tree.color_of(node));
    }
    painter.galley(pos2(rect.min.x + pad.x + swatch_w, y), name.clone(), theme::INK);
    y += name.size().y + 4.0;
    for line in lines {
        let h = line.size().y;
        painter.galley(pos2(rect.min.x + pad.x, y), line, theme::INK);
        y += h + 4.0;
    }
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
