//! Liljencrants–Fant (LF) glottal flow pulse with Fant's Rd parameter, used as the shape of the
//! glottal area pulse (opt-in: `GlottisParams::lf_rd > 0`).
//!
//! Flow derivative over one period (normalised to 1), with E(te) = −1:
//!   open phase   0 ≤ t ≤ te:  E(t) = E0·e^{αt}·sin(ωt),  ω = π/tp
//!   return phase te < t ≤ 1:  E(t) = −(1/(ε·ta))·[e^{−ε(t−te)} − e^{−ε(1−te)}]
//! with ε·ta = 1 − e^{−ε(1−te)}, and α chosen so the flow returns to zero at the end of the
//! period. Rd → (Ra, Rk, Rg) by Fant's regression (Fant 1995, "The LF-model revisited"):
//!   Ra = (−1 + 4.8·Rd)/100, Rk = (22.4 + 11.8·Rd)/100, Rg = Rk / (4·(0.11·Rd/(0.5 + 1.2·Rk) − Ra)),
//!   tp = 1/(2·Rg), te = tp·(1 + Rk), ta = Ra.
//!
//! The return phase is what the default area pulse lacks: the folds stop closing over ta, so the
//! excitation's spectrum falls off above 1/(2π·ta·T0) instead of staying flat. Measured against
//! real singers in Doc/reviews/2026-11-08-lf-source. Solving allocates nothing.

use crate::math::{PI, clamp, cos, exp, js_max, js_min, sin};

pub const RD_MIN: f64 = 0.3;
pub const RD_MAX: f64 = 2.7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LfShape {
    pub rd: f64,
    pub tp: f64,
    pub te: f64,
    pub ta: f64,
    alpha: f64,
    eps: f64,
    omega: f64,
    e0: f64,
    /// flow at te (start of the return phase)
    ute: f64,
    /// peak flow, for normalising the pulse to 1
    pub upeak: f64,
}

impl LfShape {
    pub fn new(rd: f64) -> LfShape {
        let rd = clamp(rd, RD_MIN, RD_MAX);
        let ra = (-1.0 + 4.8 * rd) / 100.0;
        let rk = (22.4 + 11.8 * rd) / 100.0;
        let rg = rk / (4.0 * ((0.11 * rd) / (0.5 + 1.2 * rk) - ra));
        let tp = 0.5 / rg;
        let te = js_min(tp * (1.0 + rk), 0.97);
        let ta = clamp(ra, 1e-4, (1.0 - te) * 0.95);
        let tb = 1.0 - te;
        // ε·ta = 1 − e^{−ε·tb} (Newton from 1/ta)
        let mut eps = 1.0 / ta;
        for _ in 0..40 {
            let e = exp(-eps * tb);
            let g = eps * ta - 1.0 + e;
            let dg = ta - tb * e;
            let next = eps - g / dg;
            eps = if next > 0.0 { next } else { 0.5 * eps };
            if g.abs() < 1e-12 {
                break;
            }
        }
        let omega = PI / tp;
        let eb = exp(-eps * tb);
        // return-phase flow change (with Ee = 1): ∫ E over (te, 1]
        let i2 = -(1.0 / (eps * ta)) * ((1.0 - eb) / eps - tb * eb);
        let open = |alpha: f64| {
            let e0 = -1.0 / (exp(alpha * te) * sin(omega * te));
            e0 * (exp(alpha * te) * (alpha * sin(omega * te) - omega * cos(omega * te)) + omega) / (alpha * alpha + omega * omega)
        };
        // total flow over the period is zero: open(α) + i2 = 0. open() rises with α; bisect.
        // open(α) is not monotonic over every range: scan for the first sign change from low α
        // (the physical root keeps the pulse positive), then bisect inside that bracket.
        let g = |a: f64| open(a) + i2;
        let (mut lo, mut hi) = (-60.0, -60.0);
        let mut glo = g(lo);
        let mut a = -60.0;
        while a < 60.0 {
            let b = a + 0.25;
            let gb = g(b);
            if (glo > 0.0) != (gb > 0.0) {
                lo = a;
                hi = b;
                break;
            }
            a = b;
            glo = gb;
        }
        let up = g(hi) > 0.0;
        for _ in 0..100 {
            let mid = 0.5 * (lo + hi);
            if (g(mid) > 0.0) == up {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let alpha = 0.5 * (lo + hi);
        let e0 = -1.0 / (exp(alpha * te) * sin(omega * te));
        let mut s = LfShape { rd, tp, te, ta, alpha, eps, omega, e0, ute: 0.0, upeak: 1.0 };
        s.ute = s.open_flow(te);
        let mut peak = 0.0;
        for i in 0..=512 {
            peak = js_max(peak, s.raw_flow(i as f64 / 512.0));
        }
        s.upeak = js_max(peak, 1e-12);
        s
    }

    #[inline]
    fn open_flow(&self, t: f64) -> f64 {
        let (a, w) = (self.alpha, self.omega);
        self.e0 * (exp(a * t) * (a * sin(w * t) - w * cos(w * t)) + w) / (a * a + w * w)
    }

    #[inline]
    fn raw_flow(&self, t: f64) -> f64 {
        if t <= self.te {
            return self.open_flow(t);
        }
        let tb = 1.0 - self.te;
        let eb = exp(-self.eps * tb);
        let dt = t - self.te;
        self.ute - (1.0 / (self.eps * self.ta)) * ((1.0 - exp(-self.eps * dt)) / self.eps - dt * eb)
    }

    /// Glottal flow at cycle phase `t` (0..1), normalised to a peak of 1 and never negative.
    #[inline]
    pub fn flow(&self, t: f64) -> f64 {
        js_max(0.0, self.raw_flow(clamp(t, 0.0, 1.0)) / self.upeak)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_voice_matches_fants_timing() {
        let s = LfShape::new(1.0);
        // Rd 1: Ra 3.8 %, Rk 34.2 %; the pulse opens, peaks near tp and returns to ~0 at the end
        assert!((s.ta - 0.038).abs() < 1e-9);
        assert!(s.te > s.tp && s.te < 0.9);
        assert!(s.flow(0.0) < 1e-6);
        assert!(s.flow(1.0) < 1e-3, "flow at period end {}", s.flow(1.0));
        let peak = (0..1000).map(|i| s.flow(i as f64 / 1000.0)).fold(0.0, f64::max);
        assert!((peak - 1.0).abs() < 0.01, "peak {peak} upeak {} te {} tp {} ta {} alpha {} f(.3) {} f(.6) {}", s.upeak, s.te, s.tp, s.ta, s.alpha, s.flow(0.3), s.flow(0.6));
    }

    #[test]
    fn pulses_are_valid_across_the_rd_range() {
        for i in 0..=24 {
            let rd = 0.3 + 0.1 * i as f64;
            let s = LfShape::new(rd);
            let v: Vec<f64> = (0..=1000).map(|k| s.flow(k as f64 / 1000.0)).collect();
            let peak = v.iter().cloned().fold(0.0, f64::max);
            assert!((peak - 1.0).abs() < 0.01, "Rd {rd}: peak {peak}");
            assert!(v[1000] < 0.02, "Rd {rd}: flow left at period end {}", v[1000]);
            assert!(v.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn higher_rd_has_a_longer_return_phase() {
        assert!(LfShape::new(2.0).ta > LfShape::new(0.6).ta);
    }
}
