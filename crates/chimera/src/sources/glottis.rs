//! Parametric vocal-fold source: generates the glottal *area* waveform, which drives the
//! interactive glottal port of the airway network. Flow is computed by the network
//! (Bernoulli + inertance), so pulse skewing, F1 ripple and devoicing under oral closure
//! emerge from the physics.
//!
//! Voicing is gated physiologically:
//!  - transglottal pressure must exceed the phonation threshold (≈2–3 cmH₂O),
//!  - wide abduction (voiceless consonants, /h/) stops vibration.

use crate::sources::lf::LfShape;

/// LF shapes tabulated at Rd 0.3 … 2.7 in steps of 0.01
const LF_TABLE: usize = 241;
use crate::math::{Drift, PI, Rng, TAU, clamp, cos, exp, js_max, js_min, js_round, pow, sin, smoothstep, sqrt};
use crate::physics::CM_H2O;

#[derive(Clone, Copy, Debug)]
pub struct GlottisParams {
    /// fundamental frequency (Hz)
    pub f0: f64,
    /// peak vibratory area (cm²) at full effort
    pub amplitude: f64,
    /// abduction: static opening between the vocal processes (cm²)
    pub abduction: f64,
    /// posterior chink leakage (cm²), always open
    pub chink: f64,
    /// open quotient 0..1
    pub open_quotient: f64,
    /// fraction of the open phase spent opening (0.5 symmetric, larger = faster closing)
    pub skew: f64,
    /// 0 = smooth (raised-cosine) closure, 1 = abrupt slope at closure
    pub closure_sharpness: f64,
    /// vocal effort 0..1 (scales amplitude; pressed voice lowers OQ)
    pub effort: f64,
    /// cycle-to-cycle period perturbation (fraction, e.g. 0.006)
    pub jitter: f64,
    /// cycle-to-cycle amplitude perturbation (fraction, e.g. 0.04)
    pub shimmer: f64,
    /// vibrato depth (cents) and rate (Hz)
    pub vibrato_depth: f64,
    pub vibrato_rate: f64,
    /// slow random pitch drift (cents)
    pub drift: f64,
    /// vocal fry 0..1: low-frequency double-pulsing with short open phase
    pub fry: f64,
    /// diplophonia / period doubling 0..1 (alternating amplitude & period)
    pub diplophonia: f64,
    /// extra glottal closure force 0..1 (glottal stop / ejective: holds the folds shut)
    pub closure: f64,
    /// bifurcation 0..1: a logistic map drives per-cycle period and amplitude. Rising values walk
    /// through period doubling (subharmonics, ~0.3), period four (~0.55) into deterministic chaos (>0.62).
    pub chaos: f64,
    /// Liveliness, 0 = off (the defaults keep the voice as it was). Human singers wander on
    /// several time scales at once (Ternström & Friberg 1989; measured against the Choral
    /// Singing Dataset in Doc/reviews/2026-11-08-spectral-match):
    /// vibrato as noise through a resonator at `vibrato_rate` instead of a sine, 0..1 mix
    pub vib_irregular: f64,
    /// bandwidth of that resonator (Hz): wider = less regular
    pub vib_bandwidth: f64,
    /// slow amplitude wander (fraction SD, 0.12 ≈ 1 dB)
    pub amp_wander: f64,
    /// slow open-quotient wander (SD), which moves the partials' levels independently
    pub oq_wander: f64,
    /// slow wander of the closure's sharpness (SD), which moves the upper partials' levels
    pub sharp_wander: f64,
    /// > 0: the area pulse takes the shape of the Liljencrants–Fant glottal flow with this Rd
    /// (0.3 pressed … 2.7 breathy; see `sources::lf`), replacing open_quotient, skew and
    /// closure_sharpness. 0 keeps the default pulse.
    pub lf_rd: f64,
    /// fast irregular pitch movement (cents SD), band ~4–40 Hz: Ternström & Friberg's flutter.
    /// Spreads each partial by k·cents, so upper partials turn blunt without filling the gaps
    /// between them (white per-cycle jitter does fill them).
    pub flutter: f64,
    /// fast, correlated variation of the closure's sharpness (SD), band ~3–30 Hz, latched per
    /// cycle: the upper partials' levels move and blur ("cloudy" top) while the low ones stay
    pub sharp_flutter: f64,
    /// LF mode: slow wander of Rd (fraction SD, band ~0.5–5 Hz). The pulse's shape and so the
    /// spectral tilt drift: the upper partials move against the lower ones, as in sung tones
    /// measured in VocalSet and the Choral Singing Dataset.
    pub rd_wander: f64,
    /// LF mode: × on the return phase ta (closure abruptness) independent of Rd's open phase;
    /// 1 = Fant's regression (LfShape::with_ra_scale)
    pub lf_ra_scale: f64,
    /// slow random swing of the cycle-to-cycle jitter and shimmer (log-scale, 1 ≈ ×e per unit
    /// drift; 0 = steady). Real sung tones fill the gaps between partials in bursts (peak-hold
    /// gap fill 12–19 dB above the median frame, the engine's 3–7 dB): their irregularity comes
    /// and goes (Doc/design/2026-11-08-aantovaylan-haviot).
    pub aperiodic_wander: f64,
    /// pitch at the start of each voiced stretch (cents, 0 = off): the note starts this far
    /// sharp and settles as A·e^(−t/τ)·cos(2πt/T), τ = onset_time, T = 4·onset_time. CSD
    /// singers start ≈ +90 c and settle in ≈ 0.3 s; held over a note it widens each
    /// partial's base in a peak-hold view (Doc/design/2026-11-08-aantovaylan-haviot).
    pub onset_cents: f64,
    pub onset_time: f64,
}

pub const DEFAULT_GLOTTIS: GlottisParams = GlottisParams {
    f0: 110.0,
    amplitude: 0.16,
    abduction: 0.0,
    chink: 0.006,
    open_quotient: 0.62,
    skew: 0.62,
    closure_sharpness: 0.6,
    effort: 0.7,
    jitter: 0.004,
    shimmer: 0.03,
    vibrato_depth: 0.0,
    vibrato_rate: 6.0,
    drift: 6.0,
    fry: 0.0,
    diplophonia: 0.0,
    closure: 0.0,
    chaos: 0.0,
    vib_irregular: 0.0,
    vib_bandwidth: 1.5,
    amp_wander: 0.0,
    oq_wander: 0.0,
    sharp_wander: 0.0,
    lf_rd: 0.0,
    flutter: 0.0,
    sharp_flutter: 0.0,
    rd_wander: 0.0,
    lf_ra_scale: 1.0,
    aperiodic_wander: 0.0,
    onset_cents: 0.0,
    onset_time: 0.15,
};

#[derive(Clone, Debug)]
pub struct ParametricGlottis {
    pub fs: f64,
    pub params: GlottisParams,
    phase: f64,
    cycle_amp: f64,
    cycle_period_scale: f64,
    cycle_index: u64,
    chaos_x: f64,
    vib_phase: f64,
    rng: Rng,
    drift: Drift,
    drift_value: f64,
    drift_counter: i32,
    pitch_factor: f64,
    // liveliness state (control rate, its own random streams so the defaults stay bit-exact)
    live_rng: Rng,
    vib_y: [f64; 2],
    amp_drift: Drift,
    oq_drift: Drift,
    amp_live: f64,
    oq_live: f64,
    /// oq_live latched at the start of each cycle (a pulse changing shape mid-cycle is noise)
    oq_cycle: f64,
    sharp_drift: Drift,
    sharp_live: f64,
    sharp_cycle: f64,
    /// the fast part alone (sharp_flutter), latched per cycle: in LF mode it moves Rd
    rd_fast_live: f64,
    rd_fast_cycle: f64,
    rd_drift: Drift,
    /// LF pulse of the current cycle
    lf: LfShape,
    /// LF pulses at Rd 0.3, 0.31 … 2.7, built on first use (no allocation: a fixed array)
    lf_table: [Option<LfShape>; LF_TABLE],
    flutter_drift: Drift,
    sharp_flutter_drift: Drift,
    aper_drift: Drift,
    /// time since the voicing gate opened (s), for onset_cents
    onset_t: f64,
    /// current multiplier on jitter and shimmer from aperiodic_wander
    aper_live: f64,
    /// smoothed voicing gate 0..1 (telemetry)
    pub voicing: f64,
    voicing_sm: f64,
    /// smoothed transglottal pressure (dyn/cm²)
    dp_sm: f64,
    dp_coef: f64,
    voice_coef: f64,
    /// last area (telemetry)
    pub area: f64,
    /// instantaneous f0 actually used (telemetry)
    pub f0_now: f64,
    /// phonation threshold pressure (dyn/cm²)
    pub threshold: f64,
    // per-sample smoothing of control-rate parameters (steps in the glottal area at high airflow
    // radiate as clicks, because the output is the derivative of flow)
    static_sm: f64,
    closure_sm: f64,
    amp_sm: f64,
    sm_coef: f64,
}

impl ParametricGlottis {
    pub fn new(fs: f64) -> Self {
        ParametricGlottis {
            fs,
            params: DEFAULT_GLOTTIS,
            phase: 0.0,
            cycle_amp: 1.0,
            cycle_period_scale: 1.0,
            cycle_index: 0,
            chaos_x: 0.37,
            vib_phase: 0.0,
            rng: Rng::new(777),
            drift: Drift::new(fs / 32.0, 0.3, 4242),
            drift_value: 0.0,
            drift_counter: 0,
            pitch_factor: 1.0,
            live_rng: Rng::new(9001),
            vib_y: [0.0; 2],
            amp_drift: Drift::new(fs / 32.0, 0.25, 5150),
            oq_drift: Drift::new(fs / 32.0, 0.4, 6260),
            amp_live: 1.0,
            oq_live: 0.0,
            oq_cycle: 0.0,
            sharp_drift: Drift::new(fs / 32.0, 0.35, 7370),
            sharp_live: 0.0,
            sharp_cycle: 0.0,
            rd_fast_live: 0.0,
            rd_fast_cycle: 0.0,
            rd_drift: Drift::new(fs / 32.0, 0.5, 10_301),
            lf: LfShape::new(1.0),
            lf_table: [None; LF_TABLE],
            flutter_drift: Drift::new(fs / 32.0, 4.0, 8181),
            sharp_flutter_drift: Drift::new(fs / 32.0, 3.0, 9292),
            aper_drift: Drift::new(fs / 32.0, 1.5, 11_411),
            onset_t: 0.0,
            aper_live: 1.0,
            voicing: 0.0,
            voicing_sm: 0.0,
            dp_sm: 0.0,
            dp_coef: 1.0 - exp(-TAU * 30.0 / fs),
            voice_coef: 1.0 - exp(-TAU * 25.0 / fs),
            area: 0.0,
            f0_now: 110.0,
            threshold: 2.5 * CM_H2O,
            static_sm: 0.0,
            closure_sm: 0.0,
            amp_sm: 0.0,
            sm_coef: 1.0 - exp(-1.0 / (0.004 * fs)),
        }
    }

    /// Give one physical singer an independent, deterministic micro-variation stream.
    /// Choir voices use distinct seeds so jitter, shimmer, drift, vibrato phase and the
    /// initial fold phase do not line up like duplicated audio tracks.
    pub fn reseed(&mut self, seed: u32) {
        let mut phase_rng = Rng::new(seed ^ 0xa511_e9b3);
        self.phase = phase_rng.next();
        self.vib_phase = phase_rng.next();
        self.rng = Rng::new(seed ^ 0x63d8_3595);
        self.drift = Drift::new(self.fs / 32.0, 0.3, seed ^ 0x9e37_79b9);
        self.drift_value = 0.0;
        self.drift_counter = 0;
        self.live_rng = Rng::new(seed ^ 0x2545_f491);
        self.vib_y = [0.0; 2];
        self.amp_drift = Drift::new(self.fs / 32.0, 0.25, seed ^ 0x6c07_8965);
        self.oq_drift = Drift::new(self.fs / 32.0, 0.4, seed ^ 0x5851_f42d);
        self.sharp_drift = Drift::new(self.fs / 32.0, 0.35, seed ^ 0x1b87_3593);
        self.flutter_drift = Drift::new(self.fs / 32.0, 4.0, seed ^ 0x3c6e_f372);
        self.sharp_flutter_drift = Drift::new(self.fs / 32.0, 3.0, seed ^ 0x6a09_e667);
        self.rd_drift = Drift::new(self.fs / 32.0, 0.5, seed ^ 0x510e_527f);
        self.aper_drift = Drift::new(self.fs / 32.0, 1.5, seed ^ 0x9b05_688c);
    }

    /// Reset phase etc.
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.voicing_sm = 0.0;
        self.dp_sm = 0.0;
    }

    /// Advance one sample. `dp` is the instantaneous transglottal pressure (p_sub − p_supra).
    /// Returns the glottal area (cm²).
    #[inline]
    pub fn next(&mut self, dp: f64) -> f64 {
        let p = self.params;
        self.dp_sm += self.dp_coef * (dp - self.dp_sm);

        // drift and vibrato evaluated at a decimated rate (pitch modulation is slow)
        self.drift_counter -= 1;
        if self.drift_counter <= 0 {
            self.drift_counter = 32;
            self.drift_value = self.drift.step();
            self.vib_phase += ((p.vibrato_rate * 32.0) / self.fs) * (1.0 + 0.04 * self.drift_value);
            if self.vib_phase > 1.0 {
                self.vib_phase -= 1.0;
            }
            let mut vib = sin(TAU * self.vib_phase);
            if p.vib_irregular > 0.0 {
                // AR(2) resonator driven by white noise, scaled to the sine's RMS (1/√2)
                let cr = self.fs / 32.0;
                let r = exp(-PI * p.vib_bandwidth / cr);
                let (a1, a2) = (2.0 * r * cos(TAU * p.vibrato_rate / cr), -r * r);
                let y = a1 * self.vib_y[0] + a2 * self.vib_y[1] + self.live_rng.gauss();
                self.vib_y = [y, self.vib_y[0]];
                let var = (1.0 - a2) / ((1.0 + a2) * ((1.0 - a2) * (1.0 - a2) - a1 * a1));
                let res = y / sqrt(2.0 * var);
                vib = (1.0 - p.vib_irregular) * vib + p.vib_irregular * res;
            }
            if p.rd_wander > 0.0 || p.sharp_flutter > 0.0 {
                let fast = if p.sharp_flutter > 0.0 { p.sharp_flutter * self.sharp_flutter_drift.step() } else { 0.0 };
                let slow = if p.rd_wander > 0.0 { p.rd_wander * self.rd_drift.step() } else { 0.0 };
                self.rd_fast_live = fast + slow;
            }
            if p.sharp_wander > 0.0 || p.sharp_flutter > 0.0 {
                let slow = if p.sharp_wander > 0.0 { p.sharp_wander * self.sharp_drift.step() } else { 0.0 };
                let fast = if p.sharp_flutter > 0.0 { p.sharp_flutter * self.sharp_flutter_drift.step() } else { 0.0 };
                self.sharp_live = slow + fast;
            }
            if p.amp_wander > 0.0 || p.oq_wander > 0.0 {
                self.amp_live = js_max(0.2, 1.0 + p.amp_wander * self.amp_drift.step());
                self.oq_live = p.oq_wander * self.oq_drift.step();
            }
            if p.aperiodic_wander > 0.0 {
                self.aper_live = exp(p.aperiodic_wander * self.aper_drift.step());
            }
            let mut cents = p.vibrato_depth * vib + p.drift * self.drift_value;
            if p.onset_cents != 0.0 {
                if self.voicing_sm < 0.2 {
                    self.onset_t = 0.0;
                } else {
                    self.onset_t += 32.0 / self.fs;
                }
                let tau = js_max(p.onset_time, 0.01);
                cents += p.onset_cents * exp(-self.onset_t / tau) * cos(TAU * self.onset_t / (4.0 * tau));
            }
            if p.flutter > 0.0 {
                cents += p.flutter * self.flutter_drift.step();
            }
            self.pitch_factor = pow(2.0, cents / 1200.0);
        }
        let mut f0 = p.f0 * self.pitch_factor;
        // fry pulls the rate down toward ~45 Hz
        if p.fry > 0.0 {
            f0 = f0 * (1.0 - p.fry) + js_min(f0, 48.0) * p.fry;
        }
        self.f0_now = f0;

        // voicing gate: pressure above threshold, adduction sufficient, not forcibly closed
        let pressure_gate = smoothstep(self.threshold * 0.8, self.threshold * 1.6, self.dp_sm);
        let adduct_gate = 1.0 - smoothstep(0.08, 0.22, self.static_sm - p.chink);
        let target = pressure_gate * adduct_gate * (1.0 - self.closure_sm);
        self.voicing_sm += self.voice_coef * (target - self.voicing_sm);
        self.voicing = self.voicing_sm;

        // phase advance with per-cycle perturbations
        self.phase += (f0 / self.fs) / self.cycle_period_scale;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
            self.cycle_index += 1;
            self.oq_cycle = self.oq_live;
            self.sharp_cycle = self.sharp_live;
            self.rd_fast_cycle = self.rd_fast_live;
            if p.lf_rd > 0.0 {
                if self.rd_fast_cycle != 0.0 {
                    // fast closure variation in LF mode moves Rd cycle by cycle (tabulated shapes)
                    let rd = clamp(p.lf_rd * (1.0 + self.rd_fast_cycle), crate::sources::lf::RD_MIN, crate::sources::lf::RD_MAX);
                    let i = js_round((rd - crate::sources::lf::RD_MIN) / 0.01) as usize;
                    let i = if i >= LF_TABLE { LF_TABLE - 1 } else { i };
                    if self.lf_table[i].map_or(true, |s| s.ra_scale != p.lf_ra_scale) {
                        self.lf_table[i] = Some(LfShape::with_ra_scale(crate::sources::lf::RD_MIN + 0.01 * i as f64, p.lf_ra_scale));
                    }
                    if let Some(s) = self.lf_table[i] {
                        self.lf = s;
                    }
                } else if (p.lf_rd - self.lf.rd).abs() > 1e-9 || self.lf.ra_scale != p.lf_ra_scale {
                    self.lf = LfShape::with_ra_scale(p.lf_rd, p.lf_ra_scale);
                }
            }
            let alt = if self.cycle_index & 1 == 1 { 1.0 } else { -1.0 };
            let mut chaos_p = 0.0;
            let mut chaos_a = 0.0;
            if p.chaos > 0.0 {
                let r = 2.9 + 1.1 * js_min(1.0, p.chaos);
                self.chaos_x = r * self.chaos_x * (1.0 - self.chaos_x);
                if !(self.chaos_x > 0.0 && self.chaos_x < 1.0) {
                    self.chaos_x = 0.37;
                }
                let dev = self.chaos_x - 0.5;
                chaos_p = 0.45 * p.chaos * dev;
                chaos_a = 1.1 * p.chaos * dev;
            }
            let aper = self.aper_live;
            self.cycle_period_scale = 1.0 + p.jitter * aper * self.rng.gauss() + p.diplophonia * 0.08 * alt + chaos_p;
            self.cycle_period_scale = clamp(self.cycle_period_scale, 0.55, 1.8);
            self.cycle_amp = js_max(0.12, 1.0 + p.shimmer * aper * self.rng.gauss() - p.diplophonia * 0.35 * (if alt > 0.0 { 1.0 } else { 0.0 }) + chaos_a);
        }

        // pulse shape within the cycle
        let mut oq = clamp(p.open_quotient - 0.18 * (p.effort - 0.5) + self.oq_cycle, 0.25, 0.98);
        if p.fry > 0.0 {
            oq = oq * (1.0 - p.fry) + 0.28 * p.fry;
        }
        let sharp = clamp(p.closure_sharpness + self.sharp_cycle, 0.0, 1.0);
        let mut pulse = if p.lf_rd > 0.0 { self.lf.flow(self.phase) } else { shape(self.phase, oq, p.skew, sharp) };
        if p.fry > 0.3 {
            // second, weaker pulse shortly after the first (double pulsing typical of fry)
            let ph2 = self.phase - 0.38;
            if ph2 > 0.0 {
                pulse += 0.55 * smoothstep(0.3, 0.8, p.fry) * shape(ph2, oq * 0.8, p.skew, sharp);
            }
        }
        let effort_scale = 0.45 + 0.75 * p.effort;
        let sc = self.sm_coef;
        self.amp_sm += sc * (p.amplitude * effort_scale * self.amp_live - self.amp_sm);
        self.closure_sm += sc * (p.closure - self.closure_sm);
        self.static_sm += sc * (p.abduction + p.chink - self.static_sm);
        let vib = self.amp_sm * self.cycle_amp * self.voicing_sm * pulse;
        let statics = self.static_sm * (1.0 - 0.92 * self.closure_sm);
        let a = js_max(0.0, vib + statics);
        self.area = a;
        a
    }

    pub fn cycle_phase(&self) -> f64 {
        self.phase
    }
}

/// Normalised area pulse (0..1) at cycle phase ph.
#[inline]
fn shape(ph: f64, oq: f64, skew: f64, sharp: f64) -> f64 {
    if ph >= oq {
        return 0.0;
    }
    let t = ph / oq;
    let s = clamp(skew, 0.3, 0.9);
    if t < s {
        return 0.5 - 0.5 * cos((PI * t) / s);
    }
    let u = (t - s) / (1.0 - s);
    let soft = 0.5 + 0.5 * cos(PI * u);
    let hard = cos(0.5 * PI * u);
    soft + (hard - soft) * sharp
}
