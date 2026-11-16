//! Articulator dynamics: each articulator is a critically damped second-order system tracking
//! its target (Task-Dynamics style), with physiological natural frequencies. Runs at control
//! rate. Velocities are bounded implicitly by the dynamics, so UI drags and gestures produce
//! physiologically plausible motion (and therefore plausible dV/dt in the airway).

use super::score::Blended;
use crate::anatomy::human::{ArticKey, ArticPatch, HumanArtic, N_ARTIC, NEUTRAL, rest_tip};
use crate::math::{Drift, PI, js_max, js_min};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ArticSpec {
    pub min: f64,
    pub max: f64,
    /// natural frequency (Hz) of the critically damped tracking dynamics
    pub hz: f64,
}

const MAX_WDT: f64 = 0.5;

const fn spec(min: f64, max: f64, hz: f64) -> ArticSpec {
    ArticSpec { min, max, hz }
}

/// Per-articulator range and speed, in `ArticKey::ALL` order.
pub const ARTIC_SPECS: [ArticSpec; N_ARTIC] = [
    // vowel articulators fast enough to reach their targets in Klatt-length vowels (VocalTractLab's
    // fitted time constants are mostly 5–40 ms; second-order ω = 2π·hz here)
    spec(-0.1, 2.6, 10.0), // jaw
    spec(1.0, 4.8, 12.0),  // tbX
    spec(4.8, 9.2, 12.0),  // tbY
    spec(3.5, 8.0, 13.0),  // ttX
    spec(5.0, 10.8, 13.0), // ttY
    spec(0.0, 1.0, 9.0),   // ttCurl
    spec(-1.0, 1.0, 9.0),  // root
    spec(-0.4, 2.4, 11.0), // lipAp
    spec(-0.2, 1.5, 11.0), // lipProt
    spec(0.0, 1.0, 11.0),  // lipWidth
    spec(0.0, 1.0, 10.0),  // labiodental
    // the velum moves fast (it must seal within a few tens of ms after a nasal) and rests a
    // little past closed, so a closure seals in finite time instead of creeping to zero
    spec(-0.25, 1.4, 12.0), // velum
    spec(-1.6, 1.6, 8.0),   // larynx
    spec(0.0, 1.0, 6.0),    // epilarynx
    spec(0.0, 1.0, 6.0),    // ventricular
    spec(0.0, 1.0, 10.0),   // lateral
    spec(0.0, 1.0, 9.0),    // dorsum
];

pub fn artic_spec(k: ArticKey) -> ArticSpec {
    ARTIC_SPECS[k.index()]
}

/// keys blended as base + min(1, w/2)·(value − base) instead of a weighted mean
#[inline]
fn bounded(k: ArticKey) -> bool {
    k == ArticKey::Velum
}

#[derive(Clone, Debug)]
pub struct ArticulatorBank {
    pub x: HumanArtic,
    pub v: [f64; N_ARTIC],
    pub target: HumanArtic,
    /// when true the tongue tip rests behind the lower incisors (follows jaw & tongue body)
    pub tip_free: bool,
    /// per-articulator speed multiplier (gestures can request faster/slower motion)
    pub speed: [f64; N_ARTIC],
    omega: [f64; N_ARTIC],
    /// effective targets used in the last step (base blended with gestures), for telemetry
    pub effective: HumanArtic,
    /// Slow, irregular micro-movement of the tract (0 = off, the default; 1 = the reference
    /// amplitudes in WANDER_CM). A held vowel is never frozen: jaw, tongue, lips and larynx drift
    /// by a millimetre or so, which moves the formants a few per cent and the partials' levels
    /// with them (Doc/reviews/2026-11-08-choral-vowel).
    pub wander: f64,
    /// micro-movement of the upper-formant shapers (0 = off), × WANDER_UPPER
    pub wander_upper: f64,
    wander_drift: Option<[Drift; N_ARTIC]>,
    wander_seed: u32,
}

/// Reference micro-movement per articulator (cm, or the articulator's own unit), canonical order.
const WANDER_CM: [f64; N_ARTIC] = {
    let mut a = [0.0; N_ARTIC];
    a[0] = 0.06; // jaw
    a[1] = 0.05; // tbX
    a[2] = 0.05; // tbY
    a[6] = 0.04; // root
    a[7] = 0.04; // lipAp
    a[8] = 0.03; // lipProt
    a[12] = 0.05; // larynx
    a
};

/// Micro-movement of the parts that shape the upper formants (epilarynx, lip spreading), scaled
/// by `wander_upper` separately: they move the singer's-formant cluster and the radiation, so
/// the upper partials vary while the low ones stay.
const WANDER_UPPER: [f64; N_ARTIC] = {
    let mut a = [0.0; N_ARTIC];
    a[9] = 0.04; // lipWidth
    a[13] = 0.06; // epilarynx
    a
};

impl Default for ArticulatorBank {
    fn default() -> Self {
        Self::new()
    }
}

impl ArticulatorBank {
    pub fn new() -> Self {
        let mut b = ArticulatorBank { x: NEUTRAL, v: [0.0; N_ARTIC], target: NEUTRAL, tip_free: true, speed: [1.0; N_ARTIC], omega: ARTIC_SPECS.map(|s| 2.0 * PI * s.hz), effective: NEUTRAL, wander: 0.0, wander_upper: 0.0, wander_drift: None, wander_seed: 0x77a1 };
        let tip = rest_tip(NEUTRAL.jaw, NEUTRAL.tb_x, NEUTRAL.tb_y);
        b.x.tt_x = tip.x;
        b.target.tt_x = tip.x;
        b.x.tt_y = tip.y;
        b.target.tt_y = tip.y;
        b
    }

    pub fn set_target(&mut self, values: &ArticPatch) {
        for (k, val) in values.iter() {
            if !val.is_finite() {
                continue;
            }
            let s = artic_spec(k);
            self.target.set(k, js_min(s.max, js_max(s.min, val)));
        }
    }

    /// Jump immediately (no dynamics), e.g. on voice creation.
    pub fn snap(&mut self, values: &ArticPatch) {
        self.set_target(values);
        self.x = self.target;
        self.v = [0.0; N_ARTIC];
    }

    /// Advance the dynamics. `blend[k]` (when given) is the gesture blend for each key: weighted
    /// mean value, total weight and a speed multiplier. The performer's base target joins with
    /// weight `base_weight`.
    /// Give the micro-movement its own deterministic stream (one singer of a choir).
    pub fn reseed_wander(&mut self, seed: u32) {
        self.wander_seed = seed;
        self.wander_drift = None;
    }

    pub fn step(&mut self, dt: f64, blend: Option<&[Blended]>, base_weight: f64) {
        if (self.wander > 0.0 || self.wander_upper > 0.0) && self.wander_drift.is_none() {
            let seed = self.wander_seed;
            self.wander_drift = Some(core::array::from_fn(|i| Drift::new(1.0 / dt, 0.35, seed.wrapping_mul(2654435761).wrapping_add(i as u32 * 7919))));
        }
        if self.tip_free {
            let tip = rest_tip(self.x.jaw, self.x.tb_x, self.x.tb_y);
            self.target.tt_x = tip.x;
            self.target.tt_y = tip.y;
        }
        for k in ArticKey::ALL {
            let i = k.index();
            let mut tgt = self.target.get(k);
            let mut sp = self.speed[i];
            if let Some(blend) = blend {
                let b = blend[i];
                if b.weight > 0.0 {
                    if bounded(k) {
                        // keys only consonant gestures touch (the velum): follow the gesture's own ramps
                        // (with the weighted mean, a strength-4 gesture at 5 % activation already took 80 %
                        // of the step, so nasality spread far into neighbouring vowels)
                        tgt += js_min(1.0, b.weight / 2.0) * (b.value - tgt);
                    } else {
                        let bw = base_weight;
                        tgt = (tgt * bw + b.value * b.weight) / (bw + b.weight);
                    }
                    sp *= b.speed;
                }
            }
            let amp = self.wander * WANDER_CM[i] + self.wander_upper * WANDER_UPPER[i];
            if amp > 0.0 {
                if let Some(d) = self.wander_drift.as_mut() {
                    tgt += amp * d[i].step();
                }
            }
            let s = ARTIC_SPECS[i];
            if tgt < s.min {
                tgt = s.min;
            } else if tgt > s.max {
                tgt = s.max;
            }
            self.effective.set(k, tgt);
            let w = self.omega[i] * sp;
            // semi-implicit Euler on x'' = w²(T − x) − 2w x'  (stable for w·dt < 1)
            let wdt = w * dt;
            let n = if wdt > MAX_WDT { (wdt / MAX_WDT) as usize + 1 } else { 1 };
            let h = dt / n as f64;
            for _ in 0..n {
                let x = self.x.get(k);
                let acc = w * w * (tgt - x) - 2.0 * w * self.v[i];
                self.v[i] += acc * h;
                self.x.set(k, x + self.v[i] * h);
            }
        }
    }
}
