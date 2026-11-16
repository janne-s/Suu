use crate::params::{HornState, MAX_WORDS, SuuParams};
use crate::{Shared, g2p};
use nih_plug::prelude::*;
use nih_plug_webview::{HTMLSource, Key, WebViewEditor, WindowHandler, http};
use raw_window_handle::{AppKitDisplayHandle, AppKitWindowHandle, DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle, WindowHandle};
use raw_window_handle_05::HasRawWindowHandle;
use rfd::{AsyncFileDialog, FileHandle};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::io::Write;
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

const WINDOW_SIZE: (u32, u32) = (1280, 640);
const SMALLEST_WINDOW: (u32, u32) = (1024, 640);
const GESTURE_IDLE: Duration = Duration::from_millis(300);
const LOG_FILE: &str = "suu-plugin.log";

static PAGE: &[u8] = include_bytes!("../../web/index.html");
static APP: &[u8] = include_bytes!("../../web/app.js");
static STYLE: &[u8] = include_bytes!("../../web/style.css");
static FONT_LIGHT: &[u8] = include_bytes!("../../data/fonts/AVHersheySimplexLight.otf");
static FONT_MEDIUM: &[u8] = include_bytes!("../../data/fonts/AVHersheySimplexMedium.otf");

fn served(path: &str) -> Option<(&'static [u8], &'static str)> {
    Some(match path.trim_start_matches('/') {
        "" | "index.html" => (PAGE, "text/html"),
        "app.js" => (APP, "text/javascript"),
        "style.css" => (STYLE, "text/css"),
        "fonts/AVHersheySimplexLight.otf" => (FONT_LIGHT, "font/otf"),
        "fonts/AVHersheySimplexMedium.otf" => (FONT_MEDIUM, "font/otf"),
        _ => return None,
    })
}

fn log(text: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join(LOG_FILE)) {
        let _ = writeln!(file, "{text}");
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Message {
    Ready,
    Set { id: String, value: f32 },
    Text { text: String },
    Words { text: String },
    Horn { areas: Vec<f64>, wrap: bool },
    Ui { json: String },
    Slots { json: String },
    Typing { on: bool },
    Keys,
    Snapshot,
    Learn { on: bool },
    Resize { width: u32, height: u32 },
    Log { text: String },
    Save { name: String, text: String },
    Open,
    Reset,
}

type Chosen = Pin<Box<dyn Future<Output = Option<FileHandle>> + Send>>;

enum Dialog {
    Open(Chosen),
    Save(Chosen, Vec<u8>),
}

struct Parent(RawWindowHandle);

impl Parent {
    fn of(window: &impl HasRawWindowHandle) -> Option<Parent> {
        match window.raw_window_handle() {
            raw_window_handle_05::RawWindowHandle::AppKit(handle) => NonNull::new(handle.ns_view).map(|view| Parent(RawWindowHandle::AppKit(AppKitWindowHandle::new(view)))),
            _ => None,
        }
    }
}

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}

impl HasDisplayHandle for Parent {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(unsafe { DisplayHandle::borrow_raw(RawDisplayHandle::AppKit(AppKitDisplayHandle::new())) })
    }
}

enum Ask {
    Open,
    Save(String, Vec<u8>),
}

type Asked = Arc<Mutex<Option<Dialog>>>;

fn start_dialog(parent: Option<&Parent>, ask: Ask) -> Dialog {
    let d = match parent {
        Some(parent) => AsyncFileDialog::new().set_parent(parent),
        None => AsyncFileDialog::new(),
    };
    let mut dialog = match ask {
        Ask::Open => Dialog::Open(Box::pin(d.add_filter("Suu preset", &["json"]).pick_file())),
        Ask::Save(name, bytes) => Dialog::Save(Box::pin(d.set_file_name(&name).save_file()), bytes),
    };
    let (Dialog::Open(chosen) | Dialog::Save(chosen, _)) = &mut dialog;
    let _ = chosen.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    dialog
}

#[cfg(target_os = "macos")]
fn ask_dialog(window: &impl HasRawWindowHandle, ask: Ask, asked: &Asked) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    let view = match window.raw_window_handle() {
        raw_window_handle_05::RawWindowHandle::AppKit(handle) => handle.ns_view as *mut Object,
        _ => std::ptr::null_mut(),
    };
    if !view.is_null() {
        let _: *mut Object = unsafe { msg_send![view, retain] };
    }
    let job = Box::new(DialogJob { view, ask, asked: asked.clone() });
    unsafe { dispatch_async_f(&_dispatch_main_q, Box::into_raw(job).cast(), dialog_job) };
}

#[cfg(not(target_os = "macos"))]
fn ask_dialog(window: &impl HasRawWindowHandle, ask: Ask, asked: &Asked) {
    let dialog = start_dialog(Parent::of(window).as_ref(), ask);
    if let Ok(mut slot) = asked.lock() {
        *slot = Some(dialog);
    }
}

#[cfg(target_os = "macos")]
struct DialogJob {
    view: *mut objc::runtime::Object,
    ask: Ask,
    asked: Asked,
}

#[cfg(target_os = "macos")]
extern "C" fn dialog_job(job: *mut std::ffi::c_void) {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    let job = unsafe { Box::from_raw(job.cast::<DialogJob>()) };
    let DialogJob { view, ask, asked } = *job;
    let parent = NonNull::new(view.cast()).map(|v| Parent(RawWindowHandle::AppKit(AppKitWindowHandle::new(v))));
    let dialog = start_dialog(parent.as_ref(), ask);
    if !view.is_null() {
        let () = unsafe { msg_send![view as *mut Object, release] };
    }
    if let Ok(mut slot) = asked.lock() {
        *slot = Some(dialog);
    }
}

fn settle(dialog: &mut Dialog, page: &WindowHandler) -> bool {
    let chosen = match dialog {
        Dialog::Open(chosen) | Dialog::Save(chosen, _) => chosen,
    };
    let Poll::Ready(file) = chosen.as_mut().poll(&mut Context::from_waker(Waker::noop())) else {
        return false;
    };
    let Some(file) = file else {
        return true;
    };
    let path = file.path();
    match dialog {
        Dialog::Open(_) => match std::fs::read_to_string(path) {
            Ok(text) => page.send_json(json!({ "type": "opened", "name": path.file_name().map(|n| n.to_string_lossy().into_owned()), "text": text })),
            Err(error) => log(&format!("open {}: {error}", path.display())),
        },
        Dialog::Save(_, bytes) => {
            if let Err(error) = std::fs::write(path, bytes) {
                log(&format!("save {}: {error}", path.display()));
            }
        }
    }
    true
}

struct Gui {
    params: Vec<(String, ParamPtr)>,
    index: HashMap<String, usize>,
    known: Vec<f32>,
    gestures: HashMap<usize, Instant>,
    ready: bool,
    generation: u64,
    ui_seq: u64,
    ui_sent: (Vec<f32>, f64),
    key_sent: i32,
    text_shown: Option<(bool, String)>,
    dialog: Option<Dialog>,
    asking: bool,
    asked: Asked,
    frame: u64,
    keys_held: Option<Instant>,
    keys_pending: Arc<AtomicBool>,
}

const KEYS_BACK: Duration = Duration::from_millis(300);

#[cfg(target_os = "macos")]
fn keys_to_host(window: &impl HasRawWindowHandle, held: &mut Option<Instant>, now: bool, pending: &Arc<AtomicBool>) {
    use objc::runtime::{BOOL, NO, Object, YES};
    use objc::{class, msg_send, sel, sel_impl};
    let raw_window_handle_05::RawWindowHandle::AppKit(handle) = window.raw_window_handle() else { return };
    let view = handle.ns_view as *mut Object;
    if view.is_null() || pending.load(Ordering::Acquire) {
        return;
    }
    unsafe {
        let win: *mut Object = msg_send![view, window];
        if win.is_null() {
            return;
        }
        let first: *mut Object = msg_send![win, firstResponder];
        let in_webview: BOOL = if first.is_null() || first == view {
            NO
        } else {
            let is_view: BOOL = msg_send![first, isKindOfClass: class!(NSView)];
            if is_view == YES { msg_send![first, isDescendantOf: view] } else { NO }
        };
        if in_webview != YES {
            *held = None;
            return;
        }
        let since = *held.get_or_insert_with(Instant::now);
        if now || since.elapsed() >= KEYS_BACK {
            *held = None;
            pending.store(true, Ordering::Release);
            let _: *mut Object = msg_send![view, retain];
            let _: *mut Object = msg_send![win, retain];
            let job = Box::new(KeysBack { view, win, pending: pending.clone() });
            dispatch_async_f(&_dispatch_main_q, Box::into_raw(job).cast(), keys_back);
        }
    }
}

#[cfg(target_os = "macos")]
struct KeysBack {
    view: *mut objc::runtime::Object,
    win: *mut objc::runtime::Object,
    pending: Arc<AtomicBool>,
}

#[cfg(target_os = "macos")]
extern "C" fn keys_back(job: *mut std::ffi::c_void) {
    use objc::runtime::{BOOL, Object};
    use objc::{msg_send, sel, sel_impl};
    let job = unsafe { Box::from_raw(job.cast::<KeysBack>()) };
    unsafe {
        let now_in: *mut Object = msg_send![job.view, window];
        if now_in == job.win {
            let _: BOOL = msg_send![job.win, makeFirstResponder: job.view];
        }
        let () = msg_send![job.view, release];
        let () = msg_send![job.win, release];
    }
    job.pending.store(false, Ordering::Release);
}

#[cfg(target_os = "macos")]
#[repr(C)]
struct DispatchQueue {
    _private: [u8; 0],
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    static _dispatch_main_q: DispatchQueue;
    fn dispatch_async_f(queue: *const DispatchQueue, context: *mut std::ffi::c_void, work: extern "C" fn(*mut std::ffi::c_void));
}

fn plain(p: ParamPtr) -> f32 {
    unsafe { p.unmodulated_plain_value() }
}

impl Gui {
    fn param(&self, id: &str) -> Option<(usize, ParamPtr)> {
        self.index.get(id).map(|&i| (i, self.params[i].1))
    }

    fn values(&mut self) -> Map<String, Value> {
        let mut out = Map::new();
        for (i, (id, p)) in self.params.iter().enumerate() {
            let v = plain(*p);
            self.known[i] = v;
            out.insert(id.clone(), json!(v));
        }
        out
    }

    fn set(&mut self, setter: &ParamSetter, i: usize, value: f32) {
        let p = self.params[i].1;
        unsafe {
            if !self.gestures.contains_key(&i) {
                setter.raw_context.raw_begin_set_parameter(p);
            }
            setter.raw_context.raw_set_parameter_normalized(p, p.preview_normalized(value));
        }
        self.gestures.insert(i, Instant::now());
        self.known[i] = plain(p);
    }

    fn end_gestures(&mut self, setter: &ParamSetter, all: bool) {
        let now = Instant::now();
        let params = &self.params;
        self.gestures.retain(|&i, last| {
            let keep = !all && now.duration_since(*last) < GESTURE_IDLE;
            if !keep {
                unsafe { setter.raw_context.raw_end_set_parameter(params[i].1) };
            }
            keep
        });
    }
}

fn init_message(gui: &mut Gui, params: &SuuParams, synth: bool) -> Value {
    let words = params.words.read().map(|w| w.clone()).unwrap_or_default();
    let horn = params.horn.read().map(|h| h.clone()).unwrap_or_default();
    let ui = params.ui.read().map(|u| u.clone()).unwrap_or_default();
    let slots = params.slots.read().map(|u| u.clone()).unwrap_or_default();
    json!({
        "type": "init",
        "synth": synth,
        "values": gui.values(),
        "words": words,
        "horn": if horn.areas.len() == suu::HORN_POINTS { Some(horn.areas) } else { None },
        "hornWrap": horn.wrap,
        "ui": if ui.is_empty() { None } else { Some(ui) },
        "slots": if slots.is_empty() { None } else { Some(slots) },
    })
}

pub fn create(params: Arc<SuuParams>, shared: Arc<Shared>, synth: bool) -> Option<Box<dyn Editor>> {
    let all = params.param_map();
    let index = all.iter().enumerate().map(|(i, (id, _, _))| (id.clone(), i)).collect();
    let n = all.len();
    let gui = Mutex::new(Gui {
        params: all.into_iter().map(|(id, p, _)| (id, p)).collect(),
        index,
        known: vec![f32::NAN; n],
        gestures: HashMap::new(),
        ready: false,
        generation: 0,
        ui_seq: 0,
        ui_sent: (Vec::new(), f64::NAN),
        key_sent: i32::MIN,
        text_shown: None,
        dialog: None,
        asking: false,
        asked: Arc::new(Mutex::new(None)),
        frame: 0,
        keys_held: None,
        keys_pending: Arc::new(AtomicBool::new(false)),
    });
    let typing = Arc::new(AtomicBool::new(false));
    let typing_keys = typing.clone();
    let editor = WebViewEditor::new(HTMLSource::URL("suu://localhost/index.html"), WINDOW_SIZE)
        .with_background_color((13, 18, 23, 255))
        .with_developer_mode(cfg!(debug_assertions))
        .with_custom_protocol("suu".to_owned(), |request| {
            let path = request.uri().path();
            Ok(match served(path) {
                Some((bytes, kind)) => http::Response::builder().header("Content-Type", kind).body(Cow::Borrowed(bytes)).expect("a valid response"),
                None => http::Response::builder().status(404).body(Cow::Borrowed(&[][..])).expect("a valid response"),
            })
        })
        .with_keyboard_handler(move |event| event.key == Key::Escape || typing_keys.load(Ordering::Relaxed))
        .with_event_loop(move |page, setter, window| {
            let Ok(mut gui) = gui.lock() else { return };
            let gui = &mut *gui;
            gui.frame += 1;
            if gui.asking
                && let Ok(mut slot) = gui.asked.try_lock()
                && let Some(d) = slot.take()
            {
                gui.dialog = Some(d);
                gui.asking = false;
            }
            if let Some(d) = gui.dialog.as_mut()
                && settle(d, page)
            {
                gui.dialog = None;
            }
            let generation = shared.generation.load(Ordering::Acquire);
            if gui.ready && generation != gui.generation {
                gui.generation = generation;
                gui.end_gestures(&setter, true);
                page.send_json(init_message(gui, &params, synth));
                gui.text_shown = None;
            }
            let mut keys_now = false;
            while let Ok(value) = page.next_event() {
                let message = match serde_json::from_value::<Message>(value) {
                    Ok(m) => m,
                    Err(error) => {
                        log(&format!("unreadable message: {error}"));
                        continue;
                    }
                };
                match message {
                    Message::Ready => {
                        gui.ready = true;
                        gui.generation = generation;
                        gui.text_shown = None;
                        gui.ui_sent = (Vec::new(), f64::NAN);
                        gui.key_sent = i32::MIN;
                        page.send_json(init_message(gui, &params, synth));
                    }
                    Message::Set { id, value } => {
                        if let Some((i, _)) = gui.param(&id) {
                            gui.set(&setter, i, value);
                        }
                    }
                    Message::Text { text } => {
                        let text: String = text.chars().take(MAX_WORDS).collect();
                        let say = !text.trim().is_empty();
                        if say && let Ok(mut w) = params.words.write() {
                            *w = text;
                        }
                        if let Some((i, p)) = gui.param("text") {
                            unsafe {
                                setter.raw_context.raw_begin_set_parameter(p);
                                setter.raw_context.raw_set_parameter_normalized(p, if say { 1.0 } else { 0.0 });
                                setter.raw_context.raw_end_set_parameter(p);
                            }
                            gui.known[i] = plain(p);
                        }
                        shared.words_changed.store(true, Ordering::Release);
                    }
                    Message::Words { text } => {
                        if let Ok(mut w) = params.words.write() {
                            *w = text.chars().take(MAX_WORDS).collect();
                        }
                        shared.words_changed.store(true, Ordering::Release);
                    }
                    Message::Horn { areas, wrap } => {
                        if areas.len() == suu::HORN_POINTS {
                            let mut a = [0.0; suu::HORN_POINTS];
                            a.copy_from_slice(&areas);
                            let _ = shared.horn.0.try_send((a, wrap));
                            if let Ok(mut h) = params.horn.write() {
                                *h = HornState { areas, wrap };
                            }
                        }
                    }
                    Message::Ui { json } => {
                        if let Ok(mut u) = params.ui.write() {
                            *u = json;
                        }
                    }
                    Message::Slots { json } => {
                        if let Ok(mut u) = params.slots.write() {
                            *u = json;
                        }
                    }
                    Message::Typing { on } => typing.store(on, Ordering::Relaxed),
                    Message::Keys => keys_now = true,
                    Message::Snapshot => shared.snap_state.store(1, Ordering::Release),
                    Message::Learn { on } => shared.learn.store(on, Ordering::Release),
                    Message::Resize { width, height } => page.resize(window, width.max(SMALLEST_WINDOW.0), height.max(SMALLEST_WINDOW.1)),
                    Message::Log { text } => log(&text),
                    Message::Save { name, text } => {
                        if gui.dialog.is_none() && !gui.asking {
                            gui.asking = true;
                            ask_dialog(window, Ask::Save(name, text.into_bytes()), &gui.asked);
                        }
                    }
                    Message::Reset => {
                        gui.end_gestures(&setter, true);
                        for (_, p) in &gui.params {
                            unsafe {
                                setter.raw_context.raw_begin_set_parameter(*p);
                                setter.raw_context.raw_set_parameter_normalized(*p, p.default_normalized_value());
                                setter.raw_context.raw_end_set_parameter(*p);
                            }
                        }
                        if let Ok(mut w) = params.words.write() {
                            *w = crate::params::DEFAULT_WORDS.to_string();
                        }
                        if let Ok(mut h) = params.horn.write() {
                            *h = HornState::default();
                        }
                        if let Ok(mut u) = params.ui.write() {
                            u.clear();
                        }
                        shared.words_changed.store(true, Ordering::Release);
                        shared.horn_reset.store(true, Ordering::Release);
                        shared.keys_reset.store(true, Ordering::Release);
                        page.send_json(init_message(gui, &params, synth));
                        gui.text_shown = None;
                    }
                    Message::Open => {
                        if gui.dialog.is_none() && !gui.asking {
                            gui.asking = true;
                            ask_dialog(window, Ask::Open, &gui.asked);
                        }
                    }
                }
            }
            if !gui.ready {
                return;
            }
            gui.end_gestures(&setter, false);
            if gui.frame % 2 == 0 {
                let mut changed = Map::new();
                for i in 0..gui.params.len() {
                    if gui.gestures.contains_key(&i) {
                        continue;
                    }
                    let v = plain(gui.params[i].1);
                    if v != gui.known[i] {
                        gui.known[i] = v;
                        changed.insert(gui.params[i].0.clone(), json!(v));
                    }
                }
                if !changed.is_empty() {
                    page.send_json(json!({ "type": "params", "values": changed }));
                }
                let said = params.text.value();
                let words = params.words.read().map(|w| w.clone()).unwrap_or_default();
                if gui.text_shown.as_ref() != Some(&(said, words.clone())) {
                    let syllables = if said { chimera::phonetics::syllables::lyrics_to_syllables(&words, g2p()).len() } else { 0 };
                    page.send_json(json!({ "type": "text", "syllables": syllables, "words": words }));
                    gui.text_shown = Some((said, words));
                }
            }
            let learned = shared.learned.swap(-1, Ordering::AcqRel);
            if learned >= 0
                && let Some((_, p)) = gui.param("text_reset_note")
            {
                unsafe {
                    setter.raw_context.raw_begin_set_parameter(p);
                    setter.raw_context.raw_set_parameter_normalized(p, p.preview_normalized(learned as f32));
                    setter.raw_context.raw_end_set_parameter(p);
                }
                page.send_json(json!({ "type": "learned", "note": learned }));
            }
            let seq = shared.ui_seq.load(Ordering::Acquire);
            if seq != gui.ui_seq {
                gui.ui_seq = seq;
                let len = shared.ui_len.load(Ordering::Acquire).min(shared.ui.len());
                let ui: Vec<f32> = shared.ui[..len].iter().map(|v| f32::from_bits(v.load(Ordering::Relaxed))).collect();
                let bpm = f64::from_bits(shared.bpm.load(Ordering::Relaxed));
                if (&ui, bpm) != (&gui.ui_sent.0, gui.ui_sent.1) {
                    page.send_json(json!({ "type": "ui", "ui": ui, "bpm": bpm }));
                    gui.ui_sent = (ui, bpm);
                }
            }
            let key = shared.key.load(Ordering::Relaxed);
            if gui.ready && key != gui.key_sent {
                gui.key_sent = key;
                page.send_json(json!({ "type": "key", "note": key }));
            }
            let snap = shared.snap_state.load(Ordering::Acquire);
            if snap >= 2 {
                let values: Option<Vec<f32>> = (snap == 2).then(|| shared.snap.iter().map(|v| f32::from_bits(v.load(Ordering::Relaxed))).collect());
                page.send_json(json!({ "type": "snapshot", "values": values }));
                shared.snap_state.store(0, Ordering::Release);
            }
            #[cfg(target_os = "macos")]
            if !typing.load(Ordering::Relaxed) {
                keys_to_host(window, &mut gui.keys_held, keys_now, &gui.keys_pending);
            }
        });
    Some(Box::new(editor))
}
