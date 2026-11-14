//! Suu: a sound passed through CHIMERA's vocal tract, articulated by hand (vowel pad, nasal,
//! rounding), by typed text (sung per note, or looped at its own rate), or by the step sequencer,
//! which also plays the internal oscillator's pitch.
//!
//! Chain, per block: source (input or the internal oscillator) → resonator (a comb tuned to the
//! played note, so an outside sound can be played; off by default) → tilt (one-pole low-pass: the
//! mouth radiates the derivative of the flow, so an unfiltered synth comes out a step too bright)
//! → injected as volume velocity into the airway (`HumanVoice::render_driven`) → sub layer (the
//! dry source's low end, which a mouth hardly radiates) → dry/wet mix → output gain.
//!
//! Ranges go past the anatomy on purpose (a 0.4× or 4× mouth, extrapolated vowels, spread lips):
//! the oversampling follows the mouth's size so the airway stays stable (CFL), and the voice's
//! own output limiter bounds the level.
//!
//! Timing: breath, the sequencer and the text loop update on a fixed 32-sample grid counted from
//! the start, so the output does not depend on the host's block size.
//!
//! Real-time rules: `process` does not allocate. `set_text` and `set_size` do (they plan
//! gestures or build an airway) and belong off the audio thread; the web app calls them between
//! blocks (a size change takes well under a millisecond), the plug-in will move them to a worker.

use chimera::anatomy::human::{ArticKey, ArticPatch, BODY_MALE, BodyMix, HumanBody, N_ARTIC};
use chimera::math::{PI, clamp, cos, exp, js_max, midi_to_hz, pow, sin};
use chimera::phonetics::Syllable;
use chimera::phonetics::english::EnglishG2P;
use chimera::phonetics::planner::vowel_artic;
use chimera::control::articulators::ArticulatorBank;
use chimera::control::score::{ControlKey, Gesture};
use chimera::phonetics::sing::{NoteOnOptions, SongOptions, SungNote, plan_note_off, plan_note_on, plan_song_at};
use chimera::phonetics::syllables::lyrics_to_syllables;
use chimera::voice::human_airway::HumanAirway;
use chimera::voice::human_voice::{HumanVoice, SourcePatch};

mod track;
mod vowel;
use track::PitchTracker;
use vowel::VowelTracker;
pub use vowel::pad_of;

/// The largest block `process` handles in one piece (longer calls are split).
pub const MAX_BLOCK: usize = 256;
/// Control grid (samples).
const CTRL: u64 = 32;
/// Crossfade from the old airway to a rebuilt one (samples).
const FADE: u32 = 1024;
pub const STEPS: usize = 16;

/// The vowels at the pad's corners: x 0 front … 1 back, y 0 close … 1 open.
const CORNERS: [&str; 4] = ["i", "u", "æ", "ɑ"];
/// A unit's plans start this long before now (the live planner's lead, mostly taken back).
const LIVE_LEAD: f64 = -0.045;

/// The earliest a plan's gestures begin (relative), or infinity for none.
fn start_of(gs: &[Gesture]) -> f64 {
    gs.iter().fold(f64::INFINITY, |m, g| m.min(g.t0 - g.ramp_in))
}

/// Which precomputed text plan to schedule.
#[derive(Clone, Copy)]
enum Plan {
    HoldOn(usize),
    HoldOff(usize),
    Shot(usize),
    Tick(usize),
    Pass,
}

/// The sequencer's step vowels, in the order the UI offers them, and where each puts the pad
/// (the pad's corners are i, u, æ, ɑ; Motion then moves around that point).
pub const STEP_VOWELS: [&str; 5] = ["ɑ", "e", "i", "o", "u"];
const STEP_PAD: [(f64, f64); 5] = [(1.0, 1.0), (0.0, 0.5), (0.0, 0.0), (1.0, 0.5), (1.0, 0.0)];

/// Motion: what an LFO (or the envelope) can move, with each target's range; the last is
/// modulated on a log scale.
pub const MOD_TARGETS: [&str; N_MOD] = ["vowelX", "vowelY", "round", "nasal", "inject", "damping", "morph", "shift", "open"];
pub const N_MOD: usize = 9;
const MOD_RANGE: [(f64, f64); N_MOD] = [(-0.3, 1.3), (-0.3, 1.3), (-1.0, 1.0), (0.0, 1.5), (0.0, 1.0), (0.1, 20.0), (0.0, 1.0), (-0.5, 0.5), (0.25, 4.0)];
/// targets modulated on a log scale
const MOD_LOG: [bool; N_MOD] = [false, false, false, false, false, true, false, false, true];
/// The drawn horn: this many areas from the glottis to the lips.
pub const HORN_POINTS: usize = 32;
/// The injected source's flow per unit at 0 dB (cm³/s): calibrated so that the oscillator at 0 dB
/// and Folds at 100 % sound about as loud at the default breath.
const INJECT_REF: f64 = 562.0;
/// Sleep: below this level (dry source and mouth output, −120 dB) a control period is quiet,
/// and after SLEEP_AFTER quiet seconds with nothing to wake it the airway is not rendered.
const QUIET: f32 = 1e-6;
const SLEEP_AFTER: f64 = 0.1;
/// The source history a snapshot reads from (samples, a power of two: 85 ms at 48 kHz).
const SNAP_RING: usize = 4096;
/// The live area function the UI gets: this many samples from the glottis to the lips.
pub const UI_AREAS: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Sine,
    Triangle,
    Saw,
    Square,
    /// a new random value each cycle (sample and hold)
    Random,
    /// the source's envelope (unipolar; rate ignored)
    Follow,
    /// the ADSR, 0 … 1 (unipolar; rate ignored)
    Adsr,
}

/// One target's modulation. `depth` 0..1 swings the target by up to half its range each way
/// (negative inverts); `sync` > 0 locks a cycle to that many beats, else `rate_hz` runs free.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mod {
    pub depth: f64,
    pub rate_hz: f64,
    pub sync: f64,
    pub shape: Shape,
    /// 0 sharp … 1 smooth: how much of a cycle the shape's jumps take, eased (raised cosine).
    /// Random glides from one value to the next (1 = a gliding sample and hold), a square's
    /// edges round off (1 = a cosine), a saw's drop slows (1 = rise and fall alike).
    pub smooth: f64,
}

impl Default for Mod {
    fn default() -> Self {
        Mod { depth: 0.0, rate_hz: 1.0, sync: 0.0, shape: Shape::Sine, smooth: 0.0 }
    }
}

/// The internal oscillator's wave (Off: none; the input and the folds can still sound).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Off,
    Saw,
    Supersaw,
    Square,
}

/// Every control, in the units the UI shows. `Suu::set_params` clamps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// the oscillator's wave, and its level and the input's into the mouth (dB; −60 and below is
    /// silent). The two are mixed, and keys, hold and the sequencer gate the mix.
    pub source: Source,
    pub osc_db: f64,
    pub input_db: f64,
    /// root note (MIDI, fractional allowed): the oscillator's pitch, the sequence's transposition
    pub note: f64,
    /// a key held: the oscillator sounds, or the external input passes (when the sequencer is not
    /// playing)
    pub gate: bool,
    /// drive into the mouth, dB (0 dB = 100 cm³/s per unit of tilted input)
    pub drive_db: f64,
    /// tilt corner, Hz (0 = off)
    pub tilt_hz: f64,
    /// lung pressure, × the speaking default (the consonants' air)
    pub breath: f64,
    /// 0..1: how far the breath follows the source's envelope instead of staying steady
    pub follow: f64,
    /// vowel pad, −0.3..1.3 (outside 0..1 extrapolates past the corner vowels)
    pub vowel_x: f64,
    pub vowel_y: f64,
    /// −1 spread … 0 the vowel's own … 1 rounded and protruded
    pub round: f64,
    /// 0..1.5 velum lowering
    pub nasal: f64,
    /// 0..1 injection point, glottis … lips
    pub inject: f64,
    /// × on the airway's viscous loss (resonance damping), 0.1..20
    pub damping: f64,
    /// 0..2 the model's own folds vibrating with the source (at the sounding note, while a note
    /// sounds: they open and close with the gate)
    pub folds: f64,
    /// sub layer corner (Hz) and level (0..1)
    pub sub_hz: f64,
    pub sub_level: f64,
    /// 0 dry … 1 wet
    pub mix: f64,
    pub out_db: f64,
    /// tempo for the sequencer and the text loop
    pub bpm: f64,
    /// glide: the time a pitch change takes (linear in semitones), seconds; `glide_sync` > 0 locks
    /// it to that many beats instead
    pub glide_s: f64,
    pub glide_sync: f64,
    /// which way the pitch glides: both, only up or only down (the other way it jumps)
    pub glide_dir: GlideDir,
    /// Typed text (see `set_text`): what moves it on to the next syllable or word, which of the
    /// two, and how each is said
    pub text_step: TextStep,
    pub text_unit: TextUnit,
    pub text_say: TextSay,
    /// the text clock's rate, units per second; `text_sync` > 0 makes a unit that many beats
    pub text_rate_hz: f64,
    pub text_sync: f64,
    /// on the clock, held units last this fraction of a tick (1 = joined, legato)
    pub text_len: f64,
    /// the text starts over with the sequence (each time it returns to its first step); off, it
    /// goes on where it was
    pub text_seq: bool,
    /// the text is said once (per pass of the sequence, or once when set) instead of over and
    /// over
    pub text_once: bool,
    /// how much of each unit is heard (0.05..1): the sound is gated after this part of a tick
    /// (on the clock) or of the unit's spoken length (on notes). 1: not cut.
    pub text_gate: f64,
    /// speaking speed, × (0.25..4): how fast the consonants and transitions go. On the clock the
    /// vowels stay on the ticks; on a note the unit is said faster or slower.
    pub speech: f64,
    /// 0..1 the resonator: a comb tuned to the sounding note (and its glide) that gives any sound
    /// the played pitch; more rings longer and replaces more of the dry source. 0 = off.
    pub reson: f64,
    /// the drawn horn: 0 the articulated mouth … 1 the drawn area function; its shift along the
    /// tract (−0.5..0.5, + toward the lips) and opening (× its areas, 0.25..4)
    pub morph: f64,
    pub shift: f64,
    pub open: f64,
    /// ENV gate: the input's own level opens the sound, as a held key does, while it is at or
    /// above `env_thresh_db` (dBFS, measured before INPUT's gain); it closes when the input has
    /// stayed below for the release (`env_release_s`, or `env_release_sync` beats if > 0). Each
    /// opening is a new note, so the text says its next part on every onset.
    pub env_gate: bool,
    pub env_thresh_db: f64,
    pub env_release_s: f64,
    pub env_release_sync: f64,
    /// PITCH: the note, or the input's pitch (tracked; held through unvoiced and quiet parts,
    /// the note until a first pitch is found)
    pub pitch_from: PitchFrom,
    /// VOWEL = IN: the input's vowel (its first two formants) places the pad, as a talk box
    /// does; held through unvoiced and quiet parts. Motion moves around it.
    pub vowel_in: bool,
    /// The ADSR of what the gate opens (the oscillator, the input, the folds): attack (linear
    /// to the top), decay to sustain and release (each the time to within 1 %), each in s or,
    /// if its sync is > 0, in beats. Sustain is a share of the top.
    pub attack_s: f64,
    pub attack_sync: f64,
    pub decay_s: f64,
    pub decay_sync: f64,
    pub sustain: f64,
    pub release_s: f64,
    pub release_sync: f64,
    /// LEGATO: a note while the sound is open (another key, an untied step) goes on with the
    /// envelope; else (RETRIG) it starts the attack again, from where the level is
    pub legato: bool,
    /// COARSE: semitones on the sounding pitch, whatever PITCH takes it from (−48 … 48)
    pub coarse: f64,
    /// STEADY (PITCH = IN): a new pitch is taken only once the input has stayed on it (within a
    /// semitone's width) this long; movement within the note (vibrato, a slide) passes at once.
    /// In s, or in beats if `steady_sync` > 0. 0: every pitch at once.
    pub steady_s: f64,
    pub steady_sync: f64,
    /// HQ: the airway at the rate and resolution it was made for. Off: a lean airway at half the
    /// rate or less (sizes 0.4 … 1.6), much lighter, with fewer sections; it changes the sound.
    /// Takes effect when the mouth is rebuilt (`Suu::set_size`).
    pub hq: bool,
    /// GRID: the mouth's shape (pad, Motion, the input's vowel, step vowels, the horn) and the
    /// pitch change only on this grid (beats; 0 = free, as they come), on the host's bars when
    /// its transport runs. A played note (PITCH NOTE) takes its pitch at once; the input's pitch
    /// and every change of shape wait for the next line, however fast notes come. The
    /// articulators' own motion and GLIDE still shape each move.
    pub grid: f64,
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
            grid: 0.0,
        }
    }
}

/// One sequencer step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step {
    pub on: bool,
    /// semitones from the root note
    pub note: i32,
    /// hold into the next step without a new attack (and glide to its pitch)
    pub tie: bool,
    /// index into `STEP_VOWELS`, or −1 to leave the mouth where it is
    pub vowel: i32,
    /// louder (and with more air): the source is raised by `ACCENT`
    pub accent: bool,
    /// how long the gate stays open, a fraction of the step (a tie holds it into the next)
    pub len: f64,
}

/// An accented step's level, × the source.
pub const ACCENT: f64 = 1.6;

/// Where the pitch comes from: the note (root, keys, MIDI), or the input's own pitch, as it
/// moves or snapped to semitones. The sequencer's steps transpose from it either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PitchFrom {
    Note,
    Input,
    InputSnap,
}

/// Which pitch changes glide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GlideDir {
    Both,
    Up,
    Down,
}

/// What moves typed text on to its next unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextStep {
    /// each note (a key, hold or a step) says the next unit
    Note,
    /// the text's own clock (`text_rate_hz` or `text_sync`); it does not gate the sound
    Clock,
}

/// What the text says at a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextUnit {
    Syllable,
    Word,
}

/// How a unit is said.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextSay {
    /// its (last) vowel holds while the note does, or for `text_len` of a tick; then its coda
    Hold,
    /// said through at its own pace, however long the note
    Shot,
}

struct Seq {
    steps: [Step; STEPS],
    len: usize,
    /// steps per beat (4 = sixteenths)
    div: f64,
    /// × every step's length (a global gate that keeps the steps' proportions; 0 = silent)
    gate_scale: f64,
    /// passes through the pattern since it started playing
    cycles: u64,
    playing: bool,
    /// samples into the current step
    t: f64,
    step: usize,
    gate_open: bool,
    /// the sounding step's note, in semitones from the base note
    note: f64,
}

/// The sequencer's starting pattern: root, octave, fifth, flat seventh, with vowels.
pub fn default_pattern() -> [Step; STEPS] {
    // a starting pattern: root, octave, fifth, flat seventh, with vowels
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
    fn next(&mut self, src: Source, dt: f64) -> f64 {
        match src {
            Source::Saw | Source::Off => Self::saw(&mut self.phase[0], dt),
            Source::Square => {
                let p = &mut self.phase[0];
                let mut y = if *p < 0.5 { 1.0 } else { -1.0 };
                y += poly_blep(*p, dt);
                y -= poly_blep((*p + 0.5) % 1.0, dt);
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

/// The drawn horn's starting shape: narrow at the glottis, a throat, a wide bell at the lips.
pub fn default_horn() -> [f64; HORN_POINTS] {
    core::array::from_fn(|i| {
        let u = i as f64 / (HORN_POINTS - 1) as f64;
        0.6 + 1.5 * exp(-((u - 0.3) / 0.12) * ((u - 0.3) / 0.12)) + 9.0 * u * u * u
    })
}

/// A comb resonator tuned by its delay (Karplus-Strong style): a fractional delay line with a
/// gentle low-pass in the loop, so the partials decay like a string's, and a DC blocker after.
struct Reson {
    buf: Vec<f64>,
    w: usize,
    lp: f64,
    dc_x: f64,
    dc_y: f64,
}

impl Reson {
    /// A line long enough for 15 Hz (below MIDI 12) at `fs`. Allocates; build once.
    fn new(fs: f64) -> Self {
        Reson { buf: vec![0.0; ((fs / 15.0) as usize + 4).next_power_of_two()], w: 0, lp: 0.0, dc_x: 0.0, dc_y: 0.0 }
    }

    /// One sample through a comb of period `period` samples and loop gain `fb`.
    fn tick(&mut self, x: f64, period: f64, fb: f64) -> f64 {
        let mask = self.buf.len() - 1;
        let d = clamp(period, 2.0, mask as f64 - 2.0);
        let di = d as usize;
        let fr = d - di as f64;
        let a = self.buf[(self.w + mask + 1 - di) & mask];
        let b = self.buf[(self.w + mask - di) & mask];
        let delayed = a + fr * (b - a);
        // the loop's low-pass: the high partials die first
        self.lp += 0.7 * (delayed - self.lp);
        let y = x + fb * self.lp;
        self.buf[self.w] = y;
        self.w = (self.w + 1) & mask;
        // DC blocker (a tuned comb has gain at 0 Hz too)
        let out = y - self.dc_x + 0.995 * self.dc_y;
        self.dc_x = y;
        self.dc_y = out;
        out
    }
}

/// HQ off: the fewest sections per tract tube (CHIMERA's own is `MIN_CELLS`, 8).
const LEAN_CELLS: usize = 6;

/// Oversampling for a mouth of scale `k`: a small mouth has short cells and needs a finer time
/// step (CFL); a large one is stable at the output rate and stays affordable there. HQ off: the
/// lowest rate at which a lean airway (`LEAN_CELLS`, fewer sections in the nose and the
/// piriform sinus too) is stable: half the rate, or less, between sizes 0.4 and 1.6.
fn oversample_for(fs: f64, k: f64, hq: bool) -> usize {
    if !hq {
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

/// What building a mouth of a new size needs from the playing one (`Suu::mouth_seed`): plain
/// values, made on the audio thread without allocating and sent to the thread that builds.
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
    hq: bool,
}

impl MouthSeed {
    pub fn scale(&self) -> f64 {
        self.scale
    }
}

/// A mouth (an airway and its voice) built off the audio thread, for `Suu::set_mouth`; also an
/// old one handed back by `Suu::take_retired`, to be dropped there.
pub struct Mouth {
    voice: Box<HumanVoice>,
    scale: f64,
    hq: bool,
}

impl Mouth {
    /// Build a mouth of the seed's size, shaped like the seed's, and let it settle in silence at
    /// its breath (250 ms rendered and dropped: a cold airway blew a transient that set the
    /// output limiter clamping for 300 ms). Allocates; a few milliseconds.
    pub fn build(seed: &MouthSeed) -> Mouth {
        let body = HumanBody { name: "suu", scale: seed.scale, ..BODY_MALE };
        let os = oversample_for(seed.fs, seed.scale, seed.hq);
        let mut v = Box::new(if seed.hq { HumanVoice::new(seed.fs, body, os) } else { HumanVoice::with_min_cells(seed.fs, body, os, LEAN_CELLS) });
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
        Mouth { voice: v, scale: seed.scale, hq: seed.hq }
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }
}

/// What the text's timing depends on: a plan made for other values is stale.
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

/// The text clock's beats per unit: synced, or from its free rate; never under 60 ms.
fn loop_beats(p: &Params) -> f64 {
    let beats = if p.text_sync > 0.0 { p.text_sync } else { p.bpm / 60.0 / p.text_rate_hz };
    js_max(beats, 0.06 * p.bpm / 60.0)
}

/// Typed text, planned: its syllables and, per unit (syllable or word), the gestures the audio
/// thread only copies: a note's held unit (its last vowel open until the note-off) and its
/// ending; a unit said through, with how far it reaches before its vowel; a unit on the clock,
/// held `text_len` of a tick, the same; and the joined pass (on the clock, held, `text_len` 1).
/// Made by `TextPlan::new`, which allocates (off the audio thread in the plug-in); put in place
/// by `Suu::set_text_plan`, which does not.
pub struct TextPlan {
    text: Vec<Syllable>,
    hold_on: Vec<Vec<Gesture>>,
    hold_off: Vec<Vec<Gesture>>,
    shot: Vec<Vec<Gesture>>,
    shot_lead: f64,
    /// each unit's spoken length (s), for `text_gate` on notes
    shot_len: Vec<f64>,
    tick_plan: Vec<Vec<Gesture>>,
    tick_lead: f64,
    pass_plan: Vec<Gesture>,
    /// the joined pass's length (s) and how far it reaches before its start
    pass_len: f64,
    pass_lead: f64,
    /// the text clock's tick (s) when planned
    tick_s: f64,
    /// the copy buffer, holding the largest plan, so scheduling never grows it
    scratch: Vec<Gesture>,
    key: PlanKey,
}

impl TextPlan {
    /// No text (the pad). Does not allocate.
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

    /// Plan `text` (with the dictionary `g2p`; without one, or with blank text, the plan is
    /// empty) for the parameters `p` (clamped ones: `Suu::params`). Allocates.
    pub fn new(text: &str, g2p: Option<&EnglishG2P>, p: &Params) -> Self {
        let syllables = match g2p {
            Some(g) if !text.trim().is_empty() => lyrics_to_syllables(text, g),
            _ => Vec::new(),
        };
        let mut plan = TextPlan { text: syllables, key: PlanKey::of(p), ..TextPlan::empty() };
        plan.plan(p);
        plan
    }

    /// The number of syllables (0: no text).
    pub fn syllables(&self) -> usize {
        self.text.len()
    }

    /// The syllable ranges of the units (each syllable, or each word).
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
    // pitch comes from the oscillator or the sequencer, not the text: drop pitch gestures
    let unpitched = |gs: Vec<Gesture>| -> Vec<Gesture> { gs.into_iter().filter(|g| !g.targets.has(ControlKey::F0) && !g.targets.has(ControlKey::VIBRATO)).collect() };
    // speaking speed: a plan's times stretch from its first gesture's start (so nothing
    // slips into the past), slower = longer
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
    // the clock's tick, and a held unit's length on it
    let beats = loop_beats(&p);
    let tick_s = beats * beat;
    for r in &units {
        let last = &self.text[r.end - 1];
        // on a note, held: the unit's syllables before the last said through, then the last
        // with its vowel open (`plan_note_on`), all starting now
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
        // said through: its first vowel at 0, its onset before
        shot.push(paced(sung(&notes(r, 0.5), 0.8)));
        // on the clock, held for `text_len` of a tick (its syllables share it)
        tick.push(sung(&notes(r, p.text_len * beats / r.len() as f64), 0.8 / p.speech));
    }
    let lead_of = |plans: &[Vec<Gesture>]| plans.iter().fold(0.0f64, |m, gs| m.max(-start_of(gs)));
    self.shot_lead = lead_of(&shot);
    self.shot_len = shot.iter().map(|gs| gs.iter().map(|g| g.t1 + g.ramp_out).fold(0.0, f64::max) - start_of(gs)).collect();
    self.tick_lead = lead_of(&tick);
    // the joined pass: every unit a tick, its syllables sharing it
    let all: Vec<SungNote> = units.iter().flat_map(|r| notes(r, beats / r.len() as f64)).collect();
    self.pass_plan = sung(&all, 0.8 / p.speech);
    self.pass_len = units.len() as f64 * tick_s;
    self.pass_lead = -start_of(&self.pass_plan).min(0.0);
    self.tick_s = tick_s;
    (self.hold_on, self.hold_off, self.shot, self.tick_plan) = (hold_on, hold_off, shot, tick);
    // the copy buffer holds the largest plan, so scheduling never grows it
    let most = self.hold_on.iter().chain(&self.hold_off).chain(&self.shot).chain(&self.tick_plan).map(Vec::len).chain([self.pass_plan.len()]).max().unwrap_or(0);
    self.scratch = Vec::with_capacity(most.max(256));
    }
}

pub struct Suu {
    fs: f64,
    voice: Box<HumanVoice>,
    /// the airway being faded out after a rebuild, and the samples left
    fade: Option<(Box<HumanVoice>, u32)>,
    /// the faded-out airway, kept until `take_retired` (so the audio thread never frees it)
    retired: Option<Box<HumanVoice>>,
    scale: f64,
    /// the playing mouth was built at HQ
    hq: bool,
    p: Params,
    osc: Osc,
    /// the oscillator's pitch (MIDI, gliding toward `glide_to` by `glide_rate` per sample) and Hz
    pitch: f64,
    glide_to: f64,
    glide_rate: f64,
    osc_hz: f64,
    /// the accent's level, gliding (1, or `ACCENT` on an accented step)
    osc_env: f64,
    reson: Reson,
    /// sleep: the loudest dry and wet sample this control period, quiet samples so far, asleep
    period_peak: f32,
    quiet: u64,
    asleep: bool,
    /// the last SNAP_RING samples of the ungated source (for `snapshot`)
    snap_ring: Vec<f32>,
    snap_w: usize,
    tilt_y: f64,
    env: f64,
    sub1: f64,
    sub2: f64,
    dry: Vec<f32>,
    drive: Vec<f32>,
    /// the drive into a rebuilt mouth while it fades in
    drive_new: Vec<f32>,
    wet: Vec<f32>,
    wet_old: Vec<f32>,
    rest: [f64; N_ARTIC],
    corners: [[f64; N_ARTIC]; 4],
    mods: [Mod; N_MOD],
    mod_phase: [f64; N_MOD],
    mod_hold: [f64; N_MOD],
    /// random: the value a glide to the held one starts from (the last output at the cycle's start)
    mod_from: [f64; N_MOD],
    /// each target's last LFO value (−1..1), before depth
    mod_out: [f64; N_MOD],
    mod_rng: u32,
    /// the targets' values after modulation, and the last ones sent to the voice
    eff: [f64; N_MOD],
    applied: [f64; N_MOD],
    /// a step's vowel holds the pad here while the sequencer plays
    seq_pad: Option<(f64, f64)>,
    g2p: Option<EnglishG2P>,
    plan: TextPlan,
    /// next unit, and units said since the text or the pattern pass started (for `text_once`)
    cursor: usize,
    fired: usize,
    /// the clock's next tick, and the joined pass's next start (voice clock, s)
    next_tick: f64,
    next_pass: f64,
    /// joined pass, said once: one is still to come
    once_pending: bool,
    /// the pattern pass (`Seq::cycles`) the text's restart is scheduled for, and the last seen
    restart_for: Option<u64>,
    seen_cycles: u64,
    /// the voice-clock time of the last restart scheduled (for tests)
    last_restart: f64,
    /// the held unit (index, tag) on a note
    open_unit: Option<(usize, f64)>,
    /// `text_gate`: the window the last unit is heard in, and the joined passes' span (voice
    /// clock, s); the gate after it (the note's gate and the text's)
    cut: (f64, f64),
    pass_cut: (f64, f64),
    gate_eff: bool,
    /// units said in all, and the last one (for tests)
    said: u64,
    last_unit: usize,
    next_tag: f64,
    prev_gate: bool,
    retrig: bool,
    pos: u64,
    env_peak: f64,
    seq: Seq,
    /// the note and gate actually sounding (manual or sequenced), and the folds' last pitch
    note_now: f64,
    gate_now: bool,
    folds_f0: f64,
    /// the sounding step is accented
    accent: bool,
    /// the folds' gate (0..1, control rate): they sound with a note, as the source does; their
    /// lung pressure alone kept them sounding with no note at all
    fold_env: f64,
    /// ENV gate: the input's envelope, its peak this control period, open, and how long it has
    /// been below the threshold (s)
    in_env: f64,
    in_peak: f64,
    env_open: bool,
    env_below: f64,
    /// PITCH = INPUT: the input's pitch, analysed every `TRACK_EVERY` control periods
    tracker: PitchTracker,
    track_count: u32,
    /// the input's pitch taken (after STEADY), and a new one waiting since the given sample
    steady: Option<f64>,
    candidate: Option<(f64, u64)>,
    /// GRID: the line last crossed, the pitch and shape taken there (held until the next)
    grid_line: i64,
    grid_held: Option<(f64, [f64; N_MOD])>,
    /// the host's song position (beats) and the sample it was at
    song: Option<(f64, u64)>,
    /// VOWEL = IN: the input's vowel, and where the pad is gliding to it
    vowels: VowelTracker,
    in_pad: Option<(f64, f64)>,
    /// the ADSR: its level (0 … the accent's top), stage, and per-sample steps for this
    /// control period
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

/// The input's pitch is analysed this often (control periods: 5.3 ms at 48 kHz).
const TRACK_EVERY: u32 = 8;

impl Suu {
    pub fn new(fs: f64) -> Self {
        chimera::phonetics::planner::init();
        let mut s = Suu {
            fs,
            voice: Box::new(HumanVoice::new(fs, BODY_MALE, oversample_for(fs, 1.0, true))),
            fade: None,
            retired: None,
            scale: 1.0,
            hq: true,
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
            candidate: None,
            grid_line: i64::MIN,
            grid_held: None,
            song: None,
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
        s.configure_voice();
        s.refresh_mouth();
        s.warm_up(&mut |v: &mut HumanVoice, inp, out| v.render_driven(inp, 100.0, out));
        s
    }

    /// Let a new airway settle in silence (250 ms, rendered and dropped) at the resting breath:
    /// the lungs filling from zero blew a −44 dBFS blip through it in its first 50 ms, heard
    /// when an instance is created. Runs the voice alone, so Suu's own clock and timing are untouched.
    fn warm_up(&mut self, render: &mut dyn FnMut(&mut HumanVoice, &[f32], &mut [f32])) {
        // at the breath's pressure now (the resting one when new, as the first control period
        // will set it: a jump there from zero was the blip)
        let p = self.p;
        if self.fade.is_none() {
            self.voice.set_source(&SourcePatch { pressure: Some(9.0 * p.breath * (1.0 - p.follow)), ..Default::default() });
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
        for x in [&mut q.attack_sync, &mut q.decay_sync, &mut q.release_sync] {
            *x = clamp(*x, 0.0, 16.0);
        }
        self.p = q;
        self.configure_voice();
        self.refresh_mouth();
    }

    /// Mouth size: body scale 0.4 … 4 (1 = adult male). Builds a new airway (allocates) that takes
    /// over the old one's articulation, gestures and clock, and crossfades into it. The plug-in
    /// splits this: `mouth_seed` and `set_mouth` on the audio thread, `Mouth::build` elsewhere.
    pub fn set_size(&mut self, scale: f64) {
        let s = clamp(scale, 0.4, 4.0);
        if s == self.scale && self.p.hq == self.hq {
            return;
        }
        let m = Mouth::build(&self.mouth_seed(s));
        // a change during a crossfade replaces the fading airway
        (self.fade, self.retired) = (None, None);
        let _ = self.set_mouth(m);
    }

    /// What `Mouth::build` needs to make a mouth of scale `scale` (clamped to 0.4 … 4) that
    /// takes over from this one. No allocation.
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
            hq: self.p.hq,
        }
    }

    /// Put a mouth from `Mouth::build` in place: it takes over this one's articulation, horn,
    /// gestures and clock, and the old one crossfades out. No allocation. Refused (the mouth is
    /// handed back) while an earlier change still fades or its airway waits in `take_retired`.
    pub fn set_mouth(&mut self, m: Mouth) -> Result<(), Mouth> {
        if self.fade.is_some() || self.retired.is_some() {
            return Err(m);
        }
        let Mouth { voice: mut nv, scale, hq } = m;
        (self.scale, self.hq) = (scale, hq);
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

    /// The airway a size change faded out, once it has (to be dropped off the audio thread).
    pub fn take_retired(&mut self) -> Option<Mouth> {
        self.retired.take().map(|voice| Mouth { voice, scale: 0.0, hq: true })
    }

    /// The drawn horn: `HORN_POINTS` areas (cm², adult scale) from the glottis to the lips,
    /// clamped to 0..20. `wrap`: the areas are one period of a wave (points at i/n) and Shift
    /// wraps around, so the wave travels. Copies into a buffer sized at construction (no
    /// allocation).
    pub fn set_profile(&mut self, areas: &[f64], wrap: bool) {
        self.voice.airway.profile_wrap = wrap;
        let n = HORN_POINTS.min(areas.len());
        for (d, &a) in self.voice.airway.profile.iter_mut().zip(&areas[..n]) {
            *d = clamp(a, 0.0, 20.0);
        }
    }

    /// One period of the source's waveform (the sounding note's, from the ungated source:
    /// the oscillator, or the file or mic) as `HORN_POINTS` values in −1..1, each the mean of
    /// its stretch (so the shape is not aliased), normalised to the peak. False if silent.
    pub fn snapshot(&self, out: &mut [f64; HORN_POINTS]) -> bool {
        let len = clamp(self.fs / self.osc_hz, HORN_POINTS as f64, (SNAP_RING - 1) as f64) as usize;
        let start = self.snap_w + SNAP_RING - len;
        for (k, o) in out.iter_mut().enumerate() {
            let (a, b) = (k * len / HORN_POINTS, ((k + 1) * len / HORN_POINTS).max(k * len / HORN_POINTS + 1));
            *o = (a..b).map(|j| self.snap_ring[(start + j) & (SNAP_RING - 1)] as f64).sum::<f64>() / (b - a) as f64;
        }
        // centred and normalised: the shape, not its offset or level
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

    /// Whether the mouth playing was built at HQ (`Params::hq`; a change rebuilds it, as a size
    /// does: `set_size`, or the plug-in's worker).
    pub fn hq(&self) -> bool {
        self.hq
    }

    /// The voice-wide settings that follow the parameters (cheap, no allocation).
    fn configure_voice(&mut self) {
        let p = self.p;
        self.voice.glottal_area_scale = p.folds * self.fold_env;
        // the folds follow Suu's own (already glided) pitch: no glide of their own (their default
        // 60 ms kept a slide even with Glide at 0)
        self.voice.set_source(&SourcePatch { voiced: Some(true), tension: Some(0.55), drift: Some(0.0), glide: Some(0.002), ..Default::default() });
        self.voice.set_gate(1.0);
        self.folds_f0 = 0.0; // re-send the pitch on the next control tick
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

    /// The targets' unmodulated values: the pad (or a step's vowel), rounding, nasal, injection
    /// and damping.
    fn bases(&self) -> [f64; N_MOD] {
        let p = self.p;
        let pad = if p.vowel_in { self.in_pad } else { None };
        let (x, y) = pad.or(self.seq_pad).unwrap_or((p.vowel_x, p.vowel_y));
        [x, y, p.round, p.nasal, p.inject, p.damping, p.morph, p.shift, p.open]
    }

    /// Advance the LFOs one control period and compute the modulated values.
    fn run_mods(&mut self, envn: f64) {
        let bases = self.bases();
        let beats = self.beats_now();
        for i in 0..N_MOD {
            let m = self.mods[i];
            let (lo, hi) = MOD_RANGE[i];
            if m.depth == 0.0 {
                self.eff[i] = bases[i];
                continue;
            }
            let old = self.mod_phase[i];
            let ph = if m.sync > 0.0 { (beats / m.sync).fract() } else { (old + m.rate_hz * CTRL as f64 / self.fs).fract() };
            if ph < old || self.mod_hold[i] == 0.0 {
                // a new cycle: a new random value, glided to from where the last one got to
                self.mod_from[i] = self.mod_out[i];
                let r = &mut self.mod_rng;
                *r ^= *r << 13;
                *r ^= *r >> 17;
                *r ^= *r << 5;
                self.mod_hold[i] = (*r as f64 / u32::MAX as f64) * 2.0 - 1.0 + 1e-9;
            }
            self.mod_phase[i] = ph;
            // raised-cosine ease 0 → 1 over x in 0..1
            let ease = |x: f64| 0.5 - 0.5 * cos(PI * clamp(x, 0.0, 1.0));
            let sm = m.smooth;
            let v = match m.shape {
                Shape::Sine => sin(2.0 * PI * ph),
                Shape::Triangle => 1.0 - 4.0 * (ph - 0.5).abs(),
                Shape::Saw => {
                    // rise over 1 − r of the cycle, fall back over r (r up to half)
                    let r = 0.5 * sm;
                    if ph < 1.0 - r { -1.0 + 2.0 * ph / (1.0 - r) } else { 1.0 - 2.0 * ease((ph - (1.0 - r)) / r) }
                }
                Shape::Square => {
                    // each edge takes e of the cycle (up to half: a cosine)
                    let e = 0.5 * sm;
                    let (half, sign) = if ph < 0.5 { (ph, 1.0) } else { (ph - 0.5, -1.0) };
                    if e > 0.0 && half < e { sign * (2.0 * ease(half / e) - 1.0) } else { sign }
                }
                Shape::Random => {
                    let (a, b) = (self.mod_from[i], self.mod_hold[i]);
                    if sm > 0.0 { a + (b - a) * ease(ph / sm) } else { b }
                }
                Shape::Follow => envn,
                Shape::Adsr => self.adsr,
            };
            self.mod_out[i] = v;
            self.eff[i] = if MOD_LOG[i] {
                clamp(bases[i] * pow(hi / lo, 0.5 * m.depth * v), lo, hi)
            } else {
                clamp(bases[i] + 0.5 * m.depth * v * (hi - lo), lo, hi)
            };
        }
    }

    /// Send the modulated mouth to the voice, where it changed.
    fn apply_mouth(&mut self) {
        let e = self.eff;
        if e[4] != self.applied[4] {
            self.voice.set_injection(e[4]);
        }
        if e[5] != self.applied[5] {
            self.voice.airway.net.viscous_scale = e[5];
        }
        // the drawn horn: read by the airway at its next geometry update
        let aw = &mut self.voice.airway;
        (aw.profile_mix, aw.profile_shift, aw.profile_open) = (e[6], e[7], e[8]);
        let shape_changed = (0..4).any(|i| (e[i] - self.applied[i]).abs() > 1e-6 || self.applied[i].is_nan());
        self.applied = e;
        if !shape_changed {
            return;
        }
        let (x, y, round, nasal) = (e[0], e[1], e[2], e[3]);
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
        self.voice.set_articulation(&patch, None);
    }

    /// Re-apply the mouth now (after a parameter change), without advancing the LFOs.
    fn refresh_mouth(&mut self) {
        let b = self.bases();
        for i in 0..N_MOD {
            if self.mods[i].depth == 0.0 {
                self.eff[i] = b[i];
            }
        }
        // GRID: a changed parameter waits for the next line, as every change does
        if self.p.grid > 0.0
            && let Some((_, held)) = self.grid_held
        {
            self.eff = held;
        }
        self.applied = [f64::NAN; N_MOD];
        self.apply_mouth();
    }

    /// For the UI: `[eff ×6, base x, base y, lipAp, lipWidth, lipProt, jaw, tbX, tbY, velum,
    /// lip area, envelope, step, gate, playing, morph, shift, open, areas ×UI_AREAS]` (areas in
    /// cm², glottis → lips). Reuses `out`'s capacity.
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
        // 20..23: morph, shift, open (modulated); 23..: the live area function, glottis → lips;
        // then the input's tracked pitch (MIDI; 0: none, or PITCH is the note)
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
        // (as it sounds: snapped for IN ♮, with COARSE)
        let tracked = match self.p.pitch_from {
            PitchFrom::Note => None,
            PitchFrom::Input => self.steady,
            PitchFrom::InputSnap => self.steady.map(f64::round),
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

    /// pattern length, steps per beat, and the global gate (× every step's length, 0..2)
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
            // the text starts with the sequence: from its beginning if it follows it, and its
            // count for "once" anew either way
            self.fired = 0;
            self.once_pending = true;
            if self.p.text_seq && !self.plan.text.is_empty() {
                self.cursor = 0;
                if self.p.text_step == TextStep::Clock {
                    let now = self.voice.clock;
                    self.voice.release_all(now);
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

    /// A new key while another is held (legato on the keyboard): a new note for the text.
    pub fn retrigger(&mut self) {
        self.retrig = true;
    }

    /// (current step, gate open, sounding note) for the UI
    pub fn seq_state(&self) -> (usize, bool, f64) {
        (self.seq.step, self.gate_now, self.note_now)
    }

    fn step_samples(&self) -> f64 {
        self.fs * 60.0 / self.p.bpm / self.seq.div
    }

    /// Start the current step.
    fn trigger(&mut self) {
        let st = self.seq.steps[self.seq.step];
        // a gate of 0 opens nothing (not even for a control period)
        if st.on && self.gate_len() > 0.0 {
            self.seq.note = st.note as f64; // from the base note (`base_note`)
            // a tied step continues the previous note; any other starts a new one
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

    /// The current step's gate length (fraction of a step), scaled by the global gate.
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
            // a tie holds the gate into the next step
            if !self.seq.steps[self.seq.step].tie {
                self.seq.gate_open = false;
            }
        }
    }

    // ------------------------------------------------------------ text

    /// Load the pronouncing dictionary (CMUdict text). Allocates; call once at start.
    pub fn load_dict(&mut self, text: &str) {
        let mut g = EnglishG2P::new();
        g.load_dict(text);
        self.g2p = Some(g);
    }

    /// The dictionary loaded by `load_dict` (for `TextPlan::new`).
    pub fn g2p(&self) -> Option<&EnglishG2P> {
        self.g2p.as_ref()
    }

    /// Typed text, said as the `text_*` parameters say; empty text returns the mouth to the
    /// pad. Returns the number of syllables. Allocates (it plans every unit up front, so the
    /// audio thread only copies gestures): call off the audio thread, and again after a change
    /// of the unit, the clock's rate or length, the tempo or the speaking speed (`text_stale`).
    pub fn set_text(&mut self, text: &str) -> usize {
        let plan = TextPlan::new(text, self.g2p.as_ref(), &self.p);
        drop(self.set_text_plan(plan));
        self.plan.text.len()
    }

    /// Put a plan from `TextPlan::new` in place, as `set_text` does, and return the old one (to
    /// be dropped off the audio thread). Does not allocate.
    pub fn set_text_plan(&mut self, plan: TextPlan) -> TextPlan {
        self.voice.cancel_score();
        let old = core::mem::replace(&mut self.plan, plan);
        // an edit keeps the place in the text (taken modulo the new length when used), so typing
        // while it sings carries on instead of starting over
        self.open_unit = None;
        if self.plan.text.is_empty() {
            self.cursor = 0;
        }
        self.fired = 0;
        self.once_pending = true;
        self.restart_for = None;
        let now = self.voice.clock;
        (self.next_tick, self.next_pass) = (now + 0.1, now + 0.1);
        self.applied = [f64::NAN; N_MOD];
        self.refresh_mouth();
        old
    }

    /// The text's plan was made for other values of what its timing depends on (the unit, the
    /// clock's rate or length, the tempo, the speaking speed): plan it again (`set_text`).
    pub fn text_stale(&self) -> bool {
        !self.plan.text.is_empty() && self.plan.key != PlanKey::of(&self.p)
    }

    /// The number of syllables in the text (0: the pad).
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

    /// On the clock, held at full length: the joined pass (as sung legato) rather than unit by
    /// unit.
    fn joined(&self) -> bool {
        self.p.text_say == TextSay::Hold && self.p.text_len >= 0.95
    }

    /// Copy a plan onto the voice clock at `at` (with `tag`, if any) and schedule it. No
    /// allocation: the copy buffer was sized in `plan_text`.
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
    }

    /// The next unit to say, or None once the text has been said through and `text_once` holds.
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
            self.voice.release(tag);
            self.schedule_at(Plan::HoldOff(u), now, None);
        }
    }

    /// A note started or ended (step: note).
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
                    // its first gesture now, its vowel a lead later
                    let at = now + self.plan.shot_lead;
                    self.schedule_at(Plan::Shot(u), at, None);
                }
            }
        }
    }

    /// The text at each control period: note edges, the pattern's passes, the clock.
    fn run_text(&mut self) {
        if self.plan.text.is_empty() {
            return;
        }
        let p = self.p;
        let now = self.voice.clock;
        // a new pass of the pattern: "once" counts anew; on notes, following the sequence, the
        // text starts over (on the clock, the restart below was scheduled ahead)
        if self.seq.playing && self.seq.cycles != self.seen_cycles {
            self.seen_cycles = self.seq.cycles;
            // (on the clock following the sequence, the restart ahead of the pass already counted
            // anew, and its first unit may already be said: counting again here doubled it)
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
            if rising {
                self.text_note(true);
            } else if falling {
                self.text_note(false);
            }
            return;
        }
        let joined = self.joined();
        let lead = if joined { self.plan.pass_lead } else if p.text_say == TextSay::Hold { self.plan.tick_lead } else { self.plan.shot_lead };
        // following the sequence: start the text over where the pattern next returns to its
        // first step, scheduled ahead by the lead so its first consonants land before it
        if p.text_seq && self.seq.playing {
            let to_wrap = ((self.seq.len - self.seq.step) as f64 * self.step_samples() - self.seq.t) / self.fs;
            let next = self.seq.cycles + 1;
            if to_wrap <= lead + 0.05 && self.restart_for != Some(next) {
                let at = now + to_wrap;
                self.voice.release_all(at);
                (self.next_tick, self.next_pass) = (at, at);
                self.cursor = 0;
                self.fired = 0;
                self.once_pending = true;
                self.restart_for = Some(next);
                self.last_restart = at;
            }
        }
        if joined {
            // the joined pass, scheduled a pass ahead; said once, only the one asked for
            if now + 0.5 + lead >= self.next_pass && (!p.text_once || self.once_pending) {
                let at = self.next_pass;
                // passes that follow on extend the span the ticks are heard in
                self.pass_cut = if (at - self.pass_cut.1).abs() < 1e-9 { (self.pass_cut.0, at + self.plan.pass_len) } else { (at, at + self.plan.pass_len) };
                self.schedule_at(Plan::Pass, at, None);
                self.next_pass += self.plan.pass_len;
                self.once_pending = false;
            }
            return;
        }
        // unit by unit, just in time: each unit's vowel on its tick
        if self.next_tick < now - self.plan.tick_s {
            self.next_tick = now + lead; // the clock fell behind (it was stopped): pick it up
        }
        let horizon = now + 2.0 * CTRL as f64 / self.fs;
        while self.next_tick - lead <= horizon {
            let at = self.next_tick;
            if let Some(u) = self.next_unit() {
                let from = at - lead.min(0.5 * self.plan.tick_s);
                self.cut = (from, from + p.text_gate * self.plan.tick_s);
                let plan = if p.text_say == TextSay::Hold { Plan::Tick(u) } else { Plan::Shot(u) };
                self.schedule_at(plan, at, None);
            }
            self.next_tick += self.plan.tick_s;
        }
    }

    /// `text_gate`: is the sound let through now? Always, unless the text sings and is cut.
    fn text_window_open(&self) -> bool {
        let p = self.p;
        if self.plan.text.is_empty() || p.text_gate >= 1.0 {
            return true;
        }
        let now = self.voice.clock;
        if self.on_clock() && self.joined() {
            // joined passes: a window at each tick, from just before its vowel
            let tick = self.plan.tick_s;
            let from = self.pass_cut.0 - self.plan.pass_lead.min(0.5 * tick);
            return now >= from && now < self.pass_cut.1 && (now - from).rem_euclid(tick) < p.text_gate * tick;
        }
        now >= self.cut.0 && now < self.cut.1
    }

    // ------------------------------------------------------------ audio

    /// Process one block. `input` is the external source (ignored unless `Source::Input`); its
    /// length must equal `out`'s.
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let mut off = 0;
        while off < out.len() {
            let to_grid = (CTRL - self.pos % CTRL) as usize;
            let n = (out.len() - off).min(MAX_BLOCK).min(to_grid);
            if self.pos % CTRL == 0 {
                self.control();
            }
            self.process_block(&input[off..off + n], off, n, out);
            off += n;
            self.pos += n as u64;
        }
    }

    /// At each control-grid point: sequencer, breath from the last period's envelope, the folds'
    /// pitch, and the text loop.
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
                // the vowel, from voiced frames only (a consonant or noise would throw it)
                if self.p.vowel_in && self.tracker.voiced() {
                    self.vowels.analyse();
                }
            }
            // the pad glides to the vowel found (20 ms); the articulators' slew does the rest
            if let Some((x, y)) = self.vowels.pad() {
                let k = 1.0 - exp(-(CTRL as f64) / (0.02 * self.fs));
                let (px, py) = self.in_pad.unwrap_or((x, y));
                self.in_pad = Some((px + (x - px) * k, py + (y - py) * k));
            }
        }
        if self.seq.playing {
            self.advance_seq();
            self.note_now = self.base_note() + self.seq.note;
            self.gate_now = self.seq.gate_open;
        } else {
            self.note_now = self.base_note();
            self.run_env_gate();
            self.gate_now = self.p.gate || self.env_open;
        }
        // GRID: on a new line the pitch and shape are taken; between, held. A played note (PITCH
        // NOTE: a key, MIDI, a step) takes its pitch at once; the input's pitch and every change
        // of shape wait for the line.
        let grid_take = self.grid_take();
        let played = self.p.pitch_from == PitchFrom::Note && self.gate_now && (!self.prev_gate || self.retrig);
        if let Some((note, shape)) = self.grid_held.filter(|_| !grid_take) {
            if played {
                self.grid_held = Some((self.note_now, shape));
            } else {
                self.note_now = note;
            }
        }
        self.run_text();
        let retrig = self.retrig;
        self.retrig = false;
        self.prev_gate = self.gate_now;
        let p = self.p;
        let was_open = self.gate_eff;
        self.gate_eff = self.gate_now && self.text_window_open();
        self.run_adsr(was_open, retrig);
        // sleep: nothing sounds and nothing can start a sound on its own (no note, no folds, no
        // text loop, no crossfade), and the last SLEEP_AFTER s were quiet; decided per control
        // period, so it does not depend on the host's block size
        let quiet_period = self.period_peak < QUIET;
        self.period_peak = 0.0;
        self.quiet = if quiet_period { self.quiet + CTRL } else { 0 };
        // the folds follow the ADSR (5 ms up, at least 60 ms down)
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
            // the voice's clock stops while asleep: drop what the text still had scheduled (it
            // would have sounded silent, the source being gated), or it would fire on waking
            self.voice.cancel_score();
            self.open_unit = None;
        }
        self.asleep = asleep;
        let secs = if p.glide_sync > 0.0 { p.glide_sync * 60.0 / p.bpm } else { p.glide_s };
        // Glide moves the vowels too: the articulators' speed is ×1 at the default 30 ms, up to
        // ×4 at 0 (the airway's area slew limits still keep a change click-free) and down to
        // ×¼ for long glides
        let speed = clamp(0.03 / js_max(secs, 0.0075), 0.25, 4.0);
        if speed != self.voice.artic.speed[0] {
            self.voice.artic.speed = [speed; N_ARTIC];
        }
        if self.note_now != self.glide_to {
            // a new pitch: glide there in the glide time, linear in semitones, from where it is
            self.glide_to = self.note_now;
            let n = js_max(1.0, secs * self.fs);
            let up = self.glide_to > self.pitch;
            // the other way, it jumps (at once: the whole interval in one sample)
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
            match self.grid_held {
                Some((_, held)) if !grid_take => self.eff = held,
                _ => self.grid_held = Some((self.note_now, self.eff)),
            }
        } else {
            self.grid_held = None;
        }
        self.apply_mouth();
        // the breath follows the source's level; while the folds sound, at their full pressure
        // (so their level depends on Folds and Breath alone: a louder oscillator drove them up
        // by 20 dB before)
        let drive_env = if p.folds > 0.0 { js_max(envn, self.fold_env) } else { envn };
        let pressure = 9.0 * p.breath * (1.0 - p.follow + p.follow * drive_env);
        // the folds follow the glide too (at the control rate)
        let f0 = midi_to_hz(self.pitch);
        if (pressure - self.voice.source.pressure).abs() > 1e-6 || (p.folds > 0.0 && f0 != self.folds_f0) {
            self.voice.set_source(&SourcePatch { pressure: Some(pressure), f0: Some(f0), ..Default::default() });
            self.folds_f0 = f0;
        }
    }

    /// GRID: whether this control period takes a new pitch and shape: off, or a new grid line
    /// crossed (or nothing taken yet).
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

    /// The song position, in beats: the host's (`set_song_position`) moved on by the samples
    /// since, or, without a host transport, Suu's own clock.
    fn beats_now(&self) -> f64 {
        let since = |pos: u64| pos as f64 / self.fs * self.p.bpm / 60.0;
        match self.song {
            Some((beats, at)) => beats + since(self.pos - at),
            None => since(self.pos),
        }
    }

    /// The host's song position in beats at the start of the next `process` (None: no transport
    /// running; Suu's own clock then). GRID's lines and Motion's synced cycles follow it, so they
    /// line up with the host's bars.
    pub fn set_song_position(&mut self, beats: Option<f64>) {
        self.song = beats.map(|b| (b, self.pos));
    }

    /// STEADY: a voiced reading of the input's pitch. Near the pitch taken (within half a
    /// semitone) it moves it at once; elsewhere it waits as a candidate until the input has
    /// stayed near the candidate for the steady time.
    fn take_pitch(&mut self, n: f64) {
        let p = self.p;
        let hold = if p.steady_sync > 0.0 { p.steady_sync * 60.0 / p.bpm } else { p.steady_s };
        if self.steady.is_some_and(|a| (n - a).abs() < 0.5) {
            self.steady = Some(n);
            self.candidate = None;
            return;
        }
        // the candidate goes on while the input stays near where it started; else a new one
        let cand = match self.candidate {
            Some((c, t0)) if (n - c).abs() < 0.5 => (c, t0),
            _ => (n, self.pos),
        };
        if (self.pos - cand.1) as f64 >= hold * self.fs {
            self.steady = Some(n);
            self.candidate = None;
        } else {
            self.candidate = Some(cand);
        }
    }

    /// The pitch the sound is at, before the sequencer's steps: the note, or the input's (after
    /// STEADY), then COARSE.
    fn base_note(&self) -> f64 {
        let n = match (self.p.pitch_from, self.steady) {
            (PitchFrom::Input, Some(n)) => n,
            (PitchFrom::InputSnap, Some(n)) => n.round(),
            _ => self.p.note,
        };
        clamp(n + self.p.coarse, 12.0, 108.0)
    }

    /// The input's pitch as taken (after STEADY; MIDI), if one has been found (for the UI).
    pub fn tracked_note(&self) -> Option<f64> {
        self.steady
    }

    /// The ADSR at a control point: a new note starts the attack (from the level it is at), a
    /// closed gate the release; then this period's per-sample steps.
    fn run_adsr(&mut self, was_open: bool, retrig: bool) {
        let p = self.p;
        if self.gate_eff && (!was_open || (retrig && !p.legato)) {
            self.stage = Stage::Attack;
        } else if !self.gate_eff && matches!(self.stage, Stage::Attack | Stage::Decay) {
            self.stage = Stage::Release;
        }
        let secs = |s: f64, sync: f64| if sync > 0.0 { sync * 60.0 / p.bpm } else { s };
        let fs = self.fs;
        // the time to within 1 % is 4.6 time constants
        let coef = |t: f64| exp(-4.6 / (js_max(t, 0.0005) * fs));
        self.adsr_step = [1.0 / (js_max(secs(p.attack_s, p.attack_sync), 0.0005) * fs), coef(secs(p.decay_s, p.decay_sync)), coef(secs(p.release_s, p.release_sync))];
    }

    /// VOWEL = IN: the last vowel found in the input (pad x, y), and whether the last frame
    /// analysed was voiced (for the UI and measurements).
    pub fn input_vowel(&self) -> (Option<(f64, f64)>, bool) {
        (self.vowels.pad(), self.tracker.voiced())
    }

    /// VOWEL = IN: the last vowel's F1 and F2 (Hz; 0 before one is found).
    pub fn input_formants(&self) -> (f64, f64) {
        self.vowels.formants()
    }

    /// The ADSR's level (0 … 1, the accent left out), for the UI.
    pub fn adsr_level(&self) -> f64 {
        self.adsr
    }

    /// ENV gate, per control period: open at the threshold, closed after the release below it.
    fn run_env_gate(&mut self) {
        let p = self.p;
        let peak = self.in_peak;
        self.in_peak = 0.0;
        if !p.env_gate {
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

    /// Whether the ENV gate is open (for tests and the UI).
    pub fn env_open(&self) -> bool {
        self.env_open
    }

    fn process_block(&mut self, input: &[f32], off: usize, n: usize, out: &mut [f32]) {
        let p = self.p;
        let fs = self.fs;
        // 1) source
        let env_a = 1.0 - exp(-1.0 / (0.005 * fs));
        let env_r = 1.0 - exp(-1.0 / (0.08 * fs));
        let acc_k = 1.0 - exp(-1.0 / (0.004 * fs));
        let level_of = |db: f64| if db <= -60.0 { 0.0 } else { pow(10.0, db / 20.0) };
        let (osc_g, input_g) = (level_of(p.osc_db), level_of(p.input_db));
        let level = if self.seq.playing && self.accent { ACCENT } else { 1.0 };
        let mut env_peak = self.env_peak;
        // the ENV gate's follower: 1 ms up, 20 ms down, on the input as it comes
        let (in_a, in_r) = (1.0 - exp(-1.0 / (0.001 * fs)), 1.0 - exp(-1.0 / (0.02 * fs)));
        let mut in_peak = self.in_peak;
        let track = p.pitch_from != PitchFrom::Note || p.vowel_in;
        let vowel = p.vowel_in;
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
            // the oscillator and the input, mixed at their levels; keys, hold and the sequencer
            // gate the mix (with no oscillator it closes fast, so a rhythm reads at sixteenths)
            let input_src = p.source == Source::Off;
            let osc = if input_src { 0.0 } else { self.osc.next(p.source, self.osc_hz / fs) * 0.5 * osc_g };
            let raw = osc + input[i] as f64 * input_g;
            // the ungated source, for a snapshot of its waveform
            self.snap_ring[self.snap_w] = raw as f32;
            self.snap_w = (self.snap_w + 1) & (SNAP_RING - 1);
            // the ADSR (0 … 1), then the accent
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
            // the accent's level glides (4 ms), so an accented step does not click
            self.osc_env += (level - self.osc_env) * acc_k;
            let x = raw * self.adsr * self.osc_env;
            // the resonator: the dry source crossfades into the comb, whose loop gain rises with
            // the amount
            let x = if p.reson > 0.0 {
                let fb = 0.85 + 0.145 * p.reson;
                // between noise power (^0.5, a noise input stays level) and a matched tone's
                // peak gain: a tone on the played note rises 7-10 dB, noise falls 6 dB (measured)
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
        // 2) tilt
        let c = if p.tilt_hz > 0.0 { exp(-2.0 * PI * p.tilt_hz / fs) } else { 0.0 };
        for i in 0..n {
            self.tilt_y = c * self.tilt_y + (1.0 - c) * self.dry[i] as f64;
            self.drive[i] = self.tilt_y as f32;
        }
        // 3) the mouth (and the old one, fading out after a rebuild)
        let scale = INJECT_REF * pow(10.0, p.drive_db / 20.0);
        if self.asleep {
            // asleep: the airway has rung out and nothing drives it (see `control`)
            self.wet[..n].fill(0.0);
        } else if let Some((_, left)) = &self.fade {
            // a rebuilt mouth: its input rises as it fades in (a step into it was a transient)
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
                let k = left.saturating_sub(i as u32) as f64 / FADE as f64; // weight of the old mouth
                let w = 0.5 - 0.5 * cos(PI * k); // raised cosine: 1 at the change → 0
                self.wet[i] = (self.wet[i] as f64 * (1.0 - w) + self.wet_old[i] as f64 * w) as f32;
            }
            *left = left.saturating_sub(n as u32);
            if *left == 0 {
                self.retired = self.fade.take().map(|(v, _)| v);
            }
        }
        // 4) sub, mix, gain
        let cs = exp(-2.0 * PI * p.sub_hz / fs);
        let g = pow(10.0, p.out_db / 20.0);
        for i in 0..n {
            let d = self.dry[i] as f64;
            self.sub1 = cs * self.sub1 + (1.0 - cs) * d;
            self.sub2 = cs * self.sub2 + (1.0 - cs) * self.sub1;
            let wet = self.wet[i] as f64 + p.sub_level * self.sub2;
            out[off + i] = ((p.mix * wet + (1.0 - p.mix) * d) * g) as f32;
        }
        let peak = self.dry[..n].iter().chain(&self.wet[..n]).fold(self.period_peak, |m, v| m.max(v.abs()));
        self.period_peak = peak;
    }
}

#[cfg(test)]
mod alloc_count {
    //! Counts heap allocations on this thread, so a test can assert that `process` makes none.
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        pub static COUNT: Cell<u64> = const { Cell::new(0) };
    }

    struct Counting;
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
            unsafe { System.realloc(p, l, n) }
        }
    }

    #[global_allocator]
    static A: Counting = Counting;

    pub fn count() -> u64 {
        COUNT.with(|c| c.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `process` and the control calls the audio thread makes do not allocate, in every mode.
    #[test]
    fn processing_does_not_allocate() {
        let dict = "WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n";
        let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 3.0 + 2.0 * (i as f64 * 0.4).sin()).collect();
        let combos = [TextStep::Note, TextStep::Clock].into_iter().flat_map(|st| [TextUnit::Syllable, TextUnit::Word].into_iter().flat_map(move |u| [(TextSay::Hold, 1.0), (TextSay::Hold, 0.5), (TextSay::Shot, 1.0)].into_iter().map(move |(y, l)| (st, u, y, l))));
        for (step, unit, say, len) in combos {
            let mode = (step, unit, say, len);
            let mut s = Suu::new(48000.0);
            s.load_dict(dict);
            s.set_params(Params { text_step: step, text_unit: unit, text_say: say, text_len: len, ..Params::default() });
            s.set_text("wow yeah now wow");
            s.set_profile(&wave, true);
            s.set_params(Params { gate: true, morph: 0.6, reson: 0.5, folds: 0.5, glide_s: 0.1, pitch_from: PitchFrom::Input, vowel_in: true, sustain: 0.6, ..s.params() });
            for t in 0..N_MOD {
                s.set_mod(t, Mod { depth: 0.4, rate_hz: 1.0 + t as f64, sync: 0.0, shape: Shape::Random, smooth: 0.5 });
            }
            let input = vec![0.1f32; 256];
            let mut out = vec![0f32; 256];
            // warm-up, then everything the audio thread does: notes, steps, sleep and waking
            for _ in 0..200 {
                s.process(&input, &mut out);
            }
            // the counter sees an allocation (so a pass below means none happened)
            let c = alloc_count::count();
            std::hint::black_box(vec![0u8; 64]);
            assert!(alloc_count::count() > c);
            let before = alloc_count::count();
            s.set_playing(true);
            for i in 0..2000 {
                s.process(&input[..(i % 7) * 32 + 32], &mut out[..(i % 7) * 32 + 32]);
                if i == 700 {
                    s.set_playing(false);
                    s.set_params(Params { gate: false, folds: 0.0, ..s.params() });
                }
                if i == 1500 {
                    s.retrigger();
                    s.set_params(Params { gate: true, note: 50.0, ..s.params() });
                }
            }
            let mut snap = [0.0; HORN_POINTS];
            s.snapshot(&mut snap);
            s.set_profile(&wave, false);
            assert_eq!(alloc_count::count() - before, 0, "{mode:?}: process allocated");
        }
    }

    fn run(s: &mut Suu, block: usize, n: usize) -> Vec<f32> {
        let input = vec![0f32; n];
        let mut out = vec![0f32; n];
        for (o, i) in out.chunks_mut(block).zip(input.chunks(block)) {
            s.process(i, o);
        }
        out
    }

    fn playing() -> Suu {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, ..Params::default() });
        s
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0f32, |m, v| if v.is_finite() { m.max(v.abs()) } else { f32::INFINITY })
    }

    #[test]
    fn block_size_does_not_change_the_output() {
        let a = run(&mut playing(), 128, 24000);
        let b = run(&mut playing(), 37, 24000);
        assert_eq!(a, b);
    }

    #[test]
    fn sequencer_is_block_size_independent_too() {
        let mk = || {
            let mut s = Suu::new(48000.0);
            s.set_params(Params { bpm: 137.0, ..Params::default() });
            s.set_playing(true);
            s
        };
        assert_eq!(run(&mut mk(), 128, 48000), run(&mut mk(), 53, 48000));
    }

    #[test]
    fn supersaw_through_the_mouth_is_audible_and_bounded() {
        let out = run(&mut playing(), 128, 48000);
        let rms = (out[24000..].iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / 24000.0).sqrt();
        assert!(peak(&out) < 1.0, "peak {}", peak(&out));
        assert!(rms > 0.005, "rms {rms}");
    }

    #[test]
    fn a_size_change_neither_drops_out_nor_clicks() {
        let mut s = playing();
        let a = run(&mut s, 128, 24000);
        let level = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
        let before = level(&a[19200..]);
        s.set_size(1.4);
        let b = run(&mut s, 128, 4800);
        // the 100 ms after the change keep most of the level (no silence while a new mouth builds)
        assert!(level(&b) > 0.4 * before, "after {} before {before}", level(&b));
        // no sample-to-sample jump larger than the steady signal's own
        let jump = |x: &[f32]| x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(jump(&b) < 2.0 * jump(&a[19200..]), "jump {} vs {}", jump(&b), jump(&a[19200..]));
    }

    #[test]
    fn extremes_stay_finite_and_bounded() {
        for &size in &[0.4, 0.55, 0.7, 1.0, 2.0, 4.0] {
            for &(x, y, round, nasal, damping, folds, breath) in &[(-0.3, -0.3, -1.0, 1.5, 0.1, 2.0, 6.0), (1.3, 1.3, 1.0, 0.0, 20.0, 0.0, 0.0), (0.5, 0.5, 0.0, 0.7, 1.0, 1.0, 3.0)] {
                let mut s = Suu::new(48000.0);
                s.set_size(size);
                s.set_params(Params { gate: true, vowel_x: x, vowel_y: y, round, nasal, damping, folds, breath, drive_db: 40.0, reson: x.clamp(0.0, 1.0), ..Params::default() });
                let out = run(&mut s, 128, 24000);
                let pk = peak(&out);
                assert!(pk.is_finite() && pk < 1.0, "size {size} x {x} y {y}: peak {pk}");
            }
        }
    }

    fn text_params(step: TextStep, unit: TextUnit, say: TextSay) -> Params {
        Params { text_step: step, text_unit: unit, text_say: say, ..Params::default() }
    }

    #[test]
    fn notes_say_the_text_in_every_unit_and_way() {
        let dict = "WOW  W AW1\nYEAH  Y AE1\n";
        for unit in [TextUnit::Syllable, TextUnit::Word] {
            for say in [TextSay::Hold, TextSay::Shot] {
                let mut s = Suu::new(48000.0);
                s.load_dict(dict);
                s.set_params(text_params(TextStep::Note, unit, say));
                assert_eq!(s.set_text("wow yeah"), 2);
                s.set_playing(true);
                let out = run(&mut s, 128, 48000);
                assert!(peak(&out).is_finite() && peak(&out) < 1.0);
                assert!(s.said >= 2, "{unit:?} {say:?}: said {}", s.said);
            }
        }
    }

    #[test]
    fn on_notes_seq_starts_the_text_over_each_pass() {
        // 5 steps, all on, against 3 syllables: free, the second pass starts at the third
        // syllable; following the sequence, every pass starts at the first
        let first_units = |seq: bool| {
            let mut s = Suu::new(48000.0);
            s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
            s.set_params(Params { text_seq: seq, bpm: 120.0, ..Params::default() });
            s.set_text("wow yeah now");
            for i in 0..STEPS {
                s.set_step(i, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.5 });
            }
            s.set_seq(5, 4.0, 1.0);
            s.set_playing(true);
            let mut firsts = Vec::new();
            let mut said = 0;
            for _ in 0..(48000 * 3 / 32) {
                run(&mut s, 32, 32);
                if s.said != said {
                    said = s.said;
                    if s.seq.step == 0 {
                        firsts.push(s.last_unit);
                    }
                }
            }
            firsts
        };
        let (free, seq) = (first_units(false), first_units(true));
        assert!(seq.len() >= 3 && seq.iter().all(|&u| u == 0), "seq {seq:?}");
        assert!(free.iter().any(|&u| u != 0), "free {free:?}");
    }

    #[test]
    fn on_the_clock_a_short_length_closes_each_unit_early() {
        let held_until = |len: f64| {
            let mut s = Suu::new(48000.0);
            s.load_dict("WOW  W AW1\nYEAH  Y AE1\n");
            s.set_params(Params { text_len: len, ..text_params(TextStep::Clock, TextUnit::Syllable, TextSay::Hold) });
            s.set_text("wow yeah");
            // the longest-held gesture of a tick's plan, as a fraction of the tick
            s.plan.tick_plan[1].iter().map(|g| g.t1).fold(0.0, f64::max) / s.plan.tick_s
        };
        let (half, full) = (held_until(0.5), held_until(0.9));
        // (the coda reaches a little past the vowel's end)
        assert!(half < 0.65 && full > 0.9, "len 0.5 holds to {half:.2}, 0.9 to {full:.2} of a tick");
    }

    #[test]
    fn the_sequencer_gates_an_external_sound() {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { source: Source::Off, bpm: 120.0, ..Params::default() });
        for i in 0..STEPS {
            s.set_step(i, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.5 });
        }
        s.set_playing(true);
        // a steady external tone
        let input: Vec<f32> = (0..48000).map(|i| (0.3 * (2.0 * std::f64::consts::PI * 110.0 * i as f64 / 48000.0).sin()) as f32).collect();
        let mut out = vec![0f32; 48000];
        for (o, x) in out.chunks_mut(128).zip(input.chunks(128)) {
            s.process(x, o);
        }
        // sixteenths at 120 bpm (125 ms), gate 50 %: the level rises and falls each step
        let rms = |a: &[f32]| (a.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / a.len() as f64).sqrt();
        let win: Vec<f64> = out[12000..].chunks(1500).map(rms).collect(); // 31 ms windows
        let (lo, hi) = win.iter().fold((f64::MAX, 0f64), |(l, h), v| (l.min(*v), h.max(*v)));
        assert!(hi > 4.0 * lo, "gated: max {hi} min {lo}");
    }

    #[test]
    fn keys_gate_an_external_sound() {
        let tone: Vec<f32> = (0..24000).map(|i| (0.3 * (2.0 * std::f64::consts::PI * 110.0 * i as f64 / 48000.0).sin()) as f32).collect();
        let level = |gate: bool| {
            let mut s = Suu::new(48000.0);
            s.set_params(Params { source: Source::Off, gate, ..Params::default() });
            let mut out = vec![0f32; 24000];
            for (o, x) in out.chunks_mut(128).zip(tone.chunks(128)) {
                s.process(x, o);
            }
            (out[12000..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 12000.0).sqrt()
        };
        let (closed, open) = (level(false), level(true));
        assert!(open > 0.005 && closed < 1e-4 * open, "closed {closed} open {open}");
    }

    #[test]
    fn the_drawn_horn_morphs_the_airway_and_moves_smoothly() {
        let areas = |s: &Suu| {
            let mut ui = Vec::new();
            s.ui_state(&mut ui);
            ui[23..23 + UI_AREAS].to_vec()
        };
        let mut s = playing();
        run(&mut s, 128, 9600);
        let vowel = areas(&s);
        // a flat 5 cm² tube, fully morphed: every section reaches it (the slew limits take time)
        s.set_profile(&[5.0; HORN_POINTS], false);
        s.set_params(Params { gate: true, morph: 1.0, ..Params::default() });
        let out = run(&mut s, 128, 24000);
        let horn = areas(&s);
        assert!(horn.iter().all(|a| (a - 5.0).abs() < 0.05), "{horn:?}");
        assert!(vowel.iter().zip(&horn).any(|(a, b)| (a - b).abs() > 1.0));
        // open doubles it, and the move is bounded and without a click
        s.set_params(Params { gate: true, morph: 1.0, open: 2.0, ..Params::default() });
        let out2 = run(&mut s, 128, 24000);
        assert!(areas(&s).iter().all(|a| (a - 10.0).abs() < 0.1));
        let jump = |x: &[f32]| x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(peak(&out2) < 1.0 && jump(&out2) < 3.0 * jump(&out[12000..]), "jump {} vs {}", jump(&out2), jump(&out[12000..]));
        // block size still does not matter, with the horn moving
        let mk = || {
            let mut s = playing();
            s.set_params(Params { gate: true, morph: 0.7, ..Params::default() });
            s.set_mod(7, Mod { depth: 0.8, rate_hz: 2.0, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
            s
        };
        assert_eq!(run(&mut mk(), 128, 24000), run(&mut mk(), 45, 24000));
    }

    #[test]
    fn a_wave_horn_wraps_so_it_can_travel() {
        // one sine period of area along the tract; a shift of half a period inverts it, and a
        // shift of −0.5 lands where +0.5 does (no seam where the wave wraps)
        let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 3.0 + 2.0 * (2.0 * std::f64::consts::PI * i as f64 / HORN_POINTS as f64).sin()).collect();
        let areas = |shift: f64| {
            let mut s = playing();
            s.set_profile(&wave, true);
            s.set_params(Params { gate: true, morph: 1.0, shift, ..Params::default() });
            run(&mut s, 128, 24000);
            let mut ui = Vec::new();
            s.ui_state(&mut ui);
            ui[23..23 + UI_AREAS].to_vec()
        };
        let (a, b, c) = (areas(0.0), areas(0.5), areas(-0.5));
        for k in 0..UI_AREAS {
            assert!((a[k] - 3.0 + (b[k] - 3.0)).abs() < 0.15, "half a period inverts: {k} {} {}", a[k], b[k]);
            assert!((b[k] - c[k]).abs() < 1e-4, "no seam: {k}");
        }
        // travelling (a saw on Shift, one pass a second): no click where the saw jumps back
        let mk = |depth: f64| {
            let mut s = playing();
            s.set_profile(&wave, true);
            s.set_params(Params { gate: true, morph: 1.0, ..Params::default() });
            s.set_mod(7, Mod { depth, rate_hz: 1.0, sync: 0.0, shape: Shape::Saw, smooth: 0.0 });
            run(&mut s, 128, 96000)[24000..].to_vec()
        };
        let jump = |x: &[f32]| x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()));
        let (still, moving) = (mk(0.0), mk(1.0));
        assert!(jump(&moving) < 1.5 * jump(&still), "jump moving {} still {}", jump(&moving), jump(&still));
    }

    #[test]
    fn it_sleeps_when_silent_and_wakes_on_a_note() {
        let run_gated = |block: usize| {
            let mut s = Suu::new(48000.0);
            s.set_params(Params { gate: true, ..Params::default() });
            let mut out = run(&mut s, block, 14400);
            s.set_params(Params { gate: false, ..Params::default() });
            out.extend(run(&mut s, block, 48000));
            let slept = s.asleep;
            s.set_params(Params { gate: true, ..Params::default() });
            let again = run(&mut s, block, 9600);
            out.extend(&again);
            (out, slept, again)
        };
        let (a, slept, again) = run_gated(128);
        assert!(slept, "silent for a second: asleep");
        let rms = (again[4800..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 4800.0).sqrt();
        assert!(rms > 0.005, "a note wakes it: rms {rms}");
        // still the same at any block size
        assert_eq!(a, run_gated(37).0);
    }

    #[test]
    fn smooth_takes_the_jumps_out_of_random_square_and_saw() {
        // the largest step in a target between control periods, sharp vs smooth
        let largest_step = |shape: Shape, smooth: f64| {
            let mut s = playing();
            s.set_mod(4, Mod { depth: 1.0, rate_hz: 4.0, sync: 0.0, shape, smooth });
            let (mut last, mut most) = (f64::NAN, 0f64);
            for _ in 0..3000 {
                run(&mut s, 32, 32);
                if last.is_finite() {
                    most = most.max((s.eff[4] - last).abs());
                }
                last = s.eff[4];
            }
            most
        };
        for shape in [Shape::Random, Shape::Square, Shape::Saw] {
            let (sharp, smooth) = (largest_step(shape, 0.0), largest_step(shape, 1.0));
            assert!(smooth < 0.05 * sharp.max(0.2), "{shape:?}: sharp {sharp:.3}, smooth {smooth:.4}");
        }
        // random at full smooth still reaches each new value by the end of its cycle
        let mut s = playing();
        s.set_mod(0, Mod { depth: 1.0, rate_hz: 2.0, sync: 0.0, shape: Shape::Random, smooth: 1.0 });
        run(&mut s, 32, 48000);
        assert!((s.mod_out[0] - s.mod_hold[0]).abs() <= (s.mod_hold[0] - s.mod_from[0]).abs() + 1e-9);
    }

    #[test]
    fn the_folds_sound_only_with_a_note() {
        // the owner's settings: folds 68 %, breath ×2.79, follow 0 (full lung pressure always)
        let level = |gate: bool, block: usize| {
            let mut s = Suu::new(48000.0);
            s.set_params(Params { folds: 0.68, breath: 2.79, follow: 0.0, gate, ..Params::default() });
            let out = run(&mut s, block, 96000);
            let rms = (out[48000..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 48000.0).sqrt();
            (20.0 * rms.max(1e-12).log10(), out)
        };
        let ((off, _), (on, a)) = (level(false, 128), level(true, 128));
        // no note: only the breath's own air is left (it was −11.5 dBFS with the folds sounding)
        assert!(off < -50.0 && on > -30.0, "no note {off:.1} dB, a note {on:.1} dB");
        assert_eq!(a, level(true, 37).1);
    }

    #[test]
    fn once_says_the_text_one_time_per_pass_of_the_sequence() {
        // 3 syllables at 1/16 (0.375 s at 120 bpm) in a 16-step pattern (2 s): looping, the
        // text comes several times a pass; once, one time per pass (3 passes in 6 s)
        let passes = |step: TextStep, say: TextSay, len: f64, once: bool| {
            let mut s = Suu::new(48000.0);
            s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
            s.set_params(Params { bpm: 120.0, text_sync: 0.25, text_once: once, text_say: say, text_len: len, ..text_params(step, TextUnit::Syllable, TextSay::Hold) });
            s.set_text("wow yeah now");
            s.set_playing(true);
            let (mut n, mut last, said0) = (0, s.next_pass, s.said);
            for _ in 0..(48000 * 6 / 32) {
                run(&mut s, 32, 32);
                if s.next_pass != last {
                    n += 1;
                    last = s.next_pass;
                }
            }
            // joined: passes scheduled; unit by unit or on notes: units said, in passes of 3
            if step == TextStep::Clock && say == TextSay::Hold && len == 1.0 { n } else { (s.said - said0) as usize / 3 }
        };
        // on the joined clock, unit by unit (shots), and on notes (16 steps on: 16 notes a pass)
        for (step, say, len) in [(TextStep::Clock, TextSay::Hold, 1.0), (TextStep::Clock, TextSay::Shot, 1.0), (TextStep::Note, TextSay::Hold, 1.0)] {
            let (looping, once) = (passes(step, say, len, false), passes(step, say, len, true));
            assert!(looping >= 9 && (3..=4).contains(&once), "{step:?} {say:?}: looping {looping}, once {once}");
        }
    }

    #[test]
    fn speech_speed_stretches_a_word_and_keeps_the_loop_on_its_beats() {
        let plans = |speech: f64| {
            let mut s = Suu::new(48000.0);
            s.load_dict("WOW  W AW1\nYEAH  Y AE1\nSTOP  S T AA1 P\n");
            s.set_params(Params { speech, ..text_params(TextStep::Note, TextUnit::Word, TextSay::Shot) });
            s.set_text("stop wow");
            let span = |gs: &[Gesture]| gs.iter().map(|g| g.t1 + g.ramp_out).fold(0.0, f64::max) - gs.iter().map(|g| g.t0 - g.ramp_in).fold(f64::MAX, f64::min);
            (span(&s.plan.shot[0]), s.plan.pass_len)
        };
        let ((slow, len_slow), (normal, len), (fast, len_fast)) = (plans(0.5), plans(1.0), plans(2.0));
        assert!((slow / normal - 2.0).abs() < 0.05 && (normal / fast - 2.0).abs() < 0.05, "word: {slow:.3} {normal:.3} {fast:.3}");
        // the loop's pass keeps its length (its rate, not the speech, sets the rhythm)
        assert!(len_slow == len && len_fast == len);
    }

    /// rms (dBFS) of the second second, a note held.
    fn level_db(p: Params, input: f32) -> f64 {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, ..p });
        let inp: Vec<f32> = (0..96000).map(|i| input * (2.0 * ((i as f64 * 110.0 / 48000.0) % 1.0) - 1.0) as f32).collect();
        let mut out = vec![0f32; 96000];
        for (o, i) in out.chunks_mut(128).zip(inp.chunks(128)) {
            s.process(i, o);
        }
        20.0 * (out[48000..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 48000.0).sqrt().max(1e-12).log10()
    }

    #[test]
    fn the_sources_mix_at_matched_levels() {
        let osc = level_db(Params { folds: 0.0, ..Params::default() }, 0.0);
        let folds = level_db(Params { source: Source::Off, folds: 1.0, ..Params::default() }, 0.0);
        let input = level_db(Params { source: Source::Off, folds: 0.0, ..Params::default() }, 0.5);
        let both = level_db(Params { folds: 1.0, ..Params::default() }, 0.0);
        // the oscillator at 0 dB and Folds at 100 % alike (calibrated), and together as two
        // equal sources: louder by a few dB, not the folds taking over (they rose 20 dB before)
        assert!((osc - folds).abs() < 3.0, "osc {osc:.1}, folds {folds:.1}");
        assert!(both - osc.max(folds) < 4.5, "both {both:.1}");
        // a saw input at half scale sounds like the oscillator; −60 dB is silent
        assert!((input - osc).abs() < 6.0, "input {input:.1}, osc {osc:.1}");
        assert!(level_db(Params { osc_db: -60.0, folds: 0.0, ..Params::default() }, 0.0) < -100.0);
        // the folds' level no longer follows the oscillator's
        let folds_quiet_osc = level_db(Params { folds: 1.0, osc_db: -60.0, ..Params::default() }, 0.0);
        let folds_with_osc_low = level_db(Params { folds: 1.0, osc_db: -30.0, ..Params::default() }, 0.0);
        assert!((folds_quiet_osc - folds_with_osc_low).abs() < 1.5, "{folds_quiet_osc:.1} vs {folds_with_osc_low:.1}");
    }

    #[test]
    fn the_text_gate_lets_through_part_of_each_unit() {
        // the share of control periods the sound is let through, a note held throughout
        let open_share = |step: TextStep, len: f64, gate: f64| {
            let mut s = Suu::new(48000.0);
            s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
            s.set_params(Params { gate: true, bpm: 120.0, text_sync: 0.5, text_len: len, text_gate: gate, ..text_params(step, TextUnit::Syllable, TextSay::Hold) });
            s.set_text("wow yeah now");
            if step == TextStep::Note {
                // a note each quarter second, held through
                for i in 0..STEPS {
                    s.set_step(i, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 1.0 });
                }
                s.set_seq(16, 2.0, 1.0);
                s.set_playing(true);
            }
            run(&mut s, 32, 24000);
            let (mut open, mut all) = (0, 0);
            for _ in 0..(48000 * 2 / 32) {
                run(&mut s, 32, 32);
                all += 1;
                open += s.gate_eff as u32;
            }
            open as f64 / all as f64
        };
        for (step, len) in [(TextStep::Clock, 0.5), (TextStep::Clock, 1.0)] {
            let (full, half) = (open_share(step, len, 1.0), open_share(step, len, 0.5));
            assert!(full > 0.99 && (half - 0.5).abs() < 0.08, "{step:?} len {len}: gate 1 {full:.2}, gate 0.5 {half:.2}");
        }
        // on notes, a share of each syllable's spoken length: less than with no cut
        let (full, cut) = (open_share(TextStep::Note, 1.0, 1.0), open_share(TextStep::Note, 1.0, 0.3));
        assert!(full > 0.95 && cut < 0.6 * full, "notes: {full:.2} vs {cut:.2}");
    }

    #[test]
    fn a_gate_of_zero_is_silent() {
        // the same as no note at all, and that is quiet from the first sample (warmed up)
        let mut s = Suu::new(48000.0);
        s.set_seq(16, 4.0, 0.0);
        s.set_playing(true);
        let gated = run(&mut s, 128, 48000);
        let none = run(&mut Suu::new(48000.0), 128, 48000);
        assert_eq!(gated, none);
        assert!(peak(&none) < 1e-4, "a new instance is silent: peak {}", peak(&none));
    }

    #[test]
    fn the_text_loop_starts_over_with_the_sequence() {
        let dict = "WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n";
        // 3 syllables at 1/8 (0.25 s at 120 bpm) against a 5-step pattern of sixteenths
        // (0.625 s): free, the text drifts against the pattern; following it, each pattern pass
        // restarts the text exactly one pattern apart
        let restarts = |follow: bool| {
            let mut s = Suu::new(48000.0);
            s.load_dict(dict);
            s.set_params(Params { bpm: 120.0, text_sync: 0.5, text_seq: follow, ..text_params(TextStep::Clock, TextUnit::Syllable, TextSay::Hold) });
            s.set_text("wow yeah now");
            s.set_seq(5, 4.0, 1.0);
            s.set_playing(true);
            let mut times = Vec::new();
            for _ in 0..(48000 * 4 / 32) {
                run(&mut s, 32, 32);
                if times.last() != Some(&s.last_restart) && s.last_restart > 0.0 {
                    times.push(s.last_restart);
                }
            }
            times
        };
        let t = restarts(true);
        assert!(t.len() >= 5, "{t:?}");
        for w in t.windows(2) {
            assert!((w[1] - w[0] - 0.625).abs() < 2e-3, "pattern apart: {t:?}");
        }
        assert!(restarts(false).is_empty());
    }

    #[test]
    fn a_snapshot_is_one_period_of_the_source() {
        // a saw at 110 Hz: one period comes back as a ramp (low → high, then the drop at the end)
        let mut s = Suu::new(48000.0);
        s.set_params(Params { source: Source::Saw, note: 45.0, ..Params::default() });
        run(&mut s, 128, 9600);
        let mut snap = [0.0; HORN_POINTS];
        assert!(s.snapshot(&mut snap));
        let rising = snap.windows(2).filter(|w| w[1] > w[0]).count();
        assert!(rising >= HORN_POINTS - 3, "a ramp: {snap:?}");
        assert!(snap.iter().all(|v| v.abs() <= 1.0 + 1e-9) && snap.iter().any(|v| v.abs() > 0.99));
        // silence: nothing to take
        let mut q = Suu::new(48000.0);
        q.set_params(Params { source: Source::Off, ..Params::default() });
        run(&mut q, 128, 4800);
        assert!(!q.snapshot(&mut snap));
    }

    /// Normalised autocorrelation of `x` at `lag`.
    fn autocorr(x: &[f32], lag: usize) -> f64 {
        let (mut num, mut den) = (0.0, 0.0);
        for i in lag..x.len() {
            num += x[i] as f64 * x[i - lag] as f64;
            den += (x[i] as f64).powi(2);
        }
        num / den
    }

    #[test]
    fn the_resonator_gives_an_outside_sound_the_played_pitch() {
        // white noise in (a fixed seed), A3 (220 Hz) held
        let mut r = 0x1234_5678u32;
        let noise: Vec<f32> = (0..48000)
            .map(|_| {
                r ^= r << 13;
                r ^= r >> 17;
                r ^= r << 5;
                (r as f64 / u32::MAX as f64 - 0.5) as f32 * 0.4
            })
            .collect();
        let render = |reson: f64| {
            let mut s = Suu::new(48000.0);
            s.set_params(Params { source: Source::Off, gate: true, note: 57.0, reson, ..Params::default() });
            let mut out = vec![0f32; 48000];
            for (o, x) in out.chunks_mut(128).zip(noise.chunks(128)) {
                s.process(x, o);
            }
            out[24000..].to_vec()
        };
        let lag = (48000.0f64 / 220.0).round() as usize;
        let (off, on) = (render(0.0), render(0.8));
        let (a_off, a_on) = (autocorr(&off, lag), autocorr(&on, lag));
        assert!(a_on > 0.6 && a_on > a_off + 0.3, "periodicity at 220 Hz: off {a_off:.2}, on {a_on:.2}");
        assert!(peak(&on) < 1.0 && peak(&on) > 0.01, "peak {}", peak(&on));
        let rms = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
        println!("rms off {:.4} on {:.4}, autocorr off {a_off:.2} on {a_on:.2}", rms(&off), rms(&on));
    }

    /// Samples until the tongue body is within 5 % of a jump from the vowel i to ɑ, and the
    /// largest sample step in the output meanwhile.
    fn vowel_settle(glide_s: f64) -> (usize, f32) {
        let mut s = Suu::new(48000.0);
        let p = Params { gate: true, glide_s, vowel_x: 0.0, vowel_y: 0.0, ..Params::default() };
        s.set_params(p);
        run(&mut s, 128, 9600);
        let from = s.voice.artic.x.tb_y;
        s.set_params(Params { vowel_x: 1.0, vowel_y: 1.0, ..p });
        let to = s.voice.artic.target.tb_y;
        let (mut n, mut jump, mut last) = (0, 0f32, 0f32);
        while (s.voice.artic.x.tb_y - to).abs() > 0.05 * (to - from).abs() && n < 48000 {
            for v in run(&mut s, 32, 32) {
                jump = jump.max((v - last).abs());
                last = v;
            }
            n += 32;
        }
        (n, jump)
    }

    #[test]
    fn glide_can_go_one_way_only() {
        let base = Params { note: 45.0, glide_s: 0.25, ..Params::default() };
        let up = |dir: GlideDir, to: f64| glide_samples(Params { glide_dir: dir, ..base }, to);
        // up only: rising glides (12000 samples), falling jumps; down only the other way round
        assert!((up(GlideDir::Up, 57.0) as i64 - 12000).abs() <= 64 && up(GlideDir::Up, 33.0) <= 64);
        assert!((up(GlideDir::Down, 33.0) as i64 - 12000).abs() <= 64 && up(GlideDir::Down, 57.0) <= 64);
    }

    #[test]
    fn glide_at_zero_moves_the_vowel_fast_and_without_a_click() {
        let (fast, j_fast) = vowel_settle(0.0);
        let (normal, j_normal) = vowel_settle(0.03);
        let (slow, _) = vowel_settle(0.12);
        println!("vowel settles: glide 0 {fast}, 30 ms {normal}, 120 ms {slow} samples; jumps {j_fast} {j_normal}");
        assert!(fast * 3 < normal && normal * 3 < slow, "{fast} {normal} {slow}");
        assert!(j_fast < 1.5 * j_normal, "jump fast {j_fast} normal {j_normal}");
    }

    /// Samples until the oscillator's pitch reaches `to` after a jump there.
    fn glide_samples(p: Params, to: f64) -> usize {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, ..p });
        run(&mut s, 128, 4800);
        s.set_params(Params { gate: true, note: to, ..p });
        let mut n = 0;
        while s.pitch != to && n < 480000 {
            run(&mut s, 32, 32);
            n += 32;
        }
        n
    }

    #[test]
    fn glide_takes_its_time_free_or_synced() {
        let base = Params { note: 45.0, ..Params::default() };
        // free: 0.25 s, whatever the interval (linear in semitones)
        for to in [57.0, 33.0, 46.0] {
            let n = glide_samples(Params { glide_s: 0.25, ..base }, to);
            assert!((n as i64 - 12000).abs() <= 64, "to {to}: {n} samples");
        }
        // synced: a beat at 120 bpm = 0.5 s
        let n = glide_samples(Params { glide_s: 0.25, glide_sync: 1.0, bpm: 120.0, ..base }, 57.0);
        assert!((n as i64 - 24000).abs() <= 64, "synced: {n} samples");
        // off: at once
        assert!(glide_samples(Params { glide_s: 0.0, ..base }, 57.0) <= 64);
    }

    #[test]
    fn the_global_gate_scales_every_step_alike() {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { bpm: 120.0, ..Params::default() });
        s.set_step(0, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.4 });
        s.set_step(1, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.8 });
        s.set_seq(2, 4.0, 0.5);
        // sixteenths at 120 bpm: 6000 samples a step; open gate lengths 0.2 and 0.4 of a step
        let mut open = [0usize; 2];
        s.set_playing(true);
        for _ in 0..(12000 / 32) {
            run(&mut s, 32, 32);
            if s.gate_now {
                open[s.seq.step] += 32;
            }
        }
        assert!((open[0] as i64 - 1200).abs() <= 64 && (open[1] as i64 - 2400).abs() <= 64, "{open:?}");
        // above 100 % a step is capped at the whole step
        s.set_seq(2, 4.0, 2.0);
        s.seq.step = 0;
        assert!((s.gate_len() - 0.8).abs() < 1e-12);
        s.seq.step = 1;
        assert_eq!(s.gate_len(), 1.0);
    }

    #[test]
    fn the_text_loop_runs_at_its_own_rate() {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { bpm: 120.0, text_sync: 0.5, ..Params::default() });
        assert_eq!(s.loop_beats(), 0.5);
        s.set_params(Params { bpm: 120.0, text_sync: 0.0, text_rate_hz: 4.0, ..Params::default() });
        assert_eq!(s.loop_beats(), 0.5);
        // never shorter than 60 ms (a 1/32 at 300 bpm would be 25 ms)
        s.set_params(Params { bpm: 300.0, text_sync: 0.125, ..Params::default() });
        assert!((s.loop_beats() - 0.3).abs() < 1e-12);
        // and it sings: a looped word, a syllable per eighth
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\n");
        s.set_params(Params { gate: true, bpm: 120.0, text_sync: 0.5, ..Params::default() });
        s.set_params(Params { text_step: TextStep::Clock, ..s.params() });
        assert_eq!(s.set_text("wow yeah"), 2);
        let out = run(&mut s, 128, 48000);
        assert!(peak(&out).is_finite() && peak(&out) < 1.0 && peak(&out) > 0.01);
    }

    #[test]
    fn motion_moves_the_vowel_and_stays_block_size_independent() {
        let mk = || {
            let mut s = playing();
            s.set_mod(1, Mod { depth: 0.8, rate_hz: 1.0, sync: 1.0, shape: Shape::Sine, smooth: 0.0 });
            s.set_mod(5, Mod { depth: 0.5, rate_hz: 3.0, sync: 0.0, shape: Shape::Random, smooth: 0.0 });
            s
        };
        let mut a = mk();
        let out_a = run(&mut a, 128, 48000);
        assert_eq!(out_a, run(&mut mk(), 61, 48000));
        // one beat per cycle at 100 bpm: vowel Y swings around its base over a second
        let mut s = mk();
        let mut ys = Vec::new();
        for _ in 0..40 {
            run(&mut s, 128, 1200);
            let mut ui = Vec::new();
            s.ui_state(&mut ui);
            ys.push(ui[1]);
        }
        let (lo, hi) = ys.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
        assert!(hi - lo > 0.8, "vowel Y swing {lo}..{hi}");
    }

    #[test]
    fn steps_have_their_own_length_and_accent() {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { bpm: 120.0, ..Params::default() });
        s.set_seq(2, 4.0, 1.0);
        // step 0 short and plain, step 1 long and accented (sixteenths at 120 bpm: 6000 samples)
        s.set_step(0, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.2 });
        s.set_step(1, Step { on: true, note: 0, tie: false, vowel: -1, accent: true, len: 0.9 });
        s.set_playing(true);
        let out = run(&mut s, 128, 48000);
        let rms = |a: &[f32]| (a.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / a.len() as f64).sqrt();
        // in the second half: the middle of step 0 (gate closed) vs the middle of step 1 (open)
        let (a, b) = (rms(&out[24000 + 2400..24000 + 3600]), rms(&out[30000 + 2400..30000 + 3600]));
        assert!(b > 3.0 * a, "short step {a}, long accented step {b}");
    }

    /// The plug-in's split: a mouth built and a text planned elsewhere are put in place, and the
    /// old ones handed back, without allocating on the audio thread; a rebuilt mouth crossfades
    /// in without a jump.
    #[test]
    fn swapping_mouth_and_text_does_not_allocate() {
        let dict = "WOW\tW AW1\nYEAH\tY AE1\nNOW\tN AW1\n";
        let mut s = Suu::new(48000.0);
        s.load_dict(dict);
        s.set_params(Params { gate: true, ..Params::default() });
        let mut out = vec![0f32; 128];
        let inp = vec![0f32; 128];
        for _ in 0..100 {
            s.process(&inp, &mut out);
        }
        let plan = TextPlan::new("wow yeah now", s.g2p(), &s.params());
        assert_eq!(plan.syllables(), 3);
        let seed = s.mouth_seed(1.6);
        let mouth = Mouth::build(&seed);
        assert_eq!(mouth.scale(), 1.6);
        let before = alloc_count::count();
        let old = s.set_text_plan(plan);
        assert!(s.set_mouth(mouth).is_ok());
        let mut peak_step = 0f32;
        let mut last = out[127];
        let mut retired = None;
        for _ in 0..100 {
            s.process(&inp, &mut out);
            for &y in out.iter() {
                peak_step = peak_step.max((y - last).abs());
                last = y;
            }
            if retired.is_none() {
                retired = s.take_retired();
            }
        }
        assert_eq!(alloc_count::count(), before, "the swap or processing allocated");
        assert_eq!(old.syllables(), 0);
        assert!(retired.is_some(), "the old airway was not handed back");
        assert_eq!(s.size(), 1.6);
        assert!(peak_step < 0.2, "a jump of {peak_step} at the swap");
        // a second change waits while the first fades: refused, then accepted
        let m2 = Mouth::build(&s.mouth_seed(0.8));
        assert!(s.set_mouth(m2).is_ok());
        let m3 = Mouth::build(&s.mouth_seed(1.0));
        assert!(s.set_mouth(m3).is_err());
    }

    /// A text plan goes stale when what its timing depends on changes, and not otherwise.
    #[test]
    fn text_plan_goes_stale() {
        let mut s = Suu::new(48000.0);
        s.load_dict("WOW\tW AW1\n");
        assert!(!s.text_stale(), "no text, nothing to replan");
        s.set_text("wow wow");
        assert!(!s.text_stale());
        s.set_params(Params { vowel_x: 0.2, mix: 0.5, ..s.params() });
        assert!(!s.text_stale(), "the pad and the mix do not touch the plan");
        s.set_params(Params { text_rate_hz: 5.0, ..s.params() });
        assert!(!s.text_stale(), "the free rate does not count while synced");
        for change in [Params { bpm: 120.0, ..s.params() }, Params { speech: 2.0, ..s.params() }, Params { text_unit: TextUnit::Word, ..s.params() }, Params { text_len: 0.5, ..s.params() }, Params { text_sync: 0.0, ..s.params() }] {
            let keep = s.params();
            s.set_params(change);
            assert!(s.text_stale());
            s.set_params(keep);
            assert!(!s.text_stale());
        }
    }

    /// ENV gate: an input above the threshold opens the sound, each onset is a new note for the
    /// text, and it closes after the release (free, or synced to the tempo).
    #[test]
    fn env_gate_opens_on_the_input() {
        let fs = 48000.0;
        let mut s = Suu::new(fs);
        s.load_dict("WOW\tW AW1\nYEAH\tY AE1\n");
        s.set_params(Params { source: Source::Off, env_gate: true, env_thresh_db: -30.0, env_release_s: 0.1, ..Params::default() });
        s.set_text("wow yeah wow yeah");
        // bursts: 0.2 s of a −12 dBFS tone, 0.3 s of silence, three times
        let burst = |t: usize| if (t % 24000) < 9600 { 0.25 * (t as f32 * 0.05).sin() } else { 0.0 };
        let mut open = Vec::new();
        let mut out = [0f32; 128];
        let mut inp = [0f32; 128];
        for b in 0..(3 * 24000 / 128) {
            for (i, x) in inp.iter_mut().enumerate() {
                *x = burst(b * 128 + i);
            }
            s.process(&inp, &mut out);
            open.push(s.env_open());
        }
        // block index → seconds: 128 / 48000
        let at = |sec: f64| open[(sec * fs / 128.0) as usize];
        assert!(!at(0.0) || at(0.01), "opens at once");
        assert!(at(0.1) && at(0.25), "open through the burst and the release");
        assert!(!at(0.4), "closed after the release");
        assert!(at(0.6), "open on the next burst");
        assert_eq!(s.said, 3, "a new unit on each of three onsets");
        // synced: the release is a beat (0.6 s at 100 bpm): still open in the gap
        s.set_params(Params { env_release_sync: 1.0, ..s.params() });
        let mut open2 = Vec::new();
        for b in 0..(24000 / 128) {
            for (i, x) in inp.iter_mut().enumerate() {
                *x = burst(b * 128 + i);
            }
            s.process(&inp, &mut out);
            open2.push(s.env_open());
        }
        assert!(open2.iter().all(|&o| o), "a beat's release bridges the 0.3 s gap");
        // below the threshold: never opens
        let mut q = Suu::new(fs);
        q.set_params(Params { source: Source::Off, env_gate: true, env_thresh_db: -10.0, ..Params::default() });
        for b in 0..(24000 / 128) {
            for (i, x) in inp.iter_mut().enumerate() {
                *x = burst(b * 128 + i);
            }
            q.process(&inp, &mut out);
            assert!(!q.env_open());
        }
    }

    #[test]
    fn pitch_follows_the_input() {
        let fs = 48000.0;
        // a saw 30 cents above A3 (57.3)
        let f0 = 220.0 * pow(2.0, 0.3 / 12.0);
        let run = |s: &mut Suu, secs: f64, ph: &mut f64| {
            let (mut inp, mut out) = ([0f32; 128], [0f32; 128]);
            for _ in 0..(secs * fs / 128.0) as usize {
                for x in inp.iter_mut() {
                    *ph = (*ph + f0 / fs) % 1.0;
                    *x = (0.3 * (2.0 * *ph - 1.0)) as f32;
                }
                s.process(&inp, &mut out);
            }
        };
        let mut ph = 0.0;
        let mut s = Suu::new(fs);
        s.set_params(Params { gate: true, note: 40.0, pitch_from: PitchFrom::Input, ..Params::default() });
        run(&mut s, 0.3, &mut ph);
        assert!((s.pitch - 57.3).abs() < 0.05, "INPUT: {}", s.pitch);
        s.set_params(Params { pitch_from: PitchFrom::InputSnap, ..s.params() });
        run(&mut s, 0.2, &mut ph);
        assert_eq!(s.pitch, 57.0, "INPUT ♮ snaps to A3");
        // the sequencer's steps transpose from the input's pitch
        s.set_step(0, Step { on: true, note: 7, ..s.step(0) });
        s.set_playing(true);
        run(&mut s, 0.05, &mut ph);
        assert_eq!(s.pitch, 64.0, "a step 7 above the input");
        s.set_playing(false);
        // silence holds the last pitch; NOTE goes back to the note
        let (inp, mut out) = ([0f32; 128], [0f32; 128]);
        for _ in 0..200 {
            s.process(&inp, &mut out);
        }
        assert_eq!(s.pitch, 57.0, "held through silence");
        s.set_params(Params { pitch_from: PitchFrom::Note, ..s.params() });
        run(&mut s, 0.1, &mut ph);
        assert_eq!(s.pitch, 40.0, "NOTE: the note");
    }

    #[test]
    fn the_adsr_shapes_the_sound_and_legato_goes_on() {
        let fs = 48000.0;
        let run = |s: &mut Suu, secs: f64| {
            let (inp, mut out) = ([0f32; 64], [0f32; 64]);
            for _ in 0..(secs * fs / 64.0).round() as usize {
                s.process(&inp, &mut out);
            }
            s.adsr_level()
        };
        let base = Params { source: Source::Saw, gate: true, attack_s: 0.1, decay_s: 0.1, sustain: 0.5, release_s: 0.2, ..Params::default() };
        let mut s = Suu::new(fs);
        s.set_params(base);
        let half = run(&mut s, 0.05);
        assert!((half - 0.5).abs() < 0.03, "linear attack, halfway: {half}");
        let sus = run(&mut s, 0.3);
        assert!((sus - 0.5).abs() < 0.01, "sustain: {sus}");
        // RETRIG: a new note while open climbs again, from where it is
        s.retrigger();
        let again = run(&mut s, 0.03);
        assert!(again > 0.6, "retriggered: {again}");
        // LEGATO: it goes on
        let mut l = Suu::new(fs);
        l.set_params(Params { legato: true, ..base });
        run(&mut l, 0.4);
        l.retrigger();
        let on = run(&mut l, 0.03);
        assert!((on - 0.5).abs() < 0.01, "legato: {on}");
        // release: within 1 % in its time
        s.set_params(Params { gate: false, ..s.params() });
        let gone = run(&mut s, 0.21);
        assert!(gone < 0.01, "released: {gone}");
        // synced: a beat at 120 bpm is 0.5 s of attack
        let mut y = Suu::new(fs);
        y.set_params(Params { attack_sync: 1.0, bpm: 120.0, ..base });
        let q = run(&mut y, 0.25);
        assert!((q - 0.5).abs() < 0.03, "a synced attack, halfway: {q}");
    }

    #[test]
    fn the_input_vowel_places_the_pad() {
        let fs = 48000.0;
        // a 120 Hz pulse through an i's formants (270, 2290 Hz)
        let res = |f: f64, bw: f64| {
            let r = exp(-PI * bw / fs);
            (2.0 * r * cos(2.0 * PI * f / fs), -r * r)
        };
        let (r1, r2) = (res(270.0, 80.0), res(2290.0, 100.0));
        let (mut y1, mut y2) = ([0.0f64; 2], [0.0f64; 2]);
        let mut s = Suu::new(fs);
        s.set_params(Params { source: Source::Off, gate: true, vowel_x: 1.0, vowel_y: 1.0, vowel_in: true, ..Params::default() });
        let (mut inp, mut out) = ([0f32; 128], [0f32; 128]);
        for b in 0..(0.4 * fs / 128.0) as usize {
            for (i, x) in inp.iter_mut().enumerate() {
                let p = if (b * 128 + i) % 400 == 0 { 1.0 } else { 0.0 };
                let a = p + r1.0 * y1[0] + r1.1 * y1[1];
                y1 = [a, y1[0]];
                let v = a + r2.0 * y2[0] + r2.1 * y2[1];
                y2 = [v, y2[0]];
                *x = (v * 0.02) as f32;
            }
            s.process(&inp, &mut out);
        }
        let b = s.bases();
        assert!(b[0] < 0.3 && b[1] < 0.3, "the pad near i, not the knob's ɑ: {:.2}, {:.2}", b[0], b[1]);
        // off: the pad's own place
        s.set_params(Params { vowel_in: false, ..s.params() });
        assert_eq!(&s.bases()[..2], &[1.0, 1.0]);
    }

    #[test]
    fn steady_takes_a_new_pitch_only_once_it_has_lasted_and_coarse_transposes() {
        let fs = 48000.0;
        // a saw: `f0` for `secs`
        let run = |s: &mut Suu, f0: f64, secs: f64, ph: &mut f64| {
            let (mut inp, mut out) = ([0f32; 128], [0f32; 128]);
            for _ in 0..(secs * fs / 128.0) as usize {
                for x in inp.iter_mut() {
                    *ph = (*ph + f0 / fs) % 1.0;
                    *x = (0.3 * (2.0 * *ph - 1.0)) as f32;
                }
                s.process(&inp, &mut out);
            }
        };
        let mut ph = 0.0;
        let mut s = Suu::new(fs);
        s.set_params(Params { gate: true, pitch_from: PitchFrom::InputSnap, steady_s: 0.2, ..Params::default() });
        run(&mut s, 220.0, 0.4, &mut ph);
        assert!(s.tracked_note().is_some_and(|n| (n - 57.0).abs() < 0.1), "A3 taken after it lasted");
        // a 100 ms blip a fifth up is not taken; the note stays
        run(&mut s, 330.0, 0.1, &mut ph);
        run(&mut s, 220.0, 0.1, &mut ph);
        assert_eq!(s.pitch, 57.0, "a short blip is filtered out");
        // a fifth held 0.4 s is
        run(&mut s, 330.0, 0.4, &mut ph);
        assert_eq!(s.pitch, 64.0, "a held new note is taken");
        // vibrato within the note passes at once (IN, not snapped)
        s.set_params(Params { pitch_from: PitchFrom::Input, ..s.params() });
        run(&mut s, 330.0 * pow(2.0, 0.3 / 12.0), 0.1, &mut ph);
        assert!((s.pitch - 64.3).abs() < 0.05, "30 cents up follows at once: {}", s.pitch);
        // COARSE: an octave down, on the input's pitch and on the note alike
        s.set_params(Params { coarse: -12.0, pitch_from: PitchFrom::InputSnap, ..s.params() });
        run(&mut s, 330.0, 0.1, &mut ph);
        assert_eq!(s.pitch, 52.0, "COARSE −12 on IN ♮");
        s.set_params(Params { pitch_from: PitchFrom::Note, note: 45.0, ..s.params() });
        run(&mut s, 330.0, 0.05, &mut ph);
        assert_eq!(s.pitch, 33.0, "COARSE −12 on the note");
        // synced: a beat at 120 bpm is 0.5 s
        let mut y = Suu::new(fs);
        y.set_params(Params { gate: true, note: 40.0, pitch_from: PitchFrom::InputSnap, steady_sync: 1.0, bpm: 120.0, ..Params::default() });
        run(&mut y, 220.0, 0.4, &mut ph);
        assert_eq!(y.pitch, 40.0, "not yet after 0.4 s");
        run(&mut y, 220.0, 0.2, &mut ph);
        assert_eq!(y.pitch, 57.0, "taken after a beat");
    }

    /// Skipping settled geometry updates (CHIMERA's `HumanAirway::settled_for`) changes nothing:
    /// a held vowel settles, then the drawn horn, its morph, the vowel and the size change.
    #[test]
    fn skipping_settled_geometry_changes_nothing() {
        let render = |skip: bool| {
            let mut s = Suu::new(48000.0);
            s.voice.airway.skip_settled = skip;
            s.set_params(Params { gate: true, ..Params::default() });
            let (inp, mut out) = ([0f32; 128], [0f32; 128]);
            let mut all = Vec::new();
            let mut run = |s: &mut Suu, blocks: usize| {
                for _ in 0..blocks {
                    s.process(&inp, &mut out);
                    all.extend_from_slice(&out);
                }
            };
            run(&mut s, 600); // 1.6 s: settled
            let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 3.0 + 2.0 * (i as f64 * 0.4).sin()).collect();
            s.set_profile(&wave, false);
            run(&mut s, 400);
            s.set_params(Params { morph: 0.5, ..s.params() });
            run(&mut s, 400);
            s.set_profile(&[4.0; HORN_POINTS], false);
            run(&mut s, 400);
            s.set_params(Params { vowel_x: 0.0, vowel_y: 0.0, open: 2.0, ..s.params() });
            run(&mut s, 400);
            s.set_size(1.3);
            s.voice.airway.skip_settled = skip;
            run(&mut s, 600);
            all
        };
        let (on, off) = (render(true), render(false));
        assert!(on.iter().zip(&off).all(|(a, b)| a.to_bits() == b.to_bits()), "skipping changed the output");
        assert!(on.iter().any(|v| v.abs() > 1e-3), "it sounds");
    }

    /// HQ off builds a lean mouth at a lower rate; at every size it sounds, stays bounded and
    /// has no NaN, through the vowels, folds and the drawn horn; HQ on again rebuilds as before.
    #[test]
    fn hq_off_is_lighter_and_stable_at_every_size() {
        for size in [0.4, 0.5, 0.6, 0.8, 1.0, 1.3, 1.6, 2.0, 4.0] {
            let mut s = Suu::new(48000.0);
            s.set_params(Params { gate: true, hq: false, folds: 0.5, ..Params::default() });
            s.set_size(size);
            assert!(!s.hq());
            assert!(s.voice.m <= oversample_for(48000.0, size, true), "size {size}: no higher rate than HQ");
            let (inp, mut out) = ([0f32; 128], [0f32; 128]);
            let mut peak = 0f32;
            for b in 0..1500 {
                if b % 150 == 0 {
                    let v = (b / 150) as f64 / 10.0;
                    s.set_params(Params { vowel_x: v, vowel_y: 1.0 - v, morph: if b > 750 { 0.7 } else { 0.0 }, ..s.params() });
                }
                s.process(&inp, &mut out);
                for v in out {
                    assert!(v.is_finite(), "size {size}: NaN");
                    peak = peak.max(v.abs());
                }
            }
            assert!(peak > 0.01 && peak <= 1.0, "size {size}: peak {peak}");
        }
        let mut s = Suu::new(48000.0);
        s.set_params(Params { hq: false, ..Params::default() });
        s.set_size(1.0);
        assert_eq!((s.hq(), s.voice.m), (false, 1), "size 1 lean: the output rate");
        // (the crossfade must end before another rebuild)
        let (inp, mut out) = ([0f32; 128], [0f32; 128]);
        for _ in 0..20 {
            s.process(&inp, &mut out);
        }
        let _ = s.take_retired();
        s.set_params(Params { hq: true, ..s.params() });
        s.set_size(1.0);
        assert_eq!((s.hq(), s.voice.m), (true, 2), "HQ again: twice the rate");
    }

    #[test]
    fn grid_holds_every_change_until_the_next_line_of_the_song() {
        let fs = 48000.0;
        // a beat at 120 bpm is 0.5 s
        let run = |s: &mut Suu, secs: f64| {
            let (inp, mut out) = ([0f32; 64], [0f32; 64]);
            for _ in 0..(secs * fs / 64.0).round() as usize {
                s.process(&inp, &mut out);
            }
        };
        let mut s = Suu::new(fs);
        s.set_params(Params { gate: true, note: 45.0, bpm: 120.0, grid: 1.0, glide_s: 0.0, ..Params::default() });
        s.set_mod(0, Mod { depth: 0.8, rate_hz: 3.0, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
        run(&mut s, 0.05);
        // Motion: one value per beat
        let first = s.eff[0];
        run(&mut s, 0.3);
        assert_eq!(s.eff[0], first, "held within the beat");
        run(&mut s, 0.2);
        assert_ne!(s.eff[0], first, "a new value on the next line");
        // a pitch change during the note waits for the line; so does the pad
        run(&mut s, 0.1); // 0.65 s
        s.set_params(Params { note: 50.0, vowel_x: 0.0, ..s.params() });
        run(&mut s, 0.2); // 0.85 s
        assert_eq!(s.pitch, 45.0, "the new pitch waits");
        run(&mut s, 0.2); // 1.05 s: past the line at 1.0
        assert_eq!(s.pitch, 50.0, "taken on the line");
        // a played note takes its pitch at once, but not the shape: that waits for the line
        let shape = s.eff[0];
        s.set_params(Params { gate: false, ..s.params() });
        run(&mut s, 0.1); // 1.15 s
        s.set_params(Params { gate: true, note: 57.0, ..s.params() });
        run(&mut s, 0.02);
        assert_eq!(s.pitch, 57.0, "a played note's pitch at once");
        assert_eq!(s.eff[0], shape, "its shape on the grid");
        // the host's song position sets the lines: at beat 6.88, the next is at 7 (60 ms)
        s.set_song_position(Some(6.88));
        run(&mut s, 0.005);
        s.set_params(Params { note: 62.0, ..s.params() });
        run(&mut s, 0.03);
        assert_eq!(s.pitch, 57.0, "before the host's line");
        run(&mut s, 0.03);
        assert_eq!(s.pitch, 62.0, "on the host's line");
        // off: free again
        s.set_params(Params { grid: 0.0, note: 60.0, ..s.params() });
        run(&mut s, 0.01);
        assert_eq!(s.pitch, 60.0, "free");
    }

    /// GRID with the input's pitch: quick onsets (the ENV gate) do not take it faster than the
    /// grid; it changes on the lines only.
    #[test]
    fn grid_keeps_the_input_pitch_on_the_lines_however_fast_the_onsets() {
        let fs = 48000.0;
        let mut s = Suu::new(fs);
        s.set_params(Params { source: Source::Off, env_gate: true, pitch_from: PitchFrom::InputSnap, grid: 1.0, bpm: 120.0, glide_s: 0.0, ..Params::default() });
        let mut ph = 0.0;
        let mut changes = Vec::new();
        let mut last = f64::NAN;
        let (mut inp, mut out) = ([0f32; 64], [0f32; 64]);
        // bursts every 100 ms, the pitch alternating A3 and E4
        for b in 0..(3.0 * fs / 64.0) as usize {
            for (i, x) in inp.iter_mut().enumerate() {
                let t = (b * 64 + i) as f64 / fs;
                let f0 = if (t / 0.1) as usize % 2 == 0 { 220.0 } else { 330.0 };
                ph = (ph + f0 / fs) % 1.0;
                *x = if (t % 0.1) < 0.07 { (0.3 * (2.0 * ph - 1.0)) as f32 } else { 0.0 };
            }
            s.process(&inp, &mut out);
            if s.pitch != last {
                last = s.pitch;
                changes.push((b * 64) as f64 / fs);
            }
        }
        // after the first, every change is within a control period of a beat line (0.5 s)
        for &t in &changes[1..] {
            let off = (t / 0.5 - (t / 0.5).round()).abs() * 0.5;
            assert!(off < 0.003, "a change at {t:.3} s, off the grid by {off:.4} s");
        }
        assert!(changes.len() <= 8, "no faster than the grid: {changes:?}");
    }
}
