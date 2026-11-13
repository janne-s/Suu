//! Suu's cost per scenario: `cargo run --release -p suu --example bench [seconds]`.
//! Prints, for each, the time to render that many seconds at 48 kHz in 128-sample blocks, as a
//! percentage of one core (real time = 100 %), the best of three runs.
use std::time::Instant;
use suu::{HORN_POINTS, Mod, Params, Shape, Source, Step, Suu, TextStep};

fn main() {
    let secs: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let fs = 48000.0;
    let n = (secs * fs) as usize;
    let noise: Vec<f32> = {
        let mut r = 0x1234_5678u32;
        (0..n).map(|_| { r ^= r << 13; r ^= r >> 17; r ^= r << 5; (r as f64 / u32::MAX as f64 - 0.5) as f32 * 0.4 }).collect()
    };
    let dict = "WOW  W AW1\nYEAH  Y AE1\nOH  OW1\nNOW  N AW1\n";
    let gate = Params { gate: true, ..Params::default() };
    type Setup = Box<dyn Fn(&mut Suu)>;
    let scenarios: Vec<(&str, Setup)> = vec![
        ("idle (no note)", Box::new(|_s: &mut Suu| {})),
        ("supersaw, size 1 (os 2)", Box::new(move |s: &mut Suu| s.set_params(gate))),
        ("saw, size 1", Box::new(move |s: &mut Suu| s.set_params(Params { source: Source::Saw, ..gate }))),
        ("size 0.4 (os 4)", Box::new(move |s: &mut Suu| { s.set_size(0.4); s.set_params(gate) })),
        ("size 0.6 (os 3)", Box::new(move |s: &mut Suu| { s.set_size(0.6); s.set_params(gate) })),
        ("size 1.3", Box::new(move |s: &mut Suu| { s.set_size(1.3); s.set_params(gate) })), ("size 2 (os 1)", Box::new(move |s: &mut Suu| { s.set_size(2.0); s.set_params(gate) })),
        ("size 4 (os 1)", Box::new(move |s: &mut Suu| { s.set_size(4.0); s.set_params(gate) })),
        ("Motion on all 9", Box::new(move |s: &mut Suu| {
            s.set_params(Params { morph: 0.5, ..gate });
            for t in 0..9 { s.set_mod(t, Mod { depth: 0.5, rate_hz: 0.7 + t as f64 * 0.3, sync: 0.0, shape: Shape::Sine, smooth: 0.0 }); }
        })),
        ("horn wave travelling", Box::new(move |s: &mut Suu| {
            let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 3.0 + 2.0 * (6.283 * 2.0 * i as f64 / HORN_POINTS as f64).sin()).collect();
            s.set_profile(&wave, true);
            s.set_params(Params { morph: 1.0, ..gate });
            s.set_mod(7, Mod { depth: 1.0, rate_hz: 1.0, sync: 0.0, shape: Shape::Saw, smooth: 0.0 });
        })),
        ("input + reson 0.8", Box::new(move |s: &mut Suu| s.set_params(Params { source: Source::Off, reson: 0.8, ..gate }))),
        ("folds 1", Box::new(move |s: &mut Suu| s.set_params(Params { folds: 1.0, ..gate }))),
        ("sequencer + text gate", Box::new(move |s: &mut Suu| {
            s.load_dict(dict);
            s.set_text("wow yeah oh yeah now");
            for i in 0..16 { s.set_step(i, Step { on: true, note: (i as i32 * 5) % 12, tie: false, vowel: (i % 5) as i32, accent: i % 4 == 0, len: 0.6 }); }
            s.set_playing(true);
        })),
        ("text loop", Box::new(move |s: &mut Suu| {
            s.load_dict(dict);
            s.set_params(Params { text_step: TextStep::Clock, ..gate });
            s.set_text("wow yeah oh yeah now");
        })),
    ];
    println!("{secs} s at 48 kHz, 128-sample blocks; % of one core (best of 3)");
    for (name, setup) in &scenarios {
        let mut best = f64::MAX;
        for _ in 0..3 {
            let mut s = Suu::new(fs);
            setup(&mut s);
            let mut out = vec![0f32; 128];
            let t = Instant::now();
            for blk in noise.chunks(128) {
                s.process(blk, &mut out[..blk.len()]);
            }
            best = best.min(t.elapsed().as_secs_f64());
            std::hint::black_box(&out);
        }
        println!("{:>26}  {:5.1} %", name, 100.0 * best / secs);
    }
}
