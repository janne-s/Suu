//! Suu as a plug-in, in two builds from one core: **Suu** (an effect: the track's sound through
//! the mouth, MIDI from another track) and **Suu Synth** (an instrument: the oscillator through
//! the mouth, a sidechain as the input). Every control is a host parameter (`params`).
//!
//! Threads: the audio thread owns the `Suu`. What allocates (building a mouth of a new size,
//! planning text, dropping the old ones) runs on a worker per instance, which polls its queue so
//! the audio thread never wakes it (no system call there). Results come back over a bounded
//! queue and are put in place without allocating; each carries the generation of the `Suu` it
//! was made for, and one made for an older `Suu` (the state was reloaded) is thrown away.

use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};
use nih_plug::prelude::*;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;
use suu::{Mod, Mouth, MouthSeed, N_MOD, STEPS, Step, Suu, TextPlan};

mod editor;
pub mod params;
use params::{GateMode, MAX_WORDS, SuuParams};

use chimera::phonetics::english::EnglishG2P;

/// The pronouncing dictionary (CMUdict, licence in `public/data/CMUDICT-LICENSE.txt`), shared by
/// every instance and loaded on first use, on a worker.
static CMUDICT_GZ: &[u8] = include_bytes!("../../public/data/cmudict.txt.gz");
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

/// The length of `Suu::ui_state`'s frame.
const UI_LEN: usize = 24 + suu::UI_AREAS;
/// How often the audio thread leaves the window a frame of the mouth (s).
const UI_EVERY: f64 = 1.0 / 30.0;

/// What the audio thread and the window share, without locks: the mouth's last frame and the
/// tempo (atomics), the drawn horn (a bounded queue), a source snapshot on request.
pub struct Shared {
    ui: [AtomicU32; UI_LEN],
    ui_len: AtomicUsize,
    /// counts frames written, so the window sends each once
    ui_seq: AtomicU64,
    bpm: AtomicU64,
    /// counts cores made (`setup`): a new one means a reloaded state
    generation: AtomicU64,
    horn: (Sender<([f64; suu::HORN_POINTS], bool)>, Receiver<([f64; suu::HORN_POINTS], bool)>),
    /// the words changed: plan the text again
    words_changed: AtomicBool,
    /// back to the built-in horn
    horn_reset: AtomicBool,
    /// 0 idle, 1 asked for, 2 ready, 3 ready but silent
    snap_state: AtomicU8,
    snap: [AtomicU32; suu::HORN_POINTS],
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
        }
    }
}

const QUEUE: usize = 8;
/// How often the worker looks at its queue.
const POLL: Duration = Duration::from_millis(4);
const DEFAULT_BPM: f64 = 120.0;

enum Job {
    Size { generation: u64, seed: MouthSeed },
    Text { generation: u64, params: suu::Params, on: bool },
    Drop(Junk),
}

/// Something the audio thread is done with, to be freed on the worker (held only to be dropped).
#[allow(dead_code)]
enum Junk {
    Mouth(Mouth),
    Plan(TextPlan),
}

enum Done {
    Mouth { generation: u64, mouth: Mouth },
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
            Job::Text { generation, params, on } => {
                let text = if on { words.read().map(|w| w.chars().take(MAX_WORDS).collect::<String>()).unwrap_or_default() } else { String::new() };
                Done::Text { generation, plan: TextPlan::new(&text, Some(g2p()), &params) }
            }
            Job::Drop(junk) => {
                drop(junk);
                continue;
            }
        };
        // the audio thread drains this every block; wait if it is (briefly) full
        if done.send(result).is_err() {
            return;
        }
    }
}

/// What differs between the two builds.
pub trait Kind: 'static + Send + Sync {
    const SYNTH: bool;
    const NAME: &'static str;
    const CLAP_ID: &'static str;
    const DESCRIPTION: &'static str;
    const CLAP_FEATURES: &'static [ClapFeature];
    /// Never change: hosts know the plug-in by it.
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

/// Held keys, last first (monophonic, last-note priority). Fixed size: no allocation.
struct Keys {
    notes: [u8; 128],
    channels: [u8; 128],
    len: usize,
}

impl Keys {
    fn new() -> Self {
        Keys { notes: [0; 128], channels: [0; 128], len: 0 }
    }
    fn remove(&mut self, note: u8) {
        if let Some(i) = self.notes[..self.len].iter().position(|&n| n == note) {
            self.notes.copy_within(i + 1..self.len, i);
            self.channels.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }
    fn press(&mut self, note: u8, channel: u8) {
        self.remove(note);
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
    /// the mouth's frame for the window (capacity reserved: no allocation), and samples to the next
    frame: Vec<f32>,
    frame_in: f64,
    fs: f64,
    suu: Option<Box<Suu>>,
    generation: u64,
    jobs: Option<Sender<Job>>,
    done: Option<Receiver<Done>>,
    /// mono input and output, sized to the host's largest block
    input: Vec<f32>,
    output: Vec<f32>,
    keys: Keys,
    /// the last key played (its pitch stays after the release, until the root changes), and the
    /// root it was played over
    last_key: Option<(u8, u8)>,
    last_root: i32,
    /// pitch bend per channel, −1 … 1
    bend: [f32; 16],
    size_job: bool,
    text_job: bool,
    /// whether the text was last planned on (None: plan it)
    planned_text: Option<bool>,
    mouth: Option<Mouth>,
    junk: Option<Junk>,
    mods: [Mod; N_MOD],
    steps: [Step; STEPS],
    seq: (usize, f64, f64),
    /// the host's transport was playing (last block)
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
            keys: Keys::new(),
            last_key: None,
            last_root: i32::MIN,
            bend: [0.0; 16],
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
    /// The sounding note (with bend) and whether the sound is open.
    fn note_and_gate(&self) -> (f64, bool) {
        let p = &self.params;
        let mode = p.gate.value();
        // with no key, the last key's pitch stays (its release, and the input going on under ENV
        // or HOLD), until the root is changed
        let key = self.keys.top().or(self.last_key);
        let (note, bend) = match key {
            Some((n, ch)) => (n as f64, self.bend[ch as usize & 15]),
            None => (p.root.value() as f64, self.bend[0]),
        };
        (note + bend as f64 * p.bend_range.value() as f64, mode == GateMode::Hold || self.keys.len > 0)
    }

    /// Hand something to the worker to free; kept (and retried) if its queue is full.
    fn discard(&mut self, junk: Junk) {
        let Some(jobs) = &self.jobs else { return };
        if let Err(e) = jobs.try_send(Job::Drop(junk)) {
            let Job::Drop(j) = e.into_inner() else { unreachable!() };
            self.junk = Some(j);
        }
    }

    /// Results from the worker, and old things back to it.
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

    /// Bring the core in line with the host's parameters (only what changed), and ask the
    /// worker for a new mouth or text plan when they are due.
    fn sync(&mut self, bpm: f64, playing: bool) {
        // a changed root takes over from the last key
        let root = self.params.root.value();
        if root != self.last_root {
            if self.last_root != i32::MIN {
                self.last_key = None;
            }
            self.last_root = root;
        }
        let (note, gate) = self.note_and_gate();
        let p = self.params.clone();
        let Some(suu) = self.suu.as_mut() else { return };
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
        // PLAY runs the sequencer at the host's tempo, transport or not; the transport starting
        // starts the pattern over, so it lines up with the song's start
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
        // a new size, or HQ switched: a new mouth (HQ is already in the core's parameters)
        if !self.size_job && self.mouth.is_none() && (size != suu.size() || p.hq.value() != suu.hq()) {
            // (the seed is plain values: a full queue just drops it, and it is asked for again)
            if jobs.try_send(Job::Size { generation: self.generation, seed: suu.mouth_seed(size) }).is_ok() {
                self.size_job = true;
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

    /// One block of `n` samples from `self.input` into `self.output`, with the block's note
    /// events (in time order), split at them so the result does not depend on the host's block
    /// size.
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
                suu.process(&self.input[pos..end], &mut self.output[pos..end]);
            }
            pos = end;
        }
        while let Some(e) = next {
            self.note_event(e);
            next = events();
        }
    }

    /// A plug-in with these parameters (the tests' way to set them: a host sets them through
    /// its wrapper).
    #[doc(hidden)]
    pub fn with_params(params: SuuParams) -> Self {
        SuuPlugin { params: Arc::new(params), ..Self::default() }
    }

    /// The mono input and output buffers (for the tests).
    #[doc(hidden)]
    pub fn io(&mut self) -> (&mut [f32], &[f32]) {
        (&mut self.input, &self.output)
    }

    #[doc(hidden)]
    pub fn core(&self) -> Option<&Suu> {
        self.suu.as_deref()
    }

    #[doc(hidden)]
    pub fn plugin_params(&self) -> &SuuParams {
        &self.params
    }

    /// Make a new core for rate `fs` and blocks up to `max_block` (from `initialize`; also the
    /// tests' way in). Allocates.
    #[doc(hidden)]
    pub fn setup(&mut self, fs: f64, max_block: usize) -> bool {
        self.setup_at_size(fs, max_block, self.params.size.value() as f64)
    }

    /// `setup` with the mouth made at `size` (a test starts from another size than the knob's).
    #[doc(hidden)]
    pub fn setup_at_size(&mut self, fs: f64, max_block: usize, size: f64) -> bool {
        let n = max_block.max(1);
        self.input = vec![0.0; n];
        self.output = vec![0.0; n];
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
        // a new core (a new rate, or a reloaded state): results for the old one are dropped
        self.generation += 1;
        let mut s = Box::new(Suu::new(fs));
        let p = &self.params;
        s.set_size(size);
        if let Ok(h) = p.horn.read()
            && h.areas.len() == suu::HORN_POINTS
        {
            s.set_profile(&h.areas, h.wrap);
        }
        // everything else follows in the first block's `sync`
        self.mods = [Mod::default(); N_MOD];
        self.steps = suu::default_pattern();
        self.seq = (16, 4.0, 1.0);
        self.suu = Some(s);
        self.mouth = None;
        (self.size_job, self.text_job, self.planned_text) = (false, false, None);
        self.keys = Keys::new();
        self.bend = [0.0; 16];
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        true
    }

    /// What the window left for the audio thread (the horn, a snapshot asked for, new
    /// words), and the mouth's frame for it every `UI_EVERY`. No allocation, no lock.
    fn window_exchange(&mut self, n: usize, bpm: f64) {
        let shared = self.shared.clone();
        if shared.words_changed.swap(false, Ordering::AcqRel) {
            self.planned_text = None;
        }
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
        // the text's reset note: starts the text over, and does not sound
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
                        // no other key held: a new note, even straight after the last one (back
                        // to back, the gate never closed between them and they ran together)
                        suu.note_start();
                    } else if sounding {
                        // a key over another (legato on the keys): a new note for the text
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
            NoteEvent::Choke { note, .. } => self.keys.remove(note),
            NoteEvent::MidiPitchBend { channel, value, .. } => self.bend[channel as usize & 15] = (value - 0.5) * 2.0,
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
    }

    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let n = buffer.samples();
        if self.suu.is_none() || n > self.input.len() {
            for ch in buffer.as_slice() {
                ch.fill(0.0);
            }
            return ProcessStatus::Normal;
        }
        let transport = context.transport();
        let bpm = transport.tempo.unwrap_or(DEFAULT_BPM);
        let playing = transport.playing;

        // the input, mono: the track (effect) or the sidechain (instrument)
        let source: &[&mut [f32]] = if K::SYNTH { aux.inputs.first().map_or(&[], |b| b.as_slice_immutable()) } else { buffer.as_slice_immutable() };
        if source.is_empty() {
            self.input[..n].fill(0.0);
        } else {
            let k = 1.0 / source.len() as f32;
            for i in 0..n {
                self.input[i] = source.iter().map(|c| c[i]).sum::<f32>() * k;
            }
        }

        // GRID's lines and Motion's synced cycles follow the song's position while it plays
        let song = if playing { transport.pos_beats() } else { None };
        if let Some(suu) = self.suu.as_mut() {
            suu.set_song_position(song);
        }
        self.window_exchange(n, bpm);
        self.render(n, bpm, playing, || context.next_event());
        for ch in buffer.as_slice() {
            ch.copy_from_slice(&self.output[..n]);
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
