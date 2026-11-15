use super::*;

mod alloc_count {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        pub static COUNT: Cell<u64> = const { Cell::new(0) };
    }

    struct Counting;
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
            unsafe { System.alloc(l) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            let _ = COUNT.try_with(|c| c.set(c.get() + 1));
            unsafe { System.realloc(p, l, n) }
        }
    }

    #[global_allocator]
    static A: Counting = Counting;

    pub fn count() -> u64 {
        COUNT.with(|c| c.get())
    }
}

#[test]
fn processing_does_not_allocate() {
    let dict = "WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n";
    let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 3.0 + 2.0 * (i as f64 * 0.4).sin()).collect();
    let combos = [TextStep::Note, TextStep::Clock].into_iter().flat_map(|st| [TextUnit::Syllable, TextUnit::Word].into_iter().flat_map(move |u| [(TextSay::Hold, 1.0), (TextSay::Hold, 0.5), (TextSay::Shot, 1.0)].into_iter().map(move |(y, l)| (st, u, y, l))));
    for (step, unit, say, len) in combos {
        let mode = (step, unit, say, len);
        let mut s = Suu::new(48000.0);
        s.load_dict(dict);
        s.set_params(Params { text_step: step, text_unit: unit, text_say: say, text_len: len, ..Params::default() });
        s.set_text("wow yeah now wow");
        s.set_profile(&wave, true);
        s.set_params(Params { gate: true, morph: 0.6, reson: 0.5, folds: 0.5, glide_s: 0.1, pitch_from: PitchFrom::Input, vowel_in: true, sustain: 0.6, ..s.params() });
        for t in 0..N_MOD {
            s.set_mod(t, Mod { depth: 0.4, rate_hz: 1.0 + t as f64, sync: 0.0, shape: Shape::Random, smooth: 0.5 });
        }
        let input = vec![0.1f32; 256];
        let mut out = vec![0f32; 256];
        for _ in 0..200 {
            s.process(&input, &mut out);
        }
        let c = alloc_count::count();
        std::hint::black_box(vec![0u8; 64]);
        assert!(alloc_count::count() > c);
        let before = alloc_count::count();
        s.set_playing(true);
        for i in 0..2000 {
            s.process(&input[..(i % 7) * 32 + 32], &mut out[..(i % 7) * 32 + 32]);
            if i == 700 {
                s.set_playing(false);
                s.set_params(Params { gate: false, folds: 0.0, ..s.params() });
            }
            if i == 1500 {
                s.retrigger();
                s.set_params(Params { gate: true, note: 50.0, ..s.params() });
            }
        }
        let mut snap = [0.0; HORN_POINTS];
        s.snapshot(&mut snap);
        s.set_profile(&wave, false);
        assert_eq!(alloc_count::count() - before, 0, "{mode:?}: process allocated");
    }
}

fn run(s: &mut Suu, block: usize, n: usize) -> Vec<f32> {
    let input = vec![0f32; n];
    let mut out = vec![0f32; n];
    for (o, i) in out.chunks_mut(block).zip(input.chunks(block)) {
        s.process(i, o);
    }
    out
}

fn playing() -> Suu {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { gate: true, ..Params::default() });
    s
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0f32, |m, v| if v.is_finite() { m.max(v.abs()) } else { f32::INFINITY })
}

#[test]
fn block_size_does_not_change_the_output() {
    let a = run(&mut playing(), 128, 24000);
    let b = run(&mut playing(), 37, 24000);
    assert_eq!(a, b);
}

#[test]
fn sequencer_is_block_size_independent_too() {
    let mk = || {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { bpm: 137.0, ..Params::default() });
        s.set_playing(true);
        s
    };
    assert_eq!(run(&mut mk(), 128, 48000), run(&mut mk(), 53, 48000));
}

#[test]
fn supersaw_through_the_mouth_is_audible_and_bounded() {
    let out = run(&mut playing(), 128, 48000);
    let rms = (out[24000..].iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / 24000.0).sqrt();
    assert!(peak(&out) < 1.0, "peak {}", peak(&out));
    assert!(rms > 0.005, "rms {rms}");
}

#[test]
fn a_size_change_neither_drops_out_nor_clicks() {
    let mut s = playing();
    let a = run(&mut s, 128, 24000);
    let level = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    let before = level(&a[19200..]);
    s.set_size(1.4);
    let b = run(&mut s, 128, 4800);
    assert!(level(&b) > 0.4 * before, "after {} before {before}", level(&b));
    let jump = |x: &[f32]| x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()));
    assert!(jump(&b) < 2.0 * jump(&a[19200..]), "jump {} vs {}", jump(&b), jump(&a[19200..]));
}

#[test]
fn extremes_stay_finite_and_bounded() {
    for &size in &[0.4, 0.55, 0.7, 1.0, 2.0, 4.0] {
        for &(x, y, round, nasal, damping, folds, breath) in &[(-0.3, -0.3, -1.0, 1.5, 0.1, 2.0, 6.0), (1.3, 1.3, 1.0, 0.0, 20.0, 0.0, 0.0), (0.5, 0.5, 0.0, 0.7, 1.0, 1.0, 3.0)] {
            let mut s = Suu::new(48000.0);
            s.set_size(size);
            s.set_params(Params { gate: true, vowel_x: x, vowel_y: y, round, nasal, damping, folds, breath, drive_db: 40.0, reson: x.clamp(0.0, 1.0), ..Params::default() });
            let out = run(&mut s, 128, 24000);
            let pk = peak(&out);
            assert!(pk.is_finite() && pk < 1.0, "size {size} x {x} y {y}: peak {pk}");
        }
    }
}

fn text_params(step: TextStep, unit: TextUnit, say: TextSay) -> Params {
    Params { text_step: step, text_unit: unit, text_say: say, ..Params::default() }
}

#[test]
fn notes_say_the_text_in_every_unit_and_way() {
    let dict = "WOW  W AW1\nYEAH  Y AE1\n";
    for unit in [TextUnit::Syllable, TextUnit::Word] {
        for say in [TextSay::Hold, TextSay::Shot] {
            let mut s = Suu::new(48000.0);
            s.load_dict(dict);
            s.set_params(text_params(TextStep::Note, unit, say));
            assert_eq!(s.set_text("wow yeah"), 2);
            s.set_playing(true);
            let out = run(&mut s, 128, 48000);
            assert!(peak(&out).is_finite() && peak(&out) < 1.0);
            assert!(s.said >= 2, "{unit:?} {say:?}: said {}", s.said);
        }
    }
}

#[test]
fn on_notes_seq_starts_the_text_over_each_pass() {
    let first_units = |seq: bool| {
        let mut s = Suu::new(48000.0);
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
        s.set_params(Params { text_seq: seq, bpm: 120.0, ..Params::default() });
        s.set_text("wow yeah now");
        for i in 0..STEPS {
            s.set_step(i, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.5 });
        }
        s.set_seq(5, 4.0, 1.0);
        s.set_playing(true);
        let mut firsts = Vec::new();
        let mut said = 0;
        for _ in 0..(48000 * 3 / 32) {
            run(&mut s, 32, 32);
            if s.said != said {
                said = s.said;
                if s.seq.step == 0 {
                    firsts.push(s.last_unit);
                }
            }
        }
        firsts
    };
    let (free, seq) = (first_units(false), first_units(true));
    assert!(seq.len() >= 3 && seq.iter().all(|&u| u == 0), "seq {seq:?}");
    assert!(free.iter().any(|&u| u != 0), "free {free:?}");
}

#[test]
fn on_the_clock_a_short_length_closes_each_unit_early() {
    let held_until = |len: f64| {
        let mut s = Suu::new(48000.0);
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\n");
        s.set_params(Params { text_len: len, ..text_params(TextStep::Clock, TextUnit::Syllable, TextSay::Hold) });
        s.set_text("wow yeah");
        s.plan.tick_plan[1].iter().map(|g| g.t1).fold(0.0, f64::max) / s.plan.tick_s
    };
    let (half, full) = (held_until(0.5), held_until(0.9));
    assert!(half < 0.65 && full > 0.9, "len 0.5 holds to {half:.2}, 0.9 to {full:.2} of a tick");
}

#[test]
fn the_sequencer_gates_an_external_sound() {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { source: Source::Off, bpm: 120.0, ..Params::default() });
    for i in 0..STEPS {
        s.set_step(i, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.5 });
    }
    s.set_playing(true);
    let input: Vec<f32> = (0..48000).map(|i| (0.3 * (2.0 * std::f64::consts::PI * 110.0 * i as f64 / 48000.0).sin()) as f32).collect();
    let mut out = vec![0f32; 48000];
    for (o, x) in out.chunks_mut(128).zip(input.chunks(128)) {
        s.process(x, o);
    }
    let rms = |a: &[f32]| (a.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / a.len() as f64).sqrt();
    let win: Vec<f64> = out[12000..].chunks(1500).map(rms).collect();
    let (lo, hi) = win.iter().fold((f64::MAX, 0f64), |(l, h), v| (l.min(*v), h.max(*v)));
    assert!(hi > 4.0 * lo, "gated: max {hi} min {lo}");
}

#[test]
fn keys_gate_an_external_sound() {
    let tone: Vec<f32> = (0..24000).map(|i| (0.3 * (2.0 * std::f64::consts::PI * 110.0 * i as f64 / 48000.0).sin()) as f32).collect();
    let level = |gate: bool| {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { source: Source::Off, gate, ..Params::default() });
        let mut out = vec![0f32; 24000];
        for (o, x) in out.chunks_mut(128).zip(tone.chunks(128)) {
            s.process(x, o);
        }
        (out[12000..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 12000.0).sqrt()
    };
    let (closed, open) = (level(false), level(true));
    assert!(open > 0.005 && closed < 1e-4 * open, "closed {closed} open {open}");
}

#[test]
fn the_drawn_horn_morphs_the_airway_and_moves_smoothly() {
    let areas = |s: &Suu| {
        let mut ui = Vec::new();
        s.ui_state(&mut ui);
        ui[23..23 + UI_AREAS].to_vec()
    };
    let mut s = playing();
    run(&mut s, 128, 9600);
    let vowel = areas(&s);
    s.set_profile(&[5.0; HORN_POINTS], false);
    s.set_params(Params { gate: true, morph: 1.0, ..Params::default() });
    let out = run(&mut s, 128, 24000);
    let horn = areas(&s);
    assert!(horn.iter().all(|a| (a - 5.0).abs() < 0.05), "{horn:?}");
    assert!(vowel.iter().zip(&horn).any(|(a, b)| (a - b).abs() > 1.0));
    s.set_params(Params { gate: true, morph: 1.0, open: 2.0, ..Params::default() });
    let out2 = run(&mut s, 128, 24000);
    assert!(areas(&s).iter().all(|a| (a - 10.0).abs() < 0.1));
    let jump = |x: &[f32]| x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()));
    assert!(peak(&out2) < 1.0 && jump(&out2) < 3.0 * jump(&out[12000..]), "jump {} vs {}", jump(&out2), jump(&out[12000..]));
    let mk = || {
        let mut s = playing();
        s.set_params(Params { gate: true, morph: 0.7, ..Params::default() });
        s.set_mod(7, Mod { depth: 0.8, rate_hz: 2.0, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
        s
    };
    assert_eq!(run(&mut mk(), 128, 24000), run(&mut mk(), 45, 24000));
}

#[test]
fn a_wave_horn_wraps_so_it_can_travel() {
    let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 3.0 + 2.0 * (2.0 * std::f64::consts::PI * i as f64 / HORN_POINTS as f64).sin()).collect();
    let areas = |shift: f64| {
        let mut s = playing();
        s.set_profile(&wave, true);
        s.set_params(Params { gate: true, morph: 1.0, shift, ..Params::default() });
        run(&mut s, 128, 24000);
        let mut ui = Vec::new();
        s.ui_state(&mut ui);
        ui[23..23 + UI_AREAS].to_vec()
    };
    let (a, b, c) = (areas(0.0), areas(0.5), areas(-0.5));
    for k in 0..UI_AREAS {
        assert!((a[k] - 3.0 + (b[k] - 3.0)).abs() < 0.15, "half a period inverts: {k} {} {}", a[k], b[k]);
        assert!((b[k] - c[k]).abs() < 1e-4, "no seam: {k}");
    }
    let mk = |depth: f64| {
        let mut s = playing();
        s.set_profile(&wave, true);
        s.set_params(Params { gate: true, morph: 1.0, ..Params::default() });
        s.set_mod(7, Mod { depth, rate_hz: 1.0, sync: 0.0, shape: Shape::Saw, smooth: 0.0 });
        run(&mut s, 128, 96000)[24000..].to_vec()
    };
    let jump = |x: &[f32]| x.windows(2).fold(0f32, |m, w| m.max((w[1] - w[0]).abs()));
    let (still, moving) = (mk(0.0), mk(1.0));
    assert!(jump(&moving) < 1.5 * jump(&still), "jump moving {} still {}", jump(&moving), jump(&still));
}

#[test]
fn it_sleeps_when_silent_and_wakes_on_a_note() {
    let run_gated = |block: usize| {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, ..Params::default() });
        let mut out = run(&mut s, block, 14400);
        s.set_params(Params { gate: false, ..Params::default() });
        out.extend(run(&mut s, block, 48000));
        let slept = s.asleep;
        s.set_params(Params { gate: true, ..Params::default() });
        let again = run(&mut s, block, 9600);
        out.extend(&again);
        (out, slept, again)
    };
    let (a, slept, again) = run_gated(128);
    assert!(slept, "silent for a second: asleep");
    let rms = (again[4800..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 4800.0).sqrt();
    assert!(rms > 0.005, "a note wakes it: rms {rms}");
    assert_eq!(a, run_gated(37).0);
}

#[test]
fn smooth_takes_the_jumps_out_of_random_square_and_saw() {
    let largest_step = |shape: Shape, smooth: f64| {
        let mut s = playing();
        s.set_mod(4, Mod { depth: 1.0, rate_hz: 4.0, sync: 0.0, shape, smooth });
        let (mut last, mut most) = (f64::NAN, 0f64);
        for _ in 0..3000 {
            run(&mut s, 32, 32);
            if last.is_finite() {
                most = most.max((s.eff[4] - last).abs());
            }
            last = s.eff[4];
        }
        most
    };
    for shape in [Shape::Random, Shape::Square, Shape::Saw] {
        let (sharp, smooth) = (largest_step(shape, 0.0), largest_step(shape, 1.0));
        assert!(smooth < 0.05 * sharp.max(0.2), "{shape:?}: sharp {sharp:.3}, smooth {smooth:.4}");
    }
    let mut s = playing();
    s.set_mod(0, Mod { depth: 1.0, rate_hz: 2.0, sync: 0.0, shape: Shape::Random, smooth: 1.0 });
    run(&mut s, 32, 48000);
    assert!((s.mod_out[0] - s.mod_hold[0]).abs() <= (s.mod_hold[0] - s.mod_from[0]).abs() + 1e-9);
}

#[test]
fn the_folds_sound_only_with_a_note() {
    let level = |gate: bool, block: usize| {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { folds: 0.68, breath: 2.79, follow: 0.0, gate, ..Params::default() });
        let out = run(&mut s, block, 96000);
        let rms = (out[48000..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 48000.0).sqrt();
        (20.0 * rms.max(1e-12).log10(), out)
    };
    let ((off, _), (on, a)) = (level(false, 128), level(true, 128));
    assert!(off < -50.0 && on > -30.0, "no note {off:.1} dB, a note {on:.1} dB");
    assert_eq!(a, level(true, 37).1);
}

#[test]
fn once_says_the_text_one_time_per_pass_of_the_sequence() {
    let passes = |step: TextStep, say: TextSay, len: f64, once: bool| {
        let mut s = Suu::new(48000.0);
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
        s.set_params(Params { bpm: 120.0, text_sync: 0.25, text_once: once, text_say: say, text_len: len, ..text_params(step, TextUnit::Syllable, TextSay::Hold) });
        s.set_text("wow yeah now");
        s.set_playing(true);
        let (mut n, mut last, said0) = (0, s.next_pass, s.said);
        for _ in 0..(48000 * 6 / 32) {
            run(&mut s, 32, 32);
            if s.next_pass != last {
                n += 1;
                last = s.next_pass;
            }
        }
        if step == TextStep::Clock && say == TextSay::Hold && len == 1.0 { n } else { (s.said - said0) as usize / 3 }
    };
    for (step, say, len) in [(TextStep::Clock, TextSay::Hold, 1.0), (TextStep::Clock, TextSay::Shot, 1.0), (TextStep::Note, TextSay::Hold, 1.0)] {
        let (looping, once) = (passes(step, say, len, false), passes(step, say, len, true));
        assert!(looping >= 9 && (3..=4).contains(&once), "{step:?} {say:?}: looping {looping}, once {once}");
    }
}

#[test]
fn speech_speed_stretches_a_word_and_keeps_the_loop_on_its_beats() {
    let plans = |speech: f64| {
        let mut s = Suu::new(48000.0);
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\nSTOP  S T AA1 P\n");
        s.set_params(Params { speech, ..text_params(TextStep::Note, TextUnit::Word, TextSay::Shot) });
        s.set_text("stop wow");
        let span = |gs: &[Gesture]| gs.iter().map(|g| g.t1 + g.ramp_out).fold(0.0, f64::max) - gs.iter().map(|g| g.t0 - g.ramp_in).fold(f64::MAX, f64::min);
        (span(&s.plan.shot[0]), s.plan.pass_len)
    };
    let ((slow, len_slow), (normal, len), (fast, len_fast)) = (plans(0.5), plans(1.0), plans(2.0));
    assert!((slow / normal - 2.0).abs() < 0.05 && (normal / fast - 2.0).abs() < 0.05, "word: {slow:.3} {normal:.3} {fast:.3}");
    assert!(len_slow == len && len_fast == len);
}

fn level_db(p: Params, input: f32) -> f64 {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { gate: true, ..p });
    let inp: Vec<f32> = (0..96000).map(|i| input * (2.0 * ((i as f64 * 110.0 / 48000.0) % 1.0) - 1.0) as f32).collect();
    let mut out = vec![0f32; 96000];
    for (o, i) in out.chunks_mut(128).zip(inp.chunks(128)) {
        s.process(i, o);
    }
    20.0 * (out[48000..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 48000.0).sqrt().max(1e-12).log10()
}

#[test]
fn the_sources_mix_at_matched_levels() {
    let osc = level_db(Params { folds: 0.0, ..Params::default() }, 0.0);
    let folds = level_db(Params { source: Source::Off, folds: 1.0, ..Params::default() }, 0.0);
    let input = level_db(Params { source: Source::Off, folds: 0.0, ..Params::default() }, 0.5);
    let both = level_db(Params { folds: 1.0, ..Params::default() }, 0.0);
    assert!((osc - folds).abs() < 3.0, "osc {osc:.1}, folds {folds:.1}");
    assert!(both - osc.max(folds) < 4.5, "both {both:.1}");
    assert!((input - osc).abs() < 6.0, "input {input:.1}, osc {osc:.1}");
    assert!(level_db(Params { osc_db: -60.0, folds: 0.0, ..Params::default() }, 0.0) < -100.0);
    let folds_quiet_osc = level_db(Params { folds: 1.0, osc_db: -60.0, ..Params::default() }, 0.0);
    let folds_with_osc_low = level_db(Params { folds: 1.0, osc_db: -30.0, ..Params::default() }, 0.0);
    assert!((folds_quiet_osc - folds_with_osc_low).abs() < 1.5, "{folds_quiet_osc:.1} vs {folds_with_osc_low:.1}");
}

#[test]
fn the_text_gate_lets_through_part_of_each_unit() {
    let open_share = |step: TextStep, len: f64, gate: f64| {
        let mut s = Suu::new(48000.0);
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
        s.set_params(Params { gate: true, bpm: 120.0, text_sync: 0.5, text_len: len, text_gate: gate, ..text_params(step, TextUnit::Syllable, TextSay::Hold) });
        s.set_text("wow yeah now");
        if step == TextStep::Note {
            for i in 0..STEPS {
                s.set_step(i, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 1.0 });
            }
            s.set_seq(16, 2.0, 1.0);
            s.set_playing(true);
        }
        run(&mut s, 32, 24000);
        let (mut open, mut all) = (0, 0);
        for _ in 0..(48000 * 2 / 32) {
            run(&mut s, 32, 32);
            all += 1;
            open += s.gate_eff as u32;
        }
        open as f64 / all as f64
    };
    for (step, len) in [(TextStep::Clock, 0.5), (TextStep::Clock, 1.0)] {
        let (full, half) = (open_share(step, len, 1.0), open_share(step, len, 0.5));
        assert!(full > 0.99 && (half - 0.5).abs() < 0.08, "{step:?} len {len}: gate 1 {full:.2}, gate 0.5 {half:.2}");
    }
    let (full, cut) = (open_share(TextStep::Note, 1.0, 1.0), open_share(TextStep::Note, 1.0, 0.3));
    assert!(full > 0.95 && cut < 0.6 * full, "notes: {full:.2} vs {cut:.2}");
}

#[test]
fn a_gate_of_zero_is_silent() {
    let mut s = Suu::new(48000.0);
    s.set_seq(16, 4.0, 0.0);
    s.set_playing(true);
    let gated = run(&mut s, 128, 48000);
    let none = run(&mut Suu::new(48000.0), 128, 48000);
    assert_eq!(gated, none);
    assert!(peak(&none) < 1e-4, "a new instance is silent: peak {}", peak(&none));
}

#[test]
fn the_text_loop_starts_over_with_the_sequence() {
    let dict = "WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n";
    let restarts = |follow: bool| {
        let mut s = Suu::new(48000.0);
        s.load_dict(dict);
        s.set_params(Params { bpm: 120.0, text_sync: 0.5, text_seq: follow, ..text_params(TextStep::Clock, TextUnit::Syllable, TextSay::Hold) });
        s.set_text("wow yeah now");
        s.set_seq(5, 4.0, 1.0);
        s.set_playing(true);
        let mut times = Vec::new();
        for _ in 0..(48000 * 4 / 32) {
            run(&mut s, 32, 32);
            if times.last() != Some(&s.last_restart) && s.last_restart > 0.0 {
                times.push(s.last_restart);
            }
        }
        times
    };
    let t = restarts(true);
    assert!(t.len() >= 5, "{t:?}");
    for w in t.windows(2) {
        assert!((w[1] - w[0] - 0.625).abs() < 2e-3, "pattern apart: {t:?}");
    }
    assert!(restarts(false).is_empty());
}

#[test]
fn a_snapshot_is_one_period_of_the_source() {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { source: Source::Saw, note: 45.0, ..Params::default() });
    run(&mut s, 128, 9600);
    let mut snap = [0.0; HORN_POINTS];
    assert!(s.snapshot(&mut snap));
    let rising = snap.windows(2).filter(|w| w[1] > w[0]).count();
    assert!(rising >= HORN_POINTS - 3, "a ramp: {snap:?}");
    assert!(snap.iter().all(|v| v.abs() <= 1.0 + 1e-9) && snap.iter().any(|v| v.abs() > 0.99));
    let mut q = Suu::new(48000.0);
    q.set_params(Params { source: Source::Off, ..Params::default() });
    run(&mut q, 128, 4800);
    assert!(!q.snapshot(&mut snap));
}

fn autocorr(x: &[f32], lag: usize) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for i in lag..x.len() {
        num += x[i] as f64 * x[i - lag] as f64;
        den += (x[i] as f64).powi(2);
    }
    num / den
}

#[test]
fn the_resonator_gives_an_outside_sound_the_played_pitch() {
    let mut r = 0x1234_5678u32;
    let noise: Vec<f32> = (0..48000)
        .map(|_| {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            (r as f64 / u32::MAX as f64 - 0.5) as f32 * 0.4
        })
        .collect();
    let render = |reson: f64| {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { source: Source::Off, gate: true, note: 57.0, reson, ..Params::default() });
        let mut out = vec![0f32; 48000];
        for (o, x) in out.chunks_mut(128).zip(noise.chunks(128)) {
            s.process(x, o);
        }
        out[24000..].to_vec()
    };
    let lag = (48000.0f64 / 220.0).round() as usize;
    let (off, on) = (render(0.0), render(0.8));
    let (a_off, a_on) = (autocorr(&off, lag), autocorr(&on, lag));
    assert!(a_on > 0.6 && a_on > a_off + 0.3, "periodicity at 220 Hz: off {a_off:.2}, on {a_on:.2}");
    assert!(peak(&on) < 1.0 && peak(&on) > 0.01, "peak {}", peak(&on));
    let rms = |x: &[f32]| (x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    println!("rms off {:.4} on {:.4}, autocorr off {a_off:.2} on {a_on:.2}", rms(&off), rms(&on));
}

fn vowel_settle(glide_s: f64) -> (usize, f32) {
    let mut s = Suu::new(48000.0);
    let p = Params { gate: true, glide_s, vowel_x: 0.0, vowel_y: 0.0, ..Params::default() };
    s.set_params(p);
    run(&mut s, 128, 9600);
    let from = s.voice.artic.x.tb_y;
    s.set_params(Params { vowel_x: 1.0, vowel_y: 1.0, ..p });
    let to = s.voice.artic.target.tb_y;
    let (mut n, mut jump, mut last) = (0, 0f32, 0f32);
    while (s.voice.artic.x.tb_y - to).abs() > 0.05 * (to - from).abs() && n < 48000 {
        for v in run(&mut s, 32, 32) {
            jump = jump.max((v - last).abs());
            last = v;
        }
        n += 32;
    }
    (n, jump)
}

#[test]
fn glide_can_go_one_way_only() {
    let base = Params { note: 45.0, glide_s: 0.25, ..Params::default() };
    let up = |dir: GlideDir, to: f64| glide_samples(Params { glide_dir: dir, ..base }, to);
    assert!((up(GlideDir::Up, 57.0) as i64 - 12000).abs() <= 64 && up(GlideDir::Up, 33.0) <= 64);
    assert!((up(GlideDir::Down, 33.0) as i64 - 12000).abs() <= 64 && up(GlideDir::Down, 57.0) <= 64);
}

#[test]
fn glide_at_zero_moves_the_vowel_fast_and_without_a_click() {
    let (fast, j_fast) = vowel_settle(0.0);
    let (normal, j_normal) = vowel_settle(0.03);
    let (slow, _) = vowel_settle(0.12);
    println!("vowel settles: glide 0 {fast}, 30 ms {normal}, 120 ms {slow} samples; jumps {j_fast} {j_normal}");
    assert!(fast * 3 < normal && normal * 3 < slow, "{fast} {normal} {slow}");
    assert!(j_fast < 1.5 * j_normal, "jump fast {j_fast} normal {j_normal}");
}

fn glide_samples(p: Params, to: f64) -> usize {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { gate: true, ..p });
    run(&mut s, 128, 4800);
    s.set_params(Params { gate: true, note: to, ..p });
    let mut n = 0;
    while s.pitch != to && n < 480000 {
        run(&mut s, 32, 32);
        n += 32;
    }
    n
}

#[test]
fn glide_takes_its_time_free_or_synced() {
    let base = Params { note: 45.0, ..Params::default() };
    for to in [57.0, 33.0, 46.0] {
        let n = glide_samples(Params { glide_s: 0.25, ..base }, to);
        assert!((n as i64 - 12000).abs() <= 64, "to {to}: {n} samples");
    }
    let n = glide_samples(Params { glide_s: 0.25, glide_sync: 1.0, bpm: 120.0, ..base }, 57.0);
    assert!((n as i64 - 24000).abs() <= 64, "synced: {n} samples");
    assert!(glide_samples(Params { glide_s: 0.0, ..base }, 57.0) <= 64);
}

#[test]
fn the_global_gate_scales_every_step_alike() {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { bpm: 120.0, ..Params::default() });
    s.set_step(0, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.4 });
    s.set_step(1, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.8 });
    s.set_seq(2, 4.0, 0.5);
    let mut open = [0usize; 2];
    s.set_playing(true);
    for _ in 0..(12000 / 32) {
        run(&mut s, 32, 32);
        if s.gate_now {
            open[s.seq.step] += 32;
        }
    }
    assert!((open[0] as i64 - 1200).abs() <= 64 && (open[1] as i64 - 2400).abs() <= 64, "{open:?}");
    s.set_seq(2, 4.0, 2.0);
    s.seq.step = 0;
    assert!((s.gate_len() - 0.8).abs() < 1e-12);
    s.seq.step = 1;
    assert_eq!(s.gate_len(), 1.0);
}

#[test]
fn the_text_loop_runs_at_its_own_rate() {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { bpm: 120.0, text_sync: 0.5, ..Params::default() });
    assert_eq!(s.loop_beats(), 0.5);
    s.set_params(Params { bpm: 120.0, text_sync: 0.0, text_rate_hz: 4.0, ..Params::default() });
    assert_eq!(s.loop_beats(), 0.5);
    s.set_params(Params { bpm: 300.0, text_sync: 0.125, ..Params::default() });
    assert!((s.loop_beats() - 0.3).abs() < 1e-12);
    s.load_dict("WOW  W AW1\nYEAH  Y AE1\n");
    s.set_params(Params { gate: true, bpm: 120.0, text_sync: 0.5, ..Params::default() });
    s.set_params(Params { text_step: TextStep::Clock, ..s.params() });
    assert_eq!(s.set_text("wow yeah"), 2);
    let out = run(&mut s, 128, 48000);
    assert!(peak(&out).is_finite() && peak(&out) < 1.0 && peak(&out) > 0.01);
}

#[test]
fn motion_moves_the_vowel_and_stays_block_size_independent() {
    let mk = || {
        let mut s = playing();
        s.set_mod(1, Mod { depth: 0.8, rate_hz: 1.0, sync: 1.0, shape: Shape::Sine, smooth: 0.0 });
        s.set_mod(5, Mod { depth: 0.5, rate_hz: 3.0, sync: 0.0, shape: Shape::Random, smooth: 0.0 });
        s
    };
    let mut a = mk();
    let out_a = run(&mut a, 128, 48000);
    assert_eq!(out_a, run(&mut mk(), 61, 48000));
    let mut s = mk();
    let mut ys = Vec::new();
    for _ in 0..40 {
        run(&mut s, 128, 1200);
        let mut ui = Vec::new();
        s.ui_state(&mut ui);
        ys.push(ui[1]);
    }
    let (lo, hi) = ys.iter().fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    assert!(hi - lo > 0.8, "vowel Y swing {lo}..{hi}");
}

#[test]
fn steps_have_their_own_length_and_accent() {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { bpm: 120.0, ..Params::default() });
    s.set_seq(2, 4.0, 1.0);
    s.set_step(0, Step { on: true, note: 0, tie: false, vowel: -1, accent: false, len: 0.2 });
    s.set_step(1, Step { on: true, note: 0, tie: false, vowel: -1, accent: true, len: 0.9 });
    s.set_playing(true);
    let out = run(&mut s, 128, 48000);
    let rms = |a: &[f32]| (a.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / a.len() as f64).sqrt();
    let (a, b) = (rms(&out[24000 + 2400..24000 + 3600]), rms(&out[30000 + 2400..30000 + 3600]));
    assert!(b > 3.0 * a, "short step {a}, long accented step {b}");
}

#[test]
fn swapping_mouth_and_text_does_not_allocate() {
    let dict = "WOW\tW AW1\nYEAH\tY AE1\nNOW\tN AW1\n";
    let mut s = Suu::new(48000.0);
    s.load_dict(dict);
    s.set_params(Params { gate: true, ..Params::default() });
    let mut out = vec![0f32; 128];
    let inp = vec![0f32; 128];
    for _ in 0..100 {
        s.process(&inp, &mut out);
    }
    let plan = TextPlan::new("wow yeah now", s.g2p(), &s.params());
    assert_eq!(plan.syllables(), 3);
    let seed = s.mouth_seed(1.6);
    let mouth = Mouth::build(&seed);
    assert_eq!(mouth.scale(), 1.6);
    let before = alloc_count::count();
    let old = s.set_text_plan(plan);
    assert!(s.set_mouth(mouth).is_ok());
    let mut peak_step = 0f32;
    let mut last = out[127];
    let mut retired = None;
    for _ in 0..100 {
        s.process(&inp, &mut out);
        for &y in out.iter() {
            peak_step = peak_step.max((y - last).abs());
            last = y;
        }
        if retired.is_none() {
            retired = s.take_retired();
        }
    }
    assert_eq!(alloc_count::count(), before, "the swap or processing allocated");
    assert_eq!(old.syllables(), 0);
    assert!(retired.is_some(), "the old airway was not handed back");
    assert_eq!(s.size(), 1.6);
    assert!(peak_step < 0.2, "a jump of {peak_step} at the swap");
    let m2 = Mouth::build(&s.mouth_seed(0.8));
    assert!(s.set_mouth(m2).is_ok());
    let m3 = Mouth::build(&s.mouth_seed(1.0));
    assert!(s.set_mouth(m3).is_err());
}

#[test]
fn text_plan_goes_stale() {
    let mut s = Suu::new(48000.0);
    s.load_dict("WOW\tW AW1\n");
    assert!(!s.text_stale(), "no text, nothing to replan");
    s.set_text("wow wow");
    assert!(!s.text_stale());
    s.set_params(Params { vowel_x: 0.2, mix: 0.5, ..s.params() });
    assert!(!s.text_stale(), "the pad and the mix do not touch the plan");
    s.set_params(Params { text_rate_hz: 5.0, ..s.params() });
    assert!(!s.text_stale(), "the free rate does not count while synced");
    for change in [Params { bpm: 120.0, ..s.params() }, Params { speech: 2.0, ..s.params() }, Params { text_unit: TextUnit::Word, ..s.params() }, Params { text_len: 0.5, ..s.params() }, Params { text_sync: 0.0, ..s.params() }] {
        let keep = s.params();
        s.set_params(change);
        assert!(s.text_stale());
        s.set_params(keep);
        assert!(!s.text_stale());
    }
}

#[test]
fn env_gate_opens_on_the_input() {
    let fs = 48000.0;
    let mut s = Suu::new(fs);
    s.load_dict("WOW\tW AW1\nYEAH\tY AE1\n");
    s.set_params(Params { source: Source::Off, env_gate: true, env_thresh_db: -30.0, env_release_s: 0.1, ..Params::default() });
    s.set_text("wow yeah wow yeah");
    let burst = |t: usize| if (t % 24000) < 9600 { 0.25 * (t as f32 * 0.05).sin() } else { 0.0 };
    let mut open = Vec::new();
    let mut out = [0f32; 128];
    let mut inp = [0f32; 128];
    for b in 0..(3 * 24000 / 128) {
        for (i, x) in inp.iter_mut().enumerate() {
            *x = burst(b * 128 + i);
        }
        s.process(&inp, &mut out);
        open.push(s.env_open());
    }
    let at = |sec: f64| open[(sec * fs / 128.0) as usize];
    assert!(!at(0.0) || at(0.01), "opens at once");
    assert!(at(0.1) && at(0.25), "open through the burst and the release");
    assert!(!at(0.4), "closed after the release");
    assert!(at(0.6), "open on the next burst");
    assert_eq!(s.said, 3, "a new unit on each of three onsets");
    s.set_params(Params { env_release_sync: 1.0, ..s.params() });
    let mut open2 = Vec::new();
    for b in 0..(24000 / 128) {
        for (i, x) in inp.iter_mut().enumerate() {
            *x = burst(b * 128 + i);
        }
        s.process(&inp, &mut out);
        open2.push(s.env_open());
    }
    assert!(open2.iter().all(|&o| o), "a beat's release bridges the 0.3 s gap");
    let mut q = Suu::new(fs);
    q.set_params(Params { source: Source::Off, env_gate: true, env_thresh_db: -10.0, ..Params::default() });
    for b in 0..(24000 / 128) {
        for (i, x) in inp.iter_mut().enumerate() {
            *x = burst(b * 128 + i);
        }
        q.process(&inp, &mut out);
        assert!(!q.env_open());
    }
}

#[test]
fn pitch_follows_the_input() {
    let fs = 48000.0;
    let f0 = 220.0 * pow(2.0, 0.3 / 12.0);
    let run = |s: &mut Suu, secs: f64, ph: &mut f64| {
        let (mut inp, mut out) = ([0f32; 128], [0f32; 128]);
        for _ in 0..(secs * fs / 128.0) as usize {
            for x in inp.iter_mut() {
                *ph = (*ph + f0 / fs) % 1.0;
                *x = (0.3 * (2.0 * *ph - 1.0)) as f32;
            }
            s.process(&inp, &mut out);
        }
    };
    let mut ph = 0.0;
    let mut s = Suu::new(fs);
    s.set_params(Params { gate: true, note: 40.0, pitch_from: PitchFrom::Input, ..Params::default() });
    run(&mut s, 0.3, &mut ph);
    assert!((s.pitch - 57.3).abs() < 0.05, "INPUT: {}", s.pitch);
    s.set_params(Params { pitch_from: PitchFrom::InputSnap, ..s.params() });
    run(&mut s, 0.2, &mut ph);
    assert_eq!(s.pitch, 57.0, "INPUT ♮ snaps to A3");
    s.set_step(0, Step { on: true, note: 7, ..s.step(0) });
    s.set_playing(true);
    run(&mut s, 0.05, &mut ph);
    assert_eq!(s.pitch, 64.0, "a step 7 above the input");
    s.set_playing(false);
    let (inp, mut out) = ([0f32; 128], [0f32; 128]);
    for _ in 0..200 {
        s.process(&inp, &mut out);
    }
    assert_eq!(s.pitch, 57.0, "held through silence");
    s.set_params(Params { pitch_from: PitchFrom::Note, ..s.params() });
    run(&mut s, 0.1, &mut ph);
    assert_eq!(s.pitch, 40.0, "NOTE: the note");
}

#[test]
fn the_adsr_shapes_the_sound_and_legato_goes_on() {
    let fs = 48000.0;
    let run = |s: &mut Suu, secs: f64| {
        let (inp, mut out) = ([0f32; 64], [0f32; 64]);
        for _ in 0..(secs * fs / 64.0).round() as usize {
            s.process(&inp, &mut out);
        }
        s.adsr_level()
    };
    let base = Params { source: Source::Saw, gate: true, attack_s: 0.1, decay_s: 0.1, sustain: 0.5, release_s: 0.2, ..Params::default() };
    let mut s = Suu::new(fs);
    s.set_params(base);
    let half = run(&mut s, 0.05);
    assert!((half - 0.5).abs() < 0.03, "linear attack, halfway: {half}");
    let sus = run(&mut s, 0.3);
    assert!((sus - 0.5).abs() < 0.01, "sustain: {sus}");
    s.retrigger();
    let again = run(&mut s, 0.03);
    assert!(again > 0.6, "retriggered: {again}");
    let mut l = Suu::new(fs);
    l.set_params(Params { legato: true, ..base });
    run(&mut l, 0.4);
    l.retrigger();
    let on = run(&mut l, 0.03);
    assert!((on - 0.5).abs() < 0.01, "legato: {on}");
    s.set_params(Params { gate: false, ..s.params() });
    let gone = run(&mut s, 0.21);
    assert!(gone < 0.01, "released: {gone}");
    let mut y = Suu::new(fs);
    y.set_params(Params { attack_sync: 1.0, bpm: 120.0, ..base });
    let q = run(&mut y, 0.25);
    assert!((q - 0.5).abs() < 0.03, "a synced attack, halfway: {q}");
}

#[test]
fn the_input_vowel_places_the_pad() {
    let fs = 48000.0;
    let res = |f: f64, bw: f64| {
        let r = exp(-PI * bw / fs);
        (2.0 * r * cos(2.0 * PI * f / fs), -r * r)
    };
    let (r1, r2) = (res(270.0, 80.0), res(2290.0, 100.0));
    let (mut y1, mut y2) = ([0.0f64; 2], [0.0f64; 2]);
    let mut s = Suu::new(fs);
    s.set_params(Params { source: Source::Off, gate: true, vowel_x: 1.0, vowel_y: 1.0, vowel_in: true, ..Params::default() });
    let (mut inp, mut out) = ([0f32; 128], [0f32; 128]);
    for b in 0..(0.4 * fs / 128.0) as usize {
        for (i, x) in inp.iter_mut().enumerate() {
            let p = if (b * 128 + i) % 400 == 0 { 1.0 } else { 0.0 };
            let a = p + r1.0 * y1[0] + r1.1 * y1[1];
            y1 = [a, y1[0]];
            let v = a + r2.0 * y2[0] + r2.1 * y2[1];
            y2 = [v, y2[0]];
            *x = (v * 0.02) as f32;
        }
        s.process(&inp, &mut out);
    }
    let b = s.bases();
    assert!(b[0] < 0.3 && b[1] < 0.3, "the pad near i, not the knob's ɑ: {:.2}, {:.2}", b[0], b[1]);
    s.set_params(Params { vowel_in: false, ..s.params() });
    assert_eq!(&s.bases()[..2], &[1.0, 1.0]);
}

#[test]
fn steady_takes_a_new_pitch_only_once_it_has_lasted_and_coarse_transposes() {
    let fs = 48000.0;
    let run = |s: &mut Suu, f0: f64, secs: f64, ph: &mut f64| {
        let (mut inp, mut out) = ([0f32; 128], [0f32; 128]);
        for _ in 0..(secs * fs / 128.0) as usize {
            for x in inp.iter_mut() {
                *ph = (*ph + f0 / fs) % 1.0;
                *x = (0.3 * (2.0 * *ph - 1.0)) as f32;
            }
            s.process(&inp, &mut out);
        }
    };
    let mut ph = 0.0;
    let mut s = Suu::new(fs);
    s.set_params(Params { gate: true, pitch_from: PitchFrom::InputSnap, steady_s: 0.2, ..Params::default() });
    run(&mut s, 220.0, 0.4, &mut ph);
    assert!(s.tracked_note().is_some_and(|n| (n - 57.0).abs() < 0.1), "A3 taken after it lasted");
    run(&mut s, 330.0, 0.1, &mut ph);
    run(&mut s, 220.0, 0.1, &mut ph);
    assert_eq!(s.pitch, 57.0, "a short blip is filtered out");
    run(&mut s, 330.0, 0.4, &mut ph);
    assert_eq!(s.pitch, 64.0, "a held new note is taken");
    s.set_params(Params { pitch_from: PitchFrom::Input, ..s.params() });
    run(&mut s, 330.0 * pow(2.0, 0.3 / 12.0), 0.1, &mut ph);
    assert!((s.pitch - 64.3).abs() < 0.05, "30 cents up follows at once: {}", s.pitch);
    s.set_params(Params { coarse: -12.0, pitch_from: PitchFrom::InputSnap, ..s.params() });
    run(&mut s, 330.0, 0.1, &mut ph);
    assert_eq!(s.pitch, 52.0, "COARSE −12 on IN ♮");
    s.set_params(Params { pitch_from: PitchFrom::Note, note: 45.0, ..s.params() });
    run(&mut s, 330.0, 0.05, &mut ph);
    assert_eq!(s.pitch, 33.0, "COARSE −12 on the note");
    let mut y = Suu::new(fs);
    y.set_params(Params { gate: true, note: 40.0, pitch_from: PitchFrom::InputSnap, steady_sync: 1.0, bpm: 120.0, ..Params::default() });
    run(&mut y, 220.0, 0.4, &mut ph);
    assert_eq!(y.pitch, 40.0, "not yet after 0.4 s");
    run(&mut y, 220.0, 0.2, &mut ph);
    assert_eq!(y.pitch, 57.0, "taken after a beat");
}

#[test]
fn skipping_settled_geometry_changes_nothing() {
    let render = |skip: bool| {
        let mut s = Suu::new(48000.0);
        s.voice.airway.skip_settled = skip;
        s.set_params(Params { gate: true, ..Params::default() });
        let (inp, mut out) = ([0f32; 128], [0f32; 128]);
        let mut all = Vec::new();
        let mut run = |s: &mut Suu, blocks: usize| {
            for _ in 0..blocks {
                s.process(&inp, &mut out);
                all.extend_from_slice(&out);
            }
        };
        run(&mut s, 600);
        let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 3.0 + 2.0 * (i as f64 * 0.4).sin()).collect();
        s.set_profile(&wave, false);
        run(&mut s, 400);
        s.set_params(Params { morph: 0.5, ..s.params() });
        run(&mut s, 400);
        s.set_profile(&[4.0; HORN_POINTS], false);
        run(&mut s, 400);
        s.set_params(Params { vowel_x: 0.0, vowel_y: 0.0, open: 2.0, ..s.params() });
        run(&mut s, 400);
        s.set_size(1.3);
        s.voice.airway.skip_settled = skip;
        run(&mut s, 600);
        all
    };
    let (on, off) = (render(true), render(false));
    assert!(on.iter().zip(&off).all(|(a, b)| a.to_bits() == b.to_bits()), "skipping changed the output");
    assert!(on.iter().any(|v| v.abs() > 1e-3), "it sounds");
}

#[test]
fn hq_off_is_lighter_and_stable_at_every_size() {
    for size in [0.4, 0.5, 0.6, 0.8, 1.0, 1.3, 1.6, 2.0, 4.0] {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, hq: false, folds: 0.5, ..Params::default() });
        s.set_size(size);
        assert!(!s.hq());
        assert!(s.voice.m <= oversample_for(48000.0, size, Quality::Hq), "size {size}: no higher rate than HQ");
        let (inp, mut out) = ([0f32; 128], [0f32; 128]);
        let mut peak = 0f32;
        for b in 0..1500 {
            if b % 150 == 0 {
                let v = (b / 150) as f64 / 10.0;
                s.set_params(Params { vowel_x: v, vowel_y: 1.0 - v, morph: if b > 750 { 0.7 } else { 0.0 }, ..s.params() });
            }
            s.process(&inp, &mut out);
            for v in out {
                assert!(v.is_finite(), "size {size}: NaN");
                peak = peak.max(v.abs());
            }
        }
        assert!(peak > 0.01 && peak <= 1.0, "size {size}: peak {peak}");
    }
    let mut s = Suu::new(48000.0);
    s.set_params(Params { hq: false, ..Params::default() });
    s.set_size(1.0);
    assert_eq!((s.hq(), s.voice.m), (false, 1), "size 1 lean: the output rate");
    let (inp, mut out) = ([0f32; 128], [0f32; 128]);
    for _ in 0..20 {
        s.process(&inp, &mut out);
    }
    let _ = s.take_retired();
    s.set_params(Params { hq: true, ..s.params() });
    s.set_size(1.0);
    assert_eq!((s.hq(), s.voice.m), (true, 2), "HQ again: twice the rate");
}

#[test]
fn grid_holds_every_change_until_the_next_line_of_the_song() {
    let fs = 48000.0;
    let run = |s: &mut Suu, secs: f64| {
        let (inp, mut out) = ([0f32; 64], [0f32; 64]);
        for _ in 0..(secs * fs / 64.0).round() as usize {
            s.process(&inp, &mut out);
        }
    };
    let mut s = Suu::new(fs);
    s.set_params(Params { gate: true, note: 45.0, bpm: 120.0, grid: 1.0, glide_s: 0.0, ..Params::default() });
    s.set_mod(0, Mod { depth: 0.8, rate_hz: 3.0, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
    run(&mut s, 0.05);
    let first = s.eff[0];
    run(&mut s, 0.3);
    assert_eq!(s.eff[0], first, "held within the beat");
    run(&mut s, 0.2);
    assert_ne!(s.eff[0], first, "a new value on the next line");
    run(&mut s, 0.1);
    s.set_params(Params { note: 50.0, vowel_x: 0.0, ..s.params() });
    run(&mut s, 0.2);
    assert_eq!(s.pitch, 45.0, "the new pitch waits");
    run(&mut s, 0.2);
    assert_eq!(s.pitch, 50.0, "taken on the line");
    let shape = s.eff[0];
    s.set_params(Params { gate: false, ..s.params() });
    run(&mut s, 0.1);
    s.set_params(Params { gate: true, note: 57.0, ..s.params() });
    run(&mut s, 0.02);
    assert_eq!(s.pitch, 57.0, "a played note's pitch at once");
    assert_eq!(s.eff[0], shape, "its shape on the grid");
    s.set_song_position(Some(6.88));
    run(&mut s, 0.005);
    s.set_params(Params { note: 62.0, ..s.params() });
    run(&mut s, 0.03);
    assert_eq!(s.pitch, 57.0, "before the host's line");
    run(&mut s, 0.03);
    assert_eq!(s.pitch, 62.0, "on the host's line");
    s.set_params(Params { grid: 0.0, note: 60.0, ..s.params() });
    run(&mut s, 0.01);
    assert_eq!(s.pitch, 60.0, "free");
}

#[test]
fn grid_keeps_the_input_pitch_on_the_lines_however_fast_the_onsets() {
    let fs = 48000.0;
    let mut s = Suu::new(fs);
    s.set_params(Params { source: Source::Off, env_gate: true, pitch_from: PitchFrom::InputSnap, grid: 1.0, bpm: 120.0, glide_s: 0.0, ..Params::default() });
    let mut ph = 0.0;
    let mut changes = Vec::new();
    let mut last = f64::NAN;
    let (mut inp, mut out) = ([0f32; 64], [0f32; 64]);
    for b in 0..(3.0 * fs / 64.0) as usize {
        for (i, x) in inp.iter_mut().enumerate() {
            let t = (b * 64 + i) as f64 / fs;
            let f0 = if (t / 0.1) as usize % 2 == 0 { 220.0 } else { 330.0 };
            ph = (ph + f0 / fs) % 1.0;
            *x = if (t % 0.1) < 0.07 { (0.3 * (2.0 * ph - 1.0)) as f32 } else { 0.0 };
        }
        s.process(&inp, &mut out);
        if s.pitch != last {
            last = s.pitch;
            changes.push((b * 64) as f64 / fs);
        }
    }
    for &t in &changes[1..] {
        let off = (t / 0.5 - (t / 0.5).round()).abs() * 0.5;
        assert!(off < 0.003, "a change at {t:.3} s, off the grid by {off:.4} s");
    }
    assert!(changes.len() <= 8, "no faster than the grid: {changes:?}");
}

#[test]
fn scale_snaps_the_input_and_jump_folds_or_skips() {
    let major: u16 = [0, 2, 4, 5, 7, 9, 11].iter().map(|b| 1u16 << b).sum();
    assert_eq!(snap_to_scale(60.8, major), 60.0, "C♯ −20 c → C (nearer than D)");
    assert_eq!(snap_to_scale(61.6, major), 62.0, "→ D");
    assert_eq!(snap_to_scale(66.0, major), 65.0, "F♯: F and G equally near, the lower");
    assert_eq!(snap_to_scale(66.4, 0), 66.0, "none set: every semitone");
    let fs = 48000.0;
    let run = |s: &mut Suu, f0: f64, secs: f64, ph: &mut f64| {
        let (mut inp, mut out) = ([0f32; 128], [0f32; 128]);
        for _ in 0..(secs * fs / 128.0) as usize {
            for x in inp.iter_mut() {
                *ph = (*ph + f0 / fs) % 1.0;
                *x = (0.3 * (2.0 * *ph - 1.0)) as f32;
            }
            s.process(&inp, &mut out);
        }
    };
    let mut ph = 0.0;
    let mut s = Suu::new(fs);
    s.set_params(Params { gate: true, glide_s: 0.0, pitch_from: PitchFrom::InputSnap, scale: major, jump: 7.0, jump_fold: true, steady_s: 0.05, ..Params::default() });
    run(&mut s, 277.18, 0.3, &mut ph);
    assert_eq!(s.pitch, 60.0, "C♯ snaps to C in C major");
    run(&mut s, 277.18 * 4.0, 0.3, &mut ph);
    assert_eq!(s.pitch, 60.0, "two octaves up folds back to the octave taken");
    s.set_params(Params { jump_fold: false, ..s.params() });
    run(&mut s, 440.0, 0.3, &mut ph);
    assert_eq!(s.pitch, 60.0, "skipped: too far");
    run(&mut s, 329.63, 0.3, &mut ph);
    assert_eq!(s.pitch, 64.0, "within the jump: taken");
}

#[test]
fn grid_starts_each_part_of_the_text_on_a_line() {
    let fs = 48000.0;
    let mut s = Suu::new(fs);
    s.load_dict("WOW  W AW1\nYEAH  Y AE1\n");
    s.set_params(Params { bpm: 120.0, text_grid: 1.0, ..Params::default() });
    s.set_text("wow yeah wow yeah wow yeah wow yeah");
    let (inp, mut out) = ([0f32; 64], [0f32; 64]);
    let mut said = Vec::new();
    let mut last = s.said;
    for b in 0..(2.6 * fs / 64.0) as usize {
        let t = (b * 64) as f64 / fs;
        let on = (t % 0.15) < 0.1;
        if on != s.params().gate {
            s.set_params(Params { gate: on, ..s.params() });
        }
        s.process(&inp, &mut out);
        if s.said != last {
            last = s.said;
            said.push(t);
        }
    }
    assert!(!said.is_empty());
    for &t in &said {
        let off = (t / 0.5 - (t / 0.5).round()).abs() * 0.5;
        assert!(off < 0.003, "a part at {t:.3} s, off the beat by {off:.4} s");
    }
    assert!(said.len() <= 6, "no more than one a beat: {said:?}");
}

#[test]
fn the_synced_text_clock_is_on_the_song_lines() {
    let fs = 48000.0;
    let mut s = Suu::new(fs);
    s.load_dict("WOW  W AW1\nYEAH  Y AE1\n");
    s.set_params(Params { gate: true, bpm: 120.0, text_step: TextStep::Clock, text_sync: 1.0, ..Params::default() });
    s.set_song_position(Some(0.3));
    s.set_text("wow yeah wow yeah");
    let (inp, mut out) = ([0f32; 64], [0f32; 64]);
    for _ in 0..20 {
        s.process(&inp, &mut out);
    }
    let beats = s.beats_now() + (s.next_tick - s.voice.clock) * 2.0;
    assert!((beats - beats.round()).abs() < 1e-6, "the next tick at beat {beats}");
}

#[test]
fn text_grid_holds_the_speech_and_grid_leaves_it_free() {
    let fs = 48000.0;
    let jaw = ControlKey::Artic(ArticKey::Jaw).index();
    let speak = |grid: f64, text_grid: f64, breath: f64| {
        let mut s = Suu::new(fs);
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
        s.set_params(Params { gate: true, bpm: 120.0, text_step: TextStep::Clock, grid, text_grid, grid_breath: breath, ..Params::default() });
        s.set_text("wow yeah now wow yeah now");
        let (inp, mut out) = ([0f32; 64], [0f32; 64]);
        let mut v = Vec::new();
        for _ in 0..(2.0 * fs / 64.0) as usize {
            s.process(&inp, &mut out);
            let b = if s.voice.hold.on { *s.voice.hold.held() } else { s.voice.blend_now() };
            v.push((b[jaw].value, s.voice.glottis.params.abduction));
        }
        v
    };
    let within = |v: &[(f64, f64)], which: usize| {
        let line = |i: usize| (((i + 1) * 64) as f64 / fs / 0.125).floor() as i64;
        let mut n = 0;
        for (i, w) in v.windows(2).enumerate().skip(2) {
            let same = line(i + 1) == line(i) && line(i) == line(i - 1) && line(i - 1) == line(i - 2);
            let (a, b) = if which == 0 { (w[0].0, w[1].0) } else { (w[0].1, w[1].1) };
            if same && a != b {
                n += 1;
            }
        }
        n
    };
    let free = speak(0.0, 0.0, 1.0);
    assert!(within(&free, 0) > 50, "free: the speech moves all the time");
    let held = speak(0.0, 0.25, 1.0);
    assert_eq!(within(&held, 0), 0, "TEXT GRID: the speech only changes on lines");
    assert_eq!(within(&held, 1), 0, "BREATH 100 %: the voicing too");
    let live_breath = speak(0.0, 0.25, 0.0);
    assert!(within(&live_breath, 1) > 0, "BREATH 0: the voicing as it comes");
    let mod_only = speak(0.25, 0.0, 1.0);
    assert!(within(&mod_only, 0) > 50, "GRID alone: the speech stays free");
    let follows = speak(0.25, -1.0, 1.0);
    assert_eq!(within(&follows, 0), 0, "TEXT GRID = GRID: on GRID's lines");
}

#[test]
fn the_grid_moves_the_mouth_under_an_arpeggio_as_under_a_held_note() {
    let shapes = |arp: bool| {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: !arp, bpm: 120.0, grid: 0.25, morph: 0.5, attack_s: 0.001, release_s: 0.001, ..Params::default() });
        s.set_mod(6, Mod { depth: 0.9, rate_hz: 0.7, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
        let (inp, mut out) = ([0f32; 3000], [0f32; 3000]);
        let mut got = Vec::new();
        for _ in 0..16 {
            s.process(&inp[..40], &mut out[..40]);
            s.set_params(Params { gate: true, ..s.params() });
            s.process(&inp[..1460], &mut out[..1460]);
            got.push(s.applied[6]);
            s.process(&inp[..1500], &mut out[..1500]);
            s.set_params(Params { gate: !arp, ..s.params() });
            s.process(&inp[..3000], &mut out[..3000]);
        }
        got
    };
    let (arp, held) = (shapes(true), shapes(false));
    let moved = arp.windows(2).filter(|w| (w[0] - w[1]).abs() > 1e-3).count();
    assert!(moved >= 12, "the mouth moves line by line under the arpeggio: {arp:?}");
    for (i, (a, h)) in arp.iter().zip(&held).enumerate() {
        assert!((a - h).abs() < 1e-9, "line {i}: arpeggio {a}, held note {h}");
    }
}

#[test]
fn a_moving_mouth_is_still_after_the_note_and_sleeps() {
    for grid in [0.0, 0.25, 0.5] {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, bpm: 120.0, grid, ..Params::default() });
        s.set_mod(0, Mod { depth: 0.8, rate_hz: 0.7, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
        s.set_mod(1, Mod { depth: 0.8, rate_hz: 0.45, sync: 0.0, shape: Shape::Triangle, smooth: 0.0 });
        let (inp, mut out) = ([0f32; 128], [0f32; 128]);
        for _ in 0..375 {
            s.process(&inp, &mut out);
        }
        s.set_params(Params { gate: false, ..s.params() });
        let mut peak = 0f32;
        for b in 0..(3.0 * 48000.0 / 128.0) as usize {
            s.process(&inp, &mut out);
            if b > 375 {
                peak = out.iter().fold(peak, |m, v| m.max(v.abs()));
            }
        }
        assert!(peak < 1e-5, "grid {grid}: {peak} sounds a second after the note");
        assert!(s.asleep, "grid {grid}: asleep");
    }
}

#[test]
fn the_text_restarts_from_its_beginning() {
    let fs = 48000.0;
    let run = |s: &mut Suu, secs: f64| {
        let (inp, mut out) = ([0f32; 64], [0f32; 64]);
        for _ in 0..(secs * fs / 64.0) as usize {
            s.process(&inp, &mut out);
        }
    };
    let mut s = Suu::new(fs);
    s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
    s.set_text("wow yeah now");
    for _ in 0..3 {
        s.set_params(Params { gate: true, ..s.params() });
        run(&mut s, 0.15);
        s.set_params(Params { gate: false, ..s.params() });
        run(&mut s, 0.1);
    }
    assert_eq!(s.last_unit, 2);
    s.set_params(Params { gate: true, ..s.params() });
    run(&mut s, 0.15);
    s.set_params(Params { gate: false, ..s.params() });
    run(&mut s, 0.1);
    assert_eq!(s.last_unit, 0, "past the end it wraps to the first anyway");
    run(&mut s, 0.05);
    s.set_params(Params { gate: true, ..s.params() });
    run(&mut s, 0.15);
    assert_eq!(s.last_unit, 1);
    s.restart_text();
    s.set_params(Params { gate: false, ..s.params() });
    run(&mut s, 0.1);
    s.set_params(Params { gate: true, ..s.params() });
    run(&mut s, 0.15);
    assert_eq!(s.last_unit, 0, "restarted: the first part");
    let mut c = Suu::new(fs);
    c.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
    c.set_params(Params { gate: true, text_step: TextStep::Clock, text_sync: 0.0, text_rate_hz: 4.0, text_len: 0.5, ..Params::default() });
    c.set_text("wow yeah now wow yeah now");
    run(&mut c, 0.6);
    assert!(c.last_unit > 0);
    c.restart_text();
    run(&mut c, 0.01);
    assert_eq!(c.last_unit, 0, "restarted on the clock: the first part");
}

#[test]
fn the_breath_stops_with_the_note_at_follow_0() {
    for folds in [0.0, 0.73] {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, follow: 0.0, folds, breath: 2.04, ..Params::default() });
        let (inp, mut out) = ([0f32; 480], [0f32; 480]);
        for _ in 0..50 {
            s.process(&inp, &mut out);
        }
        s.set_params(Params { gate: false, ..s.params() });
        let mut peak = 0f32;
        for b in 0..300 {
            s.process(&inp, &mut out);
            if b >= 100 {
                peak = out.iter().fold(peak, |m, v| m.max(v.abs()));
            }
        }
        assert!(peak < 1e-5, "folds {folds}: {peak} a second after the note");
        assert!(s.asleep, "folds {folds}: asleep");
    }
}

#[test]
fn the_sound_follows_a_long_release() {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { gate: true, follow: 0.0, folds: 0.73, breath: 2.04, release_s: 0.5, ..Params::default() });
    let (inp, mut out) = ([0f32; 480], [0f32; 480]);
    let mut on = 0f32;
    for _ in 0..50 {
        s.process(&inp, &mut out);
        on = out.iter().fold(on, |m, v| m.max(v.abs()));
    }
    s.set_params(Params { gate: false, ..s.params() });
    let mut window = |n: usize| {
        let mut pk = 0f32;
        for _ in 0..n {
            s.process(&inp, &mut out);
            pk = out.iter().fold(pk, |m, v| m.max(v.abs()));
        }
        pk / on
    };
    let early = window(10);
    window(20);
    let mid = window(10);
    window(50);
    let late = window(10);
    assert!(early > 0.3, "the release starts from the note: {early}");
    assert!(mid < 0.3 * early && mid > 1e-3, "and falls through it: {mid}");
    assert!(late < 1e-3, "and is gone after it: {late}");
}

#[test]
fn a_turned_knob_does_not_move_the_modulation() {
    for grid in [0.0, 0.25] {
        let mk = || {
            let mut s = Suu::new(48000.0);
            s.set_params(Params { gate: true, morph: 0.5, bpm: 120.0, grid, ..Params::default() });
            s.set_mod(6, Mod { depth: 0.9, rate_hz: 2.0, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
            s
        };
        let (mut a, mut b) = (mk(), mk());
        let (inp, mut out) = ([0f32; 480], [0f32; 480]);
        for k in 0..200 {
            if k == 50 || k == 150 {
                for s in [&mut a, &mut b] {
                    s.set_params(Params { gate: k == 150, ..s.params() });
                }
            }
            if k % 3 == 0 {
                b.set_params(Params { follow: 0.01 * (k % 100) as f64, ..b.params() });
            }
            a.process(&inp, &mut out);
            b.process(&inp, &mut out);
            assert_eq!(a.applied[6], b.applied[6], "grid {grid}, block {k}: the knob moved the mouth");
        }
    }
}

#[test]
fn a_note_starts_without_a_thump() {
    let fs = 48000.0;
    let c = (-2.0 * PI * 100.0 / fs).exp();
    let horn = [0.603, 3.921, 9.225, 9.726, 10.322, 10.964, 2.363, 2.031, 1.695, 1.695, 1.695, 1.717, 1.853, 6.575, 6.57, 6.438, 5.762, 5.236, 4.786, 4.475, 4.165, 3.735, 3.461, 3.248, 2.989, 2.989, 2.989, 2.989, 4.811, 3.727, 2.729, 1.777];
    for (attack, depth, room_db) in [(0.503, 0.76, -6.0), (0.001, 0.76, 3.0), (0.503, 0.0, -6.0)] {
        let mut s = Suu::new(fs);
        s.set_size(1.5583);
        s.set_profile(&horn, false);
        s.set_params(Params {
            source: Source::Supersaw, note: 48.0, bpm: 120.0, breath: 0.0, follow: 0.0, osc_db: -6.575, folds: 1.1244, glide_s: 0.2139, glide_sync: 0.25,
            attack_s: attack, decay_sync: 0.5, sustain: 1.0, release_s: 0.001, drive_db: 5.269, tilt_hz: 814.0, sub_level: 0.418, sub_hz: 120.0,
            vowel_x: 0.9464, vowel_y: 0.8462, round: -0.228, nasal: 0.618, inject: 0.613, damping: 0.1, morph: 0.473, shift: -0.296, open: 3.0652, mix: 1.0,
            ..Params::default()
        });
        s.set_mod(6, Mod { depth, rate_hz: 0.5, sync: 8.0, shape: Shape::Sine, smooth: 0.0 });
        let (inp, mut out) = ([0f32; 48], [0f32; 48]);
        let mut y = 0f64;
        let mut low = |s: &mut Suu, ms: usize| {
            let mut pk = 0f64;
            for _ in 0..ms {
                s.process(&inp, &mut out);
                for &v in &out {
                    y = c * y + (1.0 - c) * v as f64;
                    pk = js_max(pk, y.abs());
                }
            }
            pk
        };
        low(&mut s, 500);
        for k in 0..4 {
            s.set_params(Params { gate: true, ..s.params() });
            let onset = low(&mut s, 100);
            low(&mut s, 300);
            let held = low(&mut s, 200);
            s.set_params(Params { gate: false, ..s.params() });
            low(&mut s, 400);
            let rise = 20.0 * (onset / held).log10();
            assert!(rise < room_db, "attack {attack}, Motion {depth}, note {k}: the low band starts {rise:+.1} dB over the held note");
        }
    }
}

#[test]
fn lo_is_coarser_than_lean_and_stable_at_every_size() {
    for size in [0.4, 0.5, 0.6, 0.8, 1.0, 1.3, 1.6, 2.0, 4.0] {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, hq: false, lofi: true, folds: 0.5, ..Params::default() });
        s.set_size(size);
        assert_eq!(s.quality(), Quality::Lo);
        assert!(s.voice.m <= oversample_for(48000.0, size, Quality::Lean), "size {size}: no higher rate than lean");
        let (inp, mut out) = ([0f32; 128], [0f32; 128]);
        let mut peak = 0f32;
        for b in 0..1500 {
            if b % 150 == 0 {
                let v = (b / 150) as f64 / 10.0;
                s.set_params(Params { vowel_x: v, vowel_y: 1.0 - v, morph: if b > 750 { 0.7 } else { 0.0 }, ..s.params() });
            }
            s.process(&inp, &mut out);
            for v in out {
                assert!(v.is_finite(), "size {size}: NaN");
                peak = peak.max(v.abs());
            }
        }
        assert!(peak > 0.01 && peak <= 1.0, "size {size}: peak {peak}");
    }
    let cells = |lofi: bool| {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { hq: false, lofi, ..Params::default() });
        s.set_size(1.0);
        (s.voice.m, s.voice.airway.n_ph + s.voice.airway.n_or)
    };
    let ((m_lean, lean), (m_lo, lo)) = (cells(false), cells(true));
    assert_eq!((m_lean, m_lo), (1, 1), "size 1: both at the output rate");
    assert!(lo < lean, "lo {lo} cells, lean {lean}");
}

fn round_clock(round: f64) -> Suu {
    let mut s = Suu::new(48000.0);
    s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
    s.set_params(Params { gate: true, bpm: 120.0, text_sync: 0.5, text_round: round, ..text_params(TextStep::Clock, TextUnit::Syllable, TextSay::Hold) });
    s.set_text("wow yeah now");
    s
}

#[test]
fn round_fades_each_unit_in_and_out_without_clicks() {
    let grains = |round: f64| {
        let mut s = round_clock(round);
        run(&mut s, 32, 24000);
        let mut g = Vec::new();
        let mut out = Vec::new();
        for _ in 0..(48000 / 32) {
            out.extend(run(&mut s, 32, 32));
            g.push(s.grain);
        }
        (g, out)
    };
    let (g0, out0) = grains(0.0);
    assert!(g0.iter().all(|&g| g == 1.0), "ROUND 0 leaves the sound on");
    let (g1, out1) = grains(1.0);
    let min = g1.iter().cloned().fold(f64::MAX, f64::min);
    let mean = g1.iter().sum::<f64>() / g1.len() as f64;
    assert!(min < 0.05 && (0.35..0.65).contains(&mean), "ROUND 1: min {min:.3}, mean {mean:.3}");
    let jump = |x: &[f32]| x.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).abs()).fold(0f32, f32::max);
    assert!(jump(&out1) <= 1.2 * jump(&out0) + 1e-3, "no click: {} vs {}", jump(&out1), jump(&out0));
    assert!(peak(&out1) > 0.01, "it sounds");
}

#[test]
fn round_is_block_size_independent() {
    assert_eq!(run(&mut round_clock(0.6), 128, 48000), run(&mut round_clock(0.6), 37, 48000));
}

#[test]
fn round_on_a_held_note_fades_in_and_stays() {
    let mut s = Suu::new(48000.0);
    s.load_dict("WOW  W AW1\nYEAH  Y AE1\n");
    s.set_text("wow yeah");
    s.set_params(Params { gate: true, text_round: 1.0, ..text_params(TextStep::Note, TextUnit::Syllable, TextSay::Hold) });
    let mut g = Vec::new();
    for _ in 0..(48000 / 32) {
        run(&mut s, 32, 32);
        g.push(s.grain);
    }
    assert!(g[0] < 0.5, "it fades in: {}", g[0]);
    assert!(g[g.len() / 2..].iter().all(|&x| x == 1.0), "and stays while the note is held");
}

#[test]
fn noise_levels_scale_each_turbulence_source() {
    let energy = |frication: f64, sibilance: f64, aspiration: f64| {
        let mut s = Suu::new(48000.0);
        s.load_dict("SEES  S IY1 Z\nSHE  SH IY1\nHA  HH AA1\n");
        s.set_params(Params { gate: true, source: Source::Off, frication, sibilance, aspiration, ..text_params(TextStep::Clock, TextUnit::Syllable, TextSay::Hold) });
        s.set_text("sees she ha");
        let out = run(&mut s, 128, 96000);
        let g = s.voice.airway.noise_gain;
        let b = s.noise_base;
        assert_eq!((g.glottis, g.constriction, g.teeth), (b.glottis * aspiration, b.constriction * frication, b.teeth * sibilance));
        out.windows(2).map(|w| ((w[1] - w[0]) as f64).powi(2)).sum::<f64>()
    };
    let one = energy(1.0, 1.0, 1.0);
    let none = energy(0.0, 0.0, 0.0);
    let more = energy(3.0, 3.0, 3.0);
    assert!(none < 0.5 * one && more > 2.0 * one, "noise energy: none {none:.3e}, one {one:.3e}, more {more:.3e}");
    let fric = energy(0.0, 1.0, 1.0);
    assert!(fric < one, "frication alone lowers it: {fric:.3e} vs {one:.3e}");
}

#[test]
fn fit_ends_a_held_note_with_its_syllable() {
    let open_after = |fit: bool| {
        let mut s = Suu::new(48000.0);
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\n");
        s.set_text("wow yeah");
        s.set_params(Params { gate: true, text_fit: fit, ..text_params(TextStep::Note, TextUnit::Syllable, TextSay::Hold) });
        let out = run(&mut s, 128, 96000);
        let tail = &out[72000..];
        (s.gate_eff, (tail.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / tail.len() as f64).sqrt())
    };
    let ((held_open, held), (fit_open, fit)) = (open_after(false), open_after(true));
    assert!(held_open && !fit_open, "held stays open, FIT closes");
    assert!(held > 1e-3 && fit < 0.05 * held, "a held key sounds on {held:.2e}; FIT has released {fit:.2e}");
}

#[test]
fn fit_on_the_clock_leaves_a_gap_after_each_word() {
    let share = |fit: bool| {
        let mut s = Suu::new(48000.0);
        s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
        s.set_params(Params { gate: true, bpm: 60.0, text_sync: 1.0, text_fit: fit, ..text_params(TextStep::Clock, TextUnit::Word, TextSay::Shot) });
        s.set_text("wow yeah now");
        run(&mut s, 32, 24000);
        let (mut open, mut all) = (0, 0);
        for _ in 0..(48000 * 3 / 32) {
            run(&mut s, 32, 32);
            all += 1;
            open += s.gate_eff as u32;
        }
        open as f64 / all as f64
    };
    let (free, fit) = (share(false), share(true));
    assert!(free > 0.99 && fit > 0.1 && fit < 0.8, "open share: free {free:.2}, fit {fit:.2}");
}

#[test]
fn air_at_its_top_stays_finite_and_bounded() {
    let mut s = Suu::new(48000.0);
    s.load_dict("SEES  S IY1 Z\nSHE  SH IY1\nHA  HH AA1\n");
    s.set_params(Params { gate: true, aspiration: AIR_MAX, frication: AIR_MAX, sibilance: AIR_MAX, breath: 6.0, ..text_params(TextStep::Clock, TextUnit::Syllable, TextSay::Shot) });
    s.set_text("sees she ha");
    let out = run(&mut s, 128, 96000);
    assert!(peak(&out) <= 1.0 && peak(&out) > 0.01, "peak {}", peak(&out));
}

fn notes_through_a_slide(steady_s: f64, slide_s: f64) -> Vec<f64> {
    let fs = 48000.0;
    let mut s = Suu::new(fs);
    s.set_params(Params { gate: true, pitch_from: PitchFrom::InputSnap, steady_s, glide_s: 0.0, ..Params::default() });
    let (mut inp, mut out) = ([0f32; 128], [0f32; 128]);
    let (mut ph, mut t) = (0.0f64, 0.0f64);
    let mut heard = Vec::new();
    let (hold, end) = (0.4, 0.4 + slide_s + 0.4);
    while t < end {
        for x in inp.iter_mut() {
            let u = clamp((t - hold) / slide_s, 0.0, 1.0);
            let f0 = 220.0 * pow(1.5, u);
            ph = (ph + f0 / fs) % 1.0;
            *x = (0.3 * (2.0 * ph - 1.0)) as f32;
            t += 1.0 / fs;
        }
        s.process(&inp, &mut out);
        if let Some(n) = s.tracked_note().map(f64::round)
            && t > 0.3
            && heard.last() != Some(&n)
        {
            heard.push(n);
        }
    }
    heard
}

#[test]
fn steady_does_not_creep_through_a_slide() {
    let heard = notes_through_a_slide(0.08, 0.15);
    let notes: Vec<f64> = heard.iter().map(|n| n.round()).fold(Vec::new(), |mut v, n| {
        if v.last() != Some(&n) {
            v.push(n);
        }
        v
    });
    assert_eq!(notes.first(), Some(&57.0), "{heard:?}");
    assert_eq!(notes.last(), Some(&64.0), "{heard:?}");
    assert!(notes.len() <= 2, "only the two notes, none on the way: {notes:?}");
}

#[test]
fn expression_routes_to_motion_targets_breath_and_level() {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { gate: true, slide_to: 1, slide_depth: 0.5, pressure_to: EXPR_BREATH, pressure_depth: 0.5, ..Params::default() });
    run(&mut s, 128, 4800);
    let (y0, b0) = (s.eff[1], s.breath());
    assert_eq!((y0, b0), (0.6, 1.0), "no expression, no change");
    s.set_expression(1.0, 0.5);
    run(&mut s, 128, 4800);
    assert!((s.eff[1] - (0.6 + 0.25 * 1.6)).abs() < 1e-9, "slide moves vowel Y: {}", s.eff[1]);
    assert!((s.breath() - 4.0).abs() < 1e-9, "pressure adds breath: {}", s.breath());
    s.set_params(Params { pressure_to: EXPR_LEVEL, pressure_depth: 1.0, ..s.params() });
    s.set_expression(0.0, 0.0);
    let quiet = run(&mut s, 128, 9600);
    s.set_expression(1.0, 0.0);
    let loud = run(&mut s, 128, 9600);
    assert!(peak(&quiet[4800..]) < 1e-6 && peak(&loud[4800..]) > 0.01, "level follows pressure: {} {}", peak(&quiet[4800..]), peak(&loud[4800..]));
    assert_eq!(run(&mut round_clock(0.0), 37, 9600), run(&mut round_clock(0.0), 128, 9600));
}

fn stereo_run(s: &mut Suu, block: usize, n: usize) -> (Vec<f32>, Vec<f32>) {
    let input = vec![0f32; block];
    let (mut l, mut r) = (vec![0f32; n], vec![0f32; n]);
    for (lo, ro) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
        let k = lo.len();
        s.process_stereo(&input[..k], lo, ro);
    }
    (l, r)
}

fn with_twin(p: Params) -> Suu {
    let mut s = Suu::new(48000.0);
    s.set_params(Params { stereo: true, ..p });
    let want = s.wants_twin().expect("stereo asks for a second mouth");
    let m = Mouth::build(&s.mouth_seed(want));
    assert!(s.set_twin(m).is_ok());
    assert!(s.wants_twin().is_none());
    s
}

#[test]
fn stereo_off_is_the_mono_output_on_both_sides() {
    let mono = run(&mut playing(), 128, 24000);
    let (l, r) = stereo_run(&mut playing(), 128, 24000);
    assert_eq!(l, mono);
    assert_eq!(r, mono);
}

#[test]
fn stereo_has_two_different_mouths_and_fades_in_without_a_click() {
    let mut s = with_twin(Params { gate: true, ..Params::default() });
    let (l, r) = stereo_run(&mut s, 128, 48000);
    assert!(peak(&l) <= 1.0 && peak(&r) <= 1.0 && peak(&r) > 0.01);
    let tail = 24000..48000;
    let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum::<f64>();
    let corr = dot(&l[tail.clone()], &r[tail.clone()]) / (dot(&l[tail.clone()], &l[tail.clone()]) * dot(&r[tail.clone()], &r[tail.clone()])).sqrt();
    assert!(corr < 0.95, "the sides differ: correlation {corr:.3}");
    let jump = |x: &[f32]| x.windows(3).map(|w| (w[2] - 2.0 * w[1] + w[0]).abs()).fold(0f32, f32::max);
    let (_, mono_r) = stereo_run(&mut playing(), 128, 48000);
    assert!(jump(&r) <= 1.5 * jump(&mono_r) + 1e-3, "no click: {} vs {}", jump(&r), jump(&mono_r));
}

#[test]
fn stereo_is_block_size_independent_and_does_not_allocate() {
    let p = Params { gate: true, folds: 0.4, ..Params::default() };
    assert_eq!(stereo_run(&mut with_twin(p), 128, 24000), stereo_run(&mut with_twin(p), 37, 24000));
    let mut s = with_twin(p);
    stereo_run(&mut s, 128, 9600);
    let (input, mut l, mut r) = ([0f32; 64], [0f32; 64], [0f32; 64]);
    let before = alloc_count::count();
    for i in 0..900 {
        if i == 750 {
            s.set_params(Params { stereo: false, ..s.params() });
        }
        s.process_stereo(&input, &mut l, &mut r);
    }
    assert_eq!(alloc_count::count() - before, 0, "stereo allocated");
    assert!(!s.stereo_active() && s.take_retired().is_some(), "off: the second mouth is retired after its fade");
    let (l, r) = stereo_run(&mut s, 64, 4800);
    assert_eq!(l, r, "and the sides are one again");
}

#[test]
fn a_host_that_builds_on_demand_builds_the_second_mouth_once() {
    let mut s = playing();
    let (input, mut l, mut r) = ([0f32; 128], [0f32; 128], [0f32; 128]);
    for _ in 0..100 {
        s.process_stereo(&input, &mut l, &mut r);
    }
    s.set_params(Params { stereo: true, ..s.params() });
    let mut builds = 0;
    let mut level = 0f32;
    for i in 0..400 {
        s.process_stereo(&input, &mut l, &mut r);
        if let Some(want) = s.wants_twin() {
            builds += 1;
            let _ = s.set_twin(Mouth::build(&s.mouth_seed(want)));
        }
        let _ = s.take_retired();
        if i > 300 {
            level = level.max(peak(&l)).max(peak(&r));
        }
    }
    assert_eq!(builds, 1, "built once");
    assert!(level > 0.01, "still sounding: {level}");
}


fn band_db(x: &[f32], f0: f64) -> f64 {
    let n = 4096;
    let segs = x.len() / n;
    (0..17)
        .map(|k| {
            let f = f0 - 200.0 + 25.0 * k as f64;
            let w = 2.0 * PI * f / 48000.0;
            let c = 2.0 * cos(w);
            let mut acc = 0.0;
            for seg in x.chunks_exact(n).take(segs) {
                let (mut s1, mut s2) = (0.0, 0.0);
                for (i, v) in seg.iter().enumerate() {
                    let s0 = *v as f64 * (0.5 - 0.5 * cos(2.0 * PI * i as f64 / n as f64)) + c * s1 - s2;
                    s2 = s1;
                    s1 = s0;
                }
                acc += (s1 * s1 + s2 * s2 - c * s1 * s2).max(1e-30);
            }
            10.0 * (acc / segs as f64).log10()
        })
        .fold(f64::MIN, f64::max)
}

#[test]
fn whistle_tames_a_mouth_closed_at_two_points_and_leaves_open_vowels_alone() {
    let render = |x: f64, y: f64, whistle: f64| {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { gate: true, vowel_x: x, vowel_y: y, whistle, ..Params::default() });
        run(&mut s, 128, 9600);
        run(&mut s, 128, 4096 * 6)
    };
    let (ringing, tamed) = (band_db(&render(1.3, 0.0, 1.0), 5775.0), band_db(&render(1.3, 0.0, 0.0), 5775.0));
    assert!(ringing - tamed > 10.0, "the 5.8 kHz whistle drops: {ringing:.1} → {tamed:.1} dB");
    assert_eq!(render(0.5, 0.6, 0.0), render(0.5, 0.6, 1.0), "an open vowel is untouched");
}

#[test]
fn move_scales_the_sound_of_the_mouth_changing_shape() {
    let level = |move_sound: f64| {
        let mut s = Suu::new(48000.0);
        s.set_params(Params { source: Source::Off, breath: 0.0, drive_db: -40.0, move_sound, ..Params::default() });
        for (i, st) in default_pattern().iter().enumerate() {
            s.set_step(i, *st);
        }
        s.set_seq(16, 4.0, 0.4);
        s.set_playing(true);
        let o = run(&mut s, 128, 96000);
        10.0 * (o[48000..].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 48000.0 + 1e-30).log10()
    };
    let (off, one, more) = (level(0.0), level(1.0), level(4.0));
    assert!(one > -70.0 && off < one - 40.0, "moving makes a sound, MOVE 0 removes it: {off:.1} vs {one:.1} dB");
    assert!(more > one + 9.0, "MOVE 400% is louder: {more:.1} vs {one:.1} dB");
}

#[test]
fn canon_has_the_right_mouth_one_unit_behind() {
    let dict = "WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\nSHE  SH IY1\n";
    let make = |canon: f64| {
        let mut s = with_twin(Params { gate: true, bpm: 120.0, text_sync: 0.5, text_len: 0.6, stereo_spread: 1.0, stereo_vowel: 0.0, stereo_canon: canon, ..text_params(TextStep::Clock, TextUnit::Word, TextSay::Hold) });
        s.load_dict(dict);
        s.set_text("wow yeah now she");
        stereo_run(&mut s, 128, 48000 * 4)
    };
    let bright = |x: &[f32]| -> Vec<f64> {
        x.chunks(480)
            .map(|c| {
                let e = c.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() + 1e-12;
                let d = c.windows(2).map(|w| ((w[1] - w[0]) as f64).powi(2)).sum::<f64>();
                (d / e).sqrt()
            })
            .collect()
    };
    let corr = |a: &[f64], b: &[f64]| {
        let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
        let num: f64 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
        num / (a.iter().map(|x| (x - ma).powi(2)).sum::<f64>() * b.iter().map(|y| (y - mb).powi(2)).sum::<f64>()).sqrt()
    };
    let tick = 25;
    let (l, r) = make(1.0);
    let (bl, br) = (bright(&l), bright(&r));
    let (from, to) = (200, 375);
    let lagged = corr(&bl[from - tick..to - tick], &br[from..to]);
    let aligned = corr(&bl[from..to], &br[from..to]);
    assert!(lagged > 0.7 && lagged > aligned + 0.2, "right follows one word behind: lagged {lagged:.2}, aligned {aligned:.2}");
    let (l0, r0) = make(0.0);
    let same = corr(&bright(&l0)[from..to], &bright(&r0)[from..to]);
    assert!(same > 0.9, "no canon: the two say the same at once ({same:.2})");
}

#[test]
fn phase_and_mirror_make_the_right_mouth_move_and_shape_otherwise() {
    let mut s = with_twin(Params { gate: true, stereo_phase: 0.5, stereo_mirror: true, ..Params::default() });
    s.set_mod(0, Mod { depth: 0.6, rate_hz: 1.3, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
    let wave: Vec<f64> = (0..HORN_POINTS).map(|i| 1.0 + i as f64 * 0.2).collect();
    s.set_profile(&wave, false);
    for _ in 0..40 {
        stereo_run(&mut s, 128, 1200);
        let base = s.params().vowel_x;
        assert!(((s.eff[0] - base) + (s.eff_twin[0] - base)).abs() < 1e-9, "half a cycle apart: {} {}", s.eff[0], s.eff_twin[0]);
    }
    let t = s.twin.as_ref().unwrap();
    assert_eq!(t.airway.profile[0], s.voice.airway.profile[HORN_POINTS - 1], "the right horn is the left one turned round");
    let (l, r) = stereo_run(&mut s, 128, 24000);
    assert!(peak(&l) <= 1.0 && peak(&r) <= 1.0 && peak(&r) > 0.01);
}

#[test]
fn canon_phase_and_mirror_do_not_allocate() {
    let mut s = with_twin(Params { gate: true, stereo_canon: 2.0, stereo_phase: 0.3, stereo_mirror: true, ..text_params(TextStep::Clock, TextUnit::Syllable, TextSay::Hold) });
    s.load_dict("WOW  W AW1\nYEAH  Y AE1\nNOW  N AW1\n");
    s.set_text("wow yeah now wow");
    s.set_mod(0, Mod { depth: 0.5, rate_hz: 2.0, sync: 0.0, shape: Shape::Sine, smooth: 0.0 });
    let (input, mut l, mut r) = ([0f32; 64], [0f32; 64], [0f32; 64]);
    for _ in 0..200 {
        s.process_stereo(&input, &mut l, &mut r);
    }
    let before = alloc_count::count();
    for i in 0..3000 {
        if i == 1500 {
            s.set_params(Params { text_say: TextSay::Shot, stereo_canon: 3.0, ..s.params() });
        }
        s.process_stereo(&input, &mut l, &mut r);
    }
    assert_eq!(alloc_count::count() - before, 0, "canon allocated");
}

#[test]
fn a_changing_breath_does_not_buzz_at_the_folds_control_rate() {
    let mut s = Suu::new(48000.0);
    s.load_dict("WOW  W AW1\nYEAH  Y AE1\nOH  OW1\nNOW  N AW1\nWAH  W AA1\n");
    s.set_params(Params {
        osc_db: -59.2, drive_db: -40.0, breath: 2.8, follow: 0.68, aspiration: 0.0, frication: 0.0, sibilance: 0.0, move_sound: 0.0, release_s: 1.6,
        text_step: TextStep::Note, text_unit: TextUnit::Word, text_say: TextSay::Shot, text_round: 0.48, text_seq: false,
        ..Params::default()
    });
    s.set_size(1.37);
    s.set_text("wow yeah oh yeah now wah wow");
    let mut out = Vec::new();
    for _ in 0..8 {
        s.set_params(Params { gate: true, ..s.params() });
        s.note_start();
        out.extend(run(&mut s, 128, 14400));
        s.set_params(Params { gate: false, ..s.params() });
        out.extend(run(&mut s, 128, 4800));
    }
    let seg = &out[out.len() - 65536..];
    let bin = |f: f64| {
        let w = 2.0 * PI * f / 48000.0;
        let c = 2.0 * cos(w);
        let (mut s1, mut s2) = (0.0, 0.0);
        for (i, v) in seg.iter().enumerate() {
            let s0 = *v as f64 * (0.5 - 0.5 * cos(2.0 * PI * i as f64 / seg.len() as f64)) + c * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        10.0 * (s1 * s1 + s2 * s2 - c * s1 * s2).max(1e-30).log10()
    };
    let rms = 10.0 * (out.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / out.len() as f64 + 1e-30).log10();
    let (buzz, around) = (bin(750.0), (bin(700.0) + bin(800.0)) / 2.0);
    assert!(rms < -40.0, "the near-silent preset stays near silent: {rms:.1} dB (was -26.5 with the buzz)");
    assert!(buzz < around + 15.0, "no strong line at 750 Hz (36 dB above its surroundings with the bug): {buzz:.1} dB against {around:.1} dB beside it");
}
