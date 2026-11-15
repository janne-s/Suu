use chimera::math::{PI, atan2, cos, exp, hypot, js_max, ln, sin};

const TARGET_FS: f64 = 8_000.0;
const LP_HZ: f64 = 3_400.0;
const FRAME_S: f64 = 0.03;
const MAX_FRAME: usize = 512;
const ORDER: usize = 10;
const MAX_BW: f64 = 500.0;

const F_I: (f64, f64) = (363.0, 1878.0);
const F_U: (f64, f64) = (374.0, 906.0);
const F_AE: (f64, f64) = (756.0, 1321.0);
const F_A: (f64, f64) = (637.0, 1059.0);

pub struct VowelTracker {
    down: usize,
    phase: usize,
    fsd: f64,
    c: [f64; 5],
    z: [f64; 2],
    ring: Vec<f64>,
    w: usize,
    frame: Vec<f64>,
    window: Vec<f64>,
    re: [f64; ORDER],
    im: [f64; ORDER],
    pad: Option<(f64, f64)>,
    formants: (f64, f64),
}

impl VowelTracker {
    pub fn new(fs: f64) -> Self {
        let down = js_max(1.0, (fs / TARGET_FS).round()) as usize;
        let fsd = fs / down as f64;
        let w0 = 2.0 * PI * LP_HZ.min(0.45 * fsd) / fs;
        let alpha = sin(w0) / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
        let (a0, cw) = (1.0 + alpha, cos(w0));
        let c = [(1.0 - cw) / 2.0 / a0, (1.0 - cw) / a0, (1.0 - cw) / 2.0 / a0, -2.0 * cw / a0, (1.0 - alpha) / a0];
        let n = ((FRAME_S * fsd).round() as usize).min(MAX_FRAME);
        let window = (0..n).map(|i| 0.54 - 0.46 * cos(2.0 * PI * i as f64 / (n as f64 - 1.0))).collect();
        let mut v = VowelTracker { down, phase: 0, fsd, c, z: [0.0; 2], ring: vec![0.0; n], w: 0, frame: vec![0.0; n], window, re: [0.0; ORDER], im: [0.0; ORDER], pad: None, formants: (0.0, 0.0) };
        v.seed_roots();
        v
    }

    fn seed_roots(&mut self) {
        for i in 0..ORDER {
            let ang = 2.0 * PI * i as f64 / ORDER as f64 + 0.4;
            self.re[i] = 0.9 * cos(ang);
            self.im[i] = 0.9 * sin(ang);
        }
    }

    #[inline]
    pub fn push(&mut self, x: f64) {
        let c = &self.c;
        let y = c[0] * x + self.z[0];
        self.z[0] = c[1] * x - c[3] * y + self.z[1];
        self.z[1] = c[2] * x - c[4] * y;
        self.phase += 1;
        if self.phase >= self.down {
            self.phase = 0;
            self.ring[self.w] = y;
            self.w = (self.w + 1) % self.ring.len();
        }
    }

    pub fn analyse(&mut self) -> Option<(f64, f64)> {
        let n = self.ring.len();
        let mut prev = self.ring[self.w % n];
        for i in 0..n {
            let v = self.ring[(self.w + i) % n];
            self.frame[i] = (v - 0.97 * prev) * self.window[i];
            prev = v;
        }
        let mut r = [0.0; ORDER + 1];
        for (k, rk) in r.iter_mut().enumerate() {
            let mut s = 0.0;
            for i in k..n {
                s += self.frame[i] * self.frame[i - k];
            }
            let g = 2.0 * PI * 40.0 * k as f64 / self.fsd;
            *rk = s * exp(-0.5 * g * g);
        }
        if r[0] <= 1e-12 {
            return self.pad;
        }
        let mut a = [0.0; ORDER + 1];
        a[0] = 1.0;
        let mut err = r[0] * (1.0 + 1e-9);
        for i in 1..=ORDER {
            let mut acc = r[i];
            for j in 1..i {
                acc += a[j] * r[i - j];
            }
            let k = -acc / err;
            let t = a;
            for j in 1..i {
                a[j] = t[j] + k * t[i - j];
            }
            a[i] = k;
            err *= 1.0 - k * k;
            if err <= 0.0 {
                return self.pad;
            }
        }
        if !self.roots(&a) {
            self.seed_roots();
            if !self.roots(&a) {
                self.seed_roots();
                return self.pad;
            }
        }
        let (mut f1, mut f2) = (f64::INFINITY, f64::INFINITY);
        for i in 0..ORDER {
            if self.im[i] <= 0.0 {
                continue;
            }
            let freq = atan2(self.im[i], self.re[i]) * self.fsd / (2.0 * PI);
            let bw = -ln(hypot(self.re[i], self.im[i])) * self.fsd / PI;
            if !(150.0..=3200.0).contains(&freq) || bw > MAX_BW {
                continue;
            }
            if freq < f1 {
                (f1, f2) = (freq, f1);
            } else if freq < f2 {
                f2 = freq;
            }
        }
        if !(f1 <= 1100.0 && f2 < 3200.0 && f2 - f1 >= 150.0) {
            return self.pad;
        }
        self.formants = (f1, f2);
        self.pad = Some(pad_of(f1, f2));
        self.pad
    }

    fn roots(&mut self, a: &[f64; ORDER + 1]) -> bool {
        let (re, im) = (&mut self.re, &mut self.im);
        for _ in 0..200 {
            let mut max_delta: f64 = 0.0;
            for i in 0..ORDER {
                let (mut pr, mut pi) = (1.0, 0.0);
                for &ck in &a[1..] {
                    let nr = pr * re[i] - pi * im[i] + ck;
                    pi = pr * im[i] + pi * re[i];
                    pr = nr;
                }
                let (mut dr, mut di) = (1.0, 0.0);
                for j in 0..ORDER {
                    if j != i {
                        let (xr, xi) = (re[i] - re[j], im[i] - im[j]);
                        let nr = dr * xr - di * xi;
                        di = dr * xi + di * xr;
                        dr = nr;
                    }
                }
                let den = dr * dr + di * di + 1e-300;
                let (qr, qi) = ((pr * dr + pi * di) / den, (pi * dr - pr * di) / den);
                re[i] -= qr;
                im[i] -= qi;
                max_delta = js_max(max_delta, qr.abs() + qi.abs());
            }
            if !max_delta.is_finite() {
                return false;
            }
            if max_delta < 1e-10 {
                return true;
            }
        }
        true
    }

    pub fn pad(&self) -> Option<(f64, f64)> {
        self.pad
    }

    pub fn formants(&self) -> (f64, f64) {
        self.formants
    }
}

pub fn pad_of(f1: f64, f2: f64) -> (f64, f64) {
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t.clamp(0.0, 1.0);
    let (l1, l2) = (ln(f1), ln(f2));
    let (mut x, mut y) = (0.5, 0.5);
    for _ in 0..3 {
        let close = lerp(ln(F_I.0), ln(F_U.0), x);
        let open = lerp(ln(F_AE.0), ln(F_A.0), x);
        y = ((l1 - close) / (open - close)).clamp(-0.2, 1.2);
        let front = lerp(ln(F_I.1), ln(F_AE.1), y);
        let back = lerp(ln(F_U.1), ln(F_A.1), y);
        x = ((front - l2) / (front - back)).clamp(-0.2, 1.2);
    }
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_corners_map_to_the_corners() {
        for ((f1, f2), (x, y)) in [(F_I, (0.0, 0.0)), (F_U, (1.0, 0.0)), (F_AE, (0.0, 1.0)), (F_A, (1.0, 1.0))] {
            let (px, py) = pad_of(f1, f2);
            assert!((px - x).abs() < 0.12 && (py - y).abs() < 0.12, "{f1}/{f2} → {px:.2}, {py:.2}");
        }
    }

    #[test]
    fn finds_two_formants() {
        let fs = 48_000.0;
        for ((f1, f2), want) in [(F_I, (0.0, 0.0)), (F_A, (1.0, 1.0)), (F_U, (1.0, 0.0))] {
            let mut v = VowelTracker::new(fs);
            let res = |f: f64, bw: f64| {
                let r = exp(-PI * bw / fs);
                (2.0 * r * cos(2.0 * PI * f / fs), -r * r)
            };
            let (r1, r2) = (res(f1, 80.0), res(f2, 100.0));
            let (mut y1, mut y2) = ([0.0f64; 2], [0.0f64; 2]);
            for i in 0..(0.2 * fs) as usize {
                let x = if i % 400 == 0 { 1.0 } else { 0.0 };
                let a = x + r1.0 * y1[0] + r1.1 * y1[1];
                y1 = [a, y1[0]];
                let b = a + r2.0 * y2[0] + r2.1 * y2[1];
                y2 = [b, y2[0]];
                v.push(b * 1e-3);
            }
            let (x, y) = v.analyse().expect("a vowel");
            assert!((x - want.0).abs() < 0.3 && (y - want.1).abs() < 0.3, "{f1}/{f2}: {x:.2}, {y:.2}");
        }
    }
}
