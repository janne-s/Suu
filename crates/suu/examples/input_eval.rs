use suu::{InShape, Params, PitchFrom, Source, Suu};

fn wav16(path: &str) -> (f64, Vec<f32>) {
    let b = std::fs::read(path).expect("read the WAV");
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let (mut i, mut fs, mut ch) = (12, 0.0, 1);
    while i + 8 <= b.len() {
        let (id, len) = (&b[i..i + 4], u32_at(i + 4) as usize);
        if id == b"fmt " {
            assert_eq!((u16_at(i + 8), u16_at(i + 22)), (1, 16), "16-bit PCM only");
            ch = u16_at(i + 10) as usize;
            fs = u32_at(i + 12) as f64;
        } else if id == b"data" {
            let d = &b[i + 8..(i + 8 + len).min(b.len())];
            return (fs, d.chunks_exact(2 * ch).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect());
        }
        i += 8 + len + (len & 1);
    }
    panic!("no data chunk");
}

fn write_wav(path: &str, fs: f64, x: &[f32]) {
    let data = (2 * x.len()) as u32;
    let mut b = Vec::with_capacity(44 + 2 * x.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&(fs as u32).to_le_bytes());
    b.extend_from_slice(&(fs as u32 * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for v in x {
        b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
    }
}

fn hihat(fs: f64, secs: f64) -> Vec<f32> {
    let mut r = Lcg(7);
    let n = (secs * fs) as usize;
    let step = (fs * 60.0 / 120.0 / 4.0) as usize;
    let c = (-2.0 * std::f64::consts::PI * 6000.0 / fs).exp();
    let (mut x1, mut y1, mut x2, mut y2) = (0.0, 0.0, 0.0, 0.0);
    (0..n)
        .map(|i| {
            let k = i % step;
            let accent = if (i / step) % 4 == 2 { 1.0 } else { 0.45 };
            let decay = if (i / step) % 8 == 6 { 0.12 } else { 0.035 };
            let env = accent * (-(k as f64) / (decay * fs)).exp();
            let w = r.next();
            let a = c * (y1 + w - x1);
            x1 = w;
            y1 = a;
            let b = c * (y2 + a - x2);
            x2 = a;
            y2 = b;
            (0.6 * b * env) as f32
        })
        .collect()
}

fn pad(fs: f64, secs: f64) -> Vec<f32> {
    let mut r = Lcg(11);
    let n = (secs * fs) as usize;
    let mut parts = Vec::new();
    for f0 in [110.0, 164.81, 220.0 * 1.189] {
        let mut h = 1.0;
        while f0 * h < 0.45 * fs {
            parts.push((f0 * h * (1.0 + 0.002 * r.next()), 1.0 / (h * h), std::f64::consts::PI * r.next()));
            h += 1.0;
        }
    }
    let mut x: Vec<f64> = (0..n)
        .map(|i| {
            let t = i as f64 / fs;
            let swell = 0.6 + 0.4 * (2.0 * std::f64::consts::PI * 0.25 * t).sin();
            swell * parts.iter().map(|(f, a, p)| a * (2.0 * std::f64::consts::PI * f * t + p).sin()).sum::<f64>()
        })
        .collect();
    let pk = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    for v in x.iter_mut() {
        *v *= 0.5 / pk;
    }
    x.into_iter().map(|v| v as f32).collect()
}

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        for s in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (wr, wi) = ((ang * k as f64).cos(), (ang * k as f64).sin());
                let (a, b) = (s + k, s + k + len / 2);
                let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

fn spectrum(x: &[f32]) -> Vec<f64> {
    const N: usize = 4096;
    let mut acc = vec![0.0; N / 2];
    let mut s = 0;
    while s + N <= x.len() {
        let mut re: Vec<f64> = (0..N).map(|i| x[s + i] as f64 * (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N as f64).cos())).collect();
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im);
        for k in 0..N / 2 {
            acc[k] += re[k] * re[k] + im[k] * im[k];
        }
        s += N / 2;
    }
    acc
}

fn band(sp: &[f64], fs: f64, lo: f64, hi: f64) -> f64 {
    let bin = fs / (2.0 * sp.len() as f64);
    sp.iter().enumerate().filter(|(k, _)| (*k as f64 * bin) >= lo && (*k as f64 * bin) < hi).map(|(_, v)| v).sum::<f64>()
}

struct Stats {
    rms: f64,
    crest: f64,
    hf: f64,
    centroid: f64,
    jump: f32,
}

fn stats(x: &[f32], fs: f64) -> Stats {
    let p = x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64;
    let pk = x.iter().fold(0f32, |m, v| m.max(v.abs())) as f64;
    let sp = spectrum(x);
    let total = band(&sp, fs, 80.0, 8000.0);
    let bin = fs / (2.0 * sp.len() as f64);
    let (mut num, mut den) = (0.0, 0.0);
    for (k, v) in sp.iter().enumerate() {
        let f = k as f64 * bin;
        if (80.0..10000.0).contains(&f) {
            num += f * v;
            den += v;
        }
    }
    let rms = 10.0 * (p + 1e-20).log10();
    let jump = x.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).abs()).fold(0.0, f32::max) / pk.max(1e-9) as f32;
    Stats { rms, crest: 20.0 * pk.max(1e-12).log10() - rms, hf: 10.0 * (band(&sp, fs, 2000.0, 6000.0) / total.max(1e-30)).log10(), centroid: num / den.max(1e-30), jump }
}

fn run(fs: f64, input: &[f32], p: Params) -> Vec<f32> {
    let mut s = Suu::new(fs);
    s.set_params(p);
    let mut out = vec![0f32; input.len()];
    for (i, o) in input.chunks(128).zip(out.chunks_mut(128)) {
        s.process(i, o);
    }
    out
}

fn alias_db(fs: f64, f0: f64, shape: Option<InShape>) -> f64 {
    let n = (2.0 * fs) as usize;
    let x: Vec<f32> = (0..n).map(|i| (0.5 * (2.0 * std::f64::consts::PI * f0 * i as f64 / fs).sin()) as f32).collect();
    let note = 69.0 + 12.0 * (f0 / 440.0).log2();
    let p = match shape {
        Some(sh) => Params { source: Source::Off, in_shape: sh, env_gate: true, env_thresh_db: -50.0, mix: 0.0, tilt_hz: 0.0, ..Params::default() },
        None => Params { source: Source::Saw, input_db: -60.0, note, gate: true, mix: 0.0, tilt_hz: 0.0, glide_s: 0.0, ..Params::default() },
    };
    let y = run(fs, &x, p);
    let sp = spectrum(&y[n / 2..]);
    let bin = fs / (2.0 * sp.len() as f64);
    let (mut harm, mut other) = (0.0, 0.0);
    for (k, v) in sp.iter().enumerate() {
        let f = k as f64 * bin;
        if f < 0.5 * f0 {
            continue;
        }
        let h = f / f0;
        if (h - h.round()).abs() * f0 < 3.0 * bin {
            harm += v;
        } else {
            other += v;
        }
    }
    10.0 * (other / harm.max(1e-30)).log10()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("cpu") {
        let fs = 48000.0;
        let x = pad(fs, 10.0);
        for (name, shape) in [("RAW", InShape::Raw), ("FLAT", InShape::Flat), ("EDGE", InShape::Edge), ("VOC", InShape::Voc)] {
            let p = Params { source: Source::Off, gate: true, in_shape: shape, ..Params::default() };
            let t = std::time::Instant::now();
            let y = run(fs, &x, p);
            let el = t.elapsed().as_secs_f64();
            println!("{name}: {:.2} % of one core (10 s at 48 kHz, HQ, one mouth, 128-sample blocks), peak {:.3}", 100.0 * el / 10.0, y.iter().fold(0f32, |m, v| m.max(v.abs())));
        }
        return;
    }
    if args.first().map(String::as_str) == Some("stereo") {
        let (fs, sung) = wav16(&args[1]);
        let sung = sung[..(6.0 * fs) as usize].to_vec();
        let base = Params { source: Source::Off, env_gate: true, env_thresh_db: -45.0, env_release_s: 0.05, vowel_x: 0.6, vowel_y: 0.8, stereo: true, note: 45.0, ..Params::default() };
        println!("L-R rms dB, L/R correlation, 2-6k L / R dB");
        for (label, spread, vowel) in [("default spread 1.12 vowel 12", 1.12, 0.12), ("same mouth spread 1 vowel 0", 1.0, 0.0)] {
            for (name, x) in [("laulu", sung.clone()), ("haitsu", hihat(fs, 6.0))] {
                for (sn, sh) in [("RAW", InShape::Raw), ("FLAT", InShape::Flat), ("EDGE", InShape::Edge), ("VOC", InShape::Voc)] {
                    let mut s = Suu::new(fs);
                    s.set_params(Params { in_shape: sh, source: if sh == InShape::Voc { Source::Saw } else { Source::Off }, stereo_spread: spread, stereo_vowel: vowel, ..base });
                    let (mut l, mut r) = (vec![0f32; x.len()], vec![0f32; x.len()]);
                    for ((c, lo), ro) in x.chunks(128).zip(l.chunks_mut(128)).zip(r.chunks_mut(128)) {
                        if let Some(w) = s.wants_twin() {
                            let _ = s.set_twin(suu::Mouth::build(&s.mouth_seed(w)));
                        }
                        let _ = s.take_retired();
                        s.process_stereo(c, lo, ro);
                    }
                    let (l, r) = (&l[fs as usize..], &r[fs as usize..]);
                    let p = |v: &[f32]| v.iter().map(|a| (*a as f64).powi(2)).sum::<f64>();
                    let c = l.iter().zip(r).map(|(a, b)| *a as f64 * *b as f64).sum::<f64>() / (p(l) * p(r)).sqrt().max(1e-30);
                    let (sl, sr) = (stats(l, fs), stats(r, fs));
                    println!("{label:<30} {name:<7} {sn:<5} L-R {:6.1} dB  corr {c:5.2}  2-6k {:6.1} / {:6.1}", 10.0 * (p(l) / p(r)).log10(), sl.hf, sr.hf);
                }
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("hot") {
        let (fs, sung) = wav16(&args[1]);
        let sung = sung[..(8.0 * fs) as usize].to_vec();
        let norm = |x: &[f32], pk: f32| { let m = x.iter().fold(0f32, |a, v| a.max(v.abs())); x.iter().map(|v| v * pk / m).collect::<Vec<f32>>() };
        let base = Params { source: Source::Off, env_gate: true, env_thresh_db: -45.0, env_release_s: 0.05, vowel_x: 0.6, vowel_y: 0.8, ..Params::default() };
        println!("input peak -1 dBFS; nonlin = residual of y(x) - 10*y(x/10), dB below y; out peak");
        for (name, x) in [("laulu", norm(&sung, 0.9)), ("haitsu", norm(&hihat(fs, 8.0), 0.9)), ("padi", norm(&pad(fs, 8.0), 0.9))] {
            for (sn, sh) in [("RAW", InShape::Raw), ("FLAT", InShape::Flat), ("EDGE", InShape::Edge), ("VOC", InShape::Voc)] {
                let p = Params { in_shape: sh, source: if sh == InShape::Voc { Source::Saw } else { Source::Off }, note: 45.0, ..base };
                let y = run(fs, &x, p);
                let xs: Vec<f32> = x.iter().map(|v| v * 0.1).collect();
                let ys = run(fs, &xs, p);
                let e: f64 = y.iter().zip(&ys).map(|(a, b)| (*a as f64 - 10.0 * *b as f64).powi(2)).sum();
                let t: f64 = y.iter().map(|a| (*a as f64).powi(2)).sum();
                let pk = y.iter().fold(0f32, |m, v| m.max(v.abs()));
                println!("{name:<7} {sn:<5} out rms {:6.1} dB  peak {pk:.3}  nonlin {:6.1} dB", 10.0 * (t / y.len() as f64).log10(), 10.0 * (e / t).log10());
            }
        }
        return;
    }
    let singer = args.first().expect("singer WAV");
    let out_dir = args.get(1).cloned().unwrap_or_else(|| ".".into());
    let (fs, sung) = wav16(singer);
    let sung: Vec<f32> = sung[..(8.0 * fs) as usize].to_vec();
    let inputs: Vec<(&str, Vec<f32>, bool)> = vec![("laulu", sung, true), ("haitsu", hihat(fs, 8.0), false), ("padi", pad(fs, 8.0), false)];
    let base = Params { source: Source::Off, env_gate: true, env_thresh_db: -45.0, env_release_s: 0.05, vowel_x: 0.6, vowel_y: 0.8, ..Params::default() };
    println!("{:<34} {:>7} {:>7} {:>8} {:>9} {:>7}", "scene", "rms dB", "crest", "2-6k dB", "centroid", "jump");
    for (name, x, pitched) in &inputs {
        let pf = if *pitched { PitchFrom::Input } else { PitchFrom::Note };
        let scenes: Vec<(&str, Params)> = vec![
            ("0 kuiva input", Params { mix: 0.0, ..base }),
            ("1 saha (vertailu)", Params { source: Source::Saw, input_db: -60.0, pitch_from: pf, ..base }),
            ("2 raaka", base),
            ("3 raaka + RESON 70", Params { reson: 0.7, pitch_from: pf, ..base }),
            ("4 FLAT", Params { in_shape: InShape::Flat, ..base }),
            ("4d FLAT kuiva", Params { in_shape: InShape::Flat, mix: 0.0, ..base }),
            ("5d EDGE kuiva", Params { in_shape: InShape::Edge, mix: 0.0, ..base }),
            ("5 EDGE", Params { in_shape: InShape::Edge, ..base }),
            ("6 VOC saha nuotissa", Params { in_shape: InShape::Voc, source: Source::Saw, note: 45.0, ..base }),
            ("6d VOC kuiva", Params { in_shape: InShape::Voc, source: Source::Saw, note: 45.0, mix: 0.0, ..base }),
        ];
        for (sname, p) in scenes {
            let y = run(fs, x, p);
            let st = stats(&y, fs);
            println!("{:<34} {:7.1} {:7.1} {:8.1} {:9.0} {:7.3}", format!("{name} {sname}"), st.rms, st.crest, st.hf, st.centroid, st.jump);
            let g = 10f64.powf((-20.0 - st.rms) / 20.0);
            let pk = y.iter().fold(0f32, |m, v| m.max(v.abs())) as f64 * g;
            let g = if pk > 0.97 { g * 0.97 / pk } else { g };
            let z: Vec<f32> = y.iter().map(|v| (*v as f64 * g) as f32).collect();
            write_wav(&format!("{out_dir}/{name} {sname}.wav"), fs, &z);
        }
    }
    for f0 in [110.0, 440.0, 1760.0] {
        println!("alias {f0:6} Hz: EDGE {:6.1} dB, FLAT {:6.1} dB, built-in saw {:6.1} dB (non-harmonic / harmonic power)", alias_db(fs, f0, Some(InShape::Edge)), alias_db(fs, f0, Some(InShape::Flat)), alias_db(fs, f0, None));
    }
}
