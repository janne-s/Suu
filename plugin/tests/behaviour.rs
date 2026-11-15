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
    run_t(p, total, block, events, input, false)
}

/// `run` with the host's transport `playing` or not.
fn run_t<K: suu_plugin::Kind>(p: &mut suu_plugin::SuuPlugin<K>, total: usize, block: usize, events: &[NoteEvent<()>], input: impl Fn(usize) -> f32, playing: bool) -> Vec<f32> {
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
        p.render(n, 120.0, playing, || evs.next());
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

/// PLAY runs the sequencer whether or not the host's transport plays (the owner pressed PLAY in
/// a stopped Live and heard nothing).
#[test]
fn sequencer_plays_without_the_transport() {
    let mut params = SuuParams::new(true);
    params.seq = BoolParam::new("Sequencer", true);
    let mut p = SuuSynth::with_params(params);
    assert!(p.setup(FS, 256));
    let out = run(&mut p, 48000, 256, &[], |_| 0.0);
    assert!(p.core().unwrap().playing());
    assert!(rms(&out[12000..]) > 0.01, "silent: {}", rms(&out[12000..]));
    // the transport starting starts the pattern over: step 0 again
    run_t(&mut p, 6000, 256, &[], |_| 0.0, false);
    run_t(&mut p, 256, 256, &[], |_| 0.0, true);
    assert_eq!(p.core().unwrap().seq_state().0, 0);
}

/// Notes back to back (one ends where the next starts) are separate notes: each starts the ADSR
/// and says the next part of the text (they ran together as one until a gap let the gate close).
#[test]
fn notes_back_to_back_are_separate() {
    for legato in [false, true] {
        let mut params = SuuParams::new(true);
        params.legato = BoolParam::new("Legato", legato);
        // a 20 ms attack to the top, then down to 30 % in 50 ms: a new note rises above the last's end
        params.attack = FloatParam::new("Attack", 0.02, FloatRange::Linear { min: 0.001, max: 10.0 });
        params.decay = FloatParam::new("Decay", 0.05, FloatRange::Linear { min: 0.001, max: 10.0 });
        params.sustain = FloatParam::new("Sustain", 0.3, FloatRange::Linear { min: 0.0, max: 1.0 });
        let mut p = SuuSynth::with_params(params);
        assert!(p.setup(FS, 512));
        // four notes of 0.2 s, each starting where the last ends (off and on at the same sample)
        let mut events = Vec::new();
        for i in 0..4u32 {
            let t = 4800 + i * 9600;
            if i > 0 {
                events.push(off(t, 55 + i as u8 - 1));
            }
            events.push(on(t, 55 + i as u8));
        }
        events.push(off(4800 + 4 * 9600, 58));
        // the ADSR's level at the end of each 64-sample block
        let mut level = Vec::new();
        let mut at = 0usize;
        while at < 48000 {
            let mut evs = events.iter().filter(|e| (e.timing() as usize) >= at && (e.timing() as usize) < at + 64).map(|e| self::at(*e, e.timing() - at as u32));
            p.render(64, 120.0, false, || evs.next());
            level.push(p.core().unwrap().adsr_level());
            at += 64;
        }
        // before each note: at sustain (30 %); within its first 40 ms: back at the top (before the
        // fix it stayed at sustain: the notes ran together)
        for i in 1..4usize {
            let t = (4800 + i * 9600) / 64;
            let before = level[t - 1];
            let top = level[t..t + 30].iter().fold(0f64, |m, v| m.max(*v));
            assert!(before < 0.4 && top > 0.9, "legato {legato}, note {i}: the ADSR starts again ({before} before, {top} at most after)");
        }
    }
}

/// The effect under ENV: after a MIDI note ends and the track goes on, the last note's pitch
/// stays (it went back to the root). (A changed root takes over again: `sync`; a parameter
/// cannot be changed from here.)
#[test]
fn the_last_note_stays_after_its_release() {
    let mut p = SuuFx::default();
    assert!(p.setup(FS, 512));
    // the track sounds throughout (ENV open); a note from 0.1 to 0.3 s
    let tone = |i: usize| 0.3 * ((i as f32) * 2.0 * std::f32::consts::PI * 110.0 / 48000.0).sin();
    run(&mut p, 24000, 256, &[on(4800, 64), off(14400, 64)], tone);
    assert_eq!(p.core().unwrap().params().note, 64.0, "the last note stays after its release");
}

/// A reset note in the clip starts the text over and does not sound; other notes go on saying it.
#[test]
fn a_reset_note_starts_the_text_over() {
    let mut params = SuuParams::new(true);
    params.text = BoolParam::new("Text", true);
    params.text_reset_note = IntParam::new("Text Reset Note", 24, IntRange::Linear { min: -1, max: 127 });
    let mut p = SuuSynth::with_params(params);
    assert!(p.setup(FS, 256));
    for _ in 0..300 {
        run(&mut p, 2560, 256, &[], |_| 0.0);
        if p.core().unwrap().syllables() > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(p.core().unwrap().syllables() > 3);
    // three notes say parts 0, 1, 2; then the reset note (C1), which does not sound; the next
    // note says part 0
    let mut ev = Vec::new();
    for i in 0..3u32 {
        ev.push(on(2400 + i * 9600, 57));
        ev.push(off(2400 + i * 9600 + 7200, 57));
    }
    // (in the quiet once the last part has ended: about 0.35 s after its note)
    ev.push(on(52000, 24));
    ev.push(off(54000, 24));
    let out = run(&mut p, 57600, 64, &ev, |_| 0.0);
    assert_eq!(p.core().unwrap().text_part(), 2);
    assert!(rms(&out[52000..57600]) < 1e-4, "the reset note does not sound");
    run(&mut p, 9600, 64, &[on(100, 57), off(7000, 57)], |_| 0.0);
    assert_eq!(p.core().unwrap().text_part(), 0, "the next note says the first part");
}

/// The same note again before its last note-off (a Note Length or an arpeggiator before the
/// plug-in, notes longer than the taps): it sounds until its own note-off. The first note's
/// note-off closed the gate (the key was kept once), so a repeated note was cut short, or did
/// not sound at all when it came just before that note-off.
#[test]
fn a_repeated_note_lasts_until_its_own_note_off() {
    let mut p = SuuSynth::default();
    assert!(p.setup(FS, 512));
    // 55 at 0.1 s, 55 again at 0.25 s, the first one's off at 0.35 s, the second's at 0.5 s
    let events = [on(4800, 55), on(12000, 55), off(16800, 55), off(24000, 55)];
    let mut open = Vec::new();
    let mut at = 0usize;
    while at < 36000 {
        let mut evs = events.iter().filter(|e| (e.timing() as usize) >= at && (e.timing() as usize) < at + 64).map(|e| self::at(*e, e.timing() - at as u32));
        p.render(64, 120.0, false, || evs.next());
        open.push((at, p.core().unwrap().adsr_level()));
        at += 64;
    }
    let level = |t: usize| open.iter().find(|(a, _)| *a >= t).unwrap().1;
    assert!(level(20000) > 0.5, "the repeated note still sounds after the first one's note-off: {}", level(20000));
    assert!(level(23000) > 0.5, "until its own note-off: {}", level(23000));
    // the release (80 ms) done 125 ms after it
    assert!(level(30000) < 0.01, "and ends there: {}", level(30000));
}
