use std::time::Instant;
use suu::{HORN_POINTS, Mod, Mouth, Params, PitchFrom, STEPS, Shape, Source, Step, Suu, TextStep};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs: f64 = args.first().and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let filter = args.get(1).cloned().unwrap_or_default();
    let fs = 48000.0;
    let n = (secs * fs) as usize;
    let mut r = 0x1234_5678u32;
    let noise: Vec<f32> = (0..n)
        .map(|_| {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            (r as f64 / u32::MAX as f64 - 0.5) as f32 * 0.4
        })
        .collect();
    let mut ph = 0.0f64;
    let saw: Vec<f32> = (0..n)
        .map(|_| {
            ph = (ph + 110.0 / fs) % 1.0;
            (0.3 * (2.0 * ph - 1.0)) as f32
        })
        .collect();
    let dict = "WOW  W AW1\nYEAH  Y AE1\nOH  OW1\nNOW  N AW1\n";
    let gate = Params { gate: true, ..Params::default() };
    type Setup = Box<dyn Fn(&mut Suu)>;
    let scenarios: Vec<(&str, Setup, bool)> = vec![
        ("idle (no note)", Box::new(|_s: &mut Suu| {}), false),
        ("held supersaw (os 2)", Box::new(move |s: &mut Suu| s.set_params(gate)), false),
        ("saw, size 1", Box::new(move |s: &mut Suu| s.set_params(Params { source: Source::Saw, ..gate })), false),
        ("size 0.4 (os 4)", Box::new(move |s: &mut Suu| { s.set_size(0.4); s.set_params(gate) }), false),
        ("size 2 (os 1)", Box::new(move |s: &mut Suu| { s.set_size(2.0); s.set_params(gate) }), false),
        ("Motion on all 9", Box::new(move |s: &mut Suu| {
            s.set_params(Params { morph: 0.5, ..gate });
            for t in 0..9 { s.set_mod(t, Mod { depth: 0.5, rate_hz: 0.7 + t as f64 * 0.3, sync: 0.0, shape: Shape::Sine, smooth: 0.0 }); }
        }), false),
        ("horn wave travelling", Box::new(move |s: &mut Suu| {
            let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 3.0 + 2.0 * (6.283 * 2.0 * i as f64 / HORN_POINTS as f64).sin()).collect();
            s.set_profile(&wave, true);
            s.set_params(Params { morph: 1.0, ..gate });
            s.set_mod(7, Mod { depth: 1.0, rate_hz: 1.0, sync: 0.0, shape: Shape::Saw, smooth: 0.0 });
        }), false),
        ("sequencer + text", Box::new(move |s: &mut Suu| {
            s.load_dict(dict);
            s.set_text("wow yeah oh yeah now");
            for i in 0..STEPS { s.set_step(i, Step { on: true, note: (i as i32 * 5) % 12, tie: false, vowel: (i % 5) as i32, accent: i % 4 == 0, len: 0.6 }); }
            s.set_playing(true);
        }), false),
        ("text loop", Box::new(move |s: &mut Suu| {
            s.load_dict(dict);
            s.set_params(Params { text_step: TextStep::Clock, ..gate });
            s.set_text("wow yeah oh yeah now");
        }), false),
        ("input + reson 0.8", Box::new(move |s: &mut Suu| s.set_params(Params { source: Source::Off, reson: 0.8, ..gate })), false),
        ("input saw, PITCH + VOWEL IN", Box::new(move |s: &mut Suu| s.set_params(Params { source: Source::Off, pitch_from: PitchFrom::Input, vowel_in: true, ..gate })), true),
        ("folds 1", Box::new(move |s: &mut Suu| s.set_params(Params { folds: 1.0, ..gate })), false),
        ("HQ off: held supersaw", Box::new(move |s: &mut Suu| { s.set_params(Params { hq: false, ..gate }); s.set_size(1.0) }), false),
        ("HQ off: size 0.4", Box::new(move |s: &mut Suu| { s.set_params(Params { hq: false, ..gate }); s.set_size(0.4) }), false),
        ("HQ off: Motion on all 9", Box::new(move |s: &mut Suu| {
            s.set_params(Params { morph: 0.5, hq: false, ..gate });
            s.set_size(1.0);
            for t in 0..9 { s.set_mod(t, Mod { depth: 0.5, rate_hz: 0.7 + t as f64 * 0.3, sync: 0.0, shape: Shape::Sine, smooth: 0.0 }); }
        }), false),
        ("LO: held supersaw", Box::new(move |s: &mut Suu| { s.set_params(Params { hq: false, lofi: true, ..gate }); s.set_size(1.0) }), false),
        ("LO: size 0.4", Box::new(move |s: &mut Suu| { s.set_params(Params { hq: false, lofi: true, ..gate }); s.set_size(0.4) }), false),
        ("LO: Motion on all 9", Box::new(move |s: &mut Suu| {
            s.set_params(Params { morph: 0.5, hq: false, lofi: true, ..gate });
            s.set_size(1.0);
            for t in 0..9 { s.set_mod(t, Mod { depth: 0.5, rate_hz: 0.7 + t as f64 * 0.3, sync: 0.0, shape: Shape::Sine, smooth: 0.0 }); }
        }), false),
        ("stereo HQ: held supersaw", Box::new(move |s: &mut Suu| { s.set_params(Params { stereo: true, ..gate }); let w = s.wants_twin().unwrap(); let _ = s.set_twin(Mouth::build(&s.mouth_seed(w))); }), false),
        ("stereo LEAN: held supersaw", Box::new(move |s: &mut Suu| { s.set_params(Params { stereo: true, hq: false, ..gate }); s.set_size(1.0); let w = s.wants_twin().unwrap(); let _ = s.set_twin(Mouth::build(&s.mouth_seed(w))); }), false),
        ("stereo LO: held supersaw", Box::new(move |s: &mut Suu| { s.set_params(Params { stereo: true, hq: false, lofi: true, ..gate }); s.set_size(1.0); let w = s.wants_twin().unwrap(); let _ = s.set_twin(Mouth::build(&s.mouth_seed(w))); }), false),
        ("size automation (rebuilds)", Box::new(move |s: &mut Suu| s.set_params(gate)), false),
    ];
    let period = 128.0 / fs;
    println!("{secs} s at 48 kHz, 128-sample blocks, fastest of 5: % of one core; block p50 / p99 / max as % of its period; output hash");
    for (name, setup, use_saw) in &scenarios {
        if !name.contains(filter.as_str()) {
            continue;
        }
        let input = if *use_saw { &saw } else { &noise };
        let resize = name.starts_with("size automation");
        let (mut best, mut blocks, mut hash) = (f64::MAX, Vec::new(), 0u64);
        for run in 0..5 {
            let mut s = Suu::new(fs);
            setup(&mut s);
            let mut out = vec![0f32; n];
            let mut times = Vec::with_capacity(n / 128 + 1);
            for (i, (o, x)) in out.chunks_mut(128).zip(input.chunks(128)).enumerate() {
                if resize && i % 20 == 0 {
                    s.set_size(0.8 + 0.4 * ((i / 20) % 2) as f64);
                }
                let b = Instant::now();
                let mut r = [0f32; 128];
                s.process_stereo(x, o, &mut r[..o.len()]);
                times.push(b.elapsed().as_secs_f64());
            }
            let el: f64 = times.iter().sum();
            if el < best {
                best = el;
                blocks = times;
            }
            if run == 0 {
                hash = out.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ v.to_bits() as u64).wrapping_mul(0x0100_0000_01b3));
            }
        }
        blocks.sort_by(f64::total_cmp);
        let at = |q: f64| 100.0 * blocks[((blocks.len() - 1) as f64 * q) as usize] / period;
        println!("{name:>28}  {:5.1} %   {:5.1} / {:5.1} / {:5.1} %   {hash:016x}", 100.0 * best / secs, at(0.5), at(0.99), at(1.0));
    }
}
