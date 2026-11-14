//! How well VOWEL = IN places sung vowels: `cargo run --release -p suu --example vowel_eval
//! <dir>...`, each directory holding `*_a.wav` … `*_u.wav` (16-bit PCM mono, as VocalSet's
//! scales). Over the voiced frames, the share whose pad point is nearest its own step vowel
//! (ɑ e i o u), and the mean point per vowel. With VOWEL_DUMP=<file>, each frame's vowel index
//! and F1, F2 (Hz) are written there too, one per line (for fitting `pad_of`).
use suu::{Params, STEP_VOWELS, Suu};

const PAD: [(f64, f64); 5] = [(1.0, 1.0), (0.0, 0.5), (0.0, 0.0), (1.0, 0.5), (1.0, 0.0)];

fn wav16(path: &std::path::Path) -> (f64, Vec<f32>) {
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

fn main() {
    let letters = ["a", "e", "i", "o", "u"];
    let mut hit = [0usize; 5];
    let mut all = [0usize; 5];
    let mut confusion = [[0usize; 5]; 5];
    let mut sum = [(0.0, 0.0); 5];
    let mut dump = std::env::var("VOWEL_DUMP").ok().map(|p| std::io::BufWriter::new(std::fs::File::create(p).expect("create the dump")));
    for dir in std::env::args().skip(1) {
        for entry in std::fs::read_dir(&dir).expect("read the directory").flatten() {
            let path = entry.path();
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            let Some(v) = letters.iter().position(|l| name.ends_with(&format!("_{l}"))) else { continue };
            let (fs, x) = wav16(&path);
            let mut s = Suu::new(fs);
            s.set_params(Params { vowel_in: true, ..Params::default() });
            let mut out = vec![0f32; 256];
            let mut last = None;
            for blk in x.chunks(256) {
                s.process(blk, &mut out[..blk.len()]);
                let (pad, voiced) = s.input_vowel();
                if !voiced || pad == last {
                    continue; // only frames newly analysed
                }
                last = pad;
                let Some((px, py)) = pad else { continue };
                if let Some(d) = dump.as_mut() {
                    use std::io::Write;
                    let (f1, f2) = s.input_formants();
                    writeln!(d, "{v} {f1:.1} {f2:.1}").unwrap();
                }
                let near = (0..5).min_by(|&a, &b| {
                    let d = |k: usize| (px - PAD[k].0).powi(2) + (py - PAD[k].1).powi(2);
                    d(a).total_cmp(&d(b))
                });
                let near = near.unwrap();
                all[v] += 1;
                confusion[v][near] += 1;
                if near == v {
                    hit[v] += 1;
                }
                sum[v].0 += px;
                sum[v].1 += py;
            }
        }
    }
    println!("vowel  right   mean pad (x, y)   → {}", STEP_VOWELS.join("     "));
    for v in 0..5 {
        let n = all[v].max(1) as f64;
        let row: Vec<String> = confusion[v].iter().map(|&c| format!("{:4.0}%", 100.0 * c as f64 / n)).collect();
        println!("{:>4}  {:5.1}%   {:5.2}, {:5.2}      {}   ({} frames)", STEP_VOWELS[v], 100.0 * hit[v] as f64 / n, sum[v].0 / n, sum[v].1 / n, row.join(" "), all[v]);
    }
}
