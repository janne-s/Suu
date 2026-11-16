use chimera::anatomy::human::{ArticKey, ArticPatch, BODY_MALE, BodyMix, HumanBody, N_ARTIC};
use chimera::math::{PI, clamp, cos, exp, js_max, midi_to_hz, pow, sin};
use chimera::phonetics::Syllable;
use chimera::phonetics::english::EnglishG2P;
use chimera::phonetics::planner::vowel_artic;
use chimera::control::articulators::ArticulatorBank;
use chimera::control::score::{ControlKey, Gesture};
use chimera::phonetics::sing::{NoteOnOptions, SongOptions, SungNote, plan_note_off, plan_note_on, plan_song_at};
use chimera::phonetics::syllables::lyrics_to_syllables;
use chimera::voice::human_airway::{HumanAirway, NoiseGains};
use chimera::voice::human_voice::{HumanVoice, SourcePatch};

#[cfg(test)]
mod tests;
mod track;
mod vowel;
use track::PitchTracker;
use vowel::VowelTracker;
pub use vowel::pad_of;

pub const MAX_BLOCK: usize = 256;
const CTRL: u64 = 32;
const FADE: u32 = 1024;
pub const STEPS: usize = 16;

const CORNERS: [&str; 4] = ["i", "u", "æ", "ɑ"];
const LIVE_LEAD: f64 = -0.045;

fn start_of(gs: &[Gesture]) -> f64 {
    gs.iter().fold(f64::INFINITY, |m, g| m.min(g.t0 - g.ramp_in))
}

#[derive(Clone, Copy)]
enum Plan {
    HoldOn(usize),
    HoldOff(usize),
    Shot(usize),
    Tick(usize),
    Pass,
}

pub const STEP_VOWELS: [&str; 5] = ["ɑ", "e", "i", "o", "u"];
const STEP_PAD: [(f64, f64); 5] = [(1.0, 1.0), (0.0, 0.5), (0.0, 0.0), (1.0, 0.5), (1.0, 0.0)];

pub const MOD_TARGETS: [&str; N_MOD] = ["vowelX", "vowelY", "round", "nasal", "inject", "damping", "morph", "shift", "open"];
pub const N_MOD: usize = 9;
const MOD_RANGE: [(f64, f64); N_MOD] = [(-0.3, 1.3), (-0.3, 1.3), (-1.0, 1.0), (0.0, 1.5), (0.0, 1.0), (0.1, 20.0), (0.0, 1.0), (-0.5, 0.5), (0.25, 4.0)];
const MOD_LOG: [bool; N_MOD] = [false, false, false, false, false, true, false, false, true];
pub const HORN_POINTS: usize = 32;
const INJECT_REF: f64 = 562.0;
const QUIET: f32 = 1e-6;
const SLEEP_AFTER: f64 = 0.1;
const SNAP_RING: usize = 4096;
const GRAIN_MIN_FALL: f64 = 0.004;
pub const AIR_MAX: f64 = 10.0;
pub const WHISTLE_DAMP: f64 = 60.0;
pub const MOVE_MAX: f64 = 8.0;

fn periodic(shape: Shape, ph: f64, sm: f64) -> Option<f64> {
    let ease = |x: f64| 0.5 - 0.5 * cos(PI * clamp(x, 0.0, 1.0));
    match shape {
        Shape::Sine => Some(sin(2.0 * PI * ph)),
        Shape::Triangle => Some(1.0 - 4.0 * (ph - 0.5).abs()),
        Shape::Saw => {
            let r = 0.5 * sm;
            Some(if ph < 1.0 - r { -1.0 + 2.0 * ph / (1.0 - r) } else { 1.0 - 2.0 * ease((ph - (1.0 - r)) / r) })
        }
        Shape::Square => {
            let e = 0.5 * sm;
            let (half, sign) = if ph < 0.5 { (ph, 1.0) } else { (ph - 0.5, -1.0) };
            Some(if e > 0.0 && half < e { sign * (2.0 * ease(half / e) - 1.0) } else { sign })
        }
        _ => None,
    }
}

fn narrow_loss(whistle: f64) -> f64 {
    1.0 + (1.0 - whistle) * WHISTLE_DAMP
}
pub const UI_AREAS: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sine,
    Triangle,
    Saw,
    Square,
    Random,
    Follow,
    Adsr,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mod {
    pub depth: f64,
    pub rate_hz: f64,
    pub sync: f64,
    pub shape: Shape,
    pub smooth: f64,
}

impl Default for Mod {
    fn default() -> Self {
        Mod { depth: 0.0, rate_hz: 1.0, sync: 0.0, shape: Shape::Sine, smooth: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Off,
    Saw,
    Supersaw,
    Pulse,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SeqGate {
    Seq,
    AndIn,
    OrIn,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub source: Source,
    pub osc_db: f64,
    pub input_db: f64,
    pub note: f64,
    pub gate: bool,
    pub drive_db: f64,
    pub tilt_hz: f64,
    pub breath: f64,
    pub follow: f64,
    pub vowel_x: f64,
    pub vowel_y: f64,
    pub round: f64,
    pub nasal: f64,
    pub inject: f64,
    pub damping: f64,
    pub folds: f64,
    pub sub_hz: f64,
    pub sub_level: f64,
    pub mix: f64,
    pub out_db: f64,
    pub bpm: f64,
    pub glide_s: f64,
    pub glide_sync: f64,
    pub glide_dir: GlideDir,
    pub text_step: TextStep,
    pub text_unit: TextUnit,
    pub text_say: TextSay,
    pub text_rate_hz: f64,
    pub text_sync: f64,
    pub text_len: f64,
    pub text_seq: bool,
    pub text_once: bool,
    pub text_gate: f64,
    pub speech: f64,
    pub reson: f64,
    pub morph: f64,
    pub shift: f64,
    pub open: f64,
    pub env_gate: bool,
    pub env_thresh_db: f64,
    pub env_release_s: f64,
    pub env_release_sync: f64,
    pub pitch_from: PitchFrom,
    pub vowel_in: bool,
    pub attack_s: f64,
    pub attack_sync: f64,
    pub decay_s: f64,
    pub decay_sync: f64,
    pub sustain: f64,
    pub release_s: f64,
    pub release_sync: f64,
    pub legato: bool,
    pub coarse: f64,
    pub steady_s: f64,
    pub steady_sync: f64,
    pub hq: bool,
    pub lofi: bool,
    pub grid: f64,
    pub scale: u16,
    pub jump: f64,
    pub jump_fold: bool,
    pub text_grid: f64,
    pub grid_breath: f64,
    pub grid_smooth: f64,
    pub text_round: f64,
    pub text_fit: bool,
    pub aspiration: f64,
    pub frication: f64,
    pub sibilance: f64,
    pub whistle: f64,
    pub move_sound: f64,
    pub pressure_to: i32,
    pub pressure_depth: f64,
    pub slide_to: i32,
    pub slide_depth: f64,
    pub stereo: bool,
    pub stereo_spread: f64,
    pub stereo_vowel: f64,
    pub stereo_width: f64,
    pub stereo_canon: f64,
    pub stereo_phase: f64,
    pub stereo_mirror: bool,
    pub pulse_width: f64,
    pub pwm_depth: f64,
    pub pwm_rate_hz: f64,
    pub pwm_sync: f64,
    pub seq_gate: SeqGate,
    pub seq_vowel: f64,
}

pub const EXPR_BREATH: i32 = N_MOD as i32;
pub const EXPR_LEVEL: i32 = N_MOD as i32 + 1;

impl Params {
    pub fn quality(&self) -> Quality {
        if self.hq {
            Quality::Hq
        } else if self.lofi {
            Quality::Lo
        } else {
            Quality::Lean
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Hq,
    Lean,
    Lo,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            source: Source::Supersaw,
            osc_db: 0.0,
            input_db: 0.0,
            note: 45.0,
            gate: false,
            drive_db: 0.0,
            tilt_hz: 150.0,
            breath: 1.0,
            follow: 0.7,
            vowel_x: 0.5,
            vowel_y: 0.6,
            round: 0.0,
            nasal: 0.0,
            inject: 0.0,
            damping: 1.0,
            folds: 0.0,
            sub_hz: 120.0,
            sub_level: 0.0,
            mix: 1.0,
            out_db: 0.0,
            bpm: 100.0,
            glide_s: 0.03,
            glide_sync: 0.0,
            glide_dir: GlideDir::Both,
            text_step: TextStep::Note,
            text_unit: TextUnit::Syllable,
            text_say: TextSay::Hold,
            text_rate_hz: 2.0,
            text_sync: 1.0,
            text_len: 1.0,
            text_gate: 1.0,
            text_seq: true,
            text_once: false,
            speech: 1.0,
            reson: 0.0,
            morph: 0.0,
            shift: 0.0,
            open: 1.0,
            env_gate: false,
            env_thresh_db: -40.0,
            env_release_s: 0.15,
            env_release_sync: 0.0,
            pitch_from: PitchFrom::Note,
            vowel_in: false,
            attack_s: 0.005,
            attack_sync: 0.0,
            decay_s: 0.2,
            decay_sync: 0.0,
            sustain: 1.0,
            release_s: 0.08,
            release_sync: 0.0,
            legato: false,
            coarse: 0.0,
            steady_s: 0.0,
            steady_sync: 0.0,
            hq: true,
            lofi: false,
            grid: 0.0,
            scale: 0xfff,
            jump: 0.0,
            jump_fold: true,
            text_grid: 0.0,
            grid_breath: 1.0,
            grid_smooth: 0.0,
            text_round: 0.0,
            text_fit: false,
            aspiration: 1.0,
            frication: 1.0,
            sibilance: 1.0,
            whistle: 1.0,
            move_sound: 1.0,
            pressure_to: EXPR_BREATH,
            pressure_depth: 0.5,
            slide_to: 1,
            slide_depth: 0.5,
            stereo: false,
            stereo_spread: 1.12,
            stereo_vowel: 0.12,
            stereo_width: 1.0,
            stereo_canon: 0.0,
            stereo_phase: 0.0,
            stereo_mirror: false,
            pulse_width: 0.5,
            pwm_depth: 0.0,
            pwm_rate_hz: 0.5,
            pwm_sync: 0.0,
            seq_gate: SeqGate::Seq,
            seq_vowel: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub on: bool,
    pub note: i32,
    pub tie: bool,
    pub vowel: i32,
    pub accent: bool,
    pub len: f64,
}

pub const ACCENT: f64 = 1.6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PitchFrom {
    Note,
    Input,
    InputSnap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GlideDir {
    Both,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextStep {
    Note,
    Clock,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextUnit {
    Syllable,
    Word,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextSay {
    Hold,
    Shot,
}

struct Seq {
    steps: [Step; STEPS],
    len: usize,
    div: f64,
    gate_scale: f64,
    cycles: u64,
    playing: bool,
    t: f64,
    step: usize,
    gate_open: bool,
    note: f64,
}

pub fn default_pattern() -> [Step; STEPS] {
    let pat: [(bool, i32, bool, i32); STEPS] = [
        (true, 0, false, 0),
        (false, 0, false, -1),
        (true, 12, false, 4),
        (true, 0, false, 2),
        (true, 0, true, 0),
        (true, 7, false, 3),
        (false, 0, false, -1),
        (true, 10, false, 1),
        (true, 0, false, 0),
        (false, 0, false, -1),
        (true, 12, false, 4),
        (true, 0, false, 2),
        (true, 3, false, 3),
        (true, 5, false, 0),
        (true, 7, false, 1),
        (false, 0, false, -1),
    ];
    pat.map(|(on, note, tie, vowel)| Step { on, note, tie, vowel, accent: false, len: 0.6 })
}

impl Seq {
    fn new() -> Self {
        Seq {
            steps: default_pattern(),
            len: 16,
            div: 4.0,
            gate_scale: 1.0,
            cycles: 0,
            playing: false,
            t: 0.0,
            step: 0,
            gate_open: false,
            note: 0.0,
        }
    }
}

const PWM_SWING: f64 = 0.45;
const PWM_MIN: f64 = 0.02;

const SAW_CENTS: [f64; 5] = [0.0, -14.0, 14.0, -7.0, 7.0];

fn poly_blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

struct Osc {
    phase: [f64; 5],
    detune: [f64; 5],
}

impl Osc {
    fn new() -> Self {
        Osc { phase: [0.0, 0.31, 0.57, 0.13, 0.83], detune: SAW_CENTS.map(|c| pow(2.0, c / 1200.0)) }
    }
    fn saw(p: &mut f64, dt: f64) -> f64 {
        let y = 2.0 * *p - 1.0 - poly_blep(*p, dt);
        *p += dt;
        if *p >= 1.0 {
            *p -= 1.0;
        }
        y
    }
    fn next(&mut self, src: Source, dt: f64, width: f64) -> f64 {
        match src {
            Source::Saw | Source::Off => Self::saw(&mut self.phase[0], dt),
            Source::Pulse => {
                let p = &mut self.phase[0];
                let mut y = if *p < width { 1.0 } else { -1.0 } - (2.0 * width - 1.0);
                y += poly_blep(*p, dt);
                y -= poly_blep((*p + (1.0 - width)) % 1.0, dt);
                *p += dt;
                if *p >= 1.0 {
                    *p -= 1.0;
                }
                y
            }
            Source::Supersaw => {
                let mut s = 0.0;
                for k in 0..5 {
                    s += Self::saw(&mut self.phase[k], dt * self.detune[k]);
                }
                s * 0.45
            }
        }
    }
}

pub fn default_horn() -> [f64; HORN_POINTS] {
    core::array::from_fn(|i| {
        let u = i as f64 / (HORN_POINTS - 1) as f64;
        0.6 + 1.5 * exp(-((u - 0.3) / 0.12) * ((u - 0.3) / 0.12)) + 9.0 * u * u * u
    })
}

struct Reson {
    buf: Vec<f64>,
    w: usize,
    lp: f64,
    dc_x: f64,
    dc_y: f64,
}

impl Reson {
    fn new(fs: f64) -> Self {
        Reson { buf: vec![0.0; ((fs / 15.0) as usize + 4).next_power_of_two()], w: 0, lp: 0.0, dc_x: 0.0, dc_y: 0.0 }
    }

    fn tick(&mut self, x: f64, period: f64, fb: f64) -> f64 {
        let mask = self.buf.len() - 1;
        let d = clamp(period, 2.0, mask as f64 - 2.0);
        let di = d as usize;
        let fr = d - di as f64;
        let a = self.buf[(self.w + mask + 1 - di) & mask];
        let b = self.buf[(self.w + mask - di) & mask];
        let delayed = a + fr * (b - a);
        self.lp += 0.7 * (delayed - self.lp);
        let y = x + fb * self.lp;
        self.buf[self.w] = y;
        self.w = (self.w + 1) & mask;
        let out = y - self.dc_x + 0.995 * self.dc_y;
        self.dc_x = y;
        self.dc_y = out;
        out
    }
}

pub fn snap_to_scale(n: f64, scale: u16) -> f64 {
    let r = n.round();
    if scale & 0xfff == 0 || scale & 0xfff == 0xfff {
        return r;
    }
    let ok = |m: f64| scale >> ((m as i64).rem_euclid(12)) & 1 == 1;
    let mut best = r;
    let mut dist = f64::INFINITY;
    for k in -6..=6 {
        let m = r + k as f64;
        let d = (m - n).abs();
        if ok(m) && d < dist {
            (best, dist) = (m, d);
        }
    }
    best
}

const LEAN_CELLS: usize = 6;
const LO_REST_COARSE: f64 = 2.0;
const LO_CONTROL_EVERY: usize = 256;

fn oversample_for(fs: f64, k: f64, q: Quality) -> usize {
    if q != Quality::Hq {
        let body = HumanBody { name: "suu", scale: k, ..BODY_MALE };
        return (1..=4).find(|&m| HumanAirway::fits(fs * m as f64, &body, LEAN_CELLS)).unwrap_or(4);
    }
    let base: usize = if k < 0.5 {
        4
    } else if k < 0.68 {
        3
    } else if k < 1.6 {
        2
    } else {
        1
    };
    if fs > 60000.0 { base.div_ceil(2).max(1) } else { base }
}

#[derive(Clone)]
pub struct MouthSeed {
    fs: f64,
    scale: f64,
    artic: ArticulatorBank,
    profile: [f64; HORN_POINTS],
    wrap: bool,
    horn: (f64, f64, f64),
    inject: f64,
    damping: f64,
    folds: f64,
    pressure: f64,
    quality: Quality,
}

impl MouthSeed {
    pub fn scale(&self) -> f64 {
        self.scale
    }
}

pub struct Mouth {
    voice: Box<HumanVoice>,
    scale: f64,
    quality: Quality,
}

impl Mouth {
    pub fn build(seed: &MouthSeed) -> Mouth {
        let body = HumanBody { name: "suu", scale: seed.scale, ..BODY_MALE };
        let os = oversample_for(seed.fs, seed.scale, seed.quality);
        let mut v = Box::new(match seed.quality {
            Quality::Hq => HumanVoice::new(seed.fs, body, os),
            Quality::Lean => HumanVoice::with_min_cells(seed.fs, body, os, LEAN_CELLS),
            Quality::Lo => HumanVoice::with_layout(seed.fs, body, os, LEAN_CELLS, 1.0, LO_REST_COARSE).with_control_every(LO_CONTROL_EVERY),
        });
        v.artic = seed.artic.clone();
        v.airway.profile = seed.profile.to_vec();
        v.airway.profile_wrap = seed.wrap;
        (v.airway.profile_mix, v.airway.profile_shift, v.airway.profile_open) = seed.horn;
        let x = v.artic.x;
        v.airway.apply(&x, 0.0);
        v.airway.net.snap_to_targets();
        v.glottal_area_scale = seed.folds;
        v.set_source(&SourcePatch { voiced: Some(true), tension: Some(0.55), drift: Some(0.0), glide: Some(0.002), ..Default::default() });
        v.set_gate(1.0);
        v.set_injection(seed.inject);
        v.airway.net.viscous_scale = seed.damping;
        v.set_source(&SourcePatch { pressure: Some(seed.pressure), ..Default::default() });
        let zeros = [0f32; MAX_BLOCK];
        let mut out = [0f32; MAX_BLOCK];
        let mut left = (0.25 * seed.fs) as usize;
        while left > 0 {
            let n = left.min(MAX_BLOCK);
            v.render_driven(&zeros[..n], 100.0, &mut out[..n]);
            left -= n;
        }
        Mouth { voice: v, scale: seed.scale, quality: seed.quality }
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }
}

#[derive(Clone, Copy, PartialEq)]
struct PlanKey {
    unit: TextUnit,
    beats: f64,
    len: f64,
    bpm: f64,
    speech: f64,
}

impl PlanKey {
    fn of(p: &Params) -> Self {
        PlanKey { unit: p.text_unit, beats: loop_beats(p), len: p.text_len, bpm: p.bpm, speech: p.speech }
    }
}

fn loop_beats(p: &Params) -> f64 {
    let beats = if p.text_sync > 0.0 { p.text_sync } else { p.bpm / 60.0 / p.text_rate_hz };
    js_max(beats, 0.06 * p.bpm / 60.0)
}

pub struct TextPlan {
    text: Vec<Syllable>,
    hold_on: Vec<Vec<Gesture>>,
    hold_off: Vec<Vec<Gesture>>,
    shot: Vec<Vec<Gesture>>,
    shot_lead: f64,
    shot_len: Vec<f64>,
    tick_plan: Vec<Vec<Gesture>>,
    tick_lead: f64,
    pass_plan: Vec<Gesture>,
    pass_len: f64,
    pass_lead: f64,
    tick_s: f64,
    scratch: Vec<Gesture>,
    key: PlanKey,
}

impl TextPlan {
    pub fn empty() -> Self {
        TextPlan {
            text: Vec::new(),
            hold_on: Vec::new(),
            hold_off: Vec::new(),
            shot: Vec::new(),
            shot_lead: 0.0,
            shot_len: Vec::new(),
            tick_plan: Vec::new(),
            tick_lead: 0.0,
            pass_plan: Vec::new(),
            pass_len: 0.0,
            pass_lead: 0.0,
            tick_s: 0.5,
            scratch: Vec::new(),
            key: PlanKey::of(&Params::default()),
        }
    }

    pub fn new(text: &str, g2p: Option<&EnglishG2P>, p: &Params) -> Self {
        let syllables = match g2p {
            Some(g) if !text.trim().is_empty() => lyrics_to_syllables(text, g),
            _ => Vec::new(),
        };
        let mut plan = TextPlan { text: syllables, key: PlanKey::of(p), ..TextPlan::empty() };
        plan.plan(p);
        plan
    }

    pub fn syllables(&self) -> usize {
        self.text.len()
    }

    fn units(&self, unit: TextUnit) -> Vec<core::ops::Range<usize>> {
    match unit {
        TextUnit::Syllable => (0..self.text.len()).map(|i| i..i + 1).collect(),
        TextUnit::Word => {
            let mut out = Vec::new();
            let mut start = 0;
            for (i, s) in self.text.iter().enumerate() {
                if s.word_end || i + 1 == self.text.len() {
                    out.push(start..i + 1);
                    start = i + 1;
                }
            }
            out
        }
    }
    }

    fn plan(&mut self, p: &Params) {
    let p = *p;
    let body = BodyMix::new(0.0, 0.0);
    let beat = 60.0 / p.bpm;
    let units = self.units(p.text_unit);
    let unpitched = |gs: Vec<Gesture>| -> Vec<Gesture> { gs.into_iter().filter(|g| !g.targets.has(ControlKey::F0) && !g.targets.has(ControlKey::VIBRATO)).collect() };
    let k = 1.0 / p.speech;
    let paced = move |mut gs: Vec<Gesture>| -> Vec<Gesture> {
        let from = start_of(&gs);
        if k != 1.0 && from.is_finite() {
            for g in gs.iter_mut() {
                g.t0 = from + (g.t0 - from) * k;
                g.t1 = from + (g.t1 - from) * k;
                g.ramp_in *= k;
                g.ramp_out *= k;
                g.release_hold = g.release_hold.map(|h| h * k);
            }
        }
        gs
    };
    let shift = |gs: &mut Vec<Gesture>, dt: f64| {
        for g in gs.iter_mut() {
            g.t0 += dt;
            g.t1 += dt;
        }
    };
    let notes = |r: &core::ops::Range<usize>, beats: f64| -> Vec<SungNote> { self.text[r.clone()].iter().map(|s| SungNote { midi: 57.0, beats, syl: Syllable { phrase_end: false, ..s.clone() } }).collect() };
    let sung = |n: &[SungNote], cons: f64| unpitched(plan_song_at(n, &SongOptions { body, bpm: p.bpm, t0: 0.0, vibrato: 0.0 }, cons).gestures);
    let (mut hold_on, mut hold_off, mut shot, mut tick) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let beats = loop_beats(&p);
    let tick_s = beats * beat;
    for r in &units {
        let last = &self.text[r.end - 1];
        let mut on = Vec::new();
        let mut at = 0.0;
        if r.len() > 1 {
            on = sung(&notes(&(r.start..r.end - 1), 0.5), 0.8);
            at = (r.len() - 1) as f64 * 0.5 * beat;
        }
        let mut held = unpitched(plan_note_on(last, 57.0, &NoteOnOptions { body, vibrato: 0.0, tag: 0.0, legato_from: None }));
        shift(&mut held, at);
        on.extend(held);
        let lead = start_of(&on);
        shift(&mut on, LIVE_LEAD - lead);
        hold_on.push(paced(on));
        hold_off.push(paced(unpitched(plan_note_off(last, 57.0, body))));
        shot.push(paced(sung(&notes(r, 0.5), 0.8)));
        tick.push(sung(&notes(r, p.text_len * beats / r.len() as f64), 0.8 / p.speech));
    }
    let lead_of = |plans: &[Vec<Gesture>]| plans.iter().fold(0.0f64, |m, gs| m.max(-start_of(gs)));
    self.shot_lead = lead_of(&shot);
    self.shot_len = shot.iter().map(|gs| gs.iter().map(|g| g.t1 + g.ramp_out).fold(0.0, f64::max) - start_of(gs)).collect();
    self.tick_lead = lead_of(&tick);
    let all: Vec<SungNote> = units.iter().flat_map(|r| notes(r, beats / r.len() as f64)).collect();
    self.pass_plan = sung(&all, 0.8 / p.speech);
    self.pass_len = units.len() as f64 * tick_s;
    self.pass_lead = -start_of(&self.pass_plan).min(0.0);
    self.tick_s = tick_s;
    (self.hold_on, self.hold_off, self.shot, self.tick_plan) = (hold_on, hold_off, shot, tick);
    let most = self.hold_on.iter().chain(&self.hold_off).chain(&self.shot).chain(&self.tick_plan).map(Vec::len).chain([self.pass_plan.len()]).max().unwrap_or(0);
    self.scratch = Vec::with_capacity(most.max(256));
    }
}

pub struct Suu {
    fs: f64,
    voice: Box<HumanVoice>,
    fade: Option<(Box<HumanVoice>, u32)>,
    retired: Option<Box<HumanVoice>>,
    scale: f64,
    quality: Quality,
    noise_base: NoiseGains,
    grain: f64,
    grain_from: f64,
    expr: (f64, f64),
    level: f64,
    level_from: f64,
    p: Params,
    osc: Osc,
    pitch: f64,
    glide_to: f64,
    glide_rate: f64,
    osc_hz: f64,
    osc_env: f64,
    reson: Reson,
    period_peak: f32,
    quiet: u64,
    asleep: bool,
    snap_ring: Vec<f32>,
    snap_w: usize,
    tilt_y: f64,
    env: f64,
    sub1: f64,
    sub2: f64,
    dry: Vec<f32>,
    drive: Vec<f32>,
    drive_new: Vec<f32>,
    wet: Vec<f32>,
    wet_old: Vec<f32>,
    wet_twin: Vec<f32>,
    out_twin: Vec<f32>,
    twin: Option<Box<HumanVoice>>,
    twin_retired: Option<Box<HumanVoice>>,
    twin_scale: f64,
    twin_mix: f64,
    twin_quality: Quality,
    twin_applied: [f64; N_MOD + 1],
    eff_twin: [f64; N_MOD],
    rest: [f64; N_ARTIC],
    corners: [[f64; N_ARTIC]; 4],
    mods: [Mod; N_MOD],
    mod_phase: [f64; N_MOD],
    mod_hold: [f64; N_MOD],
    mod_from: [f64; N_MOD],
    mod_out: [f64; N_MOD],
    mod_rng: u32,
    eff: [f64; N_MOD],
    applied: [f64; N_MOD],
    seq_pad: Option<(f64, f64)>,
    pwm_phase: f64,
    g2p: Option<EnglishG2P>,
    plan: TextPlan,
    cursor: usize,
    fired: usize,
    next_tick: f64,
    next_pass: f64,
    once_pending: bool,
    restart_for: Option<u64>,
    seen_cycles: u64,
    last_restart: f64,
    open_unit: Option<(usize, f64)>,
    cut: (f64, f64),
    pass_cut: (f64, f64),
    gate_eff: bool,
    said: u64,
    last_unit: usize,
    next_tag: f64,
    prev_gate: bool,
    retrig: bool,
    fresh: bool,
    pos: u64,
    env_peak: f64,
    seq: Seq,
    note_now: f64,
    gate_now: bool,
    folds_f0: f64,
    accent: bool,
    fold_env: f64,
    in_env: f64,
    in_peak: f64,
    env_open: bool,
    env_below: f64,
    tracker: PitchTracker,
    track_count: u32,
    steady: Option<f64>,
    anchor: Option<f64>,
    candidate: Option<(f64, u64)>,
    grid_line: i64,
    grid_held: Option<(f64, [f64; N_MOD])>,
    breath_env: f64,
    src_gate: f64,
    amp: Vec<f32>,
    song: Option<(f64, u64)>,
    text_line: i64,
    text_pending: (bool, bool),
    text_crossed: bool,
    grid_eff: [f64; N_MOD],
    vowels: VowelTracker,
    in_pad: Option<(f64, f64)>,
    adsr: f64,
    stage: Stage,
    adsr_step: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

const TRACK_EVERY: u32 = 8;

impl Suu {
    pub fn new(fs: f64) -> Self {
        chimera::phonetics::planner::init();
        let mut s = Suu {
            fs,
            voice: Box::new(HumanVoice::new(fs, BODY_MALE, oversample_for(fs, 1.0, Quality::Hq))),
            fade: None,
            retired: None,
            scale: 1.0,
            quality: Quality::Hq,
            noise_base: NoiseGains { glottis: 0.0, constriction: 0.0, teeth: 0.0, glottis_lowpass: 0.0, glottis_highpass: 0.0 },
            grain: 1.0,
            grain_from: 1.0,
            expr: (0.0, 0.0),
            level: 1.0,
            level_from: 1.0,
            p: Params::default(),
            osc: Osc::new(),
            pitch: 45.0,
            glide_to: 45.0,
            glide_rate: 0.0,
            osc_hz: midi_to_hz(45.0),
            reson: Reson::new(fs),
            period_peak: 0.0,
            quiet: 0,
            asleep: false,
            snap_ring: vec![0.0; SNAP_RING],
            snap_w: 0,
            osc_env: 1.0,
            tilt_y: 0.0,
            env: 0.0,
            sub1: 0.0,
            sub2: 0.0,
            dry: vec![0.0; MAX_BLOCK],
            drive: vec![0.0; MAX_BLOCK],
            drive_new: vec![0.0; MAX_BLOCK],
            wet: vec![0.0; MAX_BLOCK],
            wet_old: vec![0.0; MAX_BLOCK],
            wet_twin: vec![0.0; MAX_BLOCK],
            out_twin: vec![0.0; MAX_BLOCK],
            twin: None,
            twin_retired: None,
            twin_scale: 0.0,
            twin_mix: 0.0,
            twin_quality: Quality::Hq,
            twin_applied: [f64::NAN; N_MOD + 1],
            eff_twin: [0.0; N_MOD],
            rest: [0.0; N_ARTIC],
            corners: [[0.0; N_ARTIC]; 4],
            mods: [Mod::default(); N_MOD],
            mod_phase: [0.0; N_MOD],
            mod_hold: [0.0; N_MOD],
            mod_from: [0.0; N_MOD],
            mod_out: [0.0; N_MOD],
            mod_rng: 0x9e37_79b9,
            eff: [0.0; N_MOD],
            applied: [f64::NAN; N_MOD],
            seq_pad: None,
            pwm_phase: 0.0,
            g2p: None,
            plan: TextPlan::empty(),
            cursor: 0,
            fired: 0,
            next_tick: 0.0,
            next_pass: 0.0,
            once_pending: false,
            restart_for: None,
            seen_cycles: 0,
            last_restart: 0.0,
            open_unit: None,
            cut: (0.0, 0.0),
            pass_cut: (0.0, 0.0),
            gate_eff: false,
            said: 0,
            last_unit: 0,
            next_tag: 1000.0,
            prev_gate: false,
            retrig: false,
            fresh: false,
            pos: 0,
            env_peak: 0.0,
            seq: Seq::new(),
            note_now: 45.0,
            gate_now: false,
            folds_f0: 0.0,
            accent: false,
            fold_env: 0.0,
            in_env: 0.0,
            in_peak: 0.0,
            env_open: false,
            env_below: 0.0,
            tracker: PitchTracker::new(fs),
            track_count: 0,
            steady: None,
            anchor: None,
            candidate: None,
            grid_line: i64::MIN,
            grid_held: None,
            breath_env: 0.0,
            src_gate: 0.0,
            amp: vec![0.0; MAX_BLOCK],
            song: None,
            text_line: i64::MIN,
            text_pending: (false, false),
            text_crossed: false,
            grid_eff: [0.0; N_MOD],
            vowels: VowelTracker::new(fs),
            in_pad: None,
            adsr: 0.0,
            stage: Stage::Idle,
            adsr_step: [0.0, 1.0, 1.0],
        };
        s.rest = s.voice.artic.target.to_array();
        let mix = BodyMix::new(0.0, 0.0);
        let full = |v: &str, rest: &[f64; N_ARTIC]| {
            let patch = vowel_artic(v, mix).unwrap_or_default();
            core::array::from_fn(|k| patch.values[k].unwrap_or(rest[k]))
        };
        for (i, v) in CORNERS.iter().enumerate() {
            s.corners[i] = full(v, &s.rest);
        }
        s.voice.airway.profile = default_horn().to_vec();
        s.noise_base = s.voice.airway.noise_gain;
        s.configure_voice();
        s.refresh_mouth();
        s.warm_up(&mut |v: &mut HumanVoice, inp, out| v.render_driven(inp, 100.0, out));
        s
    }

    fn warm_up(&mut self, render: &mut dyn FnMut(&mut HumanVoice, &[f32], &mut [f32])) {
        let p = self.p;
        if self.fade.is_none() {
            self.voice.set_source(&SourcePatch { pressure: Some(9.0 * self.breath() * (1.0 - p.follow) * self.breath_env), ..Default::default() });
        }
        let zeros = [0f32; MAX_BLOCK];
        let mut left = (0.25 * self.fs) as usize;
        while left > 0 {
            let n = left.min(MAX_BLOCK);
            render(&mut self.voice, &zeros[..n], &mut self.wet[..n]);
            left -= n;
        }
    }

    pub fn params(&self) -> Params {
        self.p
    }

    pub fn set_params(&mut self, p: Params) {
        let mut q = p;
        q.drive_db = clamp(q.drive_db, -40.0, 40.0);
        q.osc_db = clamp(q.osc_db, -60.0, 12.0);
        q.input_db = clamp(q.input_db, -60.0, 12.0);
        q.tilt_hz = clamp(q.tilt_hz, 0.0, 2000.0);
        q.breath = clamp(q.breath, 0.0, 6.0);
        q.vowel_x = clamp(q.vowel_x, -0.3, 1.3);
        q.vowel_y = clamp(q.vowel_y, -0.3, 1.3);
        q.round = clamp(q.round, -1.0, 1.0);
        q.nasal = clamp(q.nasal, 0.0, 1.5);
        q.folds = clamp(q.folds, 0.0, 2.0);
        for x in [&mut q.follow, &mut q.inject, &mut q.sub_level, &mut q.mix] {
            *x = clamp(*x, 0.0, 1.0);
        }
        q.damping = clamp(q.damping, 0.1, 20.0);
        q.sub_hz = clamp(q.sub_hz, 30.0, 400.0);
        q.out_db = clamp(q.out_db, -40.0, 20.0);
        q.pulse_width = clamp(q.pulse_width, 0.05, 0.95);
        q.pwm_depth = clamp(q.pwm_depth, 0.0, 1.0);
        q.pwm_rate_hz = clamp(q.pwm_rate_hz, 0.01, 40.0);
        q.pwm_sync = clamp(q.pwm_sync, 0.0, 64.0);
        q.seq_vowel = clamp(q.seq_vowel, 0.0, 1.0);
        q.note = clamp(q.note, 12.0, 108.0);
        q.bpm = clamp(q.bpm, 30.0, 300.0);
        q.glide_s = clamp(q.glide_s, 0.0, 4.0);
        q.glide_sync = clamp(q.glide_sync, 0.0, 16.0);
        q.text_rate_hz = clamp(q.text_rate_hz, 0.1, 16.0);
        q.text_sync = clamp(q.text_sync, 0.0, 16.0);
        q.text_len = clamp(q.text_len, 0.05, 1.0);
        q.text_gate = clamp(q.text_gate, 0.05, 1.0);
        q.speech = clamp(q.speech, 0.25, 4.0);
        q.reson = clamp(q.reson, 0.0, 1.0);
        q.morph = clamp(q.morph, 0.0, 1.0);
        q.shift = clamp(q.shift, -0.5, 0.5);
        q.open = clamp(q.open, 0.25, 4.0);
        q.env_thresh_db = clamp(q.env_thresh_db, -80.0, 0.0);
        q.env_release_s = clamp(q.env_release_s, 0.005, 4.0);
        q.env_release_sync = clamp(q.env_release_sync, 0.0, 16.0);
        q.attack_s = clamp(q.attack_s, 0.0005, 10.0);
        q.decay_s = clamp(q.decay_s, 0.001, 10.0);
        q.release_s = clamp(q.release_s, 0.001, 10.0);
        q.sustain = clamp(q.sustain, 0.0, 1.0);
        q.coarse = clamp(q.coarse.round(), -48.0, 48.0);
        q.steady_s = clamp(q.steady_s, 0.0, 4.0);
        q.steady_sync = clamp(q.steady_sync, 0.0, 16.0);
        q.grid = clamp(q.grid, 0.0, 16.0);
        q.scale &= 0xfff;
        q.jump = clamp(q.jump.round(), 0.0, 48.0);
        q.text_grid = if q.text_grid < 0.0 { -1.0 } else { clamp(q.text_grid, 0.0, 16.0) };
        q.grid_breath = clamp(q.grid_breath, 0.0, 1.0);
        q.grid_smooth = clamp(q.grid_smooth, 0.0, 1.0);
        q.text_round = clamp(q.text_round, 0.0, 1.0);
        q.aspiration = clamp(q.aspiration, 0.0, AIR_MAX);
        q.frication = clamp(q.frication, 0.0, AIR_MAX);
        q.sibilance = clamp(q.sibilance, 0.0, AIR_MAX);
        q.whistle = clamp(q.whistle, 0.0, 1.0);
        q.move_sound = clamp(q.move_sound, 0.0, MOVE_MAX);
        for t in [&mut q.pressure_to, &mut q.slide_to] {
            *t = if (0..=EXPR_LEVEL).contains(t) { *t } else { -1 };
        }
        q.pressure_depth = clamp(q.pressure_depth, -1.0, 1.0);
        q.slide_depth = clamp(q.slide_depth, -1.0, 1.0);
        q.stereo_spread = clamp(q.stereo_spread, 0.7, 1.4);
        q.stereo_vowel = clamp(q.stereo_vowel, -0.5, 0.5);
        q.stereo_width = clamp(q.stereo_width, 0.0, 1.0);
        q.stereo_canon = clamp(q.stereo_canon.round(), 0.0, 64.0);
        q.stereo_phase = clamp(q.stereo_phase, 0.0, 1.0);
        for x in [&mut q.attack_sync, &mut q.decay_sync, &mut q.release_sync] {
            *x = clamp(*x, 0.0, 16.0);
        }
        self.p = q;
        self.configure_voice();
        self.refresh_mouth();
    }

    pub fn set_size(&mut self, scale: f64) {
        let s = clamp(scale, 0.4, 4.0);
        if s == self.scale && self.p.quality() == self.quality {
            return;
        }
        let m = Mouth::build(&self.mouth_seed(s));
        (self.fade, self.retired) = (None, None);
        let _ = self.set_mouth(m);
    }

    pub fn mouth_seed(&self, scale: f64) -> MouthSeed {
        let aw = &self.voice.airway;
        let mut profile = [0.0; HORN_POINTS];
        for (d, &a) in profile.iter_mut().zip(&aw.profile) {
            *d = a;
        }
        MouthSeed {
            fs: self.fs,
            scale: clamp(scale, 0.4, 4.0),
            artic: self.voice.artic.clone(),
            profile,
            wrap: aw.profile_wrap,
            horn: (aw.profile_mix, aw.profile_shift, aw.profile_open),
            inject: self.eff[4],
            damping: self.eff[5],
            folds: self.voice.glottal_area_scale,
            pressure: self.voice.source.pressure,
            quality: self.p.quality(),
        }
    }

    pub fn set_mouth(&mut self, m: Mouth) -> Result<(), Mouth> {
        if self.fade.is_some() || self.retired.is_some() {
            return Err(m);
        }
        let Mouth { voice: mut nv, scale, quality } = m;
        (self.scale, self.quality) = (scale, quality);
        nv.artic = self.voice.artic.clone();
        nv.airway.profile.copy_from_slice(&self.voice.airway.profile);
        nv.airway.profile_wrap = self.voice.airway.profile_wrap;
        nv.score.copy_from(&self.voice.score);
        nv.clock = self.voice.clock;
        let x = nv.artic.x;
        nv.airway.apply(&x, 0.0);
        nv.airway.net.snap_to_targets();
        let old = core::mem::replace(&mut self.voice, nv);
        let pressure = old.source.pressure;
        self.fade = Some((old, FADE));
        self.folds_f0 = 0.0;
        self.applied = [f64::NAN; N_MOD];
        self.configure_voice();
        self.refresh_mouth();
        self.voice.set_source(&SourcePatch { pressure: Some(pressure), ..Default::default() });
        Ok(())
    }

    pub fn take_retired(&mut self) -> Option<Mouth> {
        self.retired.take().or_else(|| self.twin_retired.take()).map(|voice| Mouth { voice, scale: 0.0, quality: Quality::Hq })
    }

    pub fn wants_twin(&self) -> Option<f64> {
        let want = self.scale * self.p.stereo_spread;
        let differs = self.twin.is_none() || (self.twin_scale - clamp(want, 0.4, 4.0)).abs() > 1e-9 || self.twin_quality != self.quality;
        (self.p.stereo && differs && self.twin_retired.is_none()).then_some(want)
    }

    pub fn set_twin(&mut self, m: Mouth) -> Result<(), Mouth> {
        if self.twin_retired.is_some() {
            return Err(m);
        }
        let Mouth { voice: mut nv, scale, quality } = m;
        nv.airway.profile.copy_from_slice(&self.voice.airway.profile);
        nv.airway.profile_wrap = self.voice.airway.profile_wrap;
        nv.score.copy_from(&self.voice.score);
        nv.clock = self.voice.clock;
        nv.artic = self.voice.artic.clone();
        let x = nv.artic.x;
        nv.airway.apply(&x, 0.0);
        nv.airway.net.snap_to_targets();
        nv.set_source(&SourcePatch { voiced: Some(true), tension: Some(0.55), drift: Some(0.0), glide: Some(0.002), pressure: Some(self.voice.source.pressure), f0: Some(self.voice.source.f0), ..Default::default() });
        nv.set_gate(1.0);
        self.twin_applied = [f64::NAN; N_MOD + 1];
        (self.twin_scale, self.twin_quality) = (scale, quality);
        if let Some(old) = self.twin.replace(nv) {
            self.twin_retired = Some(old);
            self.twin_mix = 0.0;
        }
        self.mirror_twin(false);
        Ok(())
    }

    pub fn stereo_active(&self) -> bool {
        self.twin.is_some()
    }

    fn mirror_twin(&mut self, woke: bool) {
        if !self.p.stereo && self.twin_mix == 0.0 && self.twin_retired.is_none() {
            self.twin_retired = self.twin.take();
        }
        let e = if self.p.stereo_phase > 0.0 && self.p.grid <= 0.0 { self.eff_twin } else { self.eff };
        let dx = self.p.stereo_vowel;
        let patch = self.artic_patch(&e, dx);
        let (canon, mirror) = (self.p.stereo_canon > 0.0, self.p.stereo_mirror);
        let Some(t) = self.twin.as_mut() else { return };
        let v = &self.voice;
        if !canon {
            t.score.copy_from(&v.score);
        }
        t.clock = v.clock;
        t.hold = v.hold;
        t.artic.speed = v.artic.speed;
        t.glottal_area_scale = v.glottal_area_scale;
        t.airway.noise_gain = v.airway.noise_gain;
        t.airway.narrow_loss = v.airway.narrow_loss;
        t.airway.net.volume_source_gain = v.airway.net.volume_source_gain;
        if t.source.pressure != v.source.pressure || t.source.f0 != v.source.f0 {
            t.set_source(&SourcePatch { pressure: Some(v.source.pressure), f0: Some(v.source.f0), ..Default::default() });
        }
        if e[4] != self.twin_applied[4] {
            t.set_injection(e[4]);
        }
        t.airway.net.viscous_scale = e[5];
        let n = v.airway.profile.len();
        for i in 0..n {
            t.airway.profile[i] = v.airway.profile[if mirror { n - 1 - i } else { i }];
        }
        t.airway.profile_wrap = v.airway.profile_wrap;
        (t.airway.profile_mix, t.airway.profile_shift, t.airway.profile_open) = (e[6], e[7], e[8]);
        let shape_changed = (0..4).any(|i| (e[i] - self.twin_applied[i]).abs() > 1e-6 || self.twin_applied[i].is_nan()) || dx != self.twin_applied[N_MOD];
        if shape_changed {
            t.set_articulation(&patch, None);
        }
        if woke {
            let a = &mut t.artic;
            a.x = a.target;
            a.v = [0.0; N_ARTIC];
            let x = a.x;
            t.airway.apply(&x, 0.0);
            t.airway.net.snap_to_targets();
        }
        self.twin_applied[..N_MOD].copy_from_slice(&e);
        self.twin_applied[N_MOD] = dx;
    }

    pub fn set_profile(&mut self, areas: &[f64], wrap: bool) {
        self.voice.airway.profile_wrap = wrap;
        let n = HORN_POINTS.min(areas.len());
        for (d, &a) in self.voice.airway.profile.iter_mut().zip(&areas[..n]) {
            *d = clamp(a, 0.0, 20.0);
        }
    }

    pub fn snapshot(&self, out: &mut [f64; HORN_POINTS]) -> bool {
        let len = clamp(self.fs / self.osc_hz, HORN_POINTS as f64, (SNAP_RING - 1) as f64) as usize;
        let start = self.snap_w + SNAP_RING - len;
        for (k, o) in out.iter_mut().enumerate() {
            let (a, b) = (k * len / HORN_POINTS, ((k + 1) * len / HORN_POINTS).max(k * len / HORN_POINTS + 1));
            *o = (a..b).map(|j| self.snap_ring[(start + j) & (SNAP_RING - 1)] as f64).sum::<f64>() / (b - a) as f64;
        }
        let mean = out.iter().sum::<f64>() / HORN_POINTS as f64;
        let peak = out.iter().fold(0.0f64, |m, v| m.max((v - mean).abs()));
        if peak < 1e-6 {
            return false;
        }
        for o in out.iter_mut() {
            *o = (*o - mean) / peak;
        }
        true
    }

    pub fn profile(&self) -> &[f64] {
        &self.voice.airway.profile
    }

    pub fn size(&self) -> f64 {
        self.scale
    }

    pub fn quality(&self) -> Quality {
        self.quality
    }

    pub fn hq(&self) -> bool {
        self.quality == Quality::Hq
    }

    fn configure_voice(&mut self) {
        let p = self.p;
        self.voice.glottal_area_scale = p.folds * self.fold_env;
        self.voice.set_source(&SourcePatch { voiced: Some(true), tension: Some(0.55), drift: Some(0.0), glide: Some(0.002), ..Default::default() });
        self.voice.set_gate(1.0);
        self.folds_f0 = 0.0;
    }

    // ------------------------------------------------------------ the mouth and Motion

    pub fn set_mod(&mut self, target: usize, m: Mod) {
        if target < N_MOD {
            self.mods[target] = Mod { depth: clamp(m.depth, -1.0, 1.0), rate_hz: clamp(m.rate_hz, 0.01, 40.0), sync: clamp(m.sync, 0.0, 64.0), smooth: clamp(m.smooth, 0.0, 1.0), ..m };
        }
    }

    pub fn get_mod(&self, target: usize) -> Mod {
        self.mods[target.min(N_MOD - 1)]
    }

    fn bases(&self) -> [f64; N_MOD] {
        let p = self.p;
        let pad = if p.vowel_in { self.in_pad } else { None };
        let (bx, by) = pad.unwrap_or((p.vowel_x, p.vowel_y));
        let (x, y) = match self.seq_pad {
            Some((sx, sy)) => (bx + p.seq_vowel * (sx - bx), by + p.seq_vowel * (sy - by)),
            None => (bx, by),
        };
        [x, y, p.round, p.nasal, p.inject, p.damping, p.morph, p.shift, p.open]
    }

    fn run_mods(&mut self, envn: f64) {
        let bases = self.bases();
        let beats = self.beats_now();
        for i in 0..N_MOD {
            let m = self.mods[i];
            let (lo, hi) = MOD_RANGE[i];
            if m.depth == 0.0 {
                self.eff[i] = bases[i];
                self.eff_twin[i] = bases[i];
                continue;
            }
            let old = self.mod_phase[i];
            let ph = if m.sync > 0.0 { (beats / m.sync).fract() } else { (old + m.rate_hz * CTRL as f64 / self.fs).fract() };
            if ph < old || self.mod_hold[i] == 0.0 {
                self.mod_from[i] = self.mod_out[i];
                let r = &mut self.mod_rng;
                *r ^= *r << 13;
                *r ^= *r >> 17;
                *r ^= *r << 5;
                self.mod_hold[i] = (*r as f64 / u32::MAX as f64) * 2.0 - 1.0 + 1e-9;
            }
            self.mod_phase[i] = ph;
            let ease = |x: f64| 0.5 - 0.5 * cos(PI * clamp(x, 0.0, 1.0));
            let sm = m.smooth;
            let v = periodic(m.shape, ph, sm).unwrap_or_else(|| match m.shape {
                Shape::Random => {
                    let (a, b) = (self.mod_from[i], self.mod_hold[i]);
                    if sm > 0.0 { a + (b - a) * ease(ph / sm) } else { b }
                }
                Shape::Follow => envn,
                _ => self.adsr,
            });
            self.mod_out[i] = v;
            let at = |v: f64| if MOD_LOG[i] { clamp(bases[i] * pow(hi / lo, 0.5 * m.depth * v), lo, hi) } else { clamp(bases[i] + 0.5 * m.depth * v * (hi - lo), lo, hi) };
            self.eff[i] = at(v);
            self.eff_twin[i] = if self.p.stereo_phase > 0.0 { at(periodic(m.shape, (ph + self.p.stereo_phase).fract(), sm).unwrap_or(v)) } else { self.eff[i] };
        }
        for (to, amount) in self.expr_routes() {
            if (0..N_MOD as i32).contains(&to) && amount != 0.0 {
                let i = to as usize;
                let (lo, hi) = MOD_RANGE[i];
                let add = |x: f64| if MOD_LOG[i] { clamp(x * pow(hi / lo, amount), lo, hi) } else { clamp(x + amount * (hi - lo), lo, hi) };
                self.eff[i] = add(self.eff[i]);
                self.eff_twin[i] = add(self.eff_twin[i]);
            }
        }
    }

    fn expr_routes(&self) -> [(i32, f64); 2] {
        let p = self.p;
        [(p.pressure_to, p.pressure_depth * self.expr.0), (p.slide_to, p.slide_depth * self.expr.1)]
    }

    fn expr_amount(&self, to: i32) -> f64 {
        self.expr_routes().iter().filter(|r| r.0 == to).map(|r| r.1).sum()
    }

    fn expr_level(&self) -> f64 {
        let p = self.p;
        let mut g = 1.0;
        for (to, d, v) in [(p.pressure_to, p.pressure_depth, self.expr.0), (p.slide_to, p.slide_depth, 0.5 + 0.5 * self.expr.1)] {
            if to == EXPR_LEVEL {
                g *= if d >= 0.0 { 1.0 - d + d * v } else { 1.0 + d * v };
            }
        }
        g
    }

    fn breath(&self) -> f64 {
        clamp(self.p.breath + 6.0 * self.expr_amount(EXPR_BREATH), 0.0, 6.0)
    }

    pub fn set_expression(&mut self, pressure: f64, slide: f64) {
        self.expr = (clamp(pressure, 0.0, 1.0), clamp(slide, -1.0, 1.0));
    }

    fn apply_mouth(&mut self) {
        let e = self.eff;
        if e[4] != self.applied[4] {
            self.voice.set_injection(e[4]);
        }
        if e[5] != self.applied[5] {
            self.voice.airway.net.viscous_scale = e[5];
        }
        let aw = &mut self.voice.airway;
        (aw.profile_mix, aw.profile_shift, aw.profile_open) = (e[6], e[7], e[8]);
        let shape_changed = (0..4).any(|i| (e[i] - self.applied[i]).abs() > 1e-6 || self.applied[i].is_nan());
        self.applied = e;
        if !shape_changed {
            return;
        }
        let patch = self.artic_patch(&e, 0.0);
        self.voice.set_articulation(&patch, None);
    }

    fn artic_patch(&self, e: &[f64; N_MOD], dx: f64) -> ArticPatch {
        let (x, y, round, nasal) = (e[0] + dx, e[1], e[2], e[3]);
        let w = [(1.0 - x) * (1.0 - y), x * (1.0 - y), (1.0 - x) * y, x * y];
        let mut a: [f64; N_ARTIC] = core::array::from_fn(|k| (0..4).fold(0.0, |s, i| s + w[i] * self.corners[i][k]));
        let (pi, ai, vi) = (ArticKey::LipProt.index(), ArticKey::LipAp.index(), ArticKey::Velum.index());
        if round >= 0.0 {
            a[pi] += round * (1.0 - a[pi]);
        } else {
            a[pi] *= 1.0 + round;
        }
        a[ai] *= 1.0 - 0.6 * round;
        a[vi] = js_max(a[vi], -0.15 + 0.6 * nasal);
        let mut patch = ArticPatch::default();
        for k in 0..N_ARTIC {
            patch.values[k] = Some(a[k]);
        }
        patch
    }

    fn refresh_mouth(&mut self) {
        let b = self.bases();
        for i in 0..N_MOD {
            if self.mods[i].depth == 0.0 {
                self.eff[i] = b[i];
            }
        }
        if self.p.grid > 0.0
            && let Some((_, held)) = self.grid_held
        {
            self.eff = held;
        }
        self.applied = [f64::NAN; N_MOD];
        self.apply_mouth();
    }

    pub fn ui_state(&self, out: &mut Vec<f32>) {
        out.clear();
        out.extend(self.eff[..6].iter().map(|&v| v as f32));
        let b = self.bases();
        out.push(b[0] as f32);
        out.push(b[1] as f32);
        let a = &self.voice.artic.x;
        for k in [ArticKey::LipAp, ArticKey::LipWidth, ArticKey::LipProt, ArticKey::Jaw, ArticKey::TbX, ArticKey::TbY, ArticKey::Velum] {
            out.push(a.get(k) as f32);
        }
        out.push(self.voice.airway.frame.as_ref().map_or(0.0, |f| f.lip_area) as f32);
        out.push(self.env as f32);
        out.push(self.seq.step as f32);
        out.push(self.gate_now as u32 as f32);
        out.push(self.seq.playing as u32 as f32);
        out.extend(self.eff[6..9].iter().map(|&v| v as f32));
        let aw = &self.voice.airway;
        let total = aw.ph_len + aw.or_len;
        for k in 0..UI_AREAS {
            let s = (k as f64 + 0.5) / UI_AREAS as f64 * total;
            let a = if s < aw.ph_len {
                aw.ph_areas[((s / aw.ph_len * aw.n_ph as f64) as usize).min(aw.n_ph - 1)]
            } else {
                aw.or_areas[(((s - aw.ph_len) / aw.or_len * aw.n_or as f64) as usize).min(aw.n_or - 1)]
            };
            out.push(a as f32);
        }
        let tracked = match self.p.pitch_from {
            PitchFrom::Note => None,
            PitchFrom::Input => self.steady,
            PitchFrom::InputSnap => self.steady.map(|n| snap_to_scale(n, self.p.scale)),
        };
        let tracked = tracked.map(|n| clamp(n + self.p.coarse, 12.0, 108.0));
        out.push(tracked.map_or(0.0, |n| n as f32));
    }

    // ------------------------------------------------------------ sequencer

    pub fn set_step(&mut self, i: usize, step: Step) {
        if i < STEPS {
            self.seq.steps[i] = Step { note: step.note.clamp(-36, 36), vowel: step.vowel.clamp(-1, STEP_VOWELS.len() as i32 - 1), len: clamp(step.len, 0.05, 1.0), ..step };
        }
    }

    pub fn step(&self, i: usize) -> Step {
        self.seq.steps[i.min(STEPS - 1)]
    }

    pub fn set_seq(&mut self, len: usize, div: f64, gate_scale: f64) {
        let s = &mut self.seq;
        s.len = len.clamp(1, STEPS);
        s.div = clamp(div, 0.25, 32.0);
        s.gate_scale = clamp(gate_scale, 0.0, 2.0);
    }

    pub fn set_playing(&mut self, on: bool) {
        if on == self.seq.playing {
            return;
        }
        self.seq.playing = on;
        self.seq.t = 0.0;
        self.seq.step = 0;
        self.seq.cycles = 0;
        self.seen_cycles = 0;
        if on {
            self.fired = 0;
            self.once_pending = true;
            if self.p.text_seq && !self.plan.text.is_empty() {
                self.cursor = 0;
                if self.p.text_step == TextStep::Clock {
                    let now = self.voice.clock;
                    self.score_release_all(now);
                    (self.next_tick, self.next_pass) = (now, now);
                    self.restart_for = Some(0);
                }
            }
            self.trigger();
        } else {
            self.seq.gate_open = false;
            self.seq_pad = None;
            self.refresh_mouth();
        }
    }

    pub fn playing(&self) -> bool {
        self.seq.playing
    }

    pub fn retrigger(&mut self) {
        self.retrig = true;
    }

    pub fn text_part(&self) -> usize {
        self.last_unit
    }

    pub fn restart_text(&mut self) {
        if self.plan.text.is_empty() {
            return;
        }
        let now = self.voice.clock;
        self.score_release_all(now);
        self.open_unit = None;
        (self.cursor, self.fired, self.once_pending) = (0, 0, true);
        (self.next_tick, self.next_pass) = (now, now);
        self.text_pending = (false, false);
    }

    pub fn note_start(&mut self) {
        self.retrig = true;
        self.fresh = true;
    }

    pub fn seq_state(&self) -> (usize, bool, f64) {
        (self.seq.step, self.gate_now, self.note_now)
    }

    fn step_samples(&self) -> f64 {
        self.fs * 60.0 / self.p.bpm / self.seq.div
    }

    fn trigger(&mut self) {
        let st = self.seq.steps[self.seq.step];
        if st.on && self.gate_len() > 0.0 {
            self.seq.note = st.note as f64;
            let prev = self.seq.steps[(self.seq.step + self.seq.len - 1) % self.seq.len];
            self.retrig = !(prev.tie && prev.on && self.seq.gate_open);
            self.seq.gate_open = true;
            self.accent = st.accent;
            if st.vowel >= 0 {
                self.seq_pad = Some(STEP_PAD[st.vowel as usize]);
            }
        } else {
            self.seq.gate_open = false;
        }
    }

    fn gate_len(&self) -> f64 {
        clamp(self.seq.steps[self.seq.step].len * self.seq.gate_scale, 0.0, 1.0)
    }

    fn advance_seq(&mut self) {
        let len = self.step_samples();
        self.seq.t += CTRL as f64;
        if self.seq.t >= len {
            self.seq.t -= len;
            self.seq.step = (self.seq.step + 1) % self.seq.len;
            if self.seq.step == 0 {
                self.seq.cycles += 1;
            }
            self.trigger();
        } else if self.seq.gate_open && self.seq.t >= self.gate_len() * len {
            if !self.seq.steps[self.seq.step].tie {
                self.seq.gate_open = false;
            }
        }
    }

    // ------------------------------------------------------------ text

    pub fn load_dict(&mut self, text: &str) {
        let mut g = EnglishG2P::new();
        g.load_dict(text);
        self.g2p = Some(g);
    }

    pub fn g2p(&self) -> Option<&EnglishG2P> {
        self.g2p.as_ref()
    }

    pub fn set_text(&mut self, text: &str) -> usize {
        let plan = TextPlan::new(text, self.g2p.as_ref(), &self.p);
        drop(self.set_text_plan(plan));
        self.plan.text.len()
    }

    pub fn set_text_plan(&mut self, plan: TextPlan) -> TextPlan {
        self.score_cancel();
        let old = core::mem::replace(&mut self.plan, plan);
        self.open_unit = None;
        if self.plan.text.is_empty() {
            self.cursor = 0;
        }
        self.fired = 0;
        self.once_pending = true;
        self.restart_for = None;
        let start = self.on_text_line(self.voice.clock + 0.1);
        (self.next_tick, self.next_pass) = (start, start);
        self.applied = [f64::NAN; N_MOD];
        self.refresh_mouth();
        old
    }

    pub fn text_stale(&self) -> bool {
        !self.plan.text.is_empty() && self.plan.key != PlanKey::of(&self.p)
    }

    pub fn syllables(&self) -> usize {
        self.plan.text.len()
    }

    #[cfg(test)]
    fn loop_beats(&self) -> f64 {
        loop_beats(&self.p)
    }

    fn on_clock(&self) -> bool {
        !self.plan.text.is_empty() && self.p.text_step == TextStep::Clock
    }

    fn joined(&self) -> bool {
        self.p.text_say == TextSay::Hold && self.p.text_len >= 0.95
    }

    fn schedule_at(&mut self, which: Plan, at: f64, tag: Option<f64>) {
        let src = match which {
            Plan::HoldOn(u) => &self.plan.hold_on[u],
            Plan::HoldOff(u) => &self.plan.hold_off[u],
            Plan::Shot(u) => &self.plan.shot[u],
            Plan::Tick(u) => &self.plan.tick_plan[u],
            Plan::Pass => &self.plan.pass_plan,
        };
        self.plan.scratch.clear();
        self.plan.scratch.extend(src.iter().map(|g| {
            let mut g = g.clone();
            g.t0 += at;
            g.t1 += at;
            if tag.is_some() {
                g.tag = tag;
            }
            g
        }));
        self.voice.schedule(&self.plan.scratch);
        let k = self.p.stereo_canon as usize;
        if k == 0 || self.twin.is_none() {
            return;
        }
        let n = self.plan.hold_on.len().max(1);
        let back = |u: usize| (u + n - k % n) % n;
        let (src, at) = match which {
            Plan::HoldOn(u) => (&self.plan.hold_on[back(u)], at),
            Plan::HoldOff(u) => (&self.plan.hold_off[back(u)], at),
            Plan::Shot(u) => (&self.plan.shot[back(u)], at),
            Plan::Tick(u) => (&self.plan.tick_plan[back(u)], at),
            Plan::Pass => (&self.plan.pass_plan, at + k as f64 * self.plan.tick_s),
        };
        self.plan.scratch.clear();
        self.plan.scratch.extend(src.iter().map(|g| {
            let mut g = g.clone();
            g.t0 += at;
            g.t1 += at;
            if tag.is_some() {
                g.tag = tag;
            }
            g
        }));
        if let Some(t) = self.twin.as_mut() {
            t.schedule(&self.plan.scratch);
        }
    }

    fn score_release_all(&mut self, t: f64) {
        self.voice.release_all(t);
        if let Some(tw) = self.twin.as_mut() {
            tw.release_all(t);
        }
    }

    fn score_release(&mut self, tag: f64) {
        self.voice.release(tag);
        if let Some(tw) = self.twin.as_mut() {
            tw.release(tag);
        }
    }

    fn score_cancel(&mut self) {
        self.voice.cancel_score();
        if let Some(tw) = self.twin.as_mut() {
            tw.cancel_score();
        }
    }

    fn next_unit(&mut self) -> Option<usize> {
        let n = self.plan.hold_on.len();
        if n == 0 || (self.p.text_once && self.fired >= n) {
            return None;
        }
        let u = self.cursor % n;
        self.cursor = u + 1;
        self.fired += 1;
        self.said += 1;
        self.last_unit = u;
        Some(u)
    }

    fn close_unit(&mut self) {
        if let Some((u, tag)) = self.open_unit.take() {
            let now = self.voice.clock;
            self.score_release(tag);
            self.schedule_at(Plan::HoldOff(u), now, None);
        }
    }

    fn text_note(&mut self, on: bool) {
        let now = self.voice.clock;
        match self.p.text_say {
            TextSay::Hold => {
                self.close_unit();
                if on && let Some(u) = self.next_unit() {
                    self.cut = (now, now + self.p.text_gate * self.plan.shot_len[u]);
                    self.next_tag += 1.0;
                    let tag = self.next_tag;
                    self.schedule_at(Plan::HoldOn(u), now, Some(tag));
                    self.open_unit = Some((u, tag));
                }
            }
            TextSay::Shot => {
                if on && let Some(u) = self.next_unit() {
                    self.cut = (now, now + self.p.text_gate * self.plan.shot_len[u]);
                    let at = now + self.plan.shot_lead;
                    self.schedule_at(Plan::Shot(u), at, None);
                }
            }
        }
    }

    fn run_text(&mut self) {
        if self.plan.text.is_empty() {
            return;
        }
        let p = self.p;
        let now = self.voice.clock;
        if self.seq.playing && self.seq.cycles != self.seen_cycles {
            self.seen_cycles = self.seq.cycles;
            if !(p.text_step == TextStep::Clock && p.text_seq) {
                self.fired = 0;
            }
            if p.text_step == TextStep::Note && p.text_seq {
                self.cursor = 0;
            }
            if p.text_step == TextStep::Clock && !p.text_seq {
                self.once_pending = true;
                self.next_pass = js_max(self.next_pass, now);
            }
        }
        if p.text_step == TextStep::Note {
            let rising = self.gate_now && (!self.prev_gate || self.retrig);
            let falling = self.prev_gate && !self.gate_now;
            if self.text_grid_beats() <= 0.0 {
                self.text_pending = (false, false);
                if rising {
                    self.text_note(true);
                } else if falling {
                    self.text_note(false);
                }
                return;
            }
            if rising {
                self.text_pending = (true, false);
            } else if falling {
                self.text_pending.1 = true;
            }
            if self.text_crossed {
                if self.text_pending.0 {
                    self.text_pending.0 = false;
                    self.text_note(true);
                } else if self.text_pending.1 {
                    self.text_pending.1 = false;
                    self.text_note(false);
                }
            }
            return;
        }
        let joined = self.joined();
        let lead = if joined { self.plan.pass_lead } else if p.text_say == TextSay::Hold { self.plan.tick_lead } else { self.plan.shot_lead };
        if p.text_seq && self.seq.playing {
            let to_wrap = ((self.seq.len - self.seq.step) as f64 * self.step_samples() - self.seq.t) / self.fs;
            let next = self.seq.cycles + 1;
            if to_wrap <= lead + 0.05 && self.restart_for != Some(next) {
                let at = now + to_wrap;
                self.score_release_all(at);
                (self.next_tick, self.next_pass) = (at, at);
                self.cursor = 0;
                self.fired = 0;
                self.once_pending = true;
                self.restart_for = Some(next);
                self.last_restart = at;
            }
        }
        if joined {
            if now + 0.5 + lead >= self.next_pass && (!p.text_once || self.once_pending) {
                let at = self.next_pass;
                self.pass_cut = if (at - self.pass_cut.1).abs() < 1e-9 { (self.pass_cut.0, at + self.plan.pass_len) } else { (at, at + self.plan.pass_len) };
                self.schedule_at(Plan::Pass, at, None);
                self.next_pass += self.plan.pass_len;
                self.once_pending = false;
            }
            return;
        }
        if self.next_tick < now - self.plan.tick_s {
            self.next_tick = self.on_text_line(now + lead);
        }
        let horizon = now + 2.0 * CTRL as f64 / self.fs;
        while self.next_tick - lead <= horizon {
            let at = self.next_tick;
            if let Some(u) = self.next_unit() {
                let from = at - lead.min(0.5 * self.plan.tick_s);
                let len = if !p.text_fit {
                    self.plan.tick_s
                } else if p.text_say == TextSay::Shot {
                    self.plan.shot_len[u].min(self.plan.tick_s)
                } else {
                    p.text_len * self.plan.tick_s
                };
                self.cut = (from, from + p.text_gate * len);
                let plan = if p.text_say == TextSay::Hold { Plan::Tick(u) } else { Plan::Shot(u) };
                self.schedule_at(plan, at, None);
            }
            self.next_tick += self.plan.tick_s;
        }
    }

    fn text_window_open(&self) -> bool {
        let p = self.p;
        if self.plan.text.is_empty() || (p.text_gate >= 1.0 && !p.text_fit) {
            return true;
        }
        let now = self.voice.clock;
        if self.on_clock() && self.joined() {
            let tick = self.plan.tick_s;
            let from = self.pass_cut.0 - self.plan.pass_lead.min(0.5 * tick);
            return now >= from && now < self.pass_cut.1 && (now - from).rem_euclid(tick) < p.text_gate * tick;
        }
        now >= self.cut.0 && now < self.cut.1
    }

    fn text_grain(&self, t: f64) -> f64 {
        let p = self.p;
        if p.text_round <= 0.0 || self.plan.text.is_empty() {
            return 1.0;
        }
        let (from, to, held) = if self.on_clock() && self.joined() {
            let tick = self.plan.tick_s;
            let start = self.pass_cut.0 - self.plan.pass_lead.min(0.5 * tick);
            if t < start || t >= self.pass_cut.1 {
                return 0.0;
            }
            let from = start + ((t - start) / tick).floor() * tick;
            (from, from + p.text_gate * tick, false)
        } else {
            let held = p.text_say == TextSay::Hold && !self.on_clock() && p.text_gate >= 1.0 && !p.text_fit;
            (self.cut.0, self.cut.1, held)
        };
        let w = to - from;
        if w <= 0.0 || t < from || (t >= to && !held) {
            return 0.0;
        }
        let f = 0.5 * p.text_round * w;
        let hann = |x: f64| 0.5 - 0.5 * cos(PI * clamp(x, 0.0, 1.0));
        let rise = if t - from < f { hann((t - from) / f) } else { 1.0 };
        let fall = if !held && to - t < f { hann((to - t) / f) } else { 1.0 };
        rise * fall
    }

    fn apply_noise(&mut self) {
        let (p, b) = (self.p, self.noise_base);
        let want = NoiseGains { glottis: b.glottis * p.aspiration, constriction: b.constriction * p.frication, teeth: b.teeth * p.sibilance, ..b };
        let now = self.voice.airway.noise_gain;
        if (want.glottis, want.constriction, want.teeth) != (now.glottis, now.constriction, now.teeth) {
            self.voice.airway.noise_gain = want;
        }
        if self.voice.airway.net.volume_source_gain != p.move_sound {
            self.voice.airway.net.volume_source_gain = p.move_sound;
        }
        let fl = narrow_loss(p.whistle);
        if self.voice.airway.narrow_loss != fl {
            self.voice.airway.narrow_loss = fl;
        }
    }

    // ------------------------------------------------------------ audio

    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        self.process_into(input, out, None);
    }

    pub fn process_stereo(&mut self, input: &[f32], left: &mut [f32], right: &mut [f32]) {
        self.process_into(input, left, Some(right));
    }

    fn process_into(&mut self, input: &[f32], out: &mut [f32], mut right: Option<&mut [f32]>) {
        let mut off = 0;
        while off < out.len() {
            let to_grid = (CTRL - self.pos % CTRL) as usize;
            let n = (out.len() - off).min(MAX_BLOCK).min(to_grid);
            if self.pos % CTRL == 0 {
                self.control();
            }
            self.process_block(&input[off..off + n], off, n, out);
            if let Some(r) = right.as_deref_mut() {
                r[off..off + n].copy_from_slice(&self.out_twin[..n]);
            }
            off += n;
            self.pos += n as u64;
        }
    }

    fn control(&mut self) {
        if self.p.pitch_from != PitchFrom::Note || self.p.vowel_in {
            self.track_count += 1;
            if self.track_count >= TRACK_EVERY {
                self.track_count = 0;
                self.tracker.analyse();
                if self.tracker.voiced()
                    && let Some(n) = self.tracker.note()
                {
                    self.take_pitch(n);
                }
                if self.p.vowel_in && self.tracker.voiced() {
                    self.vowels.analyse();
                }
            }
            if let Some((x, y)) = self.vowels.pad() {
                let k = 1.0 - exp(-(CTRL as f64) / (0.02 * self.fs));
                let (px, py) = self.in_pad.unwrap_or((x, y));
                self.in_pad = Some((px + (x - px) * k, py + (y - py) * k));
            }
        }
        if self.seq.playing {
            self.advance_seq();
            self.note_now = self.base_note() + self.seq.note;
            self.gate_now = match self.p.seq_gate {
                SeqGate::Seq => self.seq.gate_open,
                SeqGate::AndIn => {
                    self.run_env_gate();
                    self.seq.gate_open && self.env_open
                }
                SeqGate::OrIn => {
                    self.run_env_gate();
                    self.seq.gate_open || self.env_open
                }
            };
        } else {
            self.note_now = self.base_note();
            self.run_env_gate();
            self.gate_now = self.p.gate || self.env_open;
        }
        let grid_take = self.grid_take();
        let tg = self.text_grid_beats();
        self.text_crossed = false;
        if tg > 0.0 {
            let line = (self.beats_now() / tg).floor() as i64;
            self.text_crossed = line != self.text_line && self.text_line != i64::MIN;
            self.text_line = line;
        } else {
            self.text_line = i64::MIN;
        }
        {
            let p = self.p;
            let h = &mut self.voice.hold;
            h.freeze = false;
            h.on = tg > 0.0;
            h.take |= h.on && self.text_crossed;
            h.breath = p.grid_breath;
            h.glide_s = p.grid_smooth * tg * 60.0 / p.bpm / 3.0;
        }
        let played = self.p.pitch_from == PitchFrom::Note && self.gate_now && (!self.prev_gate || self.retrig);
        if let Some((note, shape)) = self.grid_held.filter(|_| !grid_take) {
            if played {
                self.grid_held = Some((self.note_now, shape));
            } else {
                self.note_now = note;
            }
        }
        self.run_text();
        let (retrig, fresh) = (self.retrig, self.fresh);
        (self.retrig, self.fresh) = (false, false);
        self.prev_gate = self.gate_now;
        let p = self.p;
        let was_open = self.gate_eff;
        self.gate_eff = self.gate_now && self.text_window_open();
        self.run_adsr(was_open, retrig, fresh);
        self.grain_from = self.grain;
        let goal = self.text_grain(self.voice.clock + CTRL as f64 / self.fs);
        self.grain = if self.adsr < 1e-4 { goal } else { js_max(goal, self.grain - CTRL as f64 / (GRAIN_MIN_FALL * self.fs)) };
        self.level_from = self.level;
        self.level = self.expr_level();
        self.apply_noise();
        let quiet_period = self.period_peak < QUIET;
        self.period_peak = 0.0;
        self.quiet = if quiet_period { self.quiet + CTRL } else { 0 };
        let goal = self.adsr;
        let tau = if goal > self.fold_env { 0.005 } else { 0.06 };
        self.fold_env += (goal - self.fold_env) * (1.0 - exp(-(CTRL as f64) / (tau * self.fs)));
        let area = p.folds * self.fold_env;
        if area != self.voice.glottal_area_scale {
            self.voice.glottal_area_scale = area;
        }
        let may_sleep = !self.gate_now && !self.on_clock() && self.fade.is_none();
        let asleep = may_sleep && self.quiet as f64 >= SLEEP_AFTER * self.fs;
        if asleep && !self.asleep {
            self.score_cancel();
            self.open_unit = None;
        }
        let woke = self.asleep && !asleep;
        self.asleep = asleep;
        let secs = if p.glide_sync > 0.0 { p.glide_sync * 60.0 / p.bpm } else { p.glide_s };
        let speed = clamp(0.03 / js_max(secs, 0.0075), 0.25, 4.0);
        if speed != self.voice.artic.speed[0] {
            self.voice.artic.speed = [speed; N_ARTIC];
        }
        if self.note_now != self.glide_to {
            self.glide_to = self.note_now;
            let n = js_max(1.0, secs * self.fs);
            let up = self.glide_to > self.pitch;
            let glides = match p.glide_dir {
                GlideDir::Both => true,
                GlideDir::Up => up,
                GlideDir::Down => !up,
            };
            self.glide_rate = (self.glide_to - self.pitch).abs() / if glides { n } else { 1.0 };
        }
        let envn = clamp(self.env_peak * 4.0, 0.0, 1.0);
        self.env_peak = 0.0;
        self.run_mods(envn);
        if self.p.grid > 0.0 {
            if self.grid_held.is_none() {
                self.grid_eff = self.eff;
            }
            match self.grid_held {
                Some((_, held)) if !grid_take => self.eff = held,
                _ => self.grid_held = Some((self.note_now, self.eff)),
            }
            let tau = self.p.grid_smooth * self.p.grid * 60.0 / self.p.bpm / 3.0;
            if tau > 0.0 {
                let k = 1.0 - exp(-(CTRL as f64) / (tau * self.fs));
                for i in 0..N_MOD {
                    self.grid_eff[i] += k * (self.eff[i] - self.grid_eff[i]);
                }
                self.eff = self.grid_eff;
            } else {
                self.grid_eff = self.eff;
            }
        } else {
            self.grid_held = None;
        }
        self.apply_mouth();
        if woke {
            let a = &mut self.voice.artic;
            a.x = a.target;
            a.v = [0.0; N_ARTIC];
            let x = a.x;
            self.voice.airway.apply(&x, 0.0);
            self.voice.airway.net.snap_to_targets();
        }
        let drive_env = if p.folds > 0.0 { js_max(envn, self.fold_env) } else { envn };
        let goal = if self.stage != Stage::Idle || self.on_clock() { 1.0 } else { 0.0 };
        let tau = if goal > self.breath_env { 0.005 } else { 0.06 };
        self.breath_env += (goal - self.breath_env) * (1.0 - exp(-(CTRL as f64) / (tau * self.fs)));
        let pressure = 9.0 * self.breath() * (1.0 - p.follow + p.follow * drive_env) * self.breath_env;
        let f0 = midi_to_hz(self.pitch);
        if (pressure - self.voice.source.pressure).abs() > 1e-6 || (p.folds > 0.0 && f0 != self.folds_f0) {
            self.voice.set_source(&SourcePatch { pressure: Some(pressure), f0: Some(f0), ..Default::default() });
            self.folds_f0 = f0;
        }
        self.mirror_twin(woke);
    }

    fn text_grid_beats(&self) -> f64 {
        if self.p.text_grid < 0.0 { self.p.grid } else { self.p.text_grid }
    }

    fn grid_take(&mut self) -> bool {
        let p = self.p;
        if p.grid <= 0.0 {
            return true;
        }
        let line = (self.beats_now() / p.grid).floor() as i64;
        let crossed = line != self.grid_line;
        self.grid_line = line;
        crossed || self.grid_held.is_none()
    }

    fn on_text_line(&self, t: f64) -> f64 {
        let p = self.p;
        if p.text_sync <= 0.0 {
            return t;
        }
        let beats = self.beats_now() + (t - self.voice.clock) * p.bpm / 60.0;
        t + ((beats / p.text_sync).ceil() * p.text_sync - beats) * 60.0 / p.bpm
    }

    fn beats_now(&self) -> f64 {
        let since = |pos: u64| pos as f64 / self.fs * self.p.bpm / 60.0;
        match self.song {
            Some((beats, at)) => beats + since(self.pos - at),
            None => since(self.pos),
        }
    }

    pub fn set_song_position(&mut self, beats: Option<f64>) {
        self.song = beats.map(|b| (b, self.pos));
    }

    fn take_pitch(&mut self, n: f64) {
        let p = self.p;
        let mut n = n;
        if p.jump > 0.0
            && let Some(cur) = self.steady
            && (n - cur).abs() > p.jump
        {
            if !p.jump_fold {
                return;
            }
            n -= 12.0 * ((n - cur) / 12.0).round();
            if (n - cur).abs() > p.jump {
                return;
            }
        }
        let hold = if p.steady_sync > 0.0 { p.steady_sync * 60.0 / p.bpm } else { p.steady_s };
        let near = if hold > 0.0 { self.anchor } else { self.steady };
        if near.is_some_and(|a| (n - a).abs() < 0.5) {
            self.steady = Some(n);
            self.candidate = None;
            return;
        }
        let cand = match self.candidate {
            Some((c, t0)) if (n - c).abs() < 0.5 => (c, t0),
            _ => (n, self.pos),
        };
        if (self.pos - cand.1) as f64 >= hold * self.fs {
            self.steady = Some(n);
            self.anchor = Some(n);
            self.candidate = None;
        } else {
            self.candidate = Some(cand);
        }
    }

    fn base_note(&self) -> f64 {
        let n = match (self.p.pitch_from, self.steady) {
            (PitchFrom::Input, Some(n)) => n,
            (PitchFrom::InputSnap, Some(n)) => snap_to_scale(n, self.p.scale),
            _ => self.p.note,
        };
        clamp(n + self.p.coarse, 12.0, 108.0)
    }

    pub fn tracked_note(&self) -> Option<f64> {
        self.steady
    }

    fn run_adsr(&mut self, was_open: bool, retrig: bool, fresh: bool) {
        let p = self.p;
        if self.gate_eff && (!was_open || fresh || (retrig && !p.legato)) {
            self.stage = Stage::Attack;
        } else if !self.gate_eff && matches!(self.stage, Stage::Attack | Stage::Decay) {
            self.stage = Stage::Release;
        }
        let secs = |s: f64, sync: f64| if sync > 0.0 { sync * 60.0 / p.bpm } else { s };
        let fs = self.fs;
        let coef = |t: f64| exp(-4.6 / (js_max(t, 0.0005) * fs));
        self.adsr_step = [1.0 / (js_max(secs(p.attack_s, p.attack_sync), 0.0005) * fs), coef(secs(p.decay_s, p.decay_sync)), coef(secs(p.release_s, p.release_sync))];
    }

    pub fn input_vowel(&self) -> (Option<(f64, f64)>, bool) {
        (self.vowels.pad(), self.tracker.voiced())
    }

    pub fn input_formants(&self) -> (f64, f64) {
        self.vowels.formants()
    }

    pub fn adsr_level(&self) -> f64 {
        self.adsr
    }

    fn run_env_gate(&mut self) {
        let p = self.p;
        let peak = self.in_peak;
        self.in_peak = 0.0;
        if !p.env_gate && !(self.seq.playing && p.seq_gate != SeqGate::Seq) {
            (self.env_open, self.env_below) = (false, 0.0);
            return;
        }
        if peak >= pow(10.0, p.env_thresh_db / 20.0) {
            (self.env_open, self.env_below) = (true, 0.0);
        } else if self.env_open {
            self.env_below += CTRL as f64 / self.fs;
            let release = if p.env_release_sync > 0.0 { p.env_release_sync * 60.0 / p.bpm } else { p.env_release_s };
            if self.env_below >= release {
                self.env_open = false;
            }
        }
    }

    pub fn env_open(&self) -> bool {
        self.env_open
    }

    fn process_block(&mut self, input: &[f32], off: usize, n: usize, out: &mut [f32]) {
        let p = self.p;
        let fs = self.fs;
        let env_a = 1.0 - exp(-1.0 / (0.005 * fs));
        let env_r = 1.0 - exp(-1.0 / (0.08 * fs));
        let acc_k = 1.0 - exp(-1.0 / (0.004 * fs));
        let (gate_a, gate_r) = (1.0 - exp(-1.0 / (0.002 * fs)), 1.0 - exp(-1.0 / (0.005 * fs)));
        let level_of = |db: f64| if db <= -60.0 { 0.0 } else { pow(10.0, db / 20.0) };
        let (osc_g, input_g) = (level_of(p.osc_db), level_of(p.input_db));
        let level = if self.seq.playing && self.accent { ACCENT } else { 1.0 };
        let mut env_peak = self.env_peak;
        let (in_a, in_r) = (1.0 - exp(-1.0 / (0.001 * fs)), 1.0 - exp(-1.0 / (0.02 * fs)));
        let mut in_peak = self.in_peak;
        let track = p.pitch_from != PitchFrom::Note || p.vowel_in;
        let vowel = p.vowel_in;
        let period_at = (self.pos % CTRL) as usize;
        let pwm_dt = if p.pwm_sync > 0.0 { p.bpm / 60.0 / p.pwm_sync } else { p.pwm_rate_hz } / fs;
        for i in 0..n {
            if track {
                self.tracker.push(input[i] as f64);
            }
            if vowel {
                self.vowels.push(input[i] as f64);
            }
            let a = (input[i] as f64).abs();
            self.in_env += (a - self.in_env) * if a > self.in_env { in_a } else { in_r };
            in_peak = js_max(in_peak, self.in_env);
            if self.pitch != self.glide_to {
                let d = self.glide_to - self.pitch;
                self.pitch = if d.abs() <= self.glide_rate { self.glide_to } else { self.pitch + self.glide_rate * d.signum() };
                self.osc_hz = midi_to_hz(self.pitch);
            }
            let input_src = p.source == Source::Off;
            let width = if p.pwm_depth > 0.0 {
                self.pwm_phase = (self.pwm_phase + pwm_dt) % 1.0;
                clamp(p.pulse_width + p.pwm_depth * PWM_SWING * sin(2.0 * PI * self.pwm_phase), PWM_MIN, 1.0 - PWM_MIN)
            } else {
                p.pulse_width
            };
            let osc = if input_src { 0.0 } else { self.osc.next(p.source, self.osc_hz / fs, width) * 0.5 * osc_g };
            let raw = osc + input[i] as f64 * input_g;
            self.snap_ring[self.snap_w] = raw as f32;
            self.snap_w = (self.snap_w + 1) & (SNAP_RING - 1);
            let [a_step, d_coef, r_coef] = self.adsr_step;
            match self.stage {
                Stage::Attack => {
                    self.adsr += a_step;
                    if self.adsr >= 1.0 {
                        self.adsr = 1.0;
                        self.stage = Stage::Decay;
                    }
                }
                Stage::Decay => self.adsr = p.sustain + (self.adsr - p.sustain) * d_coef,
                Stage::Release => {
                    self.adsr *= r_coef;
                    if self.adsr < 1e-5 {
                        self.adsr = 0.0;
                        self.stage = Stage::Idle;
                    }
                }
                Stage::Idle => {}
            }
            self.osc_env += (level - self.osc_env) * acc_k;
            let open = if self.stage == Stage::Idle { 0.0 } else { 1.0 };
            self.src_gate += (open - self.src_gate) * if open > self.src_gate { gate_a } else { gate_r };
            let k = (period_at + i + 1) as f64 / CTRL as f64;
            let grain = self.grain_from + (self.grain - self.grain_from) * k;
            let level = self.level_from + (self.level - self.level_from) * k;
            self.amp[i] = (self.adsr * self.osc_env * grain * level) as f32;
            let x = raw * self.src_gate;
            let x = if p.reson > 0.0 {
                let fb = 0.85 + 0.145 * p.reson;
                let r = self.reson.tick(x, fs / self.osc_hz, fb) * pow(1.0 - fb * fb, 0.625);
                x + p.reson * (r - x)
            } else {
                x
            };
            self.dry[i] = x as f32;
            let a = x.abs();
            self.env += (a - self.env) * if a > self.env { env_a } else { env_r };
            env_peak = js_max(env_peak, self.env);
        }
        self.env_peak = env_peak;
        self.in_peak = in_peak;
        let c = if p.tilt_hz > 0.0 { exp(-2.0 * PI * p.tilt_hz / fs) } else { 0.0 };
        for i in 0..n {
            self.tilt_y = c * self.tilt_y + (1.0 - c) * self.dry[i] as f64;
            self.drive[i] = self.tilt_y as f32;
        }
        let scale = INJECT_REF * pow(10.0, p.drive_db / 20.0);
        if self.asleep {
            self.wet[..n].fill(0.0);
        } else if let Some((_, left)) = &self.fade {
            for i in 0..n {
                let k = left.saturating_sub(i as u32) as f64 / FADE as f64;
                self.drive_new[i] = (self.drive[i] as f64 * (0.5 + 0.5 * cos(PI * k))) as f32;
            }
            self.voice.render_driven(&self.drive_new[..n], scale, &mut self.wet[..n]);
        } else {
            self.voice.render_driven(&self.drive[..n], scale, &mut self.wet[..n]);
        }
        if let Some((old, left)) = &mut self.fade {
            old.render_driven(&self.drive[..n], scale, &mut self.wet_old[..n]);
            for i in 0..n {
                let k = left.saturating_sub(i as u32) as f64 / FADE as f64;
                let w = 0.5 - 0.5 * cos(PI * k);
                self.wet[i] = (self.wet[i] as f64 * (1.0 - w) + self.wet_old[i] as f64 * w) as f32;
            }
            *left = left.saturating_sub(n as u32);
            if *left == 0 {
                self.retired = self.fade.take().map(|(v, _)| v);
            }
        }
        if let Some(t) = self.twin.as_mut() {
            if self.asleep {
                self.wet_twin[..n].fill(0.0);
            } else {
                t.render_driven(&self.drive[..n], scale, &mut self.wet_twin[..n]);
            }
        }
        let cs = exp(-2.0 * PI * p.sub_hz / fs);
        let g = pow(10.0, p.out_db / 20.0);
        let twin_on = self.twin.is_some();
        let goal = if p.stereo && twin_on { 1.0 } else { 0.0 };
        let (step, a) = (1.0 / FADE as f64, 0.5 + 0.5 * p.stereo_width);
        for i in 0..n {
            let d = self.dry[i] as f64;
            self.sub1 = cs * self.sub1 + (1.0 - cs) * d;
            self.sub2 = cs * self.sub2 + (1.0 - cs) * self.sub1;
            let wet = self.wet[i] as f64 + p.sub_level * self.sub2;
            let o = (p.mix * wet + (1.0 - p.mix) * d) * g * self.amp[i] as f64;
            if twin_on {
                self.twin_mix = if goal > self.twin_mix { (self.twin_mix + step).min(goal) } else { (self.twin_mix - step).max(goal) };
                let r = (p.mix * (self.wet_twin[i] as f64 + p.sub_level * self.sub2) + (1.0 - p.mix) * d) * g * self.amp[i] as f64;
                let r = o + self.twin_mix * (r - o);
                out[off + i] = (a * o + (1.0 - a) * r) as f32;
                self.out_twin[i] = (a * r + (1.0 - a) * o) as f32;
            } else {
                out[off + i] = o as f32;
                self.out_twin[i] = o as f32;
            }
        }
        let peak = out[off..off + n].iter().fold(self.period_peak, |m, v| m.max(v.abs()));
        self.period_peak = peak;
    }
}
