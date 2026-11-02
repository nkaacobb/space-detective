//! The paper-puppet detective: his rig, reactions, eyes, thought bubbles and magnifying glass.

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use eframe::egui::epaint::{Mesh, Vertex};
use eframe::egui::{
    Color32, CornerRadius, Galley, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, text::LayoutJob, vec2,
};
use serde::Deserialize;

use crate::assets::{Assets, Sprite};
use crate::lens::Lens;
use crate::theme;
use crate::voice::{self, Line, Voice};

// ---------------------------------------------------------------------------------------------
// Rig

#[derive(Deserialize)]
struct RigFile {
    parts: Vec<PartDef>,
    lens: LensDef,
    eyes: Vec<EyeDef>,
    mouth: MouthDef,
}

/// Where the mouth opens under the moustache, in head-image pixels.
#[derive(Deserialize, Clone, Copy)]
struct MouthDef {
    center: [f32; 2],
    width: f32,
    open: f32,
}

#[derive(Deserialize)]
struct PartDef {
    name: String,
    parent: Option<String>,
    anchor: [f32; 2],
    at: [f32; 2],
    z: i32,
    angle: f32,
}

#[derive(Deserialize)]
struct LensDef {
    center: [f32; 2],
    radius: f32,
}

#[derive(Deserialize, Clone, Copy)]
struct EyeDef {
    center: [f32; 2],
    rx: f32,
    ry: f32,
    pupil_rx: f32,
    pupil_ry: f32,
}

/// 2D affine map: p -> (a*x + c*y + x0, b*x + d*y + y0).
#[derive(Clone, Copy)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    x: f32,
    y: f32,
}

impl Affine {
    const IDENTITY: Self = Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, x: 0.0, y: 0.0 };
    fn translate(v: Vec2) -> Self {
        Self { x: v.x, y: v.y, ..Self::IDENTITY }
    }
    fn rotate(rad: f32) -> Self {
        let (s, c) = rad.sin_cos();
        Self { a: c, b: s, c: -s, d: c, x: 0.0, y: 0.0 }
    }
    fn scale(sx: f32, sy: f32) -> Self {
        Self { a: sx, d: sy, ..Self::IDENTITY }
    }
    fn then(self, o: Self) -> Self {
        // self ∘ o: apply o first.
        Self {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            x: self.a * o.x + self.c * o.y + self.x,
            y: self.b * o.x + self.d * o.y + self.y,
        }
    }
    fn apply(&self, p: Vec2) -> Pos2 {
        pos2(self.a * p.x + self.c * p.y + self.x, self.b * p.x + self.d * p.y + self.y)
    }
    fn inverse(&self) -> Self {
        let det = self.a * self.d - self.b * self.c;
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Self { a, b, c, d, x: -(a * self.x + c * self.y), y: -(b * self.x + d * self.y) }
    }
    fn angle(&self) -> f32 {
        self.b.atan2(self.a)
    }
    fn scale_len(&self) -> f32 {
        (self.a * self.a + self.b * self.b).sqrt()
    }
}

struct Part {
    name: String,
    parent: Option<usize>,
    anchor: Vec2,
    at: Vec2,
    z: i32,
    angle: f32,
}

struct Rig {
    /// Parents always come before their children.
    parts: Vec<Part>,
    lens_center: Vec2,
    lens_radius: f32,
    eyes: Vec<EyeDef>,
    mouth: MouthDef,
    /// Bounds of the standing puppet, in rig units around the torso pivot.
    bounds: Rect,
    torso: usize,
    head: usize,
    hat: usize,
    sleeve_l: usize,
    sleeve_r: usize,
    forearm_l: usize,
    forearm_r: usize,
    glass: usize,
    coat_l: usize,
    coat_r: usize,
}

/// Per-part extra rotation (degrees) and scale on top of the rest pose.
#[derive(Clone)]
struct Pose {
    angle: Vec<f32>,
    scale: Vec<Vec2>,
    offset: Vec<Vec2>,
}

impl Rig {
    fn load(sizes: &dyn Fn(&str) -> Vec2) -> Self {
        let file: RigFile = serde_json::from_str(include_str!("../assets/detective/rig.json")).expect("rig.json");
        let mut ordered: Vec<&PartDef> = Vec::new();
        while ordered.len() < file.parts.len() {
            for p in &file.parts {
                let ready = p.parent.as_ref().is_none_or(|parent| ordered.iter().any(|o| &o.name == parent));
                if ready && !ordered.iter().any(|o| o.name == p.name) {
                    ordered.push(p);
                }
            }
        }
        let index = |name: &str| ordered.iter().position(|p| p.name == name).unwrap_or_else(|| panic!("rig part {name}"));
        let parts: Vec<Part> = ordered
            .iter()
            .map(|p| Part {
                name: p.name.clone(),
                parent: p.parent.as_deref().map(index),
                anchor: vec2(p.anchor[0], p.anchor[1]),
                at: vec2(p.at[0], p.at[1]),
                z: p.z,
                angle: p.angle,
            })
            .collect();
        let mut rig = Rig {
            lens_center: vec2(file.lens.center[0], file.lens.center[1]),
            lens_radius: file.lens.radius,
            eyes: file.eyes,
            mouth: file.mouth,
            bounds: Rect::NOTHING,
            torso: index("torso"),
            head: index("head"),
            hat: index("hat"),
            sleeve_l: index("sleeve_left"),
            sleeve_r: index("sleeve_right"),
            forearm_l: index("forearm_left"),
            forearm_r: index("forearm_right"),
            glass: index("glass"),
            coat_l: index("coat_left"),
            coat_r: index("coat_right"),
            parts,
        };
        let rest = rig.rest_pose();
        let worlds = rig.worlds(&rest, Affine::IDENTITY);
        let mut bounds = Rect::NOTHING;
        for (i, part) in rig.parts.iter().enumerate() {
            let size = sizes(&part.name);
            for corner in [Vec2::ZERO, vec2(size.x, 0.0), size, vec2(0.0, size.y)] {
                bounds.extend_with(worlds[i].apply(corner));
            }
        }
        rig.bounds = bounds;
        rig
    }

    fn rest_pose(&self) -> Pose {
        let n = self.parts.len();
        Pose { angle: vec![0.0; n], scale: vec![Vec2::splat(1.0); n], offset: vec![Vec2::ZERO; n] }
    }

    fn worlds(&self, pose: &Pose, root: Affine) -> Vec<Affine> {
        let mut out: Vec<Affine> = Vec::with_capacity(self.parts.len());
        for (i, p) in self.parts.iter().enumerate() {
            let local = Affine::translate(p.at + pose.offset[i])
                .then(Affine::rotate((p.angle + pose.angle[i]).to_radians()))
                .then(Affine::scale(pose.scale[i].x, pose.scale[i].y))
                .then(Affine::translate(-p.anchor));
            out.push(match p.parent {
                Some(parent) => out[parent].then(local),
                None => root.then(local),
            });
        }
        out
    }

    /// Two-bone IK for the right arm: rotations that put the lens centre on `target` (rig units
    /// around the torso pivot) with the glass tilted `glass_deg`. Mirrors art/preview_rig.py.
    fn solve_arm(&self, target: Vec2, glass_deg: f32) -> (f32, f32, f32) {
        let torso = &self.parts[self.torso];
        let sleeve = &self.parts[self.sleeve_r];
        let fore = &self.parts[self.forearm_r];
        let glass = &self.parts[self.glass];
        let shoulder = sleeve.at - torso.anchor;
        let upper = fore.at - sleeve.anchor;
        let lower = glass.at - fore.anchor;
        let grip_to_lens = self.lens_center - glass.anchor;
        let g = glass_deg.to_radians();
        let rotated = vec2(g.cos() * grip_to_lens.x - g.sin() * grip_to_lens.y, g.sin() * grip_to_lens.x + g.cos() * grip_to_lens.y);
        let fist = target - rotated;
        let (l1, l2) = (upper.length(), lower.length());
        let to = fist - shoulder;
        let d = to.length().clamp((l1 - l2).abs() + 0.01, l1 + l2 - 0.01);
        let bend = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0).acos();
        let base = to.y.atan2(to.x);
        let upper_dir = base - bend;
        let elbow = shoulder + l1 * vec2(upper_dir.cos(), upper_dir.sin());
        let fist_dir = fist - elbow;
        let lower_dir = fist_dir.y.atan2(fist_dir.x);
        let sleeve_rot = (upper_dir - upper.y.atan2(upper.x)).to_degrees();
        let fore_rot = (lower_dir - lower.y.atan2(lower.x)).to_degrees() - sleeve_rot;
        let glass_rot = glass_deg - sleeve_rot - fore_rot;
        (wrap_deg(sleeve_rot), wrap_deg(fore_rot), wrap_deg(glass_rot))
    }
}

fn wrap_deg(d: f32) -> f32 {
    (d + 180.0).rem_euclid(360.0) - 180.0
}

fn draw_part(painter: &Painter, sprite: &Sprite, m: Affine, tint: Color32) {
    let mut mesh = Mesh::with_texture(sprite.id());
    let s = sprite.size;
    for (corner, uv) in [
        (Vec2::ZERO, pos2(0.0, 0.0)),
        (vec2(s.x, 0.0), pos2(1.0, 0.0)),
        (s, pos2(1.0, 1.0)),
        (vec2(0.0, s.y), pos2(0.0, 1.0)),
    ] {
        mesh.vertices.push(Vertex { pos: m.apply(corner), uv, color: tint });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(Shape::mesh(mesh));
}

fn ellipse(m: Affine, center: Vec2, r: Vec2, from: f32, to: f32, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let t = from + (to - from) * i as f32 / n as f32;
            m.apply(center + vec2(r.x * t.cos(), r.y * t.sin()))
        })
        .collect()
}

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn approach(current: f32, goal: f32, rate: f32, dt: f32) -> f32 {
    goal + (current - goal) * (-rate * dt).exp()
}

fn approach2(current: Vec2, goal: Vec2, rate: f32, dt: f32) -> Vec2 {
    goal + (current - goal) * (-rate * dt).exp()
}

// ---------------------------------------------------------------------------------------------
// Behaviour

/// Arm targets for the glass in his hand, in rig units around the torso pivot.
const HOLD: (Vec2, f32) = (vec2(176.0, -150.0), 18.0);
const PEER: (Vec2, f32) = (vec2(79.0, -213.0), 0.0);
const INSPECT: (Vec2, f32) = (vec2(70.0, -30.0), -12.0);
/// Inner radius of the glass ring in glass.png.
const GLASS_HOLE: f32 = 51.0;

/// Reactions he acts out with the puppet itself, so his face keeps moving through them.
#[derive(Clone, Copy, PartialEq)]
pub enum Act {
    /// A fist pump with the glass held high: a scan went well.
    Cheer,
    /// The glass raised over his head like a trophy: a really big haul.
    Triumph,
    /// Fist at his chin, glass clutched to his chest, trembling: something went wrong.
    Worried,
    /// Fist at his chin, head on one side: an empty folder, a cancelled scan.
    Puzzled,
    /// Squinting through his glass: zooming into a folder.
    Suspicious,
}

struct Acting {
    act: Act,
    start: f64,
    until: f64,
}

/// Where the glass goes, and the turn (degrees) of his left sleeve, left forearm and head.
#[derive(Clone, Copy)]
struct Stance {
    glass: (Vec2, f32),
    sleeve: f32,
    forearm: f32,
    head: f32,
}

impl Act {
    fn stance(self) -> Stance {
        let (glass, sleeve, forearm, head) = match self {
            Act::Cheer => ((vec2(210.0, -320.0), -10.0), 70.0, 140.0, -5.0),
            Act::Triumph => ((vec2(140.0, -430.0), 0.0), 0.0, 0.0, -7.0),
            Act::Worried => ((vec2(95.0, -5.0), -30.0), 0.0, -95.0, 8.0),
            Act::Puzzled => (HOLD, 0.0, -95.0, -9.0),
            Act::Suspicious => (PEER, 0.0, 0.0, -4.0),
        };
        Stance { glass, sleeve, forearm, head }
    }
}

/// Dozing: glass hanging at his side, head lolling.
const ASLEEP: Stance = Stance { glass: (vec2(190.0, 10.0), 60.0), sleeve: -8.0, forearm: 0.0, head: 14.0 };

struct Bubble {
    text: String,
    start: f64,
    speak: f32,
    /// Where the bubble's little trail of puffs points: his head, the hole, a peeking face.
    from: Speaker,
}

#[derive(Clone, Copy, PartialEq)]
enum Speaker {
    Stage,
    Hole,
    Peek,
}

struct GlassGoal {
    center: Pos2,
    radius: f32,
    until: f64,
}

#[derive(Clone, Copy, PartialEq)]
enum ScanPhase {
    Diving,
    Inside,
    /// Head pops out of the hole (or from the edge of the map when `edge`).
    Peeking { edge: bool },
    Emerging,
}

struct Scrap {
    pos: Pos2,
    vel: Vec2,
    angle: f32,
    spin: f32,
    size: Vec2,
    color: Color32,
    born: f64,
}

pub struct Detective {
    rig: Rig,
    voice: Voice,
    lens: Lens,
    act: Option<Acting>,
    /// 0 = awake, 1 = fast asleep.
    sleep: f32,
    /// How far his eyelids are lowered, apart from blinking.
    lids: f32,
    arm_target: Vec2,
    arm_angle: f32,
    peer_until: f64,
    next_peer: f64,
    glass_goal: Option<GlassGoal>,
    /// 0 = in his hand, 1 = at the goal.
    glass_away: f32,
    glass_last: (Pos2, f32, f32),
    glass_goal_last: (Pos2, f32, f32),
    magnifier: Option<Pos2>,
    magnifier_on: bool,
    look: Vec2,
    /// How open his mouth is, following how loud he's speaking.
    mouth: f32,
    next_glance: f64,
    glance_at: Pos2,
    next_blink: f64,
    hop_start: f64,
    bubble: Option<Bubble>,
    pending: VecDeque<(f64, String)>,
    last_input: f64,
    idle_spoken: bool,
    asleep: bool,
    scan: Option<(ScanPhase, f64)>,
    scan_started: f64,
    next_inside_line: f64,
    next_peek: f64,
    peek_count: u32,
    scraps: Vec<Scrap>,
    next_scrap: f64,
    hole: Option<Pos2>,
    edge: Option<Rect>,
    stage_feet: Pos2,
    stage_height: f32,
    rng: u64,
    time: f64,
}

impl Detective {
    pub fn new(assets: &Assets, muted: bool) -> Self {
        let rig = Rig::load(&|name| assets.part(name).size);
        Self {
            rig,
            voice: Voice::new(muted),
            lens: Lens::default(),
            act: None,
            sleep: 0.0,
            lids: 0.0,
            arm_target: HOLD.0,
            arm_angle: HOLD.1,
            peer_until: 0.0,
            next_peer: 9.0,
            glass_goal: None,
            glass_away: 0.0,
            glass_last: (Pos2::ZERO, 0.0, 10.0),
            glass_goal_last: (Pos2::ZERO, 0.0, 10.0),
            magnifier: None,
            magnifier_on: false,
            look: vec2(0.3, 0.1),
            mouth: 0.0,
            next_glance: 2.0,
            glance_at: Pos2::ZERO,
            next_blink: 2.5,
            hop_start: -10.0,
            bubble: None,
            pending: VecDeque::new(),
            last_input: 0.0,
            idle_spoken: false,
            asleep: false,
            scan: None,
            scan_started: 0.0,
            next_inside_line: 0.0,
            next_peek: 0.0,
            peek_count: 0,
            scraps: Vec::new(),
            next_scrap: 0.0,
            hole: None,
            edge: None,
            stage_feet: Pos2::ZERO,
            stage_height: 250.0,
            rng: 0x2545_F491_4F6C_DD1D,
            time: 0.0,
        }
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn muted(&self) -> bool {
        self.voice.muted
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.voice.set_muted(muted);
    }

    pub fn magnifier_on(&self) -> bool {
        self.magnifier_on
    }

    /// Raises the glass to his eye for a moment.
    pub fn peer_now(&mut self) {
        self.peer_until = self.time + 2.6;
    }

    // ----- things the app tells him -----

    /// Any mouse or keyboard activity: wakes him up and resets the idle timer.
    pub fn activity(&mut self) {
        self.last_input = self.time;
        self.idle_spoken = false;
        if self.asleep {
            self.asleep = false;
            self.hop();
        }
    }

    /// Nods off straight away (the demo walkthrough can't wait 80 seconds).
    pub fn doze(&mut self) {
        self.asleep = true;
        self.end_act();
    }

    /// Says a line for `event` straight away.
    pub fn say(&mut self, event: &str) {
        self.say_from(event, if self.inside() { Speaker::Hole } else { Speaker::Stage });
    }

    fn say_from(&mut self, event: &str, from: Speaker) {
        if let Some(line) = self.voice.pick(event) {
            self.speak(line, from);
        }
    }

    fn speak(&mut self, line: Line, from: Speaker) {
        let speak = self.voice.duration(&line);
        self.voice.play(&line, self.time);
        // The bubble waits out the voice's lead-in so the words appear as he says them.
        let start = self.time + if self.voice.muted { 0.0 } else { voice::LEAD_IN as f64 };
        self.bubble = Some(Bubble { text: line.text, start, speak, from });
    }

    /// Says a line for `event` after `delay` seconds (or after the current line, if longer).
    pub fn say_later(&mut self, event: &str, delay: f64) {
        let after_current = self.bubble.as_ref().map_or(0.0, |b| b.start + b.speak as f64 + 0.4 - self.time);
        self.pending.push_back((self.time + delay.max(after_current), event.to_string()));
    }

    pub fn react(&mut self, act: Act, seconds: f64) {
        if self.scan.is_some() {
            return;
        }
        self.asleep = false;
        self.act = Some(Acting { act, start: self.time, until: self.time + seconds });
        if matches!(act, Act::Cheer | Act::Triumph) {
            self.hop();
        }
    }

    /// Lets the current reaction ease back to his usual stance.
    fn end_act(&mut self) {
        if let Some(a) = &mut self.act {
            a.until = a.until.min(self.time + 0.3);
        }
    }

    /// The current reaction, how far into it he is (0 to 1, easing in and out) and its age.
    fn acting(&self, now: f64) -> Option<(Act, f32, f32)> {
        let a = self.act.as_ref()?;
        let age = (now - a.start) as f32;
        Some((a.act, ease(age / 0.3) * ease(((a.until - now) / 0.3) as f32), age))
    }

    pub fn hop(&mut self) {
        self.hop_start = self.time;
    }

    /// Sends his magnifying glass over to `rect` for a closer look.
    pub fn investigate(&mut self, rect: Rect) {
        if self.magnifier_on || self.scan.is_some() {
            return;
        }
        let radius = (rect.width().min(rect.height()) * 0.42).clamp(40.0, 64.0);
        self.glass_goal = Some(GlassGoal { center: rect.center(), radius, until: self.time + 2.8 });
        self.end_act();
    }

    pub fn set_magnifier(&mut self, on: bool) {
        if on != self.magnifier_on {
            self.magnifier_on = on;
            if on {
                self.end_act();
            }
            self.say(if on { "lens_on" } else { "lens_off" });
        }
    }

    /// Where the magnifier should follow (the pointer over the map), when it's switched on.
    pub fn magnifier_at(&mut self, at: Option<Pos2>) {
        self.magnifier = if self.magnifier_on { at } else { None };
    }

    pub fn begin_scan(&mut self, whole_drive: bool) {
        self.glass_goal = None;
        self.act = None;
        self.asleep = false;
        self.say_from(if whole_drive { "scan_drive" } else { "scan_dive" }, Speaker::Stage);
        self.scan = Some((ScanPhase::Diving, self.time + 0.7));
        self.scan_started = self.time;
        self.next_inside_line = self.time + 5.5;
        self.next_peek = self.time + 9.0;
        self.peek_count = 0;
    }

    /// The scan is over: he climbs out, announces himself, then says `result`.
    pub fn end_scan(&mut self, result: Option<&str>) {
        match self.scan {
            None => {
                if let Some(event) = result {
                    self.say(event);
                }
            }
            Some((ScanPhase::Diving, start)) if self.time < start => {
                // Finished before he even jumped in.
                self.scan = None;
                if let Some(event) = result {
                    self.say_later(event, 0.2);
                }
            }
            Some(_) => {
                self.scan = Some((ScanPhase::Emerging, self.time));
                self.say_from("scan_emerge", Speaker::Hole);
                if let Some(event) = result {
                    self.say_later(event, 0.4);
                }
            }
        }
    }

    pub fn set_hole(&mut self, hole: Option<Pos2>, edge: Option<Rect>) {
        self.hole = hole;
        self.edge = edge;
    }

    fn inside(&self) -> bool {
        matches!(self.scan, Some((ScanPhase::Inside | ScanPhase::Peeking { .. }, _)))
    }

    pub fn busy(&self) -> bool {
        self.scan.is_some()
            || self.glass_away > 0.001
            || self.glass_goal.is_some()
            || self.time - self.hop_start < 0.5
            || self.act.is_some()
    }

    // ----- per frame -----

    fn update(&mut self, now: f64, dt: f32) {
        self.time = now;

        while self.pending.front().is_some_and(|(at, _)| *at <= now) {
            let (_, event) = self.pending.pop_front().unwrap();
            let from = if self.inside() { Speaker::Hole } else { Speaker::Stage };
            self.say_from(&event, from);
            match event.as_str() {
                "scan_done_huge" => self.react(Act::Triumph, 2.6),
                e if e.starts_with("scan_done") => self.react(Act::Cheer, 2.6),
                "error" | "unreadable" => self.react(Act::Worried, 3.0),
                "scan_cancel" => self.react(Act::Puzzled, 2.5),
                _ => {}
            }
        }

        if self.act.as_ref().is_some_and(|a| now > a.until) {
            self.act = None;
        }

        // Dozing off when nobody's clicked anything for a while.
        let idle = now - self.last_input;
        if self.scan.is_none() && self.bubble.is_none() {
            if idle > 45.0 && !self.idle_spoken {
                self.idle_spoken = true;
                self.say("idle");
            } else if idle > 80.0 && !self.asleep {
                self.asleep = true;
            }
        }
        self.sleep = approach(self.sleep, if self.asleep { 1.0 } else { 0.0 }, if self.asleep { 1.5 } else { 6.0 }, dt);
        let acting = self.act.as_ref().filter(|a| now < a.until - 0.3).map(|a| a.act);
        let lids = if self.asleep {
            1.0
        } else if acting == Some(Act::Suspicious) {
            0.45
        } else {
            0.0
        };
        self.lids = approach(self.lids, lids, if self.asleep { 2.0 } else { 12.0 }, dt);

        // Peering through his own glass now and then.
        let glass_home = self.glass_goal.is_none() && self.magnifier.is_none();
        let free = self.act.is_none() && !self.asleep;
        if self.scan.is_none() && free && glass_home && self.bubble.is_none() && now > self.next_peer {
            self.peer_until = now + 2.6;
            self.next_peer = now + 14.0 + 10.0 * self.random() as f64;
            if self.random() < 0.3 {
                self.say("peer");
            }
        }

        // Arm: where the glass sits in his hand.
        let talking = self.bubble.as_ref().is_some_and(|b| now < b.start + b.speak as f64);
        self.mouth = approach(self.mouth, self.voice.level(now), 30.0, dt);
        let (goal, goal_angle) = if let Some(act) = acting {
            let (mut target, angle) = act.stance().glass;
            if act == Act::Triumph {
                // Waving the trophy about.
                target.x += 22.0 * (now as f32 * 5.0).sin();
            }
            (target, angle)
        } else if self.asleep {
            ASLEEP.glass
        } else if now < self.peer_until {
            PEER
        } else if talking && (now * 0.5).fract() < 0.5 {
            INSPECT
        } else {
            HOLD
        };
        self.arm_target = approach2(self.arm_target, goal, 7.0, dt);
        self.arm_angle = approach(self.arm_angle, goal_angle, 7.0, dt);

        // The glass flying off to inspect something, and back.
        if let Some(g) = &self.glass_goal
            && now > g.until
        {
            self.glass_goal = None;
        }
        let away = self.glass_goal.is_some() || self.magnifier.is_some();
        let speed = if away { 1.0 / 0.55 } else { 1.0 / 0.5 };
        self.glass_away = (self.glass_away + if away { speed } else { -speed } * dt).clamp(0.0, 1.0);

        // Eyes: the pointer, the glass while it's away, or a random glance.
        if now > self.next_glance {
            self.next_glance = now + 1.5 + 3.0 * self.random() as f64;
            let feet = self.stage_feet;
            self.glance_at = feet + vec2(-500.0 + 700.0 * self.random(), -600.0 + 500.0 * self.random());
        }

        // Paper scraps flying out of the hole while he rummages.
        if self.inside()
            && let Some(hole) = self.hole
            && now > self.next_scrap
        {
            self.next_scrap = now + 0.12 + 0.25 * self.random() as f64;
            let colors = [theme::PAPER, theme::CREAM, Color32::from_rgb(62, 128, 204), Color32::from_rgb(236, 154, 58),
                Color32::from_rgb(226, 86, 86), Color32::from_rgb(120, 196, 98), Color32::from_rgb(244, 204, 82)];
            let color = colors[(self.random() * colors.len() as f32) as usize % colors.len()];
            let size = vec2(8.0 + 12.0 * self.random(), 6.0 + 8.0 * self.random());
            let r: [f32; 5] = std::array::from_fn(|_| self.random());
            self.scraps.push(Scrap {
                pos: hole + vec2(-30.0 + 60.0 * r[0], -4.0),
                vel: vec2(-160.0 + 320.0 * r[1], -360.0 - 260.0 * r[2]),
                angle: TAU * r[3],
                spin: -8.0 + 16.0 * r[4],
                size,
                color,
                born: now,
            });
        }
        for s in &mut self.scraps {
            s.vel.y += 900.0 * dt;
            s.pos += s.vel * dt;
            s.angle += s.spin * dt;
        }
        self.scraps.retain(|s| now - s.born < 1.6);

        self.update_scan(now);

        if let Some(b) = &self.bubble
            && now > b.start + b.speak as f64 + 2.4
        {
            self.bubble = None;
        }
    }

    fn update_scan(&mut self, now: f64) {
        let Some((phase, since)) = self.scan else { return };
        match phase {
            ScanPhase::Diving if now > since + 0.9 => self.scan = Some((ScanPhase::Inside, now)),
            ScanPhase::Inside => {
                let quiet = self.bubble.is_none();
                if quiet && now > self.next_inside_line {
                    self.next_inside_line = now + 7.0 + 5.0 * self.random() as f64;
                    let long = now - self.scan_started > 20.0 && self.random() < 0.35;
                    self.say_from(if long { "scan_long" } else { "scan_inside" }, Speaker::Hole);
                } else if quiet && now > self.next_peek {
                    self.next_peek = now + 10.0 + 6.0 * self.random() as f64;
                    self.peek_count += 1;
                    let edge = self.peek_count % 2 == 0 && self.edge.is_some();
                    self.scan = Some((ScanPhase::Peeking { edge }, now));
                }
            }
            ScanPhase::Peeking { .. } => {
                if now > since + 0.45 && now < since + 0.5 && self.bubble.is_none() {
                    self.say_from("scan_peek", Speaker::Peek);
                }
                if now > since + 3.4 {
                    self.scan = Some((ScanPhase::Inside, now));
                }
            }
            ScanPhase::Emerging if now > since + 1.0 => {
                self.scan = None;
                self.hop();
            }
            _ => {}
        }
    }

    /// Paints him. `stage` is his corner of the side panel (painted with `back`); the glass,
    /// his glass hand, bubbles and anything that flies across the window go on `front`.
    pub fn paint(&mut self, assets: &Assets, back: &Painter, front: &Painter, stage: Rect, pointer: Option<Pos2>, now: f64, dt: f32) {
        self.stage_height = (stage.height() - 26.0).min(stage.width() * 0.62);
        self.stage_feet = pos2(stage.min.x + 26.0 + self.stage_height * 0.3, stage.max.y - 10.0);
        self.update(now, dt);
        // The bubble goes under his glass hand, so a raised glass stays in front of it.
        self.paint_bubble(front, stage, now);

        // Desk-lamp glow behind him.
        let glow_center = self.stage_feet + vec2(0.0, -self.stage_height * 0.5);
        for i in 0..10 {
            let r = self.stage_height * (0.75 - i as f32 * 0.06);
            back.circle_filled(glow_center, r, Color32::from_rgba_unmultiplied(255, 214, 140, 4));
        }

        let feet = self.stage_feet;
        let look_target = if self.glass_away > 0.3 {
            Some(self.glass_last.0)
        } else {
            match self.acting(now) {
                // Eyes darting about for whatever's gone wrong.
                Some((Act::Worried, ..)) => Some(feet + vec2(if (now * 2.6).fract() < 0.5 { -420.0 } else { 260.0 }, -380.0)),
                Some((Act::Puzzled, ..)) => Some(feet + vec2(-160.0, -900.0)),
                // Side-eye at the map.
                Some((Act::Suspicious, ..)) => Some(feet + vec2(-900.0, -220.0)),
                _ => pointer.or(Some(self.glance_at)),
            }
        };

        match self.scan {
            Some((ScanPhase::Diving, since)) => {
                let t = ((now - since) / 0.9) as f32;
                if t < 0.0 {
                    self.paint_body(assets, back, front, look_target, now, dt);
                } else {
                    self.paint_flight(assets, front, t, true, now);
                    self.paint_hole(front);
                    self.paint_away_sign(assets, back, stage);
                }
            }
            Some((ScanPhase::Emerging, since)) => {
                self.paint_hole(front);
                self.paint_flight(assets, front, ((now - since) / 1.0) as f32, false, now);
                self.paint_away_sign(assets, back, stage);
            }
            Some((phase, since)) => {
                self.paint_away_sign(assets, back, stage);
                self.paint_hole(front);
                if let ScanPhase::Peeking { edge } = phase {
                    self.paint_peek(assets, front, (now - since) as f32, edge, pointer);
                }
            }
            None => self.paint_body(assets, back, front, look_target, now, dt),
        }
        self.paint_scraps(front, now);
    }

    /// Screen transform for the standing puppet at his spot on the stage.
    fn stage_root(&self, now: f64) -> Affine {
        let scale = self.stage_height / self.rig.bounds.height();
        let hop_t = ((now - self.hop_start) / 0.45) as f32;
        let hop = if (0.0..1.0).contains(&hop_t) { (hop_t * PI).sin() * 22.0 } else { 0.0 };
        let shake = match self.acting(now) {
            Some((Act::Worried, w, _)) => w * 1.8 * (now as f32 * 45.0).sin(),
            _ => 0.0,
        };
        let feet_in_rig = vec2(self.rig.bounds.center().x, self.rig.bounds.max.y);
        Affine::translate(self.stage_feet.to_vec2() + vec2(shake, -hop))
            .then(Affine::scale(scale, scale))
            .then(Affine::translate(-feet_in_rig))
    }

    /// The puppet pose for this frame: breathing, talking, and whatever he's reacting to.
    fn live_pose(&self, now: f64) -> Pose {
        let mut pose = self.rig.rest_pose();
        let t = now as f32;
        let r = &self.rig;
        let s = self.sleep;
        let breathe = (t * (1.6 - 0.7 * s)).sin();
        pose.scale[r.torso] = vec2(1.0, 1.0 + (0.012 + 0.02 * s) * breathe);
        pose.angle[r.head] = 2.2 * (t * 0.9).sin() * (1.0 - s) - 3.0 * self.mouth;
        pose.angle[r.hat] = 1.6 * (t * 0.9 - 0.6).sin();
        pose.offset[r.hat] = vec2(0.0, -3.0 * self.mouth);
        pose.angle[r.coat_l] = 1.2 * (t * 1.3).sin();
        pose.angle[r.coat_r] = -1.2 * (t * 1.3 + 0.4).sin();
        pose.angle[r.sleeve_l] = 2.5 * (t * 1.1).sin();

        // Nodding off: head lolls and drifts with his breathing.
        pose.angle[r.sleeve_l] += ASLEEP.sleeve * s;
        pose.angle[r.head] += (ASLEEP.head + 2.5 * (t * 0.9).sin()) * s;

        if let Some((act, w, age)) = self.acting(now) {
            let stance = act.stance();
            pose.angle[r.sleeve_l] += stance.sleeve * w;
            pose.angle[r.forearm_l] += stance.forearm * w;
            pose.angle[r.head] += stance.head * w;
            match act {
                Act::Cheer => {
                    let pump = (t * 11.0).sin() * w;
                    pose.angle[r.sleeve_l] += 10.0 * pump;
                    pose.angle[r.forearm_l] += 14.0 * pump;
                }
                Act::Worried => {
                    // His hat jumps off his head, then he trembles.
                    let pop = if age < 0.6 { (age / 0.6 * PI).sin() } else { 0.0 };
                    pose.offset[r.hat].y -= 80.0 * pop;
                    pose.angle[r.hat] -= 12.0 * pop;
                    pose.angle[r.head] += 1.2 * (t * 41.0).sin() * w;
                }
                Act::Puzzled => pose.angle[r.head] += 2.5 * (t * 2.2).sin() * w,
                Act::Triumph | Act::Suspicious => {}
            }
        }
        pose
    }

    fn paint_body(&mut self, assets: &Assets, back: &Painter, front: &Painter, look: Option<Pos2>, now: f64, dt: f32) {
        let root = self.stage_root(now);
        let mut pose = self.live_pose(now);

        // His eyes and head follow the pointer (or whatever he's watching), unless he's dozing.
        let head_world = self.rig.worlds(&pose, root)[self.rig.head];
        if let Some(target) = look {
            let eye_mid = head_world.apply(vec2(240.0, 140.0));
            let d = target - eye_mid;
            let turn = (d.x / 500.0).clamp(-1.0, 1.0) * 4.0 - (d.y / 700.0).clamp(-1.0, 1.0) * 1.5;
            pose.angle[self.rig.head] += turn * (1.0 - self.sleep);
        }

        // Right arm: holding the glass, or reaching after it while it's away.
        let to_rig = root.inverse();
        let in_hand_target = self.arm_target;
        let (target, angle) = if self.glass_away > 0.0 {
            let reach = to_rig.apply(self.glass_last.0.to_vec2()).to_vec2();
            let shoulder = self.rig.parts[self.rig.sleeve_r].at - self.rig.parts[self.rig.torso].anchor;
            let dir = (reach - shoulder).normalized();
            let reach_goal = shoulder + dir * 170.0 + vec2(0.0, -40.0);
            (in_hand_target + (reach_goal - in_hand_target) * ease(self.glass_away), self.arm_angle)
        } else {
            (in_hand_target, self.arm_angle)
        };
        let (s, f, g) = self.rig.solve_arm(target, angle);
        pose.angle[self.rig.sleeve_r] = s;
        pose.angle[self.rig.forearm_r] = f;
        pose.angle[self.rig.glass] = g;

        let worlds = self.rig.worlds(&pose, root);
        let shadow_w = self.stage_height * 0.32;
        back.add(Shape::ellipse_filled(self.stage_feet + vec2(0.0, -2.0), vec2(shadow_w, 7.0), Color32::from_black_alpha(70)));

        let mut order: Vec<usize> = (0..self.rig.parts.len()).collect();
        order.sort_by_key(|&i| self.rig.parts[i].z);
        let glass_z = self.rig.parts[self.rig.glass].z;
        for &i in &order {
            let part = &self.rig.parts[i];
            if part.z >= glass_z {
                continue;
            }
            draw_part(back, assets.part(&part.name), worlds[i], Color32::WHITE);
            if i == self.rig.head {
                self.paint_face(back, assets, worlds[i], look, now, dt);
            }
        }

        // Z's drifting up from under his hat while he dozes.
        if self.sleep > 0.05 {
            let from = worlds[self.rig.hat].apply(vec2(400.0, 70.0));
            let t = now as f32;
            for i in 0..3 {
                let p = (t * 0.45 + i as f32 / 3.0).fract();
                let at = from + vec2(p * 40.0 + (p * 9.0).sin() * 6.0, -p * 70.0);
                let alpha = (1.0 - p) * (p * 4.0).min(1.0) * self.sleep;
                back.text(at, eframe::egui::Align2::CENTER_CENTER, "z", theme::hand(16.0 + p * 14.0), theme::CREAM.gamma_multiply(alpha));
            }
        }

        // The glass: in his hand, or flying to whatever he's investigating.
        let hand = worlds[self.rig.glass];
        let k = hand.scale_len();
        let hand_pose = (hand.apply(self.rig.lens_center), hand.angle(), self.rig.lens_radius * k);
        let (center, angle, radius) = self.glass_pose(hand_pose, now);
        self.glass_last = (center, angle, radius);
        let peering = now < self.peer_until && self.glass_away == 0.0;
        let zoom = if self.glass_away > 0.0 { 1.9 } else if peering { 2.3 } else { 1.7 };
        let hole = radius * GLASS_HOLE / self.rig.lens_radius;
        if self.glass_away > 0.0 {
            front.circle_filled(center + vec2(8.0, 14.0), radius * 1.05, Color32::from_black_alpha(46));
        }
        self.lens.paint(front, center, hole, zoom);
        let m = Affine::translate(center.to_vec2())
            .then(Affine::rotate(angle))
            .then(Affine::scale(radius / self.rig.lens_radius, radius / self.rig.lens_radius))
            .then(Affine::translate(-self.rig.lens_center));
        draw_part(front, assets.part("glass_frame"), m, Color32::WHITE);
        for &i in &order {
            if self.rig.parts[i].z > glass_z {
                draw_part(front, assets.part(&self.rig.parts[i].name), worlds[i], Color32::WHITE);
            }
        }
    }

    /// Where the glass is this frame, blending from his hand to its goal along an arc.
    fn glass_pose(&mut self, hand: (Pos2, f32, f32), now: f64) -> (Pos2, f32, f32) {
        let goal = if let Some(at) = self.magnifier {
            Some((at, -0.35, 96.0 * self.rig.lens_radius / GLASS_HOLE))
        } else {
            self.glass_goal.as_ref().map(|g| {
                let wobble = vec2((now as f32 * 2.1).sin() * 6.0, (now as f32 * 1.7).cos() * 4.0);
                (g.center + wobble, -0.3, g.radius * self.rig.lens_radius / GLASS_HOLE)
            })
        };
        if let Some(goal) = goal {
            self.glass_goal_last = goal;
        } else if self.glass_away <= 0.0 {
            return hand;
        }
        let (goal_center, goal_angle, goal_radius) = self.glass_goal_last;
        let t = ease(self.glass_away);
        // A little arc on the way, like it's being tossed.
        let lift = vec2(0.0, -110.0) * (t * PI).sin();
        let center = hand.0 + (goal_center - hand.0) * t + lift;
        (center, hand.1 + (goal_angle - hand.1) * t, hand.2 + (goal_radius - hand.2) * t)
    }

    /// Everything that moves on his face: mouth, moustache, eyes and the nose over them.
    fn paint_face(&mut self, painter: &Painter, assets: &Assets, head: Affine, look: Option<Pos2>, now: f64, dt: f32) {
        // Mouth: opens under the moustache with how loud he's speaking, so he looks like he's talking.
        let m = self.mouth;
        let mouth = self.rig.mouth;
        if m > 0.04 {
            let center = vec2(mouth.center[0], mouth.center[1] + 6.0 * m);
            let size = vec2(mouth.width + 4.0 * m, 3.0 + mouth.open * m);
            painter.add(Shape::convex_polygon(ellipse(head, center, size, 0.0, TAU, 24), Color32::from_rgb(70, 22, 24), Stroke::NONE));
            let tongue = ellipse(head, center + vec2(0.0, size.y * 0.5), vec2(size.x * 0.6, size.y * 0.42), 0.0, TAU, 18);
            painter.add(Shape::convex_polygon(tongue, Color32::from_rgb(196, 88, 88), Stroke::NONE));
        }
        let lift = head.then(Affine::translate(vec2(0.0, -1.5 * m)));
        draw_part(painter, assets.part("head_moustache"), lift, Color32::WHITE);

        // Aim: direction to the target in the head's own frame, stronger the further away it is.
        let goal = look.map_or(vec2(0.25, 0.1), |target| {
            let mid = head.apply(vec2(240.0, 140.0));
            let d = target - mid;
            let inv = head.inverse();
            let local = vec2(inv.a * d.x + inv.c * d.y, inv.b * d.x + inv.d * d.y);
            local.normalized() * (d.length() / 240.0).min(1.0)
        });
        self.look = approach2(self.look, goal, 16.0, dt);

        if now > self.next_blink + 0.16 {
            self.next_blink = now + 2.0 + 4.0 * self.random() as f64;
        }
        let blink_t = ((now - self.next_blink) / 0.16) as f32;
        let blink = if (0.0..1.0).contains(&blink_t) { 1.0 - (blink_t * 2.0 - 1.0).abs() } else { 0.0 };
        let blink = blink.max(self.lids);

        for eye in self.rig.eyes.clone() {
            let center = vec2(eye.center[0], eye.center[1]);
            let max = vec2(eye.rx - eye.pupil_rx - 3.0, eye.ry - eye.pupil_ry - 3.0);
            let p = center + vec2(self.look.x * max.x, self.look.y * max.y);
            let iris = ellipse(head, p, vec2(eye.pupil_rx, eye.pupil_ry), 0.0, TAU, 28);
            painter.add(Shape::convex_polygon(iris, Color32::from_rgb(78, 46, 28), Stroke::NONE));
            let pupil = ellipse(head, p + vec2(0.0, 1.0), vec2(eye.pupil_rx * 0.68, eye.pupil_ry * 0.72), 0.0, TAU, 24);
            painter.add(Shape::convex_polygon(pupil, Color32::from_rgb(28, 16, 10), Stroke::NONE));
            let glint = ellipse(head, p + vec2(eye.pupil_rx * 0.35, -eye.pupil_ry * 0.42), Vec2::splat(3.2), 0.0, TAU, 10);
            painter.add(Shape::convex_polygon(glint, Color32::WHITE, Stroke::NONE));
            if blink > 0.02 {
                let h = -1.0 + 2.0 * blink;
                let a = h.clamp(-1.0, 1.0).asin();
                let lid = ellipse(head, center, vec2(eye.rx + 1.5, eye.ry + 1.5), PI - a, TAU + a, 20);
                let chord = [lid[0], lid[lid.len() - 1]];
                painter.add(Shape::convex_polygon(lid, Color32::from_rgb(250, 208, 164), Stroke::NONE));
                let ink = Stroke::new(2.0 * head.scale_len().max(0.6), theme::INK);
                if blink < 0.97 {
                    painter.line_segment(chord, ink);
                } else {
                    // Shut: a sleepy downward curve.
                    let lash = ellipse(head, center + vec2(0.0, 6.0), vec2(eye.rx * 0.75, eye.ry * 0.3), 0.15 * PI, 0.85 * PI, 12);
                    painter.add(Shape::line(lash, Stroke::new(3.0 * head.scale_len().max(0.6), theme::INK)));
                }
            }
        }
        draw_part(painter, assets.part("head_nose"), head, Color32::WHITE);
    }

    /// Diving into the hole (`into`) or climbing back out to his stage.
    fn paint_flight(&mut self, assets: &Assets, painter: &Painter, t: f32, into: bool, now: f64) {
        let Some(hole) = self.hole else { return };
        let t = t.clamp(0.0, 1.0);
        let e = ease(t);
        let (from, to) = if into { (self.stage_feet, hole) } else { (hole, self.stage_feet) };
        let arc = vec2(0.0, -260.0) * (e * PI).sin();
        let feet = from + (to - from) * e + arc;
        let size = if into { 1.0 - 0.75 * e } else { 0.25 + 0.75 * e };
        let spin = if into { e * TAU } else { (1.0 - e) * -TAU };
        let scale = self.stage_height / self.rig.bounds.height() * size;
        let center_rig = self.rig.bounds.center().to_vec2();
        let root = Affine::translate(feet.to_vec2() + vec2(0.0, -self.stage_height * size * 0.5))
            .then(Affine::rotate(spin))
            .then(Affine::scale(scale, scale))
            .then(Affine::translate(-center_rig));
        let pose = self.live_pose(now);
        let worlds = self.rig.worlds(&pose, root);
        let mut order: Vec<usize> = (0..self.rig.parts.len()).collect();
        order.sort_by_key(|&i| self.rig.parts[i].z);
        let near_hole = if into { t > 0.85 } else { t < 0.15 };
        let painter = if near_hole {
            painter.with_clip_rect(Rect::from_min_max(pos2(-1e5, -1e5), pos2(1e5, hole.y)))
        } else {
            painter.clone()
        };
        for i in order {
            draw_part(&painter, assets.part(&self.rig.parts[i].name), worlds[i], Color32::WHITE);
            if i == self.rig.head {
                self.paint_face(&painter, assets, worlds[i], None, now, 1.0 / 60.0);
            }
        }
    }

    /// Just his head and hat, rising out of the hole or leaning in from the right edge of the map.
    fn paint_peek(&mut self, assets: &Assets, painter: &Painter, t: f32, edge: bool, pointer: Option<Pos2>) {
        let up = ease(t / 0.35) * (1.0 - ease((t - 3.0) / 0.35));
        let scale = self.stage_height / self.rig.bounds.height() * 1.05;
        let pose = self.rig.rest_pose();
        let head_part = &self.rig.parts[self.rig.head];
        let head_size = 285.0 * scale;
        let (root, clip) = match (edge, self.edge, self.hole) {
            (true, Some(edge_rect), _) => {
                let y = edge_rect.top() + edge_rect.height() * 0.42;
                let x = edge_rect.right() + head_size * 0.62 - head_size * 1.1 * up;
                let root = Affine::translate(vec2(x, y)).then(Affine::rotate(-PI / 2.0 + 0.25)).then(Affine::scale(scale, scale));
                (root, Rect::from_min_max(pos2(-1e5, -1e5), pos2(edge_rect.right(), 1e5)))
            }
            (_, _, Some(hole)) => {
                let y = hole.y + head_size * 0.55 - head_size * 0.95 * up;
                let root = Affine::translate(vec2(hole.x, y)).then(Affine::rotate((t * 2.2).sin() * 0.12)).then(Affine::scale(scale, scale));
                (root, Rect::from_min_max(pos2(-1e5, -1e5), pos2(1e5, hole.y + 2.0)))
            }
            _ => return,
        };
        // Place the head's neck pin at the root origin.
        let head_root = root.then(Affine::translate(-(head_part.at - self.rig.parts[self.rig.torso].anchor)));
        let worlds = self.rig.worlds(&pose, head_root);
        let painter = painter.with_clip_rect(clip);
        draw_part(&painter, assets.part("head"), worlds[self.rig.head], Color32::WHITE);
        self.paint_face(&painter, assets, worlds[self.rig.head], pointer, self.time, 1.0 / 60.0);
        draw_part(&painter, assets.part("hat"), worlds[self.rig.hat], Color32::WHITE);
    }

    fn paint_hole(&self, painter: &Painter) {
        let Some(hole) = self.hole else { return };
        let r = vec2(92.0, 30.0);
        // Torn paper lip, then the dark inside.
        let mut rim = Vec::new();
        let mut inner = Vec::new();
        for i in 0..48 {
            let a = i as f32 / 48.0 * TAU;
            let jag = 1.0 + 0.08 * ((i * 7 % 5) as f32 / 4.0 - 0.5) + 0.05 * (a * 9.0).sin();
            rim.push(hole + vec2(a.cos() * (r.x + 8.0) * jag, a.sin() * (r.y + 6.0) * jag));
            inner.push(hole + vec2(a.cos() * r.x * jag, a.sin() * r.y * jag));
        }
        painter.add(Shape::convex_polygon(rim.iter().map(|p| *p + vec2(0.0, 3.0)).collect(), Color32::from_black_alpha(60), Stroke::NONE));
        painter.add(Shape::convex_polygon(rim, Color32::from_rgb(214, 196, 158), Stroke::NONE));
        painter.add(Shape::convex_polygon(inner.clone(), Color32::from_rgb(10, 12, 22), Stroke::NONE));
        let shade: Vec<Pos2> = inner.iter().map(|p| hole + (*p - hole) * 0.7 + vec2(0.0, 6.0)).collect();
        painter.add(Shape::convex_polygon(shade, Color32::from_rgb(4, 5, 10), Stroke::NONE));
    }

    fn paint_scraps(&self, painter: &Painter, now: f64) {
        for s in &self.scraps {
            let fade = (1.0 - ((now - s.born) as f32 / 1.6)).clamp(0.0, 1.0);
            let (sin, cos) = s.angle.sin_cos();
            let h = s.size * 0.5;
            let pts = [vec2(-h.x, -h.y), vec2(h.x, -h.y), vec2(h.x, h.y), vec2(-h.x, h.y)]
                .iter()
                .map(|v| s.pos + vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos))
                .collect();
            painter.add(Shape::convex_polygon(pts, s.color.gamma_multiply(fade), Stroke::NONE));
        }
    }

    fn paint_away_sign(&self, assets: &Assets, painter: &Painter, stage: Rect) {
        let center = pos2(stage.min.x + 120.0, stage.center().y + 20.0);
        let rect = Rect::from_center_size(center, vec2(178.0, 96.0));
        let angle: f32 = -0.06;
        let outline: Vec<Pos2> = theme::deckled(rect, 99, 1.6)
            .into_iter()
            .map(|p| {
                let v = p - center;
                center + vec2(v.x * angle.cos() - v.y * angle.sin(), v.x * angle.sin() + v.y * angle.cos())
            })
            .collect();
        theme::paper(painter, &outline, &assets.parchment, Color32::WHITE, 5.0);
        theme::pin(painter, rect.center_top() + vec2(0.0, 10.0), theme::RED_INK);
        painter.text(center + vec2(0.0, 6.0), eframe::egui::Align2::CENTER_CENTER, "Out investigating!\nBack in a jiffy.",
            theme::hand(19.0), theme::INK);
    }

    fn paint_bubble(&self, painter: &Painter, stage: Rect, now: f64) {
        let Some(b) = &self.bubble else { return };
        let age = (now - b.start) as f32;
        let end = b.speak + 2.4;
        let alpha = (age / 0.15).min(1.0) * ((end - age) / 0.35).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let wrap = 230.0;
        let font = theme::hand(20.0);
        let full = painter.layout(b.text.clone(), font.clone(), theme::INK, wrap);
        let shown = ((age / (b.speak * 0.92).max(0.3)).min(1.0) * b.text.chars().count() as f32).ceil() as usize;
        let visible: String = b.text.chars().take(shown).collect();
        let size = full.size() + vec2(34.0, 26.0);

        let (rect, tail_to) = match (b.from, self.hole) {
            (Speaker::Hole | Speaker::Peek, Some(hole)) => {
                let rect = Rect::from_center_size(hole + vec2(150.0 + size.x * 0.3, -150.0), size);
                (rect, hole + vec2(40.0, -30.0))
            }
            _ => {
                let rect = Rect::from_min_size(pos2(stage.max.x - size.x - 10.0, stage.min.y + 10.0), size);
                let head = self.stage_feet + vec2(30.0, -self.stage_height * 0.78);
                (rect, head)
            }
        };
        cloud(painter, rect, tail_to, alpha);
        let job = LayoutJob::simple(visible, font, theme::INK.gamma_multiply(alpha), wrap);
        let galley: Arc<Galley> = painter.layout_job(job);
        painter.galley(rect.min + vec2(17.0, 12.0), galley, theme::INK);
    }
}

/// A cream thought-cloud with an ink outline and a trail of little puffs towards `tail_to`.
fn cloud(painter: &Painter, rect: Rect, tail_to: Pos2, alpha: f32) {
    let ink = theme::INK.gamma_multiply(alpha);
    let fill = Color32::from_rgb(252, 245, 228).gamma_multiply(alpha);
    let shadow = Color32::from_black_alpha((60.0 * alpha) as u8);
    let c = rect.center();
    let half = rect.size() * 0.5;
    let n = ((rect.width() + rect.height()) / 22.0) as usize;
    let puffs: Vec<(Pos2, f32)> = (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * TAU;
            let r = 15.0 + 5.0 * ((i * 37 % 7) as f32 / 6.0);
            (c + vec2(a.cos() * (half.x - 4.0), a.sin() * (half.y - 4.0)), r)
        })
        .collect();
    let edge = (tail_to - c).normalized();
    let start = c + vec2(edge.x * half.x, edge.y * half.y);
    let trail: Vec<(Pos2, f32)> =
        (1..=3).map(|i| (start + (tail_to - start) * (i as f32 * 0.27), 9.0 - i as f32 * 2.2)).collect();

    for (offset, color, grow) in [(vec2(3.0, 5.0), shadow, 2.0), (Vec2::ZERO, ink, 2.0), (Vec2::ZERO, fill, 0.0)] {
        painter.rect_filled(rect.translate(offset).expand(grow), CornerRadius::same(22), color);
        for (p, r) in puffs.iter().chain(trail.iter()) {
            painter.circle_filled(*p + offset, r + grow, color);
        }
    }
}

/// What to say about a box that was clicked.
pub fn event_for(name: &str, size: u64, is_dir: bool, view_size: u64, roll: f32) -> &'static str {
    let lower = name.to_lowercase();
    if is_dir {
        return match lower.as_str() {
            "node_modules" => "folder_node_modules",
            ".git" => "folder_git",
            "downloads" => "folder_downloads",
            n if n.contains("cache") || n == "temp" || n == "tmp" => "folder_cache",
            _ if size >= 5 << 30 && size * 4 >= view_size => "folder_huge",
            _ => "folder",
        };
    }
    if matches!(lower.as_str(), "pagefile.sys" | "hiberfil.sys" | "swapfile.sys") {
        return "type_system";
    }
    if size >= 4 << 30 {
        return "file_huge";
    }
    let ext = lower.rsplit_once('.').map_or("", |(stem, e)| if stem.is_empty() { "" } else { e });
    let kind = match ext {
        "" => Some("type_noext"),
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "flv" | "mpg" | "mpeg" => Some("type_video"),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "heic" | "raw" | "cr2" | "nef" | "psd"
        | "svg" | "ico" | "exr" | "hdr" | "dds" => Some("type_image"),
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "aiff" | "aif" | "opus" | "mid" | "aup3" => {
            Some("type_audio")
        }
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "cab" => Some("type_archive"),
        "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "dmg" | "qcow2" | "vdi" => Some("type_diskimage"),
        "exe" | "dll" | "so" | "dylib" | "app" => Some("type_program"),
        "msi" | "msix" | "appx" | "apk" | "pkg" | "deb" | "rpm" => Some("type_installer"),
        "safetensors" | "gguf" | "ckpt" | "pt" | "pth" | "onnx" | "bin" | "ggml" | "h5" | "tflite" => Some("type_model"),
        "pdf" | "doc" | "docx" | "txt" | "md" | "rtf" | "odt" | "xls" | "xlsx" | "ppt" | "pptx" | "csv" | "epub" => {
            Some("type_document")
        }
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "cpp" | "h" | "hpp" | "cs" | "java" | "go" | "rb" | "php"
        | "html" | "css" | "json" | "toml" | "yaml" | "yml" | "sh" | "ps1" | "lua" | "swift" | "kt" => Some("type_code"),
        "pak" | "p4k" | "uasset" | "umap" | "bsa" | "ba2" | "vpk" | "wad" | "pk3" | "bundle" => Some("type_game"),
        "log" | "db" | "sqlite" | "etl" | "dmp" => Some("type_log"),
        "sys" => Some("type_system"),
        _ => None,
    };
    match kind {
        Some(k) if k == "type_noext" || roll < 0.7 => k,
        _ if size >= 400 << 20 => "file_large",
        _ if size < 16 << 10 => "file_tiny",
        _ => "file_medium",
    }
}
