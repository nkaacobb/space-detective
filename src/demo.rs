//! A scripted walkthrough that saves screenshots, for checking the look without clicking around.
//! Only runs when SPACE_DETECTIVE_DEMO is set to a folder for the PNGs.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, ColorImage, UserData, ViewportCommand};

use crate::detective::Act;

pub enum Step {
    Shot(String),
    InspectLargest,
    Magnifier(bool),
    Peer,
    React(Act),
    Doze,
    Quit,
}

pub struct Demo {
    dir: PathBuf,
    tree_at: Option<f64>,
    next: usize,
    next_scan_shot: f64,
    scan_shots: u32,
}

/// Seconds after the scan finishes, and what to do.
const SCRIPT: &[(f64, fn() -> Step)] = &[
    (1.2, || Step::Shot("map".into())),
    (1.6, || Step::InspectLargest),
    (1.85, || Step::Shot("inspect_flying".into())),
    (2.6, || Step::Shot("inspect_hovering".into())),
    (6.0, || Step::Magnifier(true)),
    (7.0, || Step::Shot("magnifier".into())),
    (7.4, || Step::Magnifier(false)),
    (8.6, || Step::Peer),
    (9.6, || Step::Shot("peer".into())),
    (10.2, || Step::React(Act::Cheer)),
    (10.9, || Step::Shot("cheer".into())),
    (11.8, || Step::React(Act::Triumph)),
    (12.5, || Step::Shot("triumph".into())),
    (13.4, || Step::React(Act::Worried)),
    (13.62, || Step::Shot("worried_hat".into())),
    (14.4, || Step::Shot("worried".into())),
    (15.0, || Step::React(Act::Puzzled)),
    (15.8, || Step::Shot("puzzled".into())),
    (16.6, || Step::React(Act::Suspicious)),
    (17.4, || Step::Shot("suspicious".into())),
    (18.2, || Step::Doze),
    (21.5, || Step::Shot("asleep".into())),
    (22.0, || Step::Quit),
];

impl Demo {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("SPACE_DETECTIVE_DEMO")?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self { dir, tree_at: None, next: 0, next_scan_shot: 1.0, scan_shots: 0 })
    }

    pub fn steps(&mut self, now: f64, scanning: bool, has_tree: bool) -> Vec<Step> {
        let mut steps = Vec::new();
        if scanning && now > self.next_scan_shot && self.scan_shots < 6 {
            self.scan_shots += 1;
            self.next_scan_shot = now + 2.5;
            steps.push(Step::Shot(format!("scan_{}", self.scan_shots)));
        }
        if !has_tree || scanning {
            if self.scan_shots == 0 && now > 2.0 && !scanning {
                self.scan_shots = 1;
                steps.push(Step::Shot("welcome".into()));
            }
            return steps;
        }
        let start = *self.tree_at.get_or_insert(now);
        while let Some((at, step)) = SCRIPT.get(self.next) {
            if now - start < *at {
                break;
            }
            steps.push(step());
            self.next += 1;
        }
        steps
    }

    pub fn shoot(ctx: &egui::Context, name: String) {
        ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::new(name)));
    }

    /// Saves any screenshots that arrived this frame.
    pub fn save(&self, ctx: &egui::Context) {
        let shots: Vec<(String, Arc<ColorImage>)> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { image, user_data, .. } => {
                        let name = user_data.data.as_ref()?.downcast_ref::<String>()?.clone();
                        Some((name, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (name, image) in shots {
            let [w, h] = image.size;
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
            let path = self.dir.join(format!("{name}.png"));
            if let Err(e) = image::save_buffer(&path, &bytes, w as u32, h as u32, image::ColorType::Rgba8) {
                eprintln!("couldn't save {}: {e}", path.display());
            }
        }
    }
}
