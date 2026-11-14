//! Human airway: lungs → trachea → glottis → pharynx ─┬─ oral → lips
//!                                                    └─ velum port → nasal (+maxillary sinus) → nostrils
//! plus piriform fossae (closed side branch above the epilarynx).
//! Applies HumanArtic configurations (geometry → area function → tube targets) and manages
//! the turbulence noise sources at constrictions and the teeth.

use crate::airway::network::{AirwayNetwork, Area, NoiseSpec, PortSpec, Tube, TubeSpec};
use crate::anatomy::human::{HumanArtic, HumanBody, HumanFrame, HumanVocalTract, N_ARTIC};
use crate::math::{js_max, js_min, js_round, smoothstep as smooth, sqrt};
use crate::physics::{C_SOUND, RHO, WALL_NASAL, WALL_SOFT, WALL_TRACHEA};

/// The fewest sections in the pharynx and the oral tube (`HumanAirway::new`).
pub const MIN_CELLS: usize = 8;

/// Room for the settled-geometry key: the articulation, 17 settings and a drawn horn of up to
/// 256 points (Suu's has 32), so comparing it never allocates.
const GEOM_KEY_CAP: usize = N_ARTIC + 17 + 256;

#[derive(Clone, Copy, Debug)]
pub struct NoiseGains {
    pub glottis: f64,
    pub constriction: f64,
    pub teeth: f64,
    /// band of the glottal (aspiration) noise source, Hz (one-pole filters)
    pub glottis_lowpass: f64,
    pub glottis_highpass: f64,
}

#[derive(Clone, Debug)]
pub struct HumanAirway {
    pub net: AirwayNetwork,
    pub tract: HumanVocalTract,
    pub body: HumanBody,
    pub trachea: Tube,
    pub pharynx: Tube,
    pub oral: Tube,
    pub nasal: Tube,
    pub fossa: Tube,
    pub lungs: usize,
    pub glottis_port: usize,
    pub velum_port: usize,
    pub rad_lips: usize,
    pub rad_nose: usize,
    pub n_ph: usize,
    pub n_or: usize,
    pub ph_areas: Vec<f64>,
    pub or_areas: Vec<f64>,
    pub ph_min: Vec<f64>,
    pub or_min: Vec<f64>,
    /// the frame of the last `apply` (None before the first)
    pub frame: Option<HumanFrame>,
    pub ph_len: f64,
    pub or_len: f64,
    /// noise gains (tunable)
    pub noise_gain: NoiseGains,
    /// max area change rate (cm²/s, body-scaled): a physiological guard against geometric jumps
    pub max_area_rate: f64,
    /// additional rate per cm² of current area (1/s)
    pub area_rate_gain: f64,
    /// The last part of a closure is slower than a release: contact spreads over the palate as
    /// tissue compresses, while a release is ballistic. Sealing at the opening rate drove the air
    /// the articulator displaces through a slit that shut within ~1 ms (an audible click). Below
    /// sealHi cm² the closing rate eases from maxAreaRate down to sealRate, and below sealEase it
    /// slows further (∝ √A) so the seal itself is reached smoothly.
    pub seal_rate: f64,
    pub seal_lo: f64,
    pub seal_hi: f64,
    /// below this area (cm²) the closing rate slows with √A, so the seal is reached smoothly
    pub seal_ease: f64,
    /// Opt-in (Suu's drawn horn, default off): an area function (cm², adult scale) sampled evenly
    /// from the glottis (first) to the lips (last), blended over the articulated one by
    /// `profile_mix` 0..1 before the slew limits, so moving it is as smooth as articulating.
    /// `profile_shift` slides it along the tract (a fraction of the length, + toward the lips);
    /// `profile_open` scales its areas. Empty or a mix of 0 leaves the airway untouched.
    /// `profile_wrap`: the profile is one period (points at i/n), and a shift wraps around
    /// instead of holding the end values, so a periodic shape can travel through the tract.
    pub profile: Vec<f64>,
    pub profile_wrap: bool,
    pub profile_mix: f64,
    pub profile_shift: f64,
    pub profile_open: f64,
    /// areas committed at the start of the current control step (slew-limit baseline)
    base: Option<[Vec<f64>; 4]>,
    base_dt: f64,
    noise_list: Vec<NoiseSpec>,
    scratch_frame: HumanFrame,
    /// Settled geometry (`settled_for`, `note_settled`): the inputs of the last geometry update,
    /// whether it gave the same targets as the one before (and needed no contact correction),
    /// those targets, and a buffer for the next key. All sized at construction.
    geom_key: Vec<f64>,
    key_scratch: Vec<f64>,
    prev_targets: [Vec<f64>; 4],
    settled: bool,
    /// skip settled geometry updates (on by default; off only to check that skipping changes
    /// nothing)
    pub skip_settled: bool,
}

impl HumanAirway {
    /// experiment (default 0): the fraction of the trachea's wall motion radiated as sound, i.e.
    /// the voice heard from the neck and chest. It carries the subglottal resonances and the
    /// trachea wall's own (≈200 Hz) into the output (Doc/design/2026-11-08-aantovaylan-haviot).
    pub fn set_chest_radiation(&mut self, f: f64) {
        for i in 0..self.trachea.n {
            self.net.set_wall_radiation(self.trachea.node(i), f);
        }
    }

    /// As set_chest_radiation, but the chest's sound reaches the listener `delay_s` after the
    /// mouth's (≈0.6–0.9 ms for a microphone in front of the mouth: the chest is 20–30 cm lower),
    /// so it combs gently with the mouth instead of cancelling one partial in phase.
    pub fn set_chest_radiation_delayed(&mut self, f: f64, delay_s: f64) {
        for i in 0..self.trachea.n {
            self.net.set_chest_radiation(self.trachea.node(i), f, delay_s);
        }
    }

    /// `fs`: internal sample rate (Hz).
    pub fn new(fs: f64, body: HumanBody) -> Self {
        Self::with_min_cells(fs, body, MIN_CELLS)
    }

    /// As `new`, with at least `min_cells` sections in the pharynx and the oral tube instead of
    /// `MIN_CELLS` (fewer allow a lower rate for the same body; the tube resolves less).
    pub fn with_min_cells(fs: f64, body: HumanBody, min_cells: usize) -> Self {
        let k = body.scale;
        let tract = HumanVocalTract::new(body);
        let mut net = AirwayNetwork::new(fs);
        let dx_min = (C_SOUND / fs) / 0.93;
        let (n_ph, n_or) = Self::cells(fs, &body, min_cells);

        let tr_len = 12.0 * k;
        let trachea = net.add_tube(TubeSpec::new("trachea", js_max(6.0, (tr_len / js_max(dx_min, 0.9 * k)).floor()) as usize, tr_len, Area::Uniform(2.5 * k * k)).wall(Some(WALL_TRACHEA)));
        let pharynx = net.add_tube(TubeSpec::new("pharynx", n_ph, 8.8 * k, Area::Uniform(3.0 * k * k)).wall(Some(WALL_SOFT)).radiating(0.25).perimeter(1.25));
        let oral = net.add_tube(TubeSpec::new("oral", n_or, 8.3 * k, Area::Uniform(3.0 * k * k)).wall(Some(WALL_SOFT)).radiating(0.25).perimeter(1.5));
        let na_len = 11.4 * k;
        // a lean airway (fewer cells than MIN_CELLS) lets the nose and the piriform sinus have
        // fewer too, so none is shorter than the stable step at its lower rate
        let lean = min_cells < MIN_CELLS;
        let n_na = js_max(if lean { min_cells as f64 } else { 8.0 }, (na_len / js_max(dx_min, 0.5 * k)).floor()) as usize;
        let n_fossa = if lean { (2.0 * k / dx_min).floor().clamp(2.0, 3.0) as usize } else { 3 };
        let na_areas: Vec<f64> = (0..n_na)
            .map(|i| {
                let u = (i as f64 + 0.5) / n_na as f64;
                let a = if u < 0.18 {
                    2.6
                } else if u < 0.75 {
                    2.0 - 0.6 * (u - 0.18)
                } else if u < 0.9 {
                    1.4
                } else {
                    1.0
                };
                a * k * k
            })
            .collect();
        let nasal = net.add_tube(TubeSpec::new("nasal", n_na, na_len, Area::Profile(na_areas)).wall(Some(WALL_NASAL)).perimeter(3.2).viscous(2.5));
        let fossa = net.add_tube(TubeSpec::new("piriform", n_fossa, 2.0 * k, Area::Uniform(0.9 * k * k)).wall(Some(WALL_SOFT)).perimeter(1.3));

        let lungs = net.add_pressure_node("lungs");
        let tr_a = 2.5 * k * k;
        net.add_port(lungs, trachea.first_node(), PortSpec::new(tr_a, 1.0).kinetic(0.0, 0.0).resistance((0.5 * RHO * C_SOUND) / tr_a));
        let glottis_port = net.add_port(trachea.last_node(), pharynx.first_node(), PortSpec::new(0.0, 0.3 * k).kinetic(0.37, 1.0).slit(1.4 * k));
        net.connect(pharynx.last_node(), oral.first_node());
        let velum_port = net.add_port(pharynx.last_node(), nasal.first_node(), PortSpec::new(0.0, 1.0 * k).kinetic(0.3, 0.8));
        // piriform fossae branch off ≈2.6 cm above the glottis
        let fossa_node = pharynx.node(((n_ph - 1) as f64).min(js_round((2.6 / 8.8) * n_ph as f64)) as usize);
        net.connect(fossa_node, fossa.first_node());
        // maxillary sinus (Helmholtz) on the nasal cavity
        let sinus = net.add_cavity("sinus", 14.0 * k * k * k, None);
        net.add_port(nasal.node(js_round(n_na as f64 * 0.45) as usize), sinus, PortSpec::new(0.12 * k * k, 0.5 * k).kinetic(0.0, 0.0).no_half_sections());
        let rad_lips = net.add_radiation(oral.last_node());
        let rad_nose = net.add_radiation(nasal.last_node());
        net.finalize();
        HumanAirway {
            net,
            tract,
            body,
            trachea,
            pharynx,
            oral,
            nasal,
            fossa,
            lungs,
            glottis_port,
            velum_port,
            rad_lips,
            rad_nose,
            n_ph,
            n_or,
            ph_areas: vec![0.0; n_ph],
            or_areas: vec![0.0; n_or],
            ph_min: vec![0.0; n_ph],
            or_min: vec![0.0; n_or],
            frame: None,
            ph_len: 8.8,
            or_len: 8.3,
            noise_gain: NoiseGains { glottis: 3.2e-6, constriction: 2.2e-6, teeth: 3.0e-6, glottis_lowpass: 3500.0, glottis_highpass: 300.0 },
            max_area_rate: 45.0,
            area_rate_gain: 40.0,
            seal_rate: 5.0,
            seal_lo: 0.03,
            seal_hi: 0.4,
            seal_ease: 0.03,
            profile: Vec::new(),
            profile_wrap: false,
            profile_mix: 0.0,
            profile_shift: 0.0,
            profile_open: 1.0,
            base: None,
            base_dt: 0.0,
            noise_list: Vec::with_capacity(8),
            scratch_frame: HumanFrame::with_capacity(),
            geom_key: Vec::with_capacity(GEOM_KEY_CAP),
            key_scratch: Vec::with_capacity(GEOM_KEY_CAP),
            prev_targets: [vec![0.0; n_ph], vec![0.0; n_or], vec![0.0; n_ph], vec![0.0; n_or]],
            settled: false,
            skip_settled: true,
        }
    }

    /// Everything a geometry update (`apply`) reads besides the targets it ramps from: the
    /// articulation, the step, the drawn horn and its settings, the slew limits, the body and the
    /// noise gains.
    fn geom_key_into(&self, a: &HumanArtic, dt: f64, out: &mut Vec<f64>) {
        out.clear();
        out.extend_from_slice(&a.to_array());
        let g = self.noise_gain;
        out.extend_from_slice(&[
            dt,
            if self.profile_wrap { 1.0 } else { 0.0 },
            self.profile_mix,
            self.profile_shift,
            self.profile_open,
            self.max_area_rate,
            self.area_rate_gain,
            self.seal_rate,
            self.seal_lo,
            self.seal_hi,
            self.seal_ease,
            self.body.scale,
            g.glottis,
            g.constriction,
            g.teeth,
            g.glottis_lowpass,
            g.glottis_highpass,
        ]);
        out.extend_from_slice(&self.profile);
    }

    /// Whether `apply(a, dt)` would set the same targets as the last update did: its inputs are
    /// unchanged (bit for bit) and that update had already settled. Then it need not run.
    pub fn settled_for(&mut self, a: &HumanArtic, dt: f64) -> bool {
        if !self.settled || !self.skip_settled {
            return false;
        }
        let mut k = core::mem::take(&mut self.key_scratch);
        self.geom_key_into(a, dt, &mut k);
        let same = k == self.geom_key;
        self.key_scratch = k;
        same
    }

    /// After a geometry update from `a` and `dt`: record its inputs, and whether it settled (the
    /// same targets as the update before, and `clean`: no correction re-applied after it).
    pub fn note_settled(&mut self, a: &HumanArtic, dt: f64, clean: bool) {
        let mut k = core::mem::take(&mut self.geom_key);
        self.geom_key_into(a, dt, &mut k);
        self.geom_key = k;
        let now = [&self.ph_areas, &self.or_areas, &self.ph_min, &self.or_min];
        let same = (0..4).all(|i| self.prev_targets[i] == *now[i]);
        for (i, v) in now.into_iter().enumerate() {
            self.prev_targets[i].copy_from_slice(v);
        }
        self.settled = clean && same;
    }

    /// Sections in the pharynx and the oral tube at internal rate `fs`: as many as the shortest
    /// expected lengths (larynx raised, lips retracted) allow at the stable step, at least
    /// `min_cells`.
    pub fn cells(fs: f64, body: &HumanBody, min_cells: usize) -> (usize, usize) {
        let k = body.scale;
        let dx_min = (C_SOUND / fs) / 0.93;
        let ph_min_len = 8.7 * k * (0.55 + 0.45 * body.pharynx) - 1.6 * k;
        let or_min_len = 8.3 * k - 1.0 * k;
        let n_ph = (min_cells as f64).max((ph_min_len / dx_min).floor()) as usize;
        let n_or = (min_cells as f64).max((or_min_len / dx_min).floor()) as usize;
        (n_ph, n_or)
    }

    /// Whether a lean airway (`min_cells` < `MIN_CELLS`) is stable at rate `fs`: no tube's
    /// minimum count forces its sections shorter than the stable step.
    pub fn fits(fs: f64, body: &HumanBody, min_cells: usize) -> bool {
        let k = body.scale;
        let dx_min = (C_SOUND / fs) / 0.93;
        let ph_min_len = 8.7 * k * (0.55 + 0.45 * body.pharynx) - 1.6 * k;
        let or_min_len = 8.3 * k - 1.0 * k;
        let m = min_cells as f64;
        (ph_min_len / dx_min).floor() >= m && (or_min_len / dx_min).floor() >= m && (11.4 * k / dx_min).floor() >= m && (2.0 * k / dx_min).floor() >= 2.0
    }

    /// Internal branch index helpers
    #[inline]
    pub fn branch_of(&self, t: Tube, i: isize) -> usize {
        t.branch(i.clamp(0, t.n as isize - 2) as usize)
    }

    /// Geometry → targets for the next block. `dt > 0` enables slew limiting.
    pub fn apply(&mut self, a: &HumanArtic, dt: f64) -> &HumanFrame {
        if dt > 0.0 {
            let base = self.base.get_or_insert_with(|| [vec![0.0; self.n_ph], vec![0.0; self.n_or], vec![0.0; self.n_ph], vec![0.0; self.n_or]]);
            base[0].copy_from_slice(&self.ph_areas);
            base[1].copy_from_slice(&self.or_areas);
            base[2].copy_from_slice(&self.ph_min);
            base[3].copy_from_slice(&self.or_min);
        }
        self.base_dt = dt;
        self.commit(a, dt)
    }

    /// Recompute the geometry within the same control step (after a contact correction moved an
    /// articulator), still slew-limited against the areas the step started from: re-applying
    /// without the limit let closures appear within one block, a pressure and output spike.
    pub fn reapply(&mut self, a: &HumanArtic) -> &HumanFrame {
        self.commit(a, self.base_dt)
    }

    fn commit(&mut self, a: &HumanArtic, dt: f64) -> &HumanFrame {
        // compute into the frame kept from last time (its buffers are reused)
        let mut f = self.frame.take().unwrap_or_else(|| core::mem::take(&mut self.scratch_frame));
        self.tract.compute(a, &mut f);
        let (ph_len, or_len) = self.tract.resample(a, &f, &mut self.ph_areas, &mut self.or_areas, Some(&mut self.ph_min), Some(&mut self.or_min));
        if self.profile_mix > 0.0 && self.profile.len() >= 2 {
            self.blend_profile(&mut f, ph_len, or_len);
        }
        if dt > 0.0
            && let Some(base) = &self.base
        {
            // both section areas and sub-section minima are rate-limited: the minima set the jet
            // (Borda–Carnot) losses ∝ 1/A², so a fast drop chokes the flow within a block
            // rate limit grows with cross-section: wide cavities reshape fast, narrow constrictions
            // (lateral extent ~1–2 cm) open and close at physiological rates (sets burst sharpness)
            let k2 = self.body.scale * self.body.scale;
            let open_r = self.max_area_rate * k2;
            let gain = self.area_rate_gain;
            let seal_r = self.seal_rate * k2;
            let lo = self.seal_lo * k2;
            let hi = self.seal_hi * k2;
            let ease = self.seal_ease * k2;
            // closing rate at area A: the release rate eased down to the seal rate near contact,
            // and in the last part slowing with √A, so the area (and the jet through it, ∝ A) comes
            // to rest at the seal instead of stopping dead: at a constant rate the flow's slope
            // dropped to zero at once, a step in the radiated pressure (an audible click)
            let close_r = |area: f64| -> f64 {
                if area < ease {
                    return seal_r * sqrt(js_max(0.0, area) / ease);
                }
                let u = if area <= lo {
                    0.0
                } else if area >= hi {
                    1.0
                } else {
                    (area - lo) / (hi - lo)
                };
                seal_r + (open_r - seal_r) * u * u * (3.0 - 2.0 * u)
            };
            let clamp_arr = |cur: &mut [f64], old: &[f64], mins: &mut [f64], old_mins: &[f64]| {
                for i in 0..cur.len() {
                    let d = cur[i] - old[i];
                    if d > 0.0 {
                        let lim = (open_r + gain * cur[i]) * dt;
                        if d > lim {
                            cur[i] = old[i] + lim;
                        }
                    } else {
                        let lim = (close_r(old[i]) + gain * old[i]) * dt;
                        if d < -lim {
                            cur[i] = old[i] - lim;
                        }
                    }
                    let dm = mins[i] - old_mins[i];
                    if dm > 0.0 {
                        let lim_m = (open_r + gain * mins[i]) * dt;
                        if dm > lim_m {
                            mins[i] = old_mins[i] + lim_m;
                        }
                    } else {
                        let lim_m = (close_r(old_mins[i]) + gain * old_mins[i]) * dt;
                        if dm < -lim_m {
                            mins[i] = old_mins[i] - lim_m;
                        }
                    }
                    if mins[i] > cur[i] {
                        mins[i] = cur[i];
                    }
                    if mins[i] < 0.0 {
                        mins[i] = 0.0;
                    }
                }
            };
            clamp_arr(&mut self.ph_areas, &base[0], &mut self.ph_min, &base[2]);
            clamp_arr(&mut self.or_areas, &base[1], &mut self.or_min, &base[3]);
        }
        self.ph_len = ph_len;
        self.or_len = or_len;
        self.net.set_tube_targets(self.pharynx, &self.ph_areas, Some(ph_len), Some(&self.ph_min));
        self.net.set_tube_targets(self.oral, &self.or_areas, Some(or_len), Some(&self.or_min));
        self.net.set_port_target(self.velum_port, f.velum_port);
        self.update_noise(&f);
        self.frame = Some(f);
        self.frame.as_ref().unwrap()
    }

    /// The drawn area at `u` (0 glottis … 1 lips), shifted and opened, scaled to the body.
    fn profile_at(&self, u: f64) -> f64 {
        let p = &self.profile;
        let a = if self.profile_wrap {
            let n = p.len();
            let x = (u - self.profile_shift).rem_euclid(1.0) * n as f64;
            let i = (x as usize).min(n - 1);
            p[i] + (x - i as f64) * (p[(i + 1) % n] - p[i])
        } else {
            let x = (u - self.profile_shift).clamp(0.0, 1.0) * (p.len() - 1) as f64;
            let i = (x as usize).min(p.len() - 2);
            p[i] + (x - i as f64) * (p[i + 1] - p[i])
        };
        js_max(0.0, a * self.profile_open) * self.body.scale * self.body.scale
    }

    /// Blend the drawn profile over the articulated areas (and their minima, which set the jet
    /// losses and where the noise sources sit), and the lips' area the frame reports.
    fn blend_profile(&mut self, f: &mut HumanFrame, ph_len: f64, or_len: f64) {
        let m = self.profile_mix.clamp(0.0, 1.0);
        let total = ph_len + or_len;
        for i in 0..self.n_ph {
            let d = self.profile_at((i as f64 + 0.5) * ph_len / self.n_ph as f64 / total);
            self.ph_areas[i] += m * (d - self.ph_areas[i]);
            self.ph_min[i] = js_min(self.ph_areas[i], self.ph_min[i] + m * (d - self.ph_min[i]));
        }
        for i in 0..self.n_or {
            let d = self.profile_at((ph_len + (i as f64 + 0.5) * or_len / self.n_or as f64) / total);
            self.or_areas[i] += m * (d - self.or_areas[i]);
            self.or_min[i] = js_min(self.or_areas[i], self.or_min[i] + m * (d - self.or_min[i]));
        }
        f.lip_area += m * (self.profile_at(1.0) - f.lip_area);
    }

    /// Register turbulence sources: glottis, narrowest supraglottal constriction, teeth obstacle.
    fn update_noise(&mut self, f: &HumanFrame) {
        let g = self.noise_gain;
        let mut list = core::mem::take(&mut self.noise_list);
        list.clear();
        // aspiration: jet from the glottis hits the epilarynx/ventricular region
        list.push(NoiseSpec::new(self.glottis_port, self.branch_of(self.pharynx, 1), self.branch_of(self.trachea, self.trachea.n as isize - 2), g.glottis).band(g.glottis_lowpass, g.glottis_highpass));
        // narrowest open point in pharynx+oral (global position index p over concatenated tubes)
        let (n_ph, n_or) = (self.n_ph, self.n_or);
        let mut min_a = f64::INFINITY;
        let mut min_i: isize = -1;
        for i in 0..n_ph + n_or {
            let area = if i < n_ph { self.ph_min[i] } else { self.or_min[i - n_ph] };
            if area > 0.0 && area < min_a {
                min_a = area;
                min_i = i as isize;
            }
        }
        // narrowest open point: its jet makes frication noise. Every weight below is a smooth function
        // of geometry so sources fade in/out instead of toggling (toggling = audible noise bursts).
        let k2 = self.body.scale * self.body.scale;
        let w_area = if min_i >= 0 { 1.0 - smooth(0.35 * k2, 0.9 * k2, min_a) } else { 0.0 };
        if w_area > 0.0 {
            let n_ph_i = n_ph as isize;
            let n_tot = (n_ph + n_or) as isize;
            let dx_ph = self.ph_len / n_ph as f64;
            let dx_or = self.or_len / n_or as f64;
            let to_branch = |gi: isize| -> isize {
                if gi < n_ph_i {
                    self.branch_of(self.pharynx, gi) as isize
                } else if gi == n_ph_i {
                    -1
                } else {
                    self.branch_of(self.oral, gi - n_ph_i) as isize
                }
            };
            let dx = if min_i < n_ph_i { dx_ph } else { dx_or };
            let off = js_max(1.0, js_round((0.9 * self.body.scale) / dx)) as isize;
            let src = to_branch(min_i.min(n_tot - 2));
            let fwd = to_branch((n_tot - 2).min(min_i + off));
            let bwd = to_branch(0.max(min_i - off));
            if src >= 0 && fwd >= 0 && bwd >= 0 {
                list.push(NoiseSpec::new(src as usize, fwd as usize, bwd as usize, g.constriction * w_area).band(2600.0, 350.0));
                // obstacle source: a narrow anterior jet impinging on the incisors (sibilants)
                let s_min = if min_i < n_ph_i { (min_i as f64 + 0.5) * dx_ph } else { self.ph_len + ((min_i - n_ph_i) as f64 + 0.5) * dx_or };
                let behind = (f.teeth_s - s_min) / self.body.scale;
                let w_dist = smooth(0.15, 0.5, behind) * (1.0 - smooth(2.6, 4.0, behind));
                let w_jet = 1.0 - smooth(0.2 * k2, 0.5 * k2, min_a);
                let wt = w_dist * w_jet;
                if wt > 1e-3 {
                    let teeth_i = n_ph_i + js_min((n_or - 2) as f64, js_max(0.0, js_round((f.teeth_s - self.ph_len) / dx_or))) as isize;
                    let tb = to_branch(teeth_i);
                    if tb >= 0 {
                        list.push(NoiseSpec::new(src as usize, tb as usize, bwd as usize, g.teeth * wt).band(5000.0, 1200.0));
                    }
                }
            }
        }
        self.net.set_noise_sources(&list);
        self.noise_list = list;
    }
}
