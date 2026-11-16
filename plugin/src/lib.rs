use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};
use nih_plug::prelude::*;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;
use suu::{Mod, Mouth, MouthSeed, N_MOD, STEPS, Step, Suu, TextPlan};

mod editor;
pub mod params;
use params::{GateMode, MAX_WORDS, SuuParams};

use chimera::phonetics::english::EnglishG2P;

static CMUDICT_GZ: &[u8] = include_bytes!("../../data/cmudict.txt.gz");
static G2P: OnceLock<EnglishG2P> = OnceLock::new();

pub(crate) fn g2p() -> &'static EnglishG2P {
    G2P.get_or_init(|| {
        use std::io::Read;
        let mut text = String::new();
        let mut g = EnglishG2P::new();
        if flate2::read::GzDecoder::new(CMUDICT_GZ).read_to_string(&mut text).is_ok() {
            g.load_dict(&text);
        }
        g
    })
}

const UI_LEN: usize = 24 + suu::UI_AREAS;
const UI_EVERY: f64 = 1.0 / 30.0;

pub struct Shared {
    ui: [AtomicU32; UI_LEN],
    ui_len: AtomicUsize,
    ui_seq: AtomicU64,
    bpm: AtomicU64,
    generation: AtomicU64,
    horn: (Sender<([f64; suu::HORN_POINTS], bool)>, Receiver<([f64; suu::HORN_POINTS], bool)>),
    words_changed: AtomicBool,
    horn_reset: AtomicBool,
    snap_state: AtomicU8,
    snap: [AtomicU32; suu::HORN_POINTS],
    learn: AtomicBool,
    learned: AtomicI32,
    key: AtomicI32,
    keys_reset: AtomicBool,
}

impl Default for Shared {
    fn default() -> Self {
        Shared {
            ui: core::array::from_fn(|_| AtomicU32::new(0)),
            ui_len: AtomicUsize::new(0),
            ui_seq: AtomicU64::new(0),
            bpm: AtomicU64::new(DEFAULT_BPM.to_bits()),
            generation: AtomicU64::new(0),
            horn: bounded(4),
            words_changed: AtomicBool::new(false),
            horn_reset: AtomicBool::new(false),
            snap_state: AtomicU8::new(0),
            snap: core::array::from_fn(|_| AtomicU32::new(0)),
            key: AtomicI32::new(-1),
            keys_reset: AtomicBool::new(false),
            learn: AtomicBool::new(false),
            learned: AtomicI32::new(-1),
        }
    }
}

const QUEUE: usize = 8;
const POLL: Duration = Duration::from_millis(4);
const DEFAULT_BPM: f64 = 120.0;

enum Job {
    Size { generation: u64, seed: MouthSeed },
    Twin { generation: u64, seed: MouthSeed },
    Text { generation: u64, params: suu::Params, on: bool },
    Drop(Junk),
}

#[allow(dead_code)]
enum Junk {
    Mouth(Mouth),
    Plan(TextPlan),
}

enum Done {
    Mouth { generation: u64, mouth: Mouth },
    Twin { generation: u64, mouth: Mouth },
    Text { generation: u64, plan: TextPlan },
}

fn worker(jobs: Receiver<Job>, done: Sender<Done>, words: Arc<RwLock<String>>) {
    loop {
        let job = match jobs.try_recv() {
            Ok(job) => job,
            Err(TryRecvError::Empty) => {
                std::thread::sleep(POLL);
                continue;
            }
            Err(TryRecvError::Disconnected) => return,
        };
        let result = match job {
            Job::Size { generation, seed } => Done::Mouth { generation, mouth: Mouth::build(&seed) },
            Job::Twin { generation, seed } => Done::Twin { generation, mouth: Mouth::build(&seed) },
            Job::Text { generation, params, on } => {
                let text = if on { words.read().map(|w| w.chars().take(MAX_WORDS).collect::<String>()).unwrap_or_default() } else { String::new() };
                Done::Text { generation, plan: TextPlan::new(&text, Some(g2p()), &params) }
            }
            Job::Drop(junk) => {
                drop(junk);
                continue;
            }
        };
        if done.send(result).is_err() {
            return;
        }
    }
}

pub trait Kind: 'static + Send + Sync {
    const SYNTH: bool;
    const NAME: &'static str;
    const CLAP_ID: &'static str;
    const DESCRIPTION: &'static str;
    const CLAP_FEATURES: &'static [ClapFeature];
    const VST3_CLASS_ID: [u8; 16];
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory];
    const LAYOUTS: &'static [AudioIOLayout];
}

pub struct Fx;
pub struct Synth;

impl Kind for Fx {
    const SYNTH: bool = false;
    const NAME: &'static str = "Suu";
    const CLAP_ID: &'static str = "io.github.janne-s.suu";
    const DESCRIPTION: &'static str = "A vocal tract as a talking filter";
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Filter, ClapFeature::Stereo];
    const VST3_CLASS_ID: [u8; 16] = *b"SuuMouthEffectFx";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx, Vst3SubCategory::Filter];
    const LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout { main_input_channels: NonZeroU32::new(2), main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_input_channels: NonZeroU32::new(1), main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
        AudioIOLayout { main_input_channels: NonZeroU32::new(1), main_output_channels: NonZeroU32::new(1), ..AudioIOLayout::const_default() },
    ];
}

impl Kind for Synth {
    const SYNTH: bool = true;
    const NAME: &'static str = "Suu Synth";
    const CLAP_ID: &'static str = "io.github.janne-s.suu-synth";
    const DESCRIPTION: &'static str = "A talking synthesizer: an oscillator through a vocal tract";
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Stereo];
    const VST3_CLASS_ID: [u8; 16] = *b"SuuMouthSynthIns";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
    const LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            aux_input_ports: &[new_nonzero_u32(2)],
            names: PortNames { layout: Some("Stereo"), main_input: None, main_output: Some("Output"), aux_inputs: &["Sidechain"], aux_outputs: &[] },
            ..AudioIOLayout::const_default()
        },
        AudioIOLayout { main_input_channels: None, main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() },
    ];
}

pub type SuuFx = SuuPlugin<Fx>;
pub type SuuSynth = SuuPlugin<Synth>;

struct Keys {
    notes: [u8; 128],
    channels: [u8; 128],
    len: usize,
    count: [u8; 128],
}

impl Keys {
    fn new() -> Self {
        Keys { notes: [0; 128], channels: [0; 128], len: 0, count: [0; 128] }
    }
    fn remove(&mut self, note: u8) {
        let c = &mut self.count[note as usize & 127];
        *c = c.saturating_sub(1);
        if *c == 0 {
            self.drop_note(note);
        }
    }
    fn choke(&mut self, note: u8) {
        self.count[note as usize & 127] = 0;
        self.drop_note(note);
    }
    fn drop_note(&mut self, note: u8) {
        if let Some(i) = self.notes[..self.len].iter().position(|&n| n == note) {
            self.notes.copy_within(i + 1..self.len, i);
            self.channels.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }
    fn press(&mut self, note: u8, channel: u8) {
        self.drop_note(note);
        let c = &mut self.count[note as usize & 127];
        *c = c.saturating_add(1);
        if self.len < 128 {
            self.notes[self.len] = note;
            self.channels[self.len] = channel;
            self.len += 1;
        }
    }
    fn top(&self) -> Option<(u8, u8)> {
        (self.len > 0).then(|| (self.notes[self.len - 1], self.channels[self.len - 1]))
    }
}

pub struct SuuPlugin<K: Kind> {
    params: Arc<SuuParams>,
    shared: Arc<Shared>,
    frame: Vec<f32>,
    frame_in: f64,
    fs: f64,
    suu: Option<Box<Suu>>,
    generation: u64,
    jobs: Option<Sender<Job>>,
    done: Option<Receiver<Done>>,
    input: Vec<f32>,
    output: Vec<f32>,
    output_r: Vec<f32>,
    twin_job: bool,
    twin: Option<Mouth>,
    keys: Keys,
    last_key: Option<(u8, u8)>,
    last_root: i32,
    bend: [f32; 16],
    pressure: [f32; 16],
    slide: [f32; 16],
    size_job: bool,
    text_job: bool,
    planned_text: Option<bool>,
    mouth: Option<Mouth>,
    junk: Option<Junk>,
    mods: [Mod; N_MOD],
    steps: [Step; STEPS],
    seq: (usize, f64, f64),
    transport_was: bool,
    _kind: PhantomData<K>,
}

impl<K: Kind> Default for SuuPlugin<K> {
    fn default() -> Self {
        SuuPlugin {
            params: Arc::new(SuuParams::new(K::SYNTH)),
            shared: Arc::new(Shared::default()),
            frame: Vec::new(),
            frame_in: 0.0,
            fs: 48000.0,
            suu: None,
            generation: 0,
            jobs: None,
            done: None,
            input: Vec::new(),
            output: Vec::new(),
            output_r: Vec::new(),
            twin_job: false,
            twin: None,
            keys: Keys::new(),
            last_key: None,
            last_root: i32::MIN,
            bend: [0.0; 16],
            pressure: [0.0; 16],
            slide: [0.5; 16],
            size_job: false,
            text_job: false,
            planned_text: None,
            mouth: None,
            junk: None,
            mods: [Mod::default(); N_MOD],
            steps: suu::default_pattern(),
            seq: (0, 0.0, 0.0),
            transport_was: false,
            _kind: PhantomData,
        }
    }
}

impl<K: Kind> SuuPlugin<K> {
    fn note_and_gate(&self) -> (f64, bool) {
        let p = &self.params;
        let mode = p.gate.value();
        let key = self.keys.top().or(self.last_key);
        let (note, bend) = match key {
            Some((n, ch)) => (n as f64, self.bend[ch as usize & 15]),
            None => (p.root.value() as f64, self.bend[0]),
        };
        (note + bend as f64 * p.bend_range.value() as f64, mode == GateMode::Hold || self.keys.len > 0)
    }

    fn discard(&mut self, junk: Junk) {
        let Some(jobs) = &self.jobs else { return };
        if let Err(e) = jobs.try_send(Job::Drop(junk)) {
            let Job::Drop(j) = e.into_inner() else { unreachable!() };
            self.junk = Some(j);
        }
    }

    fn exchange(&mut self) {
        if let Some(j) = self.junk.take() {
            self.discard(j);
            if self.junk.is_some() {
                return;
            }
        }
        let Some(suu) = self.suu.as_mut() else { return };
        if let Some(m) = self.mouth.take() {
            if let Err(m) = suu.set_mouth(m) {
                self.mouth = Some(m);
            }
        }
        if let Some(m) = self.twin.take() {
            if let Err(m) = suu.set_twin(m) {
                self.twin = Some(m);
            }
        }
        if let Some(old) = suu.take_retired() {
            self.discard(Junk::Mouth(old));
        }
        while self.junk.is_none() {
            let Some(done) = self.done.as_ref().and_then(|d| d.try_recv().ok()) else { break };
            let generation = self.generation;
            match done {
                Done::Mouth { generation: g, mouth } if g == generation && self.mouth.is_none() => {
                    self.size_job = false;
                    self.mouth = Some(mouth);
                }
                Done::Mouth { mouth, .. } => self.discard(Junk::Mouth(mouth)),
                Done::Twin { generation: g, mouth } if g == generation && self.twin.is_none() => {
                    self.twin_job = false;
                    self.twin = Some(mouth);
                }
                Done::Twin { mouth, .. } => self.discard(Junk::Mouth(mouth)),
                Done::Text { generation: g, plan } if g == generation => {
                    self.text_job = false;
                    let old = self.suu.as_mut().map(|s| s.set_text_plan(plan));
                    if let Some(old) = old {
                        self.discard(Junk::Plan(old));
                    }
                }
                Done::Text { plan, .. } => self.discard(Junk::Plan(plan)),
            }
        }
    }

    fn sync(&mut self, bpm: f64, playing: bool) {
        let root = self.params.root.value();
        if root != self.last_root {
            if self.last_root != i32::MIN {
                self.last_key = None;
            }
            self.last_root = root;
        }
        let (note, gate) = self.note_and_gate();
        let ch = self.keys.top().or(self.last_key).map_or(0, |k| k.1 as usize & 15);
        let (pressure, slide) = (self.pressure[ch] as f64, (self.slide[ch] as f64 - 0.5) * 2.0);
        let p = self.params.clone();
        let Some(suu) = self.suu.as_mut() else { return };
        suu.set_expression(pressure, slide);
        let core = p.to_core(note, gate, bpm);
        if core != suu.params() {
            suu.set_params(core);
        }
        for (i, m) in p.motions().iter().enumerate() {
            let m = m.to_mod();
            if m != self.mods[i] {
                self.mods[i] = m;
                suu.set_mod(i, m);
            }
        }
        for (i, s) in p.steps.iter().enumerate() {
            let s = s.to_step();
            if s != self.steps[i] {
                self.steps[i] = s;
                suu.set_step(i, s);
            }
        }
        let seq = (p.seq_length.value() as usize, p.seq_rate.value().per_beat(), p.seq_gate.value() as f64);
        if seq != self.seq {
            self.seq = seq;
            suu.set_seq(seq.0, seq.1, seq.2);
        }
        let run = p.seq.value();
        let started = playing && !self.transport_was;
        self.transport_was = playing;
        if started && p.text_restart_play.value() {
            suu.restart_text();
        }
        if run && started && suu.playing() {
            suu.set_playing(false);
        }
        if run != suu.playing() {
            suu.set_playing(run);
        }
        if self.junk.is_some() {
            return;
        }
        let Some(jobs) = &self.jobs else { return };
        let size = p.size.value() as f64;
        if !self.size_job && self.mouth.is_none() && (size != suu.size() || suu.params().quality() != suu.quality()) {
            if jobs.try_send(Job::Size { generation: self.generation, seed: suu.mouth_seed(size) }).is_ok() {
                self.size_job = true;
            }
        }
        if !self.twin_job && self.twin.is_none() && !self.size_job && self.mouth.is_none() && let Some(want) = suu.wants_twin() {
            if jobs.try_send(Job::Twin { generation: self.generation, seed: suu.mouth_seed(want) }).is_ok() {
                self.twin_job = true;
            }
        }
        let on = p.text.value();
        if !self.text_job && self.junk.is_none() && (self.planned_text != Some(on) || suu.text_stale()) {
            if jobs.try_send(Job::Text { generation: self.generation, params: suu.params(), on }).is_ok() {
                self.text_job = true;
                self.planned_text = Some(on);
            }
        }
    }

    #[doc(hidden)]
    pub fn render(&mut self, n: usize, bpm: f64, playing: bool, mut events: impl FnMut() -> Option<NoteEvent<()>>) {
        self.exchange();
        let mut pos = 0;
        let mut next = events();
        self.sync(bpm, playing);
        while pos < n {
            while let Some(e) = next {
                if e.timing() as usize > pos {
                    break;
                }
                self.note_event(e);
                next = events();
            }
            let end = next.map_or(n, |e| (e.timing() as usize).clamp(pos + 1, n));
            self.sync(bpm, playing);
            if let Some(suu) = self.suu.as_mut() {
                suu.process_stereo(&self.input[pos..end], &mut self.output[pos..end], &mut self.output_r[pos..end]);
            }
            pos = end;
        }
        while let Some(e) = next {
            self.note_event(e);
            next = events();
        }
    }

    #[doc(hidden)]
    pub fn with_params(params: SuuParams) -> Self {
        SuuPlugin { params: Arc::new(params), ..Self::default() }
    }

    #[doc(hidden)]
    pub fn io(&mut self) -> (&mut [f32], &[f32]) {
        (&mut self.input, &self.output)
    }

    #[doc(hidden)]
    pub fn right(&self) -> &[f32] {
        &self.output_r
    }

    #[doc(hidden)]
    pub fn core(&self) -> Option<&Suu> {
        self.suu.as_deref()
    }

    #[doc(hidden)]
    pub fn plugin_params(&self) -> &SuuParams {
        &self.params
    }

    #[doc(hidden)]
    pub fn setup(&mut self, fs: f64, max_block: usize) -> bool {
        self.setup_at_size(fs, max_block, self.params.size.value() as f64)
    }

    #[doc(hidden)]
    pub fn setup_at_size(&mut self, fs: f64, max_block: usize, size: f64) -> bool {
        let n = max_block.max(1);
        self.input = vec![0.0; n];
        self.output = vec![0.0; n];
        self.output_r = vec![0.0; n];
        self.frame = Vec::with_capacity(2 * UI_LEN);
        self.fs = fs;
        if self.jobs.is_none() {
            let (jobs_tx, jobs_rx) = bounded(QUEUE);
            let (done_tx, done_rx) = bounded(QUEUE);
            let words = self.params.words.clone();
            let spawned = std::thread::Builder::new().name("suu-worker".into()).spawn(move || worker(jobs_rx, done_tx, words));
            if spawned.is_err() {
                return false;
            }
            (self.jobs, self.done) = (Some(jobs_tx), Some(done_rx));
        }
        self.generation += 1;
        let mut s = Box::new(Suu::new(fs));
        let p = &self.params;
        s.set_size(size);
        if let Ok(h) = p.horn.read()
            && h.areas.len() == suu::HORN_POINTS
        {
            s.set_profile(&h.areas, h.wrap);
        }
        self.mods = [Mod::default(); N_MOD];
        self.steps = suu::default_pattern();
        self.seq = (16, 4.0, 1.0);
        self.suu = Some(s);
        self.mouth = None;
        (self.size_job, self.text_job, self.planned_text) = (false, false, None);
        (self.twin_job, self.twin) = (false, None);
        self.keys = Keys::new();
        self.bend = [0.0; 16];
        self.pressure = [0.0; 16];
        self.slide = [0.5; 16];
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        true
    }

    fn window_exchange(&mut self, n: usize, bpm: f64) {
        let shared = self.shared.clone();
        if shared.words_changed.swap(false, Ordering::AcqRel) {
            self.planned_text = None;
        }
        if shared.keys_reset.swap(false, Ordering::AcqRel) {
            self.keys = Keys::new();
            self.last_key = None;
        }
        shared.key.store(self.keys.top().or(self.last_key).map_or(-1, |k| k.0 as i32), Ordering::Relaxed);
        let Some(suu) = self.suu.as_mut() else { return };
        if shared.horn_reset.swap(false, Ordering::AcqRel) {
            suu.set_profile(&suu::default_horn(), false);
        }
        while let Ok((areas, wrap)) = shared.horn.1.try_recv() {
            suu.set_profile(&areas, wrap);
        }
        if shared.snap_state.load(Ordering::Acquire) == 1 {
            let mut v = [0.0; suu::HORN_POINTS];
            let ok = suu.snapshot(&mut v);
            for (d, s) in shared.snap.iter().zip(v) {
                d.store((s as f32).to_bits(), Ordering::Relaxed);
            }
            shared.snap_state.store(if ok { 2 } else { 3 }, Ordering::Release);
        }
        self.frame_in -= n as f64;
        if self.frame_in <= 0.0 {
            self.frame_in += UI_EVERY * self.fs;
            suu.ui_state(&mut self.frame);
            let len = self.frame.len().min(UI_LEN);
            for (d, s) in shared.ui.iter().zip(&self.frame[..len]) {
                d.store(s.to_bits(), Ordering::Relaxed);
            }
            shared.ui_len.store(len, Ordering::Relaxed);
            shared.bpm.store(bpm.to_bits(), Ordering::Relaxed);
            shared.ui_seq.fetch_add(1, Ordering::Release);
        }
    }

    fn note_event(&mut self, event: NoteEvent<()>) {
        if let NoteEvent::NoteOn { note, velocity, .. } = event
            && velocity > 0.0
            && self.shared.learn.swap(false, Ordering::AcqRel)
        {
            self.shared.learned.store(note as i32, Ordering::Release);
            return;
        }
        let reset = self.params.text_reset_note.value();
        if reset >= 0 {
            match event {
                NoteEvent::NoteOn { note, velocity, .. } if note as i32 == reset => {
                    if velocity > 0.0
                        && let Some(suu) = self.suu.as_mut()
                    {
                        suu.restart_text();
                    }
                    return;
                }
                NoteEvent::NoteOff { note, .. } | NoteEvent::Choke { note, .. } if note as i32 == reset => return,
                _ => {}
            }
        }
        match event {
            NoteEvent::NoteOn { note, channel, velocity, .. } if velocity > 0.0 => {
                let held = self.keys.len > 0;
                let sounding = self.params.gate.value() == GateMode::Hold || held || self.suu.as_ref().is_some_and(|s| s.env_open());
                self.keys.press(note, channel);
                self.last_key = Some((note, channel));
                if let Some(suu) = self.suu.as_mut() {
                    if !held {
                        suu.note_start();
                    } else if sounding {
                        suu.retrigger();
                    }
                }
            }
            NoteEvent::NoteOn { note, .. } | NoteEvent::NoteOff { note, .. } => {
                self.keys.remove(note);
                if let Some(k) = self.keys.top() {
                    self.last_key = Some(k);
                }
            }
            NoteEvent::Choke { note, .. } => self.keys.choke(note),
            NoteEvent::MidiPitchBend { channel, value, .. } => self.bend[channel as usize & 15] = (value - 0.5) * 2.0,
            NoteEvent::MidiChannelPressure { channel, pressure, .. } | NoteEvent::PolyPressure { channel, pressure, .. } => self.pressure[channel as usize & 15] = pressure,
            NoteEvent::MidiCC { channel, cc: 74, value, .. } => self.slide[channel as usize & 15] = value,
            NoteEvent::MidiCC { cc: 120 | 123, .. } => self.keys = Keys::new(),
            NoteEvent::PolyBrightness { channel, brightness, .. } => self.slide[channel as usize & 15] = brightness,
            _ => {}
        }
    }
}

impl<K: Kind> Plugin for SuuPlugin<K> {
    const NAME: &'static str = K::NAME;
    const VENDOR: &'static str = "JS";
    const URL: &'static str = "https://janne-s.github.io/Suu/";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = K::LAYOUTS;
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.setup(config.sample_rate as f64, config.max_buffer_size as usize)
    }

    fn editor(&mut self, _executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.shared.clone(), K::SYNTH)
    }

    fn reset(&mut self) {
        self.keys = Keys::new();
        self.bend = [0.0; 16];
        self.pressure = [0.0; 16];
        self.slide = [0.5; 16];
    }

    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let n = buffer.samples();
        if self.suu.is_none() || n > self.input.len() {
            for ch in buffer.as_slice() {
                ch.fill(0.0);
            }
            while let Some(e) = context.next_event() {
                self.note_event(e);
            }
            return ProcessStatus::Normal;
        }
        let transport = context.transport();
        let bpm = transport.tempo.unwrap_or(DEFAULT_BPM);
        let playing = transport.playing;

        let source: &[&mut [f32]] = if K::SYNTH { aux.inputs.first().map_or(&[], |b| b.as_slice_immutable()) } else { buffer.as_slice_immutable() };
        if source.is_empty() {
            self.input[..n].fill(0.0);
        } else {
            let k = 1.0 / source.len() as f32;
            for i in 0..n {
                self.input[i] = source.iter().map(|c| c[i]).sum::<f32>() * k;
            }
        }

        let song = if playing { transport.pos_beats() } else { None };
        if let Some(suu) = self.suu.as_mut() {
            suu.set_song_position(song);
        }
        self.window_exchange(n, bpm);
        self.render(n, bpm, playing, || context.next_event());
        for (i, ch) in buffer.as_slice().iter_mut().enumerate() {
            ch.copy_from_slice(if i == 1 { &self.output_r[..n] } else { &self.output[..n] });
        }
        ProcessStatus::Normal
    }
}

impl<K: Kind> ClapPlugin for SuuPlugin<K> {
    const CLAP_ID: &'static str = K::CLAP_ID;
    const CLAP_DESCRIPTION: Option<&'static str> = Some(K::DESCRIPTION);
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = K::CLAP_FEATURES;
}

impl<K: Kind> Vst3Plugin for SuuPlugin<K> {
    const VST3_CLASS_ID: [u8; 16] = K::VST3_CLASS_ID;
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = K::VST3_SUBCATEGORIES;
}
