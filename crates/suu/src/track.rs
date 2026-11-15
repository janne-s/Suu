use chimera::math::{PI, cos, js_max, log2, sin};

const TARGET_FS: f64 = 12_000.0;
const MIN_F0: f64 = 40.0;
const MAX_F0: f64 = 1_000.0;
const LP_HZ: f64 = 2_000.0;
const MAX_LAG: usize = 400;
const THRESHOLD: f64 = 0.2;
const FLOOR_RMS: f64 = 0.003_162;

pub struct PitchTracker {
    down: usize,
    phase: usize,
    fsd: f64,
    c: [f64; 5],
    z: [f64; 2],
    max_lag: usize,
    min_lag: usize,
    ring: Vec<f64>,
    w: usize,
    frame: Vec<f64>,
    cmnd: Vec<f64>,
    note: Option<f64>,
    voiced: bool,
}

impl PitchTracker {
    pub fn new(fs: f64) -> Self {
        let down = js_max(1.0, (fs / TARGET_FS).round()) as usize;
        let fsd = fs / down as f64;
        let max_lag = ((fsd / MIN_F0).ceil() as usize).min(MAX_LAG);
        let min_lag = js_max(2.0, (fsd / MAX_F0).floor()) as usize;
        let w0 = 2.0 * PI * LP_HZ.min(0.45 * fsd) / fs;
        let alpha = sin(w0) / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let cw = cos(w0);
        let c = [(1.0 - cw) / 2.0 / a0, (1.0 - cw) / a0, (1.0 - cw) / 2.0 / a0, -2.0 * cw / a0, (1.0 - alpha) / a0];
        let n = 2 * max_lag;
        PitchTracker { down, phase: 0, fsd, c, z: [0.0; 2], max_lag, min_lag, ring: vec![0.0; n], w: 0, frame: vec![0.0; n], cmnd: vec![0.0; max_lag + 1], note: None, voiced: false }
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

    pub fn analyse(&mut self) -> Option<f64> {
        let n = self.ring.len();
        let mut energy = 0.0;
        for i in 0..n {
            let v = self.ring[(self.w + i) % n];
            self.frame[i] = v;
            energy += v * v;
        }
        self.voiced = false;
        if energy < FLOOR_RMS * FLOOR_RMS * n as f64 {
            return self.note;
        }
        let (max_lag, w) = (self.max_lag, n - self.max_lag);
        let f = &self.frame;
        self.cmnd[0] = 1.0;
        let mut run = 0.0;
        for tau in 1..=max_lag {
            let mut s = 0.0;
            for i in 0..w {
                let d = f[i] - f[i + tau];
                s += d * d;
            }
            run += s;
            self.cmnd[tau] = if run > 0.0 { s * tau as f64 / run } else { 1.0 };
        }
        let cm = &self.cmnd;
        let mut tau = self.min_lag;
        while tau <= max_lag {
            if cm[tau] < THRESHOLD {
                while tau < max_lag && cm[tau + 1] < cm[tau] {
                    tau += 1;
                }
                break;
            }
            tau += 1;
        }
        if tau > max_lag {
            return self.note;
        }
        let mut tau_f = tau as f64;
        if tau > 1 && tau < max_lag {
            let (a, b, c) = (cm[tau - 1], cm[tau], cm[tau + 1]);
            let den = a - 2.0 * b + c;
            if den.abs() > 1e-12 {
                tau_f += 0.5 * (a - c) / den;
            }
        }
        self.voiced = true;
        let f0 = self.fsd / tau_f;
        self.note = Some(69.0 + 12.0 * log2(f0 / 440.0));
        self.note
    }

    pub fn note(&self) -> Option<f64> {
        self.note
    }

    pub fn voiced(&self) -> bool {
        self.voiced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracked(fs: f64, f0: f64, harmonics: usize) -> f64 {
        let mut t = PitchTracker::new(fs);
        let mut ph = 0.0;
        for _ in 0..(0.1 * fs) as usize {
            ph += f0 / fs;
            let x: f64 = (1..=harmonics).map(|h| sin(2.0 * PI * h as f64 * ph) / h as f64).sum();
            t.push(0.3 * x);
        }
        t.analyse().expect("voiced")
    }

    #[test]
    fn finds_the_pitch_of_a_rich_tone() {
        for fs in [44_100.0, 48_000.0, 96_000.0] {
            for f0 in [55.0, 110.0, 220.0, 440.0, 880.0] {
                let want = 69.0 + 12.0 * log2(f0 / 440.0);
                let got = tracked(fs, f0, 12);
                assert!((got - want).abs() < 0.1, "{fs} Hz, {f0} Hz: {got} for {want}");
            }
        }
    }

    #[test]
    fn noise_and_silence_hold_the_last_pitch() {
        let mut t = PitchTracker::new(48_000.0);
        for _ in 0..9600 {
            t.push(0.0);
        }
        assert_eq!(t.analyse(), None);
        let mut ph = 0.0;
        for _ in 0..9600 {
            ph += 220.0 / 48_000.0;
            t.push(0.3 * sin(2.0 * PI * ph));
        }
        let held = t.analyse().expect("voiced");
        let mut r = chimera::math::Rng::new(7);
        for _ in 0..9600 {
            t.push(0.3 * r.bipolar());
        }
        assert_eq!(t.analyse(), Some(held));
    }
}
