use nih_plug::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use suu::{GlideDir, Mod, STEPS, Shape, Source, Step, TextSay, TextStep, TextUnit};

pub const DEFAULT_WORDS: &str = "wow yeah oh yeah now wah wow";
pub const MAX_WORDS: usize = 400;

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Wave {
    #[id = "off"]
    Off,
    #[id = "saw"]
    Saw,
    #[id = "supersaw"]
    Supersaw,
    #[id = "square"]
    #[name = "Pulse"]
    Square,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum SeqOpen {
    #[id = "seq"]
    #[name = "Seq"]
    Seq,
    #[id = "and_keys"]
    #[name = "× Keys"]
    AndKeys,
    #[id = "or_keys"]
    #[name = "+ Keys"]
    OrKeys,
    #[id = "and_in"]
    #[name = "× In"]
    AndIn,
    #[id = "or_in"]
    #[name = "+ In"]
    OrIn,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum InShapeP {
    #[id = "raw"]
    #[name = "Raw"]
    Raw,
    #[id = "flat"]
    #[name = "Flat"]
    Flat,
    #[id = "edge"]
    #[name = "Edge"]
    Edge,
    #[id = "voc"]
    #[name = "Voc"]
    Voc,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum GateMode {
    #[id = "env"]
    #[name = "ENV"]
    Env,
    #[id = "hold"]
    #[name = "HOLD"]
    Hold,
    #[id = "midi"]
    #[name = "MIDI"]
    Midi,
}

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum JumpMode {
    #[id = "fold"]
    #[name = "Fold"]
    Fold,
    #[id = "skip"]
    #[name = "Skip"]
    Skip,
}

pub const PITCH_CLASSES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

#[derive(Enum, Debug, PartialEq, Eq, Clone, Copy)]
pub enum PitchMode {
    #[id = "note"]
    #[name = "Note"]
    Note,
    #[id = "input"]
    #[name = "Input"]
    Input,
    #[id = "input_snap"]
    #[name = "Input Snapped"]
    InputSnap,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Sync {
    #[id = "off"]
    Off,
    #[id = "1/128"]
    #[name = "1/128"]
    D128,
    #[id = "1/64T"]
    #[name = "1/64T"]
    D64T,
    #[id = "1/64"]
    #[name = "1/64"]
    D64,
    #[id = "1/32T"]
    #[name = "1/32T"]
    D32T,
    #[id = "1/32"]
    #[name = "1/32"]
    D32,
    #[id = "1/16T"]
    #[name = "1/16T"]
    D16T,
    #[id = "1/16"]
    #[name = "1/16"]
    D16,
    #[id = "1/16."]
    #[name = "1/16."]
    D16D,
    #[id = "1/8T"]
    #[name = "1/8T"]
    D8T,
    #[id = "1/8"]
    #[name = "1/8"]
    D8,
    #[id = "1/8."]
    #[name = "1/8."]
    D8D,
    #[id = "1/4T"]
    #[name = "1/4T"]
    D4T,
    #[id = "1/4"]
    #[name = "1/4"]
    D4,
    #[id = "1/4."]
    #[name = "1/4."]
    D4D,
    #[id = "1/2"]
    #[name = "1/2"]
    D2,
    #[id = "1/2."]
    #[name = "1/2."]
    D2D,
    #[id = "1/1"]
    #[name = "1/1"]
    D1,
    #[id = "2/1"]
    #[name = "2/1"]
    W2,
    #[id = "4/1"]
    #[name = "4/1"]
    W4,
}

impl Sync {
    pub fn beats(self) -> f64 {
        match self {
            Sync::Off => 0.0,
            Sync::D128 => 1.0 / 32.0,
            Sync::D64T => 1.0 / 24.0,
            Sync::D64 => 1.0 / 16.0,
            Sync::D32T => 1.0 / 12.0,
            Sync::D32 => 0.125,
            Sync::D16T => 1.0 / 6.0,
            Sync::D16 => 0.25,
            Sync::D16D => 0.375,
            Sync::D8T => 1.0 / 3.0,
            Sync::D8 => 0.5,
            Sync::D8D => 0.75,
            Sync::D4T => 2.0 / 3.0,
            Sync::D4 => 1.0,
            Sync::D4D => 1.5,
            Sync::D2 => 2.0,
            Sync::D2D => 3.0,
            Sync::D1 => 4.0,
            Sync::W2 => 8.0,
            Sync::W4 => 16.0,
        }
    }
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum ModShape {
    #[id = "sine"]
    Sine,
    #[id = "triangle"]
    Triangle,
    #[id = "saw"]
    Saw,
    #[id = "square"]
    Square,
    #[id = "random"]
    Random,
    #[id = "envelope"]
    Envelope,
    #[id = "adsr"]
    #[name = "ADSR"]
    Adsr,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum ExprTo {
    #[id = "off"]
    Off,
    #[id = "vowel_x"]
    #[name = "Vowel X"]
    VowelX,
    #[id = "vowel_y"]
    #[name = "Vowel Y"]
    VowelY,
    #[id = "round"]
    Round,
    #[id = "nasal"]
    Nasal,
    #[id = "inject"]
    Inject,
    #[id = "damping"]
    Damping,
    #[id = "morph"]
    Morph,
    #[id = "shift"]
    Shift,
    #[id = "open"]
    Open,
    #[id = "breath"]
    Breath,
    #[id = "level"]
    Level,
}

impl ExprTo {
    pub fn core(self) -> i32 {
        self as i32 - 1
    }
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum Glide {
    #[id = "both"]
    Both,
    #[id = "up"]
    Up,
    #[id = "down"]
    Down,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum TextStepP {
    #[id = "note"]
    Note,
    #[id = "clock"]
    Clock,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum TextUnitP {
    #[id = "syllable"]
    Syllable,
    #[id = "word"]
    Word,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum TextSayP {
    #[id = "hold"]
    Hold,
    #[id = "shot"]
    Shot,
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum SeqRate {
    #[id = "1/4"]
    #[name = "1/4"]
    Quarter,
    #[id = "1/8"]
    #[name = "1/8"]
    Eighth,
    #[id = "1/16"]
    #[name = "1/16"]
    Sixteenth,
    #[id = "1/32"]
    #[name = "1/32"]
    ThirtySecond,
    #[id = "1/64"]
    #[name = "1/64"]
    SixtyFourth,
    #[id = "1/128"]
    #[name = "1/128"]
    HundredTwentyEighth,
}

impl SeqRate {
    pub fn per_beat(self) -> f64 {
        match self {
            SeqRate::Quarter => 1.0,
            SeqRate::Eighth => 2.0,
            SeqRate::Sixteenth => 4.0,
            SeqRate::ThirtySecond => 8.0,
            SeqRate::SixtyFourth => 16.0,
            SeqRate::HundredTwentyEighth => 32.0,
        }
    }
}

#[derive(Enum, Debug, PartialEq, Clone, Copy)]
pub enum StepVowel {
    #[id = "none"]
    #[name = "–"]
    None,
    #[id = "a"]
    #[name = "a"]
    A,
    #[id = "e"]
    #[name = "e"]
    E,
    #[id = "i"]
    #[name = "i"]
    I,
    #[id = "o"]
    #[name = "o"]
    O,
    #[id = "u"]
    #[name = "u"]
    U,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct HornState {
    pub areas: Vec<f64>,
    pub wrap: bool,
}

fn percent(p: FloatParam) -> FloatParam {
    p.with_unit(" %").with_value_to_string(formatters::v2s_f32_percentage(0)).with_string_to_value(formatters::s2v_f32_percentage())
}

fn times(p: FloatParam) -> FloatParam {
    p.with_value_to_string(Arc::new(|v| format!("×{v:.2}"))).with_string_to_value(Arc::new(|s| s.trim().trim_start_matches(['×', 'x', '*']).parse().ok()))
}

fn db(p: FloatParam, off_below: f32) -> FloatParam {
    p.with_unit(" dB").with_value_to_string(Arc::new(move |v| if v <= off_below { "off".into() } else { format!("{v:.1}") }))
}

fn hz(p: FloatParam, off_below: f32) -> FloatParam {
    p.with_unit(" Hz").with_value_to_string(Arc::new(move |v| if v < off_below { "off".into() } else if v < 10.0 { format!("{v:.2}") } else { format!("{v:.0}") }))
}

fn seconds(name: &str, default: f32, min: f32, max: f32, centre: f32) -> FloatParam {
    FloatParam::new(name, default, log_range(min, max, centre))
        .with_unit(" s")
        .with_value_to_string(Arc::new(|v| if v < 1.0 { format!("{:.1} m", v * 1000.0) } else { format!("{v:.2}") }))
}

fn log_range(min: f32, max: f32, centre: f32) -> FloatRange {
    FloatRange::Skewed { min, max, factor: FloatRange::skew_factor_from_centre(min, max, centre) }
}

trait FromCentre {
    fn skew_factor_from_centre(min: f32, max: f32, centre: f32) -> f32;
}

impl FromCentre for FloatRange {
    fn skew_factor_from_centre(min: f32, max: f32, centre: f32) -> f32 {
        (0.5f32).ln() / ((centre - min) / (max - min)).ln()
    }
}

#[derive(Params)]
pub struct MotionParams {
    #[id = "base"]
    pub base: FloatParam,
    #[id = "depth"]
    pub depth: FloatParam,
    #[id = "rate"]
    pub rate: FloatParam,
    #[id = "sync"]
    pub sync: EnumParam<Sync>,
    #[id = "shape"]
    pub shape: EnumParam<ModShape>,
    #[id = "smooth"]
    pub smooth: FloatParam,
}

const MOTION: [(&str, f32, f32, f32, bool); suu::N_MOD] = [
    ("Vowel X", -0.3, 1.3, 0.5, false),
    ("Vowel Y", -0.3, 1.3, 0.6, false),
    ("Round", -1.0, 1.0, 0.0, false),
    ("Nasal", 0.0, 1.5, 0.0, false),
    ("Inject", 0.0, 1.0, 0.0, false),
    ("Damping", 0.1, 20.0, 1.0, true),
    ("Morph", 0.0, 1.0, 0.0, false),
    ("Shift", -0.5, 0.5, 0.0, false),
    ("Open", 0.25, 4.0, 1.0, true),
];

impl MotionParams {
    fn new(target: usize) -> Self {
        let (name, min, max, def, log) = MOTION[target];
        let range = if log { log_range(min, max, (min * max).sqrt()) } else { FloatRange::Linear { min, max } };
        let mut base = FloatParam::new(name, def, range).with_step_size(0.001);
        base = if log { times(base) } else { percent(base) };
        MotionParams {
            base,
            depth: percent(FloatParam::new(format!("{name} Depth"), 0.0, FloatRange::Linear { min: -1.0, max: 1.0 }).with_step_size(0.01)),
            rate: hz(FloatParam::new(format!("{name} Rate"), 0.5, log_range(0.02, 20.0, 0.632)), 0.0),
            sync: EnumParam::new(format!("{name} Sync"), Sync::Off),
            shape: EnumParam::new(format!("{name} Shape"), ModShape::Sine),
            smooth: percent(FloatParam::new(format!("{name} Smooth"), 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
        }
    }

    pub fn to_mod(&self) -> Mod {
        let shape = match self.shape.value() {
            ModShape::Sine => Shape::Sine,
            ModShape::Triangle => Shape::Triangle,
            ModShape::Saw => Shape::Saw,
            ModShape::Square => Shape::Square,
            ModShape::Random => Shape::Random,
            ModShape::Envelope => Shape::Follow,
            ModShape::Adsr => Shape::Adsr,
        };
        Mod { depth: self.depth.value() as f64, rate_hz: self.rate.value() as f64, sync: self.sync.value().beats(), shape, smooth: self.smooth.value() as f64 }
    }
}

#[derive(Params)]
pub struct ScaleParam {
    #[id = "scale"]
    pub on: BoolParam,
}

#[derive(Params)]
pub struct StepParams {
    #[id = "step_on"]
    pub on: BoolParam,
    #[id = "step_note"]
    pub note: IntParam,
    #[id = "step_tie"]
    pub tie: BoolParam,
    #[id = "step_vowel"]
    pub vowel: EnumParam<StepVowel>,
    #[id = "step_accent"]
    pub accent: BoolParam,
    #[id = "step_len"]
    pub len: FloatParam,
}

impl StepParams {
    fn new(i: usize, d: Step) -> Self {
        let n = i + 1;
        let vowel = match d.vowel {
            0 => StepVowel::A,
            1 => StepVowel::E,
            2 => StepVowel::I,
            3 => StepVowel::O,
            4 => StepVowel::U,
            _ => StepVowel::None,
        };
        StepParams {
            on: BoolParam::new(format!("Step {n} On"), d.on),
            note: IntParam::new(format!("Step {n} Note"), d.note, IntRange::Linear { min: -24, max: 24 }).with_unit(" st"),
            tie: BoolParam::new(format!("Step {n} Tie"), d.tie),
            vowel: EnumParam::new(format!("Step {n} Vowel"), vowel),
            accent: BoolParam::new(format!("Step {n} Accent"), d.accent),
            len: percent(FloatParam::new(format!("Step {n} Length"), d.len as f32, FloatRange::Linear { min: 0.05, max: 1.0 })),
        }
    }

    pub fn to_step(&self) -> Step {
        let vowel = match self.vowel.value() {
            StepVowel::None => -1,
            StepVowel::A => 0,
            StepVowel::E => 1,
            StepVowel::I => 2,
            StepVowel::O => 3,
            StepVowel::U => 4,
        };
        Step { on: self.on.value(), note: self.note.value(), tie: self.tie.value(), vowel, accent: self.accent.value(), len: self.len.value() as f64 }
    }
}

#[derive(Params)]
pub struct SuuParams {
    #[persist = "words"]
    pub words: Arc<RwLock<String>>,
    #[persist = "horn"]
    pub horn: Arc<RwLock<HornState>>,
    #[persist = "ui"]
    pub ui: Arc<RwLock<String>>,
    #[persist = "slots"]
    pub slots: Arc<RwLock<String>>,

    // ------------------------------------------------ source
    #[id = "wave"]
    pub wave: EnumParam<Wave>,
    #[id = "osc"]
    pub osc: FloatParam,
    #[id = "input"]
    pub input: FloatParam,
    #[id = "folds"]
    pub folds: FloatParam,
    #[id = "reson"]
    pub reson: FloatParam,
    #[id = "gate"]
    pub gate: EnumParam<GateMode>,
    #[id = "env_threshold"]
    pub env_threshold: FloatParam,
    #[id = "env_release"]
    pub env_release: FloatParam,
    #[id = "env_release_sync"]
    pub env_release_sync: EnumParam<Sync>,
    #[id = "attack"]
    pub attack: FloatParam,
    #[id = "attack_sync"]
    pub attack_sync: EnumParam<Sync>,
    #[id = "decay"]
    pub decay: FloatParam,
    #[id = "decay_sync"]
    pub decay_sync: EnumParam<Sync>,
    #[id = "sustain"]
    pub sustain: FloatParam,
    #[id = "release"]
    pub release: FloatParam,
    #[id = "release_sync"]
    pub release_sync: EnumParam<Sync>,
    #[id = "legato"]
    pub legato: BoolParam,
    #[id = "vowel_in"]
    pub vowel_in: BoolParam,
    #[id = "root"]
    pub root: IntParam,
    #[id = "pitch"]
    pub pitch: EnumParam<PitchMode>,
    #[id = "coarse"]
    pub coarse: IntParam,
    #[id = "steady"]
    pub steady: FloatParam,
    #[id = "steady_sync"]
    pub steady_sync: EnumParam<Sync>,
    #[id = "hq"]
    pub hq: BoolParam,
    #[id = "grid"]
    pub grid: EnumParam<Sync>,
    #[id = "text_reset_note"]
    pub text_reset_note: IntParam,
    #[id = "text_restart_play"]
    pub text_restart_play: BoolParam,
    #[id = "text_grid"]
    pub text_grid: EnumParam<Sync>,
    #[id = "text_grid_main"]
    pub text_grid_main: BoolParam,
    #[id = "grid_breath"]
    pub grid_breath: FloatParam,
    #[id = "grid_smooth"]
    pub grid_smooth: FloatParam,
    #[nested(array, group = "Scale")]
    pub scale: [ScaleParam; 12],
    #[id = "jump"]
    pub jump: IntParam,
    #[id = "jump_mode"]
    pub jump_mode: EnumParam<JumpMode>,
    #[id = "bend_range"]
    pub bend_range: IntParam,
    #[id = "glide"]
    pub glide: FloatParam,
    #[id = "glide_sync"]
    pub glide_sync: EnumParam<Sync>,
    #[id = "glide_dir"]
    pub glide_dir: EnumParam<Glide>,
    #[id = "lofi"]
    pub lofi: BoolParam,
    #[id = "text_round"]
    pub text_round: FloatParam,
    #[id = "text_fit"]
    pub text_fit: BoolParam,
    #[id = "aspiration"]
    pub aspiration: FloatParam,
    #[id = "frication"]
    pub frication: FloatParam,
    #[id = "sibilance"]
    pub sibilance: FloatParam,
    #[id = "whistle"]
    pub whistle: FloatParam,
    #[id = "move"]
    pub move_sound: FloatParam,
    #[id = "stereo_canon"]
    pub stereo_canon: IntParam,
    #[id = "stereo_phase"]
    pub stereo_phase: FloatParam,
    #[id = "stereo_mirror"]
    pub stereo_mirror: BoolParam,
    #[id = "pulse_width"]
    pub pulse_width: FloatParam,
    #[id = "pwm_depth"]
    pub pwm_depth: FloatParam,
    #[id = "pwm_rate"]
    pub pwm_rate: FloatParam,
    #[id = "pwm_sync"]
    pub pwm_sync: EnumParam<Sync>,
    #[id = "seq_open"]
    pub seq_open: EnumParam<SeqOpen>,
    #[id = "seq_vowel"]
    pub seq_vowel: FloatParam,
    #[id = "in_shape"]
    pub in_shape: EnumParam<InShapeP>,
    #[id = "seq_sync"]
    pub seq_sync: BoolParam,
    #[id = "seq_offset"]
    pub seq_offset: IntParam,
    #[id = "pressure_to"]
    pub pressure_to: EnumParam<ExprTo>,
    #[id = "pressure_depth"]
    pub pressure_depth: FloatParam,
    #[id = "slide_to"]
    pub slide_to: EnumParam<ExprTo>,
    #[id = "slide_depth"]
    pub slide_depth: FloatParam,
    #[id = "stereo"]
    pub stereo: BoolParam,
    #[id = "stereo_spread"]
    pub stereo_spread: FloatParam,
    #[id = "stereo_vowel"]
    pub stereo_vowel: FloatParam,
    #[id = "stereo_width"]
    pub stereo_width: FloatParam,

    // ------------------------------------------------ mouth
    #[id = "size"]
    pub size: FloatParam,
    #[id = "breath"]
    pub breath: FloatParam,
    #[id = "follow"]
    pub follow: FloatParam,

    // ------------------------------------------------ drive and output
    #[id = "drive"]
    pub drive: FloatParam,
    #[id = "tilt"]
    pub tilt: FloatParam,
    #[id = "sub"]
    pub sub: FloatParam,
    #[id = "sub_hz"]
    pub sub_hz: FloatParam,
    #[id = "mix"]
    pub mix: FloatParam,
    #[id = "output"]
    pub output: FloatParam,

    // ------------------------------------------------ Motion: the pad, the mouth and the horn
    #[nested(id_prefix = "vowel_x", group = "Vowel X")]
    pub vowel_x: MotionParams,
    #[nested(id_prefix = "vowel_y", group = "Vowel Y")]
    pub vowel_y: MotionParams,
    #[nested(id_prefix = "round", group = "Round")]
    pub round: MotionParams,
    #[nested(id_prefix = "nasal", group = "Nasal")]
    pub nasal: MotionParams,
    #[nested(id_prefix = "inject", group = "Inject")]
    pub inject: MotionParams,
    #[nested(id_prefix = "damping", group = "Damping")]
    pub damping: MotionParams,
    #[nested(id_prefix = "morph", group = "Morph")]
    pub morph: MotionParams,
    #[nested(id_prefix = "shift", group = "Shift")]
    pub shift: MotionParams,
    #[nested(id_prefix = "open", group = "Open")]
    pub open: MotionParams,

    // ------------------------------------------------ text
    #[id = "text"]
    pub text: BoolParam,
    #[id = "text_step"]
    pub text_step: EnumParam<TextStepP>,
    #[id = "text_unit"]
    pub text_unit: EnumParam<TextUnitP>,
    #[id = "text_say"]
    pub text_say: EnumParam<TextSayP>,
    #[id = "text_rate"]
    pub text_rate: FloatParam,
    #[id = "text_sync"]
    pub text_sync: EnumParam<Sync>,
    #[id = "text_len"]
    pub text_len: FloatParam,
    #[id = "text_gate"]
    pub text_gate: FloatParam,
    #[id = "text_seq"]
    pub text_seq: BoolParam,
    #[id = "text_once"]
    pub text_once: BoolParam,
    #[id = "speech"]
    pub speech: FloatParam,

    // ------------------------------------------------ sequencer
    #[id = "seq"]
    pub seq: BoolParam,
    #[id = "seq_rate"]
    pub seq_rate: EnumParam<SeqRate>,
    #[id = "seq_length"]
    pub seq_length: IntParam,
    #[id = "seq_gate"]
    pub seq_gate: FloatParam,
    #[nested(array, group = "Step")]
    pub steps: [StepParams; STEPS],
}

impl SuuParams {
    pub fn new(synth: bool) -> Self {
        let pattern = suu::default_pattern();
        SuuParams {
            words: Arc::new(RwLock::new(DEFAULT_WORDS.to_string())),
            horn: Arc::new(RwLock::new(HornState::default())),
            ui: Arc::new(RwLock::new(String::new())),
            slots: Arc::new(RwLock::new(String::new())),

            wave: EnumParam::new("Wave", if synth { Wave::Supersaw } else { Wave::Off }),
            osc: db(FloatParam::new("Osc", 0.0, FloatRange::Linear { min: -60.0, max: 12.0 }).with_step_size(0.1), -59.95),
            input: db(FloatParam::new("Input", if synth { -60.0 } else { 0.0 }, FloatRange::Linear { min: -60.0, max: 12.0 }).with_step_size(0.1), -59.95),
            folds: percent(FloatParam::new("Folds", 0.0, FloatRange::Linear { min: 0.0, max: 2.0 })),
            reson: percent(FloatParam::new("Reson", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            gate: EnumParam::new("Gate", if synth { GateMode::Midi } else { GateMode::Env }),
            env_threshold: db(FloatParam::new("Env Threshold", -40.0, FloatRange::Linear { min: -80.0, max: 0.0 }).with_step_size(0.1), -999.0),
            env_release: FloatParam::new("Env Release", 0.15, log_range(0.005, 4.0, 0.141))
                .with_unit(" s")
                .with_value_to_string(Arc::new(|v| if v < 1.0 { format!("{:.0} m", v * 1000.0) } else { format!("{v:.2}") })),
            env_release_sync: EnumParam::new("Env Release Sync", Sync::Off),
            attack: seconds("Attack", 0.005, 0.001, 10.0, 0.1),
            attack_sync: EnumParam::new("Attack Sync", Sync::Off),
            decay: seconds("Decay", 0.2, 0.001, 10.0, 0.3),
            decay_sync: EnumParam::new("Decay Sync", Sync::Off),
            sustain: percent(FloatParam::new("Sustain", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            release: seconds("Release", 0.08, 0.001, 10.0, 0.3),
            release_sync: EnumParam::new("Release Sync", Sync::Off),
            legato: BoolParam::new("Legato", false),
            vowel_in: BoolParam::new("Vowel From Input", false),
            root: IntParam::new("Root", 45, IntRange::Linear { min: 12, max: 108 }).with_value_to_string(formatters::v2s_i32_note_formatter()).with_string_to_value(formatters::s2v_i32_note_formatter()),
            pitch: EnumParam::new("Pitch", PitchMode::Note),
            coarse: IntParam::new("Coarse", 0, IntRange::Linear { min: -48, max: 48 }).with_unit(" st"),
            steady: seconds("Steady", 0.0, 0.0, 4.0, 0.2),
            steady_sync: EnumParam::new("Steady Sync", Sync::Off),
            hq: BoolParam::new("HQ", true),
            grid: EnumParam::new("Grid", Sync::Off),
            text_reset_note: IntParam::new("Text Reset Note", -1, IntRange::Linear { min: -1, max: 127 })
                .with_value_to_string(Arc::new(|v| if v < 0 { "Off".into() } else { v.to_string() }))
                .with_string_to_value(Arc::new(|s| if s.trim().eq_ignore_ascii_case("off") { Some(-1) } else { s.trim().parse().ok() })),
            text_restart_play: BoolParam::new("Text Restarts On Play", false),
            text_grid: EnumParam::new("Text Grid", Sync::Off),
            text_grid_main: BoolParam::new("Text Grid Follows Grid", false),
            grid_breath: percent(FloatParam::new("Text Grid Breath", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            grid_smooth: percent(FloatParam::new("Grid Smooth", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            scale: core::array::from_fn(|i| ScaleParam { on: BoolParam::new(format!("Scale {}", PITCH_CLASSES[i]), true) }),
            jump: IntParam::new("Jump", 0, IntRange::Linear { min: 0, max: 48 }).with_unit(" st"),
            jump_mode: EnumParam::new("Jump Mode", JumpMode::Fold),
            bend_range: IntParam::new("Bend Range", 2, IntRange::Linear { min: 0, max: 48 }).with_unit(" st"),
            glide: FloatParam::new("Glide", 0.03, FloatRange::Skewed { min: 0.0, max: 2.0, factor: 0.5 })
                .with_unit(" s")
                .with_value_to_string(Arc::new(|v| if v < 0.0005 { "off".into() } else if v < 1.0 { format!("{:.0} m", v * 1000.0) } else { format!("{v:.2}") })),
            glide_sync: EnumParam::new("Glide Sync", Sync::Off),
            glide_dir: EnumParam::new("Glide Direction", Glide::Both),
            lofi: BoolParam::new("Lo-Fi", false),
            text_round: percent(FloatParam::new("Text Round", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            text_fit: BoolParam::new("Text Fits Note", false),
            aspiration: percent(FloatParam::new("Aspiration", 1.0, FloatRange::Skewed { min: 0.0, max: 10.0, factor: 0.4 })),
            frication: percent(FloatParam::new("Frication", 1.0, FloatRange::Skewed { min: 0.0, max: 10.0, factor: 0.4 })),
            sibilance: percent(FloatParam::new("Sibilance", 1.0, FloatRange::Skewed { min: 0.0, max: 10.0, factor: 0.4 })),
            whistle: percent(FloatParam::new("Whistle", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            move_sound: percent(FloatParam::new("Move", 1.0, FloatRange::Skewed { min: 0.0, max: 8.0, factor: 0.4 })),
            stereo_canon: IntParam::new("Stereo Canon", 0, IntRange::Linear { min: 0, max: 16 }),
            stereo_phase: percent(FloatParam::new("Stereo Phase", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            stereo_mirror: BoolParam::new("Stereo Mirror", false),
            pulse_width: percent(FloatParam::new("Pulse Width", 0.5, FloatRange::Linear { min: 0.05, max: 0.95 })),
            pwm_depth: percent(FloatParam::new("PWM Depth", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            pwm_rate: hz(FloatParam::new("PWM Rate", 0.5, log_range(0.02, 20.0, 0.632)), 0.0),
            pwm_sync: EnumParam::new("PWM Sync", Sync::Off),
            seq_open: EnumParam::new("Sequencer Opens", SeqOpen::Seq),
            seq_vowel: percent(FloatParam::new("Sequencer Vowel", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            in_shape: EnumParam::new("Input Shape", InShapeP::Raw),
            seq_sync: BoolParam::new("Sequencer Sync", true),
            seq_offset: IntParam::new("Sequencer Offset", 0, IntRange::Linear { min: 0, max: STEPS as i32 - 1 }).with_unit(" steps"),
            pressure_to: EnumParam::new("Pressure To", ExprTo::Breath),
            pressure_depth: percent(FloatParam::new("Pressure Depth", 0.5, FloatRange::Linear { min: -1.0, max: 1.0 })),
            slide_to: EnumParam::new("Slide To", ExprTo::VowelY),
            slide_depth: percent(FloatParam::new("Slide Depth", 0.5, FloatRange::Linear { min: -1.0, max: 1.0 })),
            stereo: BoolParam::new("Stereo", false),
            stereo_spread: times(FloatParam::new("Stereo Spread", 1.12, FloatRange::Linear { min: 0.7, max: 1.4 })),
            stereo_vowel: percent(FloatParam::new("Stereo Vowel", 0.12, FloatRange::Linear { min: -0.5, max: 0.5 })),
            stereo_width: percent(FloatParam::new("Stereo Width", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })),

            size: times(FloatParam::new("Size", 1.0, log_range(0.4, 4.0, 1.265)).with_step_size(0.001)),
            breath: times(FloatParam::new("Breath", 1.0, FloatRange::Linear { min: 0.0, max: 6.0 })),
            follow: percent(FloatParam::new("Follow", 0.7, FloatRange::Linear { min: 0.0, max: 1.0 })),

            drive: FloatParam::new("Drive", 0.0, FloatRange::Linear { min: -40.0, max: 40.0 }).with_step_size(0.1).with_unit(" dB"),
            tilt: hz(FloatParam::new("Tilt", 150.0, FloatRange::Linear { min: 0.0, max: 2000.0 }).with_step_size(1.0), 1.0),
            sub: percent(FloatParam::new("Sub", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            sub_hz: hz(FloatParam::new("Sub Hz", 120.0, log_range(30.0, 400.0, 109.5)).with_step_size(1.0), 0.0),
            mix: percent(FloatParam::new("Mix", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })),
            output: FloatParam::new("Output", 0.0, FloatRange::Linear { min: -40.0, max: 20.0 }).with_step_size(0.1).with_unit(" dB"),

            vowel_x: MotionParams::new(0),
            vowel_y: MotionParams::new(1),
            round: MotionParams::new(2),
            nasal: MotionParams::new(3),
            inject: MotionParams::new(4),
            damping: MotionParams::new(5),
            morph: MotionParams::new(6),
            shift: MotionParams::new(7),
            open: MotionParams::new(8),

            text: BoolParam::new("Text", false),
            text_step: EnumParam::new("Text Step", TextStepP::Note),
            text_unit: EnumParam::new("Text Unit", TextUnitP::Syllable),
            text_say: EnumParam::new("Text Say", TextSayP::Hold),
            text_rate: hz(FloatParam::new("Text Rate", 2.0, log_range(0.25, 16.0, 2.0)), 0.0),
            text_sync: EnumParam::new("Text Sync", Sync::D4),
            text_len: percent(FloatParam::new("Text Length", 1.0, FloatRange::Linear { min: 0.05, max: 1.0 })),
            text_gate: percent(FloatParam::new("Text Gate", 1.0, FloatRange::Linear { min: 0.05, max: 1.0 })),
            text_seq: BoolParam::new("Text Follows Sequence", true),
            text_once: BoolParam::new("Text Once", false),
            speech: times(FloatParam::new("Speech", 1.0, log_range(0.25, 4.0, 1.0))),

            seq: BoolParam::new("Sequencer", false),
            seq_rate: EnumParam::new("Sequencer Rate", SeqRate::Sixteenth),
            seq_length: IntParam::new("Sequencer Steps", 16, IntRange::Linear { min: 1, max: STEPS as i32 }),
            seq_gate: percent(FloatParam::new("Sequencer Gate", 1.0, FloatRange::Linear { min: 0.0, max: 2.0 })),
            steps: core::array::from_fn(|i| StepParams::new(i, pattern[i])),
        }
    }

    pub fn motions(&self) -> [&MotionParams; suu::N_MOD] {
        [&self.vowel_x, &self.vowel_y, &self.round, &self.nasal, &self.inject, &self.damping, &self.morph, &self.shift, &self.open]
    }

    pub fn to_core(&self, note: f64, gate: bool, bpm: f64) -> suu::Params {
        let m = self.motions();
        suu::Params {
            source: match self.wave.value() {
                Wave::Off => Source::Off,
                Wave::Saw => Source::Saw,
                Wave::Supersaw => Source::Supersaw,
                Wave::Square => Source::Pulse,
            },
            osc_db: self.osc.value() as f64,
            input_db: self.input.value() as f64,
            in_shape: match self.in_shape.value() {
                InShapeP::Raw => suu::InShape::Raw,
                InShapeP::Flat => suu::InShape::Flat,
                InShapeP::Edge => suu::InShape::Edge,
                InShapeP::Voc => suu::InShape::Voc,
            },
            note,
            gate,
            drive_db: self.drive.value() as f64,
            tilt_hz: self.tilt.value() as f64,
            breath: self.breath.value() as f64,
            follow: self.follow.value() as f64,
            vowel_x: m[0].base.value() as f64,
            vowel_y: m[1].base.value() as f64,
            round: m[2].base.value() as f64,
            nasal: m[3].base.value() as f64,
            inject: m[4].base.value() as f64,
            damping: m[5].base.value() as f64,
            folds: self.folds.value() as f64,
            sub_hz: self.sub_hz.value() as f64,
            sub_level: self.sub.value() as f64,
            mix: self.mix.value() as f64,
            out_db: self.output.value() as f64,
            bpm,
            glide_s: self.glide.value() as f64,
            glide_sync: self.glide_sync.value().beats(),
            glide_dir: match self.glide_dir.value() {
                Glide::Both => GlideDir::Both,
                Glide::Up => GlideDir::Up,
                Glide::Down => GlideDir::Down,
            },
            text_step: match self.text_step.value() {
                TextStepP::Note => TextStep::Note,
                TextStepP::Clock => TextStep::Clock,
            },
            text_unit: match self.text_unit.value() {
                TextUnitP::Syllable => TextUnit::Syllable,
                TextUnitP::Word => TextUnit::Word,
            },
            text_say: match self.text_say.value() {
                TextSayP::Hold => TextSay::Hold,
                TextSayP::Shot => TextSay::Shot,
            },
            text_rate_hz: self.text_rate.value() as f64,
            text_sync: self.text_sync.value().beats(),
            text_len: self.text_len.value() as f64,
            text_seq: self.text_seq.value(),
            text_once: self.text_once.value(),
            text_gate: self.text_gate.value() as f64,
            speech: self.speech.value() as f64,
            reson: self.reson.value() as f64,
            morph: m[6].base.value() as f64,
            shift: m[7].base.value() as f64,
            open: m[8].base.value() as f64,
            env_gate: self.gate.value() == GateMode::Env,
            env_thresh_db: self.env_threshold.value() as f64,
            env_release_s: self.env_release.value() as f64,
            env_release_sync: self.env_release_sync.value().beats(),
            vowel_in: self.vowel_in.value(),
            attack_s: self.attack.value() as f64,
            attack_sync: self.attack_sync.value().beats(),
            decay_s: self.decay.value() as f64,
            decay_sync: self.decay_sync.value().beats(),
            sustain: self.sustain.value() as f64,
            release_s: self.release.value() as f64,
            release_sync: self.release_sync.value().beats(),
            legato: self.legato.value(),
            coarse: self.coarse.value() as f64,
            steady_s: self.steady.value() as f64,
            steady_sync: self.steady_sync.value().beats(),
            hq: self.hq.value(),
            lofi: self.lofi.value(),
            text_round: self.text_round.value() as f64,
            text_fit: self.text_fit.value(),
            aspiration: self.aspiration.value() as f64,
            frication: self.frication.value() as f64,
            sibilance: self.sibilance.value() as f64,
            whistle: self.whistle.value() as f64,
            move_sound: self.move_sound.value() as f64,
            stereo_canon: self.stereo_canon.value() as f64,
            stereo_phase: self.stereo_phase.value() as f64,
            stereo_mirror: self.stereo_mirror.value(),
            pulse_width: self.pulse_width.value() as f64,
            pwm_depth: self.pwm_depth.value() as f64,
            pwm_rate_hz: self.pwm_rate.value() as f64,
            pwm_sync: self.pwm_sync.value().beats(),
            seq_gate: match self.seq_open.value() {
                SeqOpen::Seq => suu::SeqGate::Seq,
                SeqOpen::AndKeys => suu::SeqGate::AndKeys,
                SeqOpen::OrKeys => suu::SeqGate::OrKeys,
                SeqOpen::AndIn => suu::SeqGate::AndIn,
                SeqOpen::OrIn => suu::SeqGate::OrIn,
            },
            seq_vowel: self.seq_vowel.value() as f64,
            seq_sync: self.seq_sync.value(),
            seq_offset: self.seq_offset.value() as f64,
            pressure_to: self.pressure_to.value().core(),
            pressure_depth: self.pressure_depth.value() as f64,
            slide_to: self.slide_to.value().core(),
            slide_depth: self.slide_depth.value() as f64,
            stereo: self.stereo.value(),
            stereo_spread: self.stereo_spread.value() as f64,
            stereo_vowel: self.stereo_vowel.value() as f64,
            stereo_width: self.stereo_width.value() as f64,
            grid: self.grid.value().beats(),
            text_grid: if self.text_grid_main.value() { -1.0 } else { self.text_grid.value().beats() },
            grid_breath: self.grid_breath.value() as f64,
            grid_smooth: self.grid_smooth.value() as f64,
            scale: self.scale.iter().enumerate().map(|(i, p)| if p.on.value() { 1u16 << i } else { 0 }).sum(),
            jump: self.jump.value() as f64,
            jump_fold: self.jump_mode.value() == JumpMode::Fold,
            pitch_from: match self.pitch.value() {
                PitchMode::Note => suu::PitchFrom::Note,
                PitchMode::Input => suu::PitchFrom::Input,
                PitchMode::InputSnap => suu::PitchFrom::InputSnap,
            },
        }
    }
}
