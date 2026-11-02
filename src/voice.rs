//! The detective's recorded lines: which to say for what, and playing them.

use std::collections::HashMap;
use std::io::Cursor;
use std::time::Duration;

use rodio::Source as _;
use serde::Deserialize;

include!(concat!(env!("OUT_DIR"), "/voice_clips.rs"));

/// Silence played before each line. Many outputs (Bluetooth and USB headsets, power-saving sound
/// cards) take a moment to wake up and would otherwise swallow the first word.
pub const LEAD_IN: f32 = 0.35;

#[derive(Deserialize, Clone)]
pub struct Line {
    pub id: String,
    pub event: String,
    pub text: String,
}

pub struct Voice {
    // The stream must outlive every sink playing through it.
    stream: Option<rodio::OutputStream>,
    sink: Option<rodio::Sink>,
    lines: Vec<Line>,
    durations: HashMap<String, f32>,
    /// Lines already used per event, so each event works through its lines before repeating.
    used: HashMap<String, Vec<usize>>,
    rng: u64,
    pub muted: bool,
    /// Loudness of the playing line, `ENVELOPE_RATE` values per second, and when it started.
    envelope: Vec<f32>,
    started: f64,
}

const ENVELOPE_RATE: f32 = 30.0;

/// The clip's loudness over time, scaled so its loudest moment is 1.
fn loudness(bytes: &'static [u8]) -> Vec<f32> {
    let Ok(decoder) = rodio::Decoder::new(Cursor::new(bytes)) else { return Vec::new() };
    let window = ((decoder.sample_rate() as f32 * decoder.channels() as f32) / ENVELOPE_RATE).max(1.0) as usize;
    let mut out = Vec::new();
    let (mut sum, mut n) = (0.0f32, 0usize);
    for sample in decoder {
        sum += sample * sample;
        n += 1;
        if n == window {
            out.push((sum / n as f32).sqrt());
            sum = 0.0;
            n = 0;
        }
    }
    let peak = out.iter().copied().fold(1e-4f32, f32::max);
    out.into_iter().map(|v| (v / peak).powf(0.7)).collect()
}

impl Voice {
    pub fn new(muted: bool) -> Self {
        let lines: Vec<Line> = serde_json::from_str(include_str!("../assets/voice/lines.json")).expect("lines.json");
        let durations = serde_json::from_str(include_str!("../assets/voice/durations.json")).unwrap_or_default();
        let stream = rodio::OutputStreamBuilder::open_default_stream().ok().map(|mut s| {
            s.log_on_drop(false);
            s
        });
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64);
        Self { stream, sink: None, lines, durations, used: HashMap::new(), rng: seed | 1, muted, envelope: Vec::new(), started: 0.0 }
    }

    fn random(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % n.max(1) as u64) as usize
    }

    /// A line for `event` that hasn't been said since its siblings all had a turn.
    pub fn pick(&mut self, event: &str) -> Option<Line> {
        let candidates: Vec<usize> = (0..self.lines.len()).filter(|&i| self.lines[i].event == event).collect();
        if candidates.is_empty() {
            return None;
        }
        let used = self.used.entry(event.to_string()).or_default().clone();
        let mut fresh: Vec<usize> = candidates.iter().copied().filter(|i| !used.contains(i)).collect();
        if fresh.is_empty() {
            // Start over, but not with the line that was just said.
            let last = used.last().copied();
            self.used.insert(event.to_string(), Vec::new());
            fresh = candidates.into_iter().filter(|i| Some(*i) != last || used.len() <= 1).collect();
        }
        let choice = fresh[self.random(fresh.len())];
        self.used.entry(event.to_string()).or_default().push(choice);
        Some(self.lines[choice].clone())
    }

    /// How long the line takes to say, in seconds.
    pub fn duration(&self, line: &Line) -> f32 {
        self.durations.get(&line.id).copied().unwrap_or_else(|| 0.9 + line.text.split_whitespace().count() as f32 * 0.36)
    }

    /// Starts `line`; `now` is the app's clock, used to follow how loud it is while it plays.
    pub fn play(&mut self, line: &Line, now: f64) {
        self.stop();
        if self.muted {
            return;
        }
        let (Some(stream), Some((_, bytes))) = (&self.stream, CLIPS.iter().find(|(id, _)| *id == line.id)) else {
            return;
        };
        if let Ok(source) = rodio::Decoder::new(Cursor::new(*bytes)) {
            let sink = rodio::Sink::connect_new(stream.mixer());
            sink.append(source.delay(Duration::from_secs_f32(LEAD_IN)));
            self.sink = Some(sink);
            self.envelope = loudness(bytes);
            self.started = now + LEAD_IN as f64;
        }
    }

    /// How loud the current line is right now, 0 to 1 (0 when nothing is playing).
    pub fn level(&self, now: f64) -> f32 {
        if self.sink.as_ref().is_none_or(|s| s.empty()) {
            return 0.0;
        }
        let t = now - self.started;
        if t < 0.0 {
            return 0.0;
        }
        self.envelope.get((t * ENVELOPE_RATE as f64) as usize).copied().unwrap_or(0.0)
    }

    pub fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
        }
        self.envelope.clear();
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if muted {
            self.stop();
        }
    }
}
