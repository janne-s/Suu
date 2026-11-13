//! `chimera mouth <in.json> <outstem> [wave] [--tilt hz] [--flow cm3/s] [--breath x] [--body b]`:
//! experiment, the vocal tract as an effect. A synth plays the melody file's notes (the `render
//! notes` format: `{"bpm", "lyrics", "notes": [[midi, beats], ...]}`) and the airway articulates
//! the lyrics over it: the synth's signal enters as volume velocity just above the glottis
//! (`HumanVoice::render_driven`) while the folds are held still. Writes `<outstem>-dry.wav` (the
//! synth alone) and `<outstem>-mouth.wav`, and prints levels and the render speed.
//!
//! Waves: saw, square, pulse (25 %), triangle, supersaw (five saws, ±14 cents). `--tilt` is the
//! corner of a leaky integrator on the drive (0 = none): a synth wave falls 6 dB/octave, a glottal
//! flow 12, and the radiated output is the flow's derivative, so without it the result is a step
//! brighter than the voice. `--flow` is the drive's RMS over the file in cm³/s. `--breath` scales
//! the lung pressure (the consonants' air). `--sub hz` layers the dry synth's low end (low-passed
//! at hz, at the mouth's RMS) under the result, as talk-box basses are.

use crate::render::g2p;
use crate::{Ctx, FS, wav_name};
use chimera::anatomy::human::BodyMix;
use chimera::math::{PI, exp, js_max, js_round, midi_to_hz, pow, sqrt};
use chimera::phonetics::sing::{SongOptions, SungNote, plan_song};
use chimera::phonetics::syllables::lyrics_to_syllables;
use chimera::voice::human_voice::{HumanVoice, SourcePatch};
use std::time::Instant;

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

#[derive(Clone, Copy)]
enum Wave {
    Saw,
    Square,
    Pulse,
    Triangle,
    Supersaw,
}

struct Osc {
    phase: [f64; 5],
    tri: f64,
}

impl Osc {
    fn new() -> Self {
        // fixed, unequal start phases: the supersaw's voices must not start in phase
        Osc { phase: [0.0, 0.31, 0.57, 0.13, 0.83], tri: 0.0 }
    }

    fn saw(p: &mut f64, dt: f64) -> f64 {
        let y = 2.0 * *p - 1.0 - poly_blep(*p, dt);
        *p += dt;
        if *p >= 1.0 {
            *p -= 1.0;
        }
        y
    }

    fn pulse(p: &mut f64, dt: f64, width: f64) -> f64 {
        let mut y = if *p < width { 1.0 } else { -1.0 };
        y += poly_blep(*p, dt);
        let q = (*p + 1.0 - width) % 1.0;
        y -= poly_blep(q, dt);
        *p += dt;
        if *p >= 1.0 {
            *p -= 1.0;
        }
        y
    }

    fn next(&mut self, wave: Wave, hz: f64) -> f64 {
        let dt = hz / FS;
        match wave {
            Wave::Saw => Self::saw(&mut self.phase[0], dt),
            Wave::Square => Self::pulse(&mut self.phase[0], dt, 0.5),
            Wave::Pulse => Self::pulse(&mut self.phase[0], dt, 0.25),
            Wave::Triangle => {
                // leaky-integrated band-limited square
                let sq = Self::pulse(&mut self.phase[0], dt, 0.5);
                self.tri = 0.999 * self.tri + 4.0 * dt * sq;
                self.tri
            }
            Wave::Supersaw => {
                const CENTS: [f64; 5] = [0.0, -14.0, 14.0, -7.0, 7.0];
                let mut s = 0.0;
                for (k, c) in CENTS.iter().enumerate() {
                    s += Self::saw(&mut self.phase[k], dt * pow(2.0, c / 1200.0));
                }
                s * 0.45
            }
        }
    }
}

fn wave(name: &str) -> Result<Wave, String> {
    Ok(match name {
        "saw" => Wave::Saw,
        "square" => Wave::Square,
        "pulse" => Wave::Pulse,
        "triangle" => Wave::Triangle,
        "supersaw" => Wave::Supersaw,
        _ => return Err(format!("unknown wave {name:?}; have saw, square, pulse, triangle, supersaw")),
    })
}

fn rms_db(x: &[f32]) -> f64 {
    let s: f64 = x.iter().map(|&v| (v as f64) * (v as f64)).sum();
    20.0 * chimera::math::log10(sqrt(s / js_max(1.0, x.len() as f64)) + 1e-12)
}

fn flag(a: &[String], name: &str) -> Option<String> {
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

pub fn run(ctx: &Ctx, a: &[String]) -> Result<(), String> {
    let pos: Vec<&String> = {
        let mut v = Vec::new();
        let mut i = 0;
        while i < a.len() {
            if a[i].starts_with("--") {
                i += 2;
            } else {
                v.push(&a[i]);
                i += 1;
            }
        }
        v
    };
    let input = pos.first().ok_or("mouth <in.json> <outstem> [wave] [--tilt hz] [--flow cm3/s] [--breath x] [--sub hz] [--body male|female|child]")?;
    let stem = pos.get(1).map(|s| s.as_str()).unwrap_or("out/mouth");
    let stem = wav_name(stem);
    let stem = stem.strip_suffix(".wav").unwrap();
    let w = wave(pos.get(2).map(|s| s.as_str()).unwrap_or("saw"))?;
    let tilt: f64 = flag(a, "--tilt").map(|s| s.parse().map_err(|_| "--tilt: not a number")).transpose()?.unwrap_or(150.0);
    let flow: f64 = flag(a, "--flow").map(|s| s.parse().map_err(|_| "--flow: not a number")).transpose()?.unwrap_or(80.0);
    let breath: f64 = flag(a, "--breath").map(|s| s.parse().map_err(|_| "--breath: not a number")).transpose()?.unwrap_or(1.0);
    let sub: f64 = flag(a, "--sub").map(|s| s.parse().map_err(|_| "--sub: not a number")).transpose()?.unwrap_or(0.0);
    let body_name = flag(a, "--body").unwrap_or_else(|| "male".to_string());
    let b = crate::body(&body_name)?;
    let mix: BodyMix = crate::body_mix(&body_name)?;

    let text = std::fs::read_to_string(input.as_str()).map_err(|e| format!("{input}: {e}"))?;
    let j: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{input}: {e}"))?;
    let bpm = j["bpm"].as_f64().ok_or("bpm missing")?;
    let glide = j["glide"].as_f64().unwrap_or(0.02);
    let raw: Vec<[f64; 2]> = serde_json::from_value(j["notes"].clone()).map_err(|e| format!("notes: {e}"))?;
    let g = g2p()?;
    let syl = lyrics_to_syllables(j["lyrics"].as_str().ok_or("lyrics missing")?, &g);
    let sounding = raw.iter().filter(|n| n[0] > 0.0).count();
    if syl.len() != sounding {
        return Err(format!("{} syllables for {sounding} notes: {}", syl.len(), syl.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ")));
    }
    let t0 = j["t0"].as_f64().unwrap_or(0.3);
    let beat = 60.0 / bpm;

    // the voice: folds still (no vibrating area), lungs and glottal chink as usual
    let mut v = HumanVoice::new(FS, b, 2);
    v.glottal_area_scale = 0.0;
    v.set_source(&SourcePatch { voiced: Some(true), pressure: Some(9.0 * breath), tension: Some(0.55), drift: Some(0.0), ..Default::default() });

    // plan the lyrics phrase by phrase (rests split phrases), and lay out the synth's notes:
    // (start, end, midi). A phrase's first note starts a little early so its onset consonant has a
    // tone to shape, as a talk-box player starts the note with the word.
    let mut notes: Vec<(f64, f64, f64)> = Vec::new();
    let (mut t, mut k, mut end) = (t0, 0usize, 0.0f64);
    let mut phrase: Vec<SungNote> = Vec::new();
    let mut start = t;
    let flush = |phrase: &mut Vec<SungNote>, start: f64, v: &mut HumanVoice, end: &mut f64| {
        if phrase.is_empty() {
            return;
        }
        let plan = plan_song(phrase, &SongOptions { body: mix, bpm, t0: start, vibrato: 0.0 });
        v.schedule(&plan.gestures);
        *end = js_max(*end, plan.end);
        phrase.clear();
    };
    let mut prev_rest = true;
    for n in &raw {
        if n[0] > 0.0 {
            if phrase.is_empty() {
                start = t;
            }
            phrase.push(SungNote { midi: n[0], beats: n[1], syl: syl[k].clone() });
            k += 1;
            let lead_in = if prev_rest { 0.08 } else { 0.0 };
            notes.push((t - lead_in, t + n[1] * beat, n[0]));
            prev_rest = false;
        } else {
            flush(&mut phrase, start, &mut v, &mut end);
            prev_rest = true;
        }
        t += n[1] * beat;
    }
    flush(&mut phrase, start, &mut v, &mut end);
    end = js_max(end, t);

    // the synth: legato within a phrase (pitch glides), 5 ms attack, 40 ms release at rests
    let n_out = js_round(FS * (end + 0.5)) as usize;
    let mut dry = vec![0f32; n_out];
    let mut osc = Osc::new();
    let mut hz = midi_to_hz(notes.first().map_or(60.0, |n| n.2));
    let mut env = 0.0;
    let glide_c = 1.0 - exp(-1.0 / (js_max(0.001, glide) * FS));
    let att = 1.0 - exp(-1.0 / (0.005 * FS));
    let rel = 1.0 - exp(-1.0 / (0.04 * FS));
    let mut ni = 0usize;
    for (i, o) in dry.iter_mut().enumerate() {
        let ts = i as f64 / FS;
        while ni + 1 < notes.len() && ts >= notes[ni + 1].0 {
            ni += 1;
        }
        let on = !notes.is_empty() && ts >= notes[ni].0 && ts < notes[ni].1;
        if on {
            let target = midi_to_hz(notes[ni].2);
            // a note after a rest starts on its pitch; legato notes glide
            if env < 1e-3 {
                hz = target;
            } else {
                hz *= pow(target / hz, glide_c);
            }
        }
        env += ((if on { 1.0 } else { 0.0 }) - env) * (if on { att } else { rel });
        *o = (osc.next(w, hz) * env * 0.5) as f32;
    }

    // the drive: the dry synth through the leaky integrator (tilt), normalised so `--flow` is the
    // drive's RMS in cm³/s whatever the wave
    let mut drive = vec![0f32; n_out];
    if tilt > 0.0 {
        let c = exp(-2.0 * PI * tilt / FS);
        let mut y = 0.0;
        for (d, &x) in drive.iter_mut().zip(&dry) {
            y = c * y + (1.0 - c) * x as f64;
            *d = y as f32;
        }
    } else {
        drive.copy_from_slice(&dry);
    }
    let dr = sqrt(drive.iter().map(|&x| (x as f64) * (x as f64)).sum::<f64>() / n_out as f64);
    let scale = flow / js_max(1e-9, dr);

    let mut out = vec![0f32; n_out];
    let t_start = Instant::now();
    for (ob, ib) in out.chunks_mut(128).zip(drive.chunks(128)) {
        v.render_driven(ib, scale, ob);
    }
    let secs = t_start.elapsed().as_secs_f64();
    let peak = crate::peak(&out);
    if sub > 0.0 {
        // the dry synth's low end under the mouth, at the mouth's RMS (two one-pole low-passes):
        // a mouth radiates little below ~150 Hz, so a talk-box bass is layered with its synth
        let c = exp(-2.0 * PI * sub / FS);
        let (mut y1, mut y2) = (0.0, 0.0);
        let low: Vec<f32> = dry
            .iter()
            .map(|&x| {
                y1 = c * y1 + (1.0 - c) * x as f64;
                y2 = c * y2 + (1.0 - c) * y1;
                y2 as f32
            })
            .collect();
        let g = pow(10.0, (rms_db(&out) - rms_db(&low)) / 20.0);
        for (o, l) in out.iter_mut().zip(&low) {
            *o += (*l as f64 * g) as f32;
        }
        let p = crate::peak(&out);
        if p > 0.98 {
            for o in out.iter_mut() {
                *o = (*o as f64 * 0.98 / p) as f32;
            }
        }
    }
    ctx.write_wav(&format!("{stem}-dry.wav"), &dry)?;
    ctx.write_wav(&format!("{stem}-mouth.wav"), &out)?;
    println!(
        "{stem}: {:.2} s audio, render {:.3} s ({:.1}x real time), mouth peak {:.3} rms {:.1} dBFS, dry rms {:.1} dBFS",
        n_out as f64 / FS,
        secs,
        (n_out as f64 / FS) / secs,
        peak,
        rms_db(&out),
        rms_db(&dry)
    );
    if peak > 0.74 {
        println!("  warning: peak reached the voice's output limiter (0.75); lower --flow");
    }
    Ok(())
}
