//! The plug-in without a host: notes, the worker's mouths and text plans, and block size.

use nih_plug::prelude::*;
use suu_plugin::params::SuuParams;
use suu_plugin::{SuuFx, SuuSynth};

const FS: f64 = 48000.0;

/// Move an event to `timing`.
fn at(e: NoteEvent<()>, timing: u32) -> NoteEvent<()> {
    match e {
        NoteEvent::NoteOn { voice_id, channel, note, velocity, .. } => NoteEvent::NoteOn { timing, voice_id, channel, note, velocity },
        NoteEvent::NoteOff { voice_id, channel, note, velocity, .. } => NoteEvent::NoteOff { timing, voice_id, channel, note, velocity },
        other => other,
    }
}

fn on(timing: u32, note: u8) -> NoteEvent<()> {
    NoteEvent::NoteOn { timing, voice_id: None, channel: 0, note, velocity: 0.8 }
}

fn off(timing: u32, note: u8) -> NoteEvent<()> {
    NoteEvent::NoteOff { timing, voice_id: None, channel: 0, note, velocity: 0.0 }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Render `total` samples in blocks of `block`, with `events` at absolute sample times, the
/// input from `input`; returns the output.
fn run<K: suu_plugin::Kind>(p: &mut suu_plugin::SuuPlugin<K>, total: usize, block: usize, events: &[NoteEvent<()>], input: impl Fn(usize) -> f32) -> Vec<f32> {
    let mut out = Vec::with_capacity(total);
    let mut at = 0;
    while at < total {
        let n = block.min(total - at);
        let (inp, _) = p.io();
        for (i, x) in inp[..n].iter_mut().enumerate() {
            *x = input(at + i);
        }
        let start = at;
        let mut evs = events.iter().filter(|e| (e.timing() as usize) >= start && (e.timing() as usize) < start + n).map(|e| self::at(*e, e.timing() - start as u32));
        p.render(n, 120.0, false, || evs.next());
        out.extend_from_slice(&p.io().1[..n]);
        at += n;
    }
    out
}

#[test]
fn synth_sounds_with_a_note_only() {
    let mut p = SuuSynth::default();
    assert!(p.setup(FS, 512));
    let out = run(&mut p, 48000, 256, &[on(12000, 57), off(30000, 57)], |_| 0.0);
    let before = rms(&out[2000..11000]);
    let during = rms(&out[18000..29000]);
    let after = rms(&out[44000..48000]);
    assert!(before < 1e-4, "sound before the note: {before}");
    assert!(during > 0.01, "too quiet with the note: {during}");
    assert!(after < during * 0.05, "still sounding after the release: {after} vs {during}");
}

#[test]
fn effect_passes_the_track_without_notes() {
    let mut p = SuuFx::default();
    assert!(p.setup(FS, 512));
    let mut r = 0x1234_5678u32;
    let noise: Vec<f32> = (0..48000).map(|_| { r ^= r << 13; r ^= r >> 17; r ^= r << 5; (r as f32 / u32::MAX as f32 - 0.5) * 0.4 }).collect();
    let out = run(&mut p, 48000, 256, &[], |i| noise[i]);
    assert!(rms(&out[24000..]) > 0.01, "the effect does not pass the track: {}", rms(&out[24000..]));
}

/// The result does not depend on the host's block size (no text or size change: those arrive
/// from the worker whenever it is done).
#[test]
fn output_is_independent_of_block_size() {
    let events = [on(3000, 57), on(9001, 60), off(15000, 60), off(20000, 57), on(26000, 50), off(40000, 50)];
    let render = |block: usize| {
        // Motion on the vowel, so the mouth moves while the notes come and go
        let mut params = SuuParams::new(true);
        params.vowel_x.depth = FloatParam::new("Vowel X Depth", 0.5, FloatRange::Linear { min: -1.0, max: 1.0 });
        let mut p = SuuSynth::with_params(params);
        assert!(p.setup(FS, 1024));
        run(&mut p, 48000, block, &events, |_| 0.0)
    };
    let a = render(64);
    for block in [1000, 1024, 37] {
        let b = render(block);
        let worst = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert_eq!(worst, 0.0, "block {block} differs from block 64 by {worst}");
    }
}

/// A size change and the text come from the worker and are put in place while it plays.
#[test]
fn worker_builds_mouth_and_plans_text() {
    let mut params = SuuParams::new(true);
    params.size = FloatParam::new("Size", 1.6, FloatRange::Linear { min: 0.4, max: 4.0 });
    params.text = BoolParam::new("Text", true);
    let mut p = SuuSynth::with_params(params);
    assert!(p.setup(FS, 256));
    // setup builds the mouth at its size; the text comes from the worker
    assert!((p.core().unwrap().size() - 1.6).abs() < 1e-3);
    for _ in 0..300 {
        run(&mut p, 2560, 256, &[], |_| 0.0);
        if p.core().unwrap().syllables() > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(p.core().unwrap().syllables(), 7, "the default words' syllables");
}

/// A size change while it plays goes through the worker and crossfades in.
#[test]
fn size_change_comes_from_the_worker() {
    let mut params = SuuParams::new(true);
    // the knob's value differs from the one the core was made with: a change to make
    params.size = FloatParam::new("Size", 2.0, FloatRange::Linear { min: 0.4, max: 4.0 });
    let mut p = SuuSynth::with_params(params);
    assert!(p.setup_at_size(FS, 256, 1.0));
    assert_eq!(p.core().unwrap().size(), 1.0);
    let mut out = Vec::new();
    for _ in 0..300 {
        out.extend(run(&mut p, 2560, 256, &[on(0, 45)], |_| 0.0));
        if p.core().unwrap().size() == 2.0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(p.core().unwrap().size(), 2.0);
    let jump = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(jump < 0.2, "a jump of {jump}");
}
