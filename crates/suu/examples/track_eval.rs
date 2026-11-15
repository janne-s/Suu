use suu::{Params, PitchFrom, Suu};

fn wav(path: &str) -> (f64, Vec<f32>) {
    let b = std::fs::read(path).expect("read the WAV");
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let (mut i, mut fs, mut fmt) = (12, 0.0, (0, 0, 0));
    while i + 8 <= b.len() {
        let (id, len) = (&b[i..i + 4], u32_at(i + 4) as usize);
        if id == b"fmt " {
            fmt = (u16_at(i + 8), u16_at(i + 10), u16_at(i + 22));
            fs = u32_at(i + 12) as f64;
        } else if id == b"data" {
            assert!(fmt == (3, 1, 32), "mono 32-bit float only, not {fmt:?}");
            let d = &b[i + 8..(i + 8 + len).min(b.len())];
            return (fs, d.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect());
        }
        i += 8 + len + (len & 1);
    }
    panic!("no data chunk");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!("{:>28}  {:>7} {:>7} {:>7} {:>8}  {:>6}", "", "≤50 c", "≤100 c", "octave", "median", "frames");
    for pair in args.chunks(2) {
        let (fs, x) = wav(&pair[0]);
        let reference: Vec<(f64, f64)> = std::fs::read_to_string(&pair[1])
            .expect("read the f0 file")
            .lines()
            .filter_map(|l| {
                let mut it = l.split_whitespace().map(|v| v.parse::<f64>().ok());
                Some((it.next()??, it.next()??))
            })
            .collect();
        let mut s = Suu::new(fs);
        s.set_params(Params { pitch_from: PitchFrom::Input, ..Params::default() });
        let mut out = vec![0f32; 64];
        let mut tracked = Vec::with_capacity(x.len() / 64 + 1);
        for blk in x.chunks(64) {
            s.process(blk, &mut out[..blk.len()]);
            tracked.push(s.tracked_note());
        }
        let (mut n, mut c50, mut c100, mut oct) = (0, 0, 0, 0);
        let mut errs = Vec::new();
        for &(t, f0) in &reference {
            if f0 <= 0.0 {
                continue;
            }
            let k = ((t * fs) / 64.0) as usize;
            let Some(Some(got)) = tracked.get(k) else { continue };
            n += 1;
            let want = 69.0 + 12.0 * (f0 / 440.0).log2();
            let e = 100.0 * (got - want);
            errs.push(e.abs());
            if e.abs() <= 50.0 {
                c50 += 1;
            }
            if e.abs() <= 100.0 {
                c100 += 1;
            }
            if (e.abs() - 1200.0).abs() <= 100.0 {
                oct += 1;
            }
        }
        errs.sort_by(f64::total_cmp);
        let pc = |k: usize| 100.0 * k as f64 / n.max(1) as f64;
        let name = std::path::Path::new(&pair[0]).file_stem().unwrap().to_string_lossy();
        println!("{name:>28}  {:6.1}% {:6.1}% {:6.1}% {:6.0} c  {n:>6}", pc(c50), pc(c100), pc(oct), errs.get(errs.len() / 2).copied().unwrap_or(f64::NAN));
    }
}
