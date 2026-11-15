use suu::{GlideDir, MAX_BLOCK, Mod, Params, PitchFrom, Shape, Source, Step, Suu, TextSay, TextStep, TextUnit};

pub struct Handle {
    suu: Suu,
    input: Vec<f32>,
    output: Vec<f32>,
    frame: Vec<f32>,
    snap: [f32; suu::HORN_POINTS],
}

fn set(p: &mut Params, id: u32, v: f64) {
    match id {
        0 => p.source = match v as i32 { 0 => Source::Off, 1 => Source::Saw, 3 => Source::Square, _ => Source::Supersaw },
        1 => p.note = v,
        2 => p.gate = v > 0.5,
        3 => p.drive_db = v,
        4 => p.tilt_hz = v,
        5 => p.breath = v,
        6 => p.follow = v,
        7 => p.vowel_x = v,
        8 => p.vowel_y = v,
        9 => p.round = v,
        10 => p.nasal = v,
        11 => p.inject = v,
        12 => p.damping = v,
        13 => p.folds = v,
        14 => p.sub_hz = v,
        15 => p.sub_level = v,
        16 => p.mix = v,
        17 => p.out_db = v,
        19 => p.bpm = v,
        20 => p.glide_s = v,
        21 => p.glide_sync = v,
        22 => p.text_rate_hz = v,
        23 => p.text_sync = v,
        24 => p.reson = v,
        25 => p.morph = v,
        26 => p.shift = v,
        27 => p.open = v,
        28 => p.text_seq = v > 0.5,
        29 => p.speech = v,
        30 => p.text_once = v > 0.5,
        31 => p.text_step = if v > 0.5 { TextStep::Clock } else { TextStep::Note },
        32 => p.text_unit = if v > 0.5 { TextUnit::Word } else { TextUnit::Syllable },
        33 => p.text_say = if v > 0.5 { TextSay::Shot } else { TextSay::Hold },
        34 => p.text_len = v,
        36 => p.osc_db = v,
        37 => p.input_db = v,
        38 => p.text_gate = v,
        39 => p.env_gate = v > 0.5,
        40 => p.env_thresh_db = v,
        41 => p.env_release_s = v,
        42 => p.env_release_sync = v,
        44 => p.vowel_in = v > 0.5,
        45 => p.attack_s = v,
        46 => p.attack_sync = v,
        47 => p.decay_s = v,
        48 => p.decay_sync = v,
        49 => p.sustain = v,
        50 => p.release_s = v,
        51 => p.release_sync = v,
        52 => p.legato = v > 0.5,
        53 => p.coarse = v,
        54 => p.steady_s = v,
        55 => p.steady_sync = v,
        56 => p.hq = v > 0.5,
        57 => p.grid = v,
        58 => p.scale = (v as i64).clamp(0, 0xfff) as u16,
        59 => p.jump = v,
        60 => p.jump_fold = v > 0.5,
        61 => p.grid_breath = v,
        62 => p.grid_smooth = v,
        63 => p.text_grid = v,
        43 => p.pitch_from = match v as i32 { 1 => PitchFrom::Input, 2 => PitchFrom::InputSnap, _ => PitchFrom::Note },
        35 => p.glide_dir = match v as i32 { 1 => GlideDir::Up, 2 => GlideDir::Down, _ => GlideDir::Both },
        _ => {}
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_free(p: *mut u8, n: usize) {
    if !p.is_null() {
        unsafe { drop(Vec::from_raw_parts(p, 0, n.max(1))) };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_new(fs: f64) -> *mut Handle {
    Box::into_raw(Box::new(Handle { suu: Suu::new(fs), input: vec![0.0; MAX_BLOCK], output: vec![0.0; MAX_BLOCK], frame: Vec::with_capacity(4096), snap: [0.0; suu::HORN_POINTS] }))
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_input(h: *mut Handle) -> *mut f32 {
    unsafe { (*h).input.as_mut_ptr() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_output(h: *mut Handle) -> *mut f32 {
    unsafe { (*h).output.as_mut_ptr() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_process(h: *mut Handle, n: usize) {
    let h = unsafe { &mut *h };
    let n = n.min(MAX_BLOCK);
    h.suu.process(&h.input[..n], &mut h.output[..n]);
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_set(h: *mut Handle, id: u32, v: f64) {
    let h = unsafe { &mut *h };
    if id == 18 {
        h.suu.set_size(v);
        return;
    }
    let mut p = h.suu.params();
    set(&mut p, id, v);
    h.suu.set_params(p);
    if h.suu.hq() != p.hq {
        let size = h.suu.size();
        h.suu.set_size(size);
    }
}

fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).unwrap_or("")
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_load_dict(h: *mut Handle, ptr: *const u8, len: usize) {
    let h = unsafe { &mut *h };
    h.suu.load_dict(text(ptr, len));
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_set_text(h: *mut Handle, ptr: *const u8, len: usize) -> u32 {
    let h = unsafe { &mut *h };
    h.suu.set_text(text(ptr, len)) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_set_profile(h: *mut Handle, ptr: *const u8, n: usize, wrap: u32) {
    let h = unsafe { &mut *h };
    let src = unsafe { std::slice::from_raw_parts(ptr, 4 * n) };
    let mut a = [0.0f64; suu::HORN_POINTS];
    for (d, b) in a.iter_mut().zip(src.chunks_exact(4)) {
        *d = f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64;
    }
    h.suu.set_profile(&a[..n.min(suu::HORN_POINTS)], wrap != 0);
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_set_step(h: *mut Handle, i: u32, on: u32, note: i32, tie: u32, vowel: i32, accent: u32, len: f64) {
    let h = unsafe { &mut *h };
    h.suu.set_step(i as usize, Step { on: on != 0, note, tie: tie != 0, vowel, accent: accent != 0, len });
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_set_seq(h: *mut Handle, len: u32, div: f64, gate: f64) {
    let h = unsafe { &mut *h };
    h.suu.set_seq(len as usize, div, gate);
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_play(h: *mut Handle, on: u32) {
    unsafe { (*h).suu.set_playing(on != 0) };
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_retrigger(h: *mut Handle) {
    unsafe { (*h).suu.retrigger() };
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_set_mod(h: *mut Handle, t: u32, depth: f64, rate_hz: f64, sync: f64, shape: u32, smooth: f64) {
    let h = unsafe { &mut *h };
    let shape = match shape { 1 => Shape::Triangle, 2 => Shape::Saw, 3 => Shape::Square, 4 => Shape::Random, 5 => Shape::Follow, 6 => Shape::Adsr, _ => Shape::Sine };
    h.suu.set_mod(t as usize, Mod { depth, rate_hz, sync, shape, smooth });
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_snapshot(h: *mut Handle) -> *const f32 {
    let h = unsafe { &mut *h };
    let mut v = [0.0f64; suu::HORN_POINTS];
    if !h.suu.snapshot(&mut v) {
        return std::ptr::null();
    }
    for (d, s) in h.snap.iter_mut().zip(v) {
        *d = s as f32;
    }
    h.snap.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_ui_update(h: *mut Handle) -> u32 {
    let h = unsafe { &mut *h };
    h.suu.ui_state(&mut h.frame);
    h.frame.len() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_ui(h: *mut Handle) -> *const f32 {
    unsafe { (*h).frame.as_ptr() }
}

#[unsafe(no_mangle)]
pub extern "C" fn suu_state(h: *mut Handle, what: u32) -> f64 {
    let h = unsafe { &*h };
    let (step, gate, note) = h.suu.seq_state();
    match what {
        0 => step as f64,
        1 => gate as u32 as f64,
        2 => note,
        4 => h.suu.tracked_note().unwrap_or(f64::NAN),
        _ => h.suu.playing() as u32 as f64,
    }
}
