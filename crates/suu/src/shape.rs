use chimera::math::{PI, clamp, cos, exp, js_max, sin, sqrt};

const ORDER: usize = 16;
const HOP: usize = 256;
const FRAME_S: f64 = 0.02;
const MAX_FRAME: usize = 4096;
const FLOOR: f64 = 1e-3;
const SLOPE_HZ: f64 = 150.0;
const AGC_S: f64 = 0.2;
const MAX_GAIN: f64 = 1000.0;
const QUIET: f64 = 1e-10;

pub struct Whitener {
    ring: Vec<f64>,
    w: usize,
    window: Vec<f64>,
    frame: Vec<f64>,
    k: usize,
    a_from: [f64; ORDER],
    a_to: [f64; ORDER],
    hist: [f64; ORDER],
    slope_c: f64,
    slope_y: f64,
}

impl Whitener {
    pub fn new(fs: f64) -> Self {
        let n = ((FRAME_S * fs).round() as usize).clamp(ORDER * 4, MAX_FRAME);
        let window = (0..n).map(|i| 0.5 - 0.5 * cos(2.0 * PI * (i as f64 + 0.5) / n as f64)).collect();
        Whitener {
            ring: vec![0.0; n],
            w: 0,
            window,
            frame: vec![0.0; n],
            k: HOP,
            a_from: [0.0; ORDER],
            a_to: [0.0; ORDER],
            hist: [0.0; ORDER],
            slope_c: exp(-2.0 * PI * SLOPE_HZ / fs),
            slope_y: 0.0,
        }
    }

    fn analyse(&mut self) {
        let n = self.ring.len();
        for i in 0..n {
            self.frame[i] = self.ring[(self.w + i) % n] * self.window[i];
        }
        let mut r = [0.0; ORDER + 1];
        for (lag, rl) in r.iter_mut().enumerate() {
            let mut s = 0.0;
            for i in lag..n {
                s += self.frame[i] * self.frame[i - lag];
            }
            *rl = s;
        }
        self.a_from = self.a_to;
        if r[0] < QUIET * n as f64 {
            return;
        }
        r[0] *= 1.0 + FLOOR;
        for (lag, rl) in r.iter_mut().enumerate().skip(1) {
            let g = PI * 0.002 * lag as f64;
            *rl *= exp(-0.5 * g * g);
        }
        let mut a = [0.0; ORDER + 1];
        let mut prev = [0.0; ORDER + 1];
        a[0] = 1.0;
        let mut e = r[0];
        for m in 1..=ORDER {
            let mut acc = r[m];
            for j in 1..m {
                acc += a[j] * r[m - j];
            }
            let km = -acc / e;
            prev[..m].copy_from_slice(&a[..m]);
            for j in 1..m {
                a[j] = prev[j] + km * prev[m - j];
            }
            a[m] = km;
            e *= 1.0 - km * km;
            if e <= 0.0 {
                return;
            }
        }
        self.a_to.copy_from_slice(&a[1..]);
    }

    #[inline]
    pub fn push(&mut self, x: f64) -> f64 {
        self.ring[self.w] = x;
        self.w = (self.w + 1) % self.ring.len();
        if self.k == HOP {
            self.analyse();
            self.k = 0;
        }
        let t = self.k as f64 / HOP as f64;
        let mut e = x;
        for i in 0..ORDER {
            e += (self.a_from[i] + t * (self.a_to[i] - self.a_from[i])) * self.hist[i];
        }
        self.hist.copy_within(0..ORDER - 1, 1);
        self.hist[0] = x;
        self.k += 1;
        self.slope_y = self.slope_c * self.slope_y + (1.0 - self.slope_c) * e;
        self.slope_y
    }
}

const EDGE_LP_HZ: f64 = 900.0;
const HYST: f64 = 0.2;
const MAX_HZ: f64 = 4000.0;
const MIN_HZ: f64 = 20.0;
const LEVEL: f64 = 1.22;

pub struct Edge {
    c: [f64; 5],
    z: [f64; 2],
    dc_x: f64,
    dc_y: f64,
    lp_prev: f64,
    track: f64,
    level: f64,
    att: f64,
    rel: f64,
    armed: bool,
    ramp: f64,
    slope: f64,
    since: f64,
    min_t: f64,
    max_t: f64,
    held: f64,
    out_x: f64,
    out_y: f64,
    dc_c: f64,
}

impl Edge {
    pub fn new(fs: f64) -> Self {
        let w0 = 2.0 * PI * EDGE_LP_HZ / fs;
        let alpha = sin(w0) / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
        let (a0, cw) = (1.0 + alpha, cos(w0));
        let c = [(1.0 - cw) / 2.0 / a0, (1.0 - cw) / a0, (1.0 - cw) / 2.0 / a0, -2.0 * cw / a0, (1.0 - alpha) / a0];
        let min_t = fs / MAX_HZ;
        let max_t = fs / MIN_HZ;
        Edge {
            c,
            z: [0.0; 2],
            dc_x: 0.0,
            dc_y: 0.0,
            lp_prev: 0.0,
            track: 0.0,
            level: 0.0,
            att: 1.0 - exp(-1.0 / (0.001 * fs)),
            rel: 1.0 - exp(-1.0 / (0.03 * fs)),
            armed: false,
            ramp: -1.0,
            slope: 2.0 / (fs / 110.0),
            since: 0.0,
            min_t,
            max_t,
            held: 0.0,
            out_x: 0.0,
            out_y: 0.0,
            dc_c: exp(-2.0 * PI * 10.0 / fs),
        }
    }

    #[inline]
    pub fn push(&mut self, x: f64) -> f64 {
        let d = x - self.dc_x + 0.995 * self.dc_y;
        self.dc_x = x;
        self.dc_y = d;
        let c = &self.c;
        let l = c[0] * d + self.z[0];
        self.z[0] = c[1] * d - c[3] * l + self.z[1];
        self.z[1] = c[2] * d - c[4] * l;
        let a = l.abs();
        self.track += (a - self.track) * if a > self.track { self.att } else { self.rel };
        let ad = d.abs();
        self.level += (ad - self.level) * if ad > self.level { self.att } else { self.rel };
        let thr = HYST * self.track;
        let mut naive = (self.ramp + self.slope).min(1.0);
        let mut cur = naive;
        let mut prev = self.held;
        if !self.armed && l <= -thr {
            self.armed = true;
        }
        if self.armed && l >= thr && self.lp_prev < thr && self.since + 1.0 >= self.min_t {
            let frac = clamp((thr - self.lp_prev) / js_max(l - self.lp_prev, 1e-30), 0.0, 1.0);
            let v = (self.ramp + self.slope * frac).min(1.0);
            let step = v + 1.0;
            let t = clamp(self.since + frac, self.min_t, self.max_t);
            self.slope = 2.0 / t;
            naive = -1.0 + self.slope * (1.0 - frac);
            prev -= 0.5 * step * (1.0 - frac) * (1.0 - frac);
            cur = naive + 0.5 * step * frac * frac;
            self.since = 1.0 - frac;
            self.armed = false;
        } else {
            self.since += 1.0;
        }
        self.lp_prev = l;
        if self.since <= self.max_t {
            self.ramp = naive;
        }
        self.held = cur;
        let y = prev * self.level * LEVEL;
        let o = y - self.out_x + self.dc_c * self.out_y;
        self.out_x = y;
        self.out_y = o;
        o
    }
}

const WEIGHT_LO_HZ: f64 = 300.0;
const WEIGHT_HI_HZ: f64 = 3000.0;

pub struct Loudness {
    c: f64,
    l: f64,
    lx: f64,
    ly: f64,
    agc: f64,
    hx: (f64, f64),
    hy: (f64, f64),
    px: f64,
    py: f64,
}

impl Loudness {
    pub fn new(fs: f64) -> Self {
        Loudness { c: exp(-2.0 * PI * WEIGHT_LO_HZ / fs), l: exp(-2.0 * PI * WEIGHT_HI_HZ / fs), lx: 0.0, ly: 0.0, agc: 1.0 - exp(-1.0 / (AGC_S * fs)), hx: (0.0, 0.0), hy: (0.0, 0.0), px: 0.0, py: 0.0 }
    }

    fn high(c: f64, s: &mut (f64, f64), x: f64) -> f64 {
        let y = c * (s.1 + x - s.0);
        *s = (x, y);
        y
    }

    #[inline]
    pub fn matched(&mut self, x: f64, y: f64) -> f64 {
        let a = Self::high(self.c, &mut self.hx, x);
        let b = Self::high(self.c, &mut self.hy, y);
        self.lx = self.l * self.lx + (1.0 - self.l) * a;
        self.ly = self.l * self.ly + (1.0 - self.l) * b;
        let (a, b) = (self.lx, self.ly);
        self.px += (a * a - self.px) * self.agc;
        self.py += (b * b - self.py) * self.agc;
        if self.py > QUIET { y * clamp(sqrt(self.px / self.py), 0.0, MAX_GAIN) } else { 0.0 }
    }
}

const BANDS: usize = 16;
const VOC_LO: f64 = 120.0;
const VOC_HI: f64 = 9000.0;
const VOC_Q: f64 = 2.5;
const VOC_ATT_S: f64 = 0.002;
const VOC_REL_S: f64 = 0.02;
const VOC_TILT: f64 = 0.5;

pub struct Vocoder {
    c: [[f64; 5]; BANDS],
    zm: [[[f64; 2]; 2]; BANDS],
    zc: [[[f64; 2]; 2]; BANDS],
    em: [f64; BANDS],
    lift: [f64; BANDS],
    att: f64,
    rel: f64,
}

impl Vocoder {
    pub fn new(fs: f64) -> Self {
        let c = core::array::from_fn(|b| {
            let f = VOC_LO * chimera::math::pow(VOC_HI / VOC_LO, b as f64 / (BANDS - 1) as f64);
            let w0 = 2.0 * PI * f.min(0.45 * fs) / fs;
            let alpha = sin(w0) / (2.0 * VOC_Q);
            let a0 = 1.0 + alpha;
            [alpha / a0, 0.0, -alpha / a0, -2.0 * cos(w0) / a0, (1.0 - alpha) / a0]
        });
        Vocoder {
            c,
            zm: [[[0.0; 2]; 2]; BANDS],
            zc: [[[0.0; 2]; 2]; BANDS],
            em: [0.0; BANDS],
            lift: core::array::from_fn(|b| chimera::math::pow(VOC_HI / VOC_LO, VOC_TILT * b as f64 / (BANDS - 1) as f64)),
            att: 1.0 - exp(-1.0 / (VOC_ATT_S * fs)),
            rel: 1.0 - exp(-1.0 / (VOC_REL_S * fs)),
        }
    }

    #[inline]
    fn band(c: &[f64; 5], z: &mut [[f64; 2]; 2], x: f64) -> f64 {
        let mut v = x;
        for s in z.iter_mut() {
            let y = c[0] * v + s[0];
            s[0] = c[1] * v - c[3] * y + s[1];
            s[1] = c[2] * v - c[4] * y;
            v = y;
        }
        v
    }

    #[inline]
    fn follow(e: &mut f64, x: f64, att: f64, rel: f64) {
        let a = x.abs();
        *e += (a - *e) * if a > *e { att } else { rel };
    }

    #[inline]
    pub fn push(&mut self, carrier: f64, modulator: f64) -> f64 {
        let (att, rel) = (self.att, self.rel);
        let mut out = 0.0;
        for b in 0..BANDS {
            let m = Self::band(&self.c[b], &mut self.zm[b], modulator);
            let k = Self::band(&self.c[b], &mut self.zc[b], carrier);
            Self::follow(&mut self.em[b], m, att, rel);
            out += k * self.em[b] * self.lift[b];
        }
        out
    }
}
