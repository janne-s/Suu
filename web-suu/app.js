// Suu web app: the page around the worklet. Engine parameter ids match crates/suu-wasm.

const $ = (id) => document.getElementById(id);
const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
const css = getComputedStyle(document.documentElement);
const color = (n) => css.getPropertyValue(n).trim();

let ctx, node, srcNode, fileBuf, micStream, analyser, fx = {};
function post(m, transfer) {
  node?.port.postMessage(m, transfer ?? []);
}
function send(id, v) {
  post({ type: "param", id, v });
}
const bpm = () => clamp(+$("bpm").value || 100, 30, 300);

// note values for tempo sync, in beats
const DIVS = [["1/32", 0.125], ["1/16T", 1 / 6], ["1/16", 0.25], ["1/16.", 0.375], ["1/8T", 1 / 3], ["1/8", 0.5], ["1/8.", 0.75], ["1/4T", 2 / 3], ["1/4", 1], ["1/4.", 1.5], ["1/2", 2], ["1/2.", 3], ["1/1", 4], ["2/1", 8], ["4/1", 16]];
const divBeats = (n) => DIVS.find((d) => d[0] === n)[1];

// ---------------------------------------------------------------- knobs
// key, label, min, max, default, format, curve, target (engine id or "space"), anatomical [lo, hi]
const pct = (v) => Math.round(v * 100) + "%";
const times = (v) => "×" + v.toFixed(2);
const KNOBS = {
  "k-mouth": [
    ["size", "SIZE", 0.4, 4, 1, times, "log", 18, [0.7, 1.25]],
    ["folds", "FOLDS", 0, 2, 0, pct, null, 13, [0, 1]],
  ],
  "k-breath": [
    ["breath", "BREATH", 0, 6, 1, times, null, 5, [0, 2]],
    ["follow", "FOLLOW", 0, 1, 0.7, pct, null, 6],
  ],
  "k-drive": [
    ["reson", "RESON", 0, 1, 0, (v) => (v < 0.005 ? "off" : pct(v)), null, 24],
    ["glide", "GLIDE", 0, 2, 0.03, (v) => (v < 0.0005 ? "off" : v < 1 ? (v * 1000).toFixed(0) + " ms" : v.toFixed(2) + " s"), "sq", 20],
    ["drive", "DRIVE", -40, 40, 0, (v) => v.toFixed(1) + " dB", null, 3],
    ["tilt", "TILT", 0, 2000, 150, (v) => (v < 1 ? "off" : v.toFixed(0) + " Hz"), null, 4],
    ["sub", "SUB", 0, 1, 0, pct, null, 15],
    ["subhz", "SUB HZ", 30, 400, 120, (v) => v.toFixed(0) + " Hz", "log", 14],
  ],
  "k-space": [
    ["delayTime", "TIME", 0.05, 1.2, 0.45, (v) => (v * 1000).toFixed(0) + " ms", null, "space"],
    ["delayFb", "FEEDBACK", 0, 0.9, 0.35, pct, null, "space"],
    ["delayMix", "DELAY", 0, 1, 0.25, pct, null, "space"],
    ["revMix", "REVERB", 0, 1, 0.3, pct, null, "space"],
    ["revSize", "LENGTH", 0.3, 6, 2.2, (v) => v.toFixed(1) + " s", null, "space"],
  ],
  "k-out": [
    ["mix", "MIX", 0, 1, 1, pct, null, 16],
    ["out", "OUTPUT", -40, 20, 0, (v) => v.toFixed(1) + " dB", null, 17],
  ],
};
// time knobs that can lock to the tempo: the division they start from, and the engine id that
// takes the synced length in beats (none: the page converts it to seconds itself)
const SYNCABLE = { delayTime: { div: "1/8." }, glide: { div: "1/16", id: 21 } };

const state = {};
const controls = new Map();
const toNorm = (c, v) =>
  c.curve === "log" ? Math.log(v / c.min) / Math.log(c.max / c.min) : c.curve === "sq" ? Math.sqrt((v - c.min) / (c.max - c.min)) : (v - c.min) / (c.max - c.min);
const fromNorm = (c, n) => (c.curve === "log" ? c.min * Math.pow(c.max / c.min, n) : c.curve === "sq" ? c.min + n * n * (c.max - c.min) : c.min + n * (c.max - c.min));

/** A knob's value in its own units: the knob's, or the synced division's length in seconds. */
function eff(key) {
  const c = controls.get(key);
  return c.sync ? clamp((divBeats(c.sync) * 60) / bpm(), 0.01, 2) : state[key];
}

let sizeSent = 0;
function update(key, v, final = true) {
  const c = controls.get(key);
  v = clamp(v, c.min, c.max);
  state[key] = v;
  const n = toNorm(c, v);
  c.input.value = n;
  c.input.style.setProperty("--turn", `${270 * n}deg`);
  c.output.textContent = c.fmt(c.sync ? eff(key) : v);
  c.cell.classList.toggle("beyond", !!c.anat && (v < c.anat[0] - 1e-9 || v > c.anat[1] + 1e-9));
  if (c.anat) c.cell.title = `${c.label}: the anatomy spans ${c.fmt(c.anat[0])} … ${c.fmt(c.anat[1])}; beyond is allowed`;
  if (c.target === "space") setSpace();
  else if (c.target === 18) {
    // a new mouth crossfades in; while dragging, build at most one every 80 ms
    const now = performance.now();
    if (final || now - sizeSent > 80) {
      send(18, v);
      sizeSent = now;
    }
  } else {
    send(c.target, v);
    if (SYNCABLE[key]?.id) send(SYNCABLE[key].id, c.sync ? divBeats(c.sync) : 0);
  }
}

function dragValue(el, get, set, perPx = 1 / 180) {
  let drag = null;
  el.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    el.focus?.();
    el.setPointerCapture(e.pointerId);
    drag = { x: e.clientX, y: e.clientY, v: get() };
  });
  el.addEventListener("pointermove", (e) => {
    if (drag) set(drag.v + (drag.y - e.clientY + (e.clientX - drag.x)) * perPx, false);
  });
  const end = () => {
    if (drag) set(get(), true);
    drag = null;
  };
  el.addEventListener("pointerup", end);
  el.addEventListener("pointercancel", end);
}

function knob(parent, [key, label, min, max, def, fmt, curve, target, anat]) {
  const cell = document.createElement("div");
  cell.className = "knob";
  cell.innerHTML = `<label for="${key}">${label}</label><input id="${key}" type="range" min="0" max="1" step="0.001" aria-label="${label}"><output for="${key}"></output>`;
  $(parent).append(cell);
  const input = cell.querySelector("input");
  const output = cell.querySelector("output");
  const c = { cell, input, output, min, max, def, fmt, curve, target, label, anat };
  controls.set(key, c);
  input.oninput = () => update(key, fromNorm(c, +input.value));
  input.ondblclick = () => update(key, def);
  input.addEventListener("change", () => update(key, state[key], true));
  dragValue(input, () => toNorm(c, state[key]), (n, final) => update(key, fromNorm(c, clamp(n, 0, 1)), final));
  output.title = "Click to type a value";
  output.onclick = () => {
    const raw = prompt(`${label} (${min} … ${max})`, String(+state[key].toFixed(3)));
    if (raw !== null && Number.isFinite(+raw)) update(key, +raw);
  };
  if (SYNCABLE[key]) {
    const btn = document.createElement("button");
    btn.className = "sync";
    btn.textContent = "♪";
    btn.title = "Sync to tempo";
    btn.setAttribute("aria-pressed", "false");
    const box = document.createElement("select");
    box.className = "field syncbox";
    box.hidden = true;
    box.setAttribute("aria-label", `${label} as a note value`);
    box.innerHTML = DIVS.slice(0, 13).map(([n]) => `<option>${n}</option>`).join("");
    input.after(box);
    cell.append(btn);
    c.setSync = (div) => {
      c.sync = div;
      if (div) box.value = div;
      btn.setAttribute("aria-pressed", String(!!c.sync));
      input.hidden = !!c.sync;
      box.hidden = !c.sync;
    };
    btn.onclick = () => {
      if (c.sync) {
        const secs = eff(key);
        c.setSync(null);
        update(key, secs);
      } else {
        c.setSync(SYNCABLE[key].div);
        update(key, state[key]);
      }
    };
    box.onchange = () => {
      c.sync = box.value;
      update(key, state[key]);
    };
  }
  update(key, def);
}
for (const [parent, list] of Object.entries(KNOBS)) for (const k of list) knob(parent, k);

// ---------------------------------------------------------------- Motion
// target index (engine), label, engine param id of its base, range, default base, format, log
const MOTION = [
  [0, "VOWEL X", 7, -0.3, 1.3, 0.5, (v) => (v < 0.15 ? "front" : v > 0.85 ? "back" : pct(v))],
  [1, "VOWEL Y", 8, -0.3, 1.3, 0.6, (v) => (v < 0.15 ? "close" : v > 0.85 ? "open" : pct(v))],
  [2, "ROUND", 9, -1, 1, 0, (v) => (v < 0 ? "spread " + pct(-v) : pct(v))],
  [3, "NASAL", 10, 0, 1.5, 0, pct],
  [4, "INJECT", 11, 0, 1, 0, (v) => (v < 0.02 ? "throat" : v > 0.98 ? "lips" : pct(v))],
  [5, "DAMPING", 12, 0.1, 20, 1, times, true],
  // the drawn horn (HORN view): blend, slide along the tract, widen
  [6, "MORPH", 25, 0, 1, 0, (v) => (v < 0.005 ? "mouth" : v > 0.995 ? "horn" : pct(v))],
  [7, "SHIFT", 26, -0.5, 0.5, 0, (v) => (Math.abs(v) < 0.005 ? "0" : (v > 0 ? "lips " : "throat ") + pct(Math.abs(v)))],
  [8, "OPEN", 27, 0.25, 4, 1, times, true],
];
const SHAPES = ["SIN", "TRI", "SAW", "SQR", "RND", "ENV"];
const rows = [];
const baseNorm = (r, v) => (r.log ? Math.log(v / r.lo) / Math.log(r.hi / r.lo) : (v - r.lo) / (r.hi - r.lo));
const baseFrom = (r, n) => (r.log ? r.lo * Math.pow(r.hi / r.lo, n) : r.lo + n * (r.hi - r.lo));

function setBase(r, v, sendIt = true) {
  r.base = clamp(v, r.lo, r.hi);
  r.input.value = baseNorm(r, r.base);
  r.out.textContent = r.fmt(r.base);
  if (sendIt) send(r.param, r.base);
}
function sendMod(r) {
  const env = r.shape === 5;
  post({ type: "mod", target: r.t, depth: r.depth, rate: r.rate, sync: r.sync ? divBeats(r.sync) : 0, shape: r.shape });
  r.el.classList.toggle("active", r.depth !== 0);
  r.el.classList.toggle("env", env);
  r.depthBox.textContent = (r.depth > 0 ? "+" : "") + Math.round(r.depth * 100) + "%";
  r.depthBox.classList.toggle("on", r.depth !== 0);
  r.rateVal.textContent = r.rate < 1 ? r.rate.toFixed(2) + " Hz" : r.rate.toFixed(1) + " Hz";
  r.rateVal.hidden = !!r.sync;
  r.rateSel.hidden = !r.sync;
  r.syncBtn.setAttribute("aria-pressed", String(!!r.sync));
}
for (const [t, label, param, lo, hi, def, fmt, log] of MOTION) {
  const el = document.createElement("div");
  el.className = "motion-row";
  el.innerHTML = `<span class="name">${label}</span>
    <span class="base"><input type="range" min="0" max="1" step="0.001" aria-label="${label} base"><i></i><output></output></span>
    <span class="field drag depth" tabindex="0" aria-label="${label} depth"></span>
    <span class="rate"><span class="field drag val" tabindex="0" aria-label="${label} rate"></span><select class="field" hidden aria-label="${label} rate as a note value">${DIVS.map(([n]) => `<option>${n}</option>`).join("")}</select><button title="Sync to tempo">♪</button></span>
    <select class="field shape" aria-label="${label} shape">${SHAPES.map((s, i) => `<option value="${i}">${s}</option>`).join("")}</select>`;
  $("motion").append(el);
  const r = {
    t, param, lo, hi, fmt, log, el, def, base: def, depth: 0, rate: 0.5, sync: null, shape: 0,
    input: el.querySelector(".base input"), marker: el.querySelector(".base i"), out: el.querySelector(".base output"),
    depthBox: el.querySelector(".depth"),
    rateVal: el.querySelector(".rate .val"), rateSel: el.querySelector(".rate select"), syncBtn: el.querySelector(".rate button"), shapeSel: el.querySelector("select.shape"),
  };
  rows.push(r);
  r.input.oninput = () => setBase(r, baseFrom(r, +r.input.value));
  r.input.ondblclick = () => setBase(r, def);
  dragValue(r.depthBox, () => r.depth, (v) => {
    r.depth = Math.round(clamp(v, -1, 1) * 100) / 100;
    sendMod(r);
  }, 1 / 150);
  r.depthBox.ondblclick = () => {
    r.depth = 0;
    sendMod(r);
  };
  // rate: drag on a log scale, 0.02 … 20 Hz
  dragValue(r.rateVal, () => Math.log(r.rate), (lv) => {
    r.rate = clamp(Math.exp(lv), 0.02, 20);
    sendMod(r);
  }, 1 / 60);
  r.syncBtn.onclick = () => {
    r.sync = r.sync ? null : "1/4";
    r.rateSel.value = r.sync ?? "1/4";
    sendMod(r);
  };
  r.rateSel.onchange = () => {
    r.sync = r.rateSel.value;
    sendMod(r);
  };
  r.shapeSel.onchange = () => {
    r.shape = +r.shapeSel.value;
    sendMod(r);
  };
  setBase(r, def, false);
  sendMod(r);
}
$("motion-reset").onclick = () => {
  for (const r of rows) {
    r.depth = 0;
    sendMod(r);
  }
};

// ---------------------------------------------------------------- the mouth (also the vowel pad)
const PAD_LO = -0.3, PAD_HI = 1.3;
const toPad = (u) => (u - PAD_LO) / (PAD_HI - PAD_LO);
const mouthEl = $("mouth");
let ui = null, textOn = false;

function fit(canvas) {
  const r = canvas.getBoundingClientRect();
  const dpr = Math.min(2, devicePixelRatio || 1);
  const w = Math.max(1, Math.round(r.width * dpr)), h = Math.max(1, Math.round(r.height * dpr));
  if (canvas.width !== w || canvas.height !== h) Object.assign(canvas, { width: w, height: h });
  return [w, h, dpr];
}
const FONT = (px, dpr) => `${px * dpr}px Hershey, 'SFMono-Regular', monospace`;

/** A lens with rounded ends: the mouth's opening and the lips' outline (superellipse). */
function lens(c, cx, cy, w, hTop, hBot, round, more = false) {
  const n = 2 + 2.5 * round; // 2 = pointed corners like a lens … 4.5 = an O
  if (!more) c.beginPath(); // more: add to the current path (a ring, filled even-odd)
  for (let i = 0; i <= 64; i++) {
    const a = (i / 64) * 2 * Math.PI;
    const ca = Math.cos(a), sa = Math.sin(a);
    const x = (w / 2) * Math.sign(ca) * Math.pow(Math.abs(ca), 2 / n);
    const hh = sa < 0 ? hTop : hBot;
    const y = hh * Math.sign(sa) * Math.pow(Math.abs(sa), 2 / n);
    i ? c.lineTo(cx + x, cy + y) : c.moveTo(cx + x, cy + y);
  }
  c.closePath();
}

// ---------------------------------------------------------------- the horn (HORN view)
// The drawn area function: HORN_POINTS areas (cm², adult scale), glottis → lips. Its start
// matches the engine's: narrow at the glottis, a throat, a wide bell.
const HORN_POINTS = 32, HORN_MAX = 20, UI_AREAS = 48;
const defaultHorn = () => Float32Array.from({ length: HORN_POINTS }, (_, i) => {
  const u = i / (HORN_POINTS - 1);
  return 0.6 + 1.5 * Math.exp(-(((u - 0.3) / 0.12) ** 2)) + 9 * u ** 3;
});
let horn = defaultHorn(), view = "pad";
// a horn built from a wave is one period (points at i/n), and Shift wraps it around
let hornWrap = false;
const hornWave = { shape: -1, cycles: 2, depth: 0.6 }; // shape −1: drawn by hand
function sendHorn() {
  post({ type: "profile", areas: horn.slice(), wrap: hornWrap });
}
const hornU = (i) => i / (hornWrap ? HORN_POINTS : HORN_POINTS - 1);
/** The horn from a wave: a 3 cm² tube rippled `depth` (1 = closing at the troughs). */
// RND: one value per cycle, drawn anew only when RND is chosen or the cycles change
let rndSteps = [];
function buildWave(newRandom = false) {
  const { shape, cycles, depth } = hornWave;
  if (newRandom || rndSteps.length !== cycles) rndSteps = Array.from({ length: cycles }, () => Math.random() * 2 - 1);
  const steps = rndSteps;
  horn = Float32Array.from({ length: HORN_POINTS }, (_, i) => {
    const ph = (cycles * i) / HORN_POINTS, f = ph % 1;
    const w = [Math.sin(2 * Math.PI * ph), 1 - 4 * Math.abs(f - 0.5), 2 * f - 1, f < 0.5 ? 1 : -1, steps[Math.floor(ph)]][shape];
    return clamp(3 * (1 + depth * w), 0, HORN_MAX);
  });
  hornWrap = true;
  bringHornIn();
  sendHorn();
}
function showHornWave() {
  $("horn-shape").value = String(hornWave.shape);
  $("horn-cycles").textContent = String(hornWave.cycles);
  $("horn-depth").textContent = Math.round(hornWave.depth * 100) + "%";
}
function resetHorn() {
  horn = defaultHorn();
  hornWrap = false;
  hornWave.shape = -1;
  showHornWave();
  sendHorn();
}
/** Drawing or building a horn with MORPH at 0 would be silent: bring the horn in fully. */
function bringHornIn() {
  if (rows[6] && rows[6].base === 0) setBase(rows[6], 1);
}
$("horn-reset").onclick = resetHorn;
$("horn-shape").onchange = () => {
  hornWave.shape = +$("horn-shape").value;
  if (hornWave.shape >= 0) buildWave(true);
  showHornWave();
};
dragValue($("horn-cycles"), () => hornWave.cycles, (v) => {
  const c = Math.round(clamp(v, 1, 8));
  if (c === hornWave.cycles) return;
  hornWave.cycles = c;
  if (hornWave.shape < 0) hornWave.shape = 0;
  buildWave();
  showHornWave();
}, 1 / 25);
dragValue($("horn-depth"), () => hornWave.depth, (v) => {
  const d = Math.round(clamp(v, 0, 1) * 100) / 100;
  if (d === hornWave.depth && hornWave.shape >= 0) return;
  hornWave.depth = d;
  if (hornWave.shape < 0) hornWave.shape = 0;
  buildWave();
  showHornWave();
}, 1 / 150);
showHornWave();
/** The horn's frame on the canvas: x for u (0 glottis … 1 lips), half-height in px for an area. */
function hornFrame(w, h) {
  const x0 = w * 0.07, x1 = w * 0.93, cy = h * 0.5;
  const perCm = (h * 0.44) / Math.sqrt(HORN_MAX / Math.PI);
  return { x0, x1, cy, x: (u) => x0 + u * (x1 - x0), r: (a) => Math.sqrt(Math.max(0, a) / Math.PI) * perCm, perCm };
}
function drawHorn(c, w, h, dpr) {
  const f = hornFrame(w, h);
  // the axis, and the ends named
  c.strokeStyle = color("--line");
  c.lineWidth = dpr;
  c.setLineDash([3 * dpr, 4 * dpr]);
  c.beginPath();
  c.moveTo(f.x0, f.cy);
  c.lineTo(f.x1, f.cy);
  c.stroke();
  c.setLineDash([]);
  c.fillStyle = color("--dim");
  c.font = FONT(11, dpr);
  c.textBaseline = "middle";
  c.textAlign = "left";
  c.fillText("GLOTTIS", f.x0, 14 * dpr);
  c.textAlign = "right";
  c.fillText("LIPS", f.x1, 14 * dpr);
  // the live tract: a horn in the lips' rose, glowing with the sound
  if (ui && ui.length >= 23 + UI_AREAS) {
    const areas = ui.subarray(23, 23 + UI_AREAS);
    const lit = clamp(ui[16] * 4, 0, 1);
    const rgb = color("--lips-rgb");
    const pts = Array.from(areas, (a, k) => [f.x((k + 0.5) / UI_AREAS), f.r(a)]);
    c.beginPath();
    pts.forEach(([x, r], k) => (k ? c.lineTo(x, f.cy - r) : c.moveTo(x, f.cy - r)));
    for (let k = pts.length - 1; k >= 0; k--) c.lineTo(pts[k][0], f.cy + pts[k][1]);
    c.closePath();
    const g = c.createLinearGradient(0, f.cy - f.r(HORN_MAX), 0, f.cy + f.r(HORN_MAX));
    g.addColorStop(0, `rgba(${rgb}, ${0.08 + 0.1 * lit})`);
    g.addColorStop(0.5, `rgba(${rgb}, ${0.32 + 0.18 * lit})`);
    g.addColorStop(1, `rgba(${rgb}, ${0.08 + 0.1 * lit})`);
    c.fillStyle = g;
    c.shadowColor = color("--lips");
    c.shadowBlur = (8 + 20 * lit) * dpr;
    c.fill();
    c.strokeStyle = color("--lips");
    c.lineWidth = 1.6 * dpr;
    c.lineJoin = "round";
    c.stroke();
    c.shadowBlur = 0;
  }
  // the drawn horn (what MORPH blends toward), as an editable outline
  c.strokeStyle = color("--ink");
  c.globalAlpha = 0.7;
  c.lineWidth = 1.2 * dpr;
  c.setLineDash([4 * dpr, 3 * dpr]);
  for (const side of [-1, 1]) {
    c.beginPath();
    horn.forEach((a, i) => {
      const x = f.x(hornU(i)), y = f.cy + side * f.r(a);
      i ? c.lineTo(x, y) : c.moveTo(x, y);
    });
    c.stroke();
  }
  c.setLineDash([]);
  c.globalAlpha = 1;
}
let hornLast = -1;
function hornAt(e) {
  const r = mouthEl.getBoundingClientRect();
  const dpr = mouthEl.width / r.width;
  const f = hornFrame(mouthEl.width, mouthEl.height);
  const x = (e.clientX - r.left) * dpr, y = (e.clientY - r.top) * dpr;
  const u = clamp((x - f.x0) / (f.x1 - f.x0), 0, 1);
  const i = hornWrap ? Math.round(u * HORN_POINTS) % HORN_POINTS : Math.round(u * (HORN_POINTS - 1));
  const cm = Math.abs(y - f.cy) / f.perCm;
  const a = clamp(Math.PI * cm * cm, 0, HORN_MAX);
  // fill every point between the last one and this, so a fast stroke leaves no gaps
  const from = hornLast < 0 ? i : hornLast;
  const a0 = hornLast < 0 ? a : horn[from];
  const n = Math.abs(i - from);
  for (let k = 0; k <= n; k++) {
    const j = from + Math.sign(i - from) * k;
    horn[j] = n ? a0 + ((a - a0) * k) / n : a;
  }
  hornLast = i;
  sendHorn();
}
function setView(v) {
  view = v;
  $("view-pad").setAttribute("aria-pressed", String(v === "pad"));
  $("view-horn").setAttribute("aria-pressed", String(v === "horn"));
  mouthEl.style.cursor = v === "horn" ? "cell" : "";
  $("horn-bar").hidden = v !== "horn";
}
$("view-pad").onclick = () => setView("pad");
$("view-horn").onclick = () => setView("horn");

function drawMouth() {
  const c = mouthEl.getContext("2d");
  const [w, h, dpr] = fit(mouthEl);
  c.clearRect(0, 0, w, h);
  if (view === "horn") return drawHorn(c, w, h, dpr);
  // the pad: inner box = the four corner vowels, the border beyond extrapolates
  const x0 = toPad(0) * w, x1 = toPad(1) * w, y0 = toPad(0) * h, y1 = toPad(1) * h;
  c.strokeStyle = color("--line");
  c.lineWidth = dpr;
  c.setLineDash([3 * dpr, 4 * dpr]);
  c.strokeRect(x0, y0, x1 - x0, y1 - y0);
  c.setLineDash([]);
  c.fillStyle = color("--dim");
  c.font = FONT(13, dpr);
  c.textAlign = "center";
  c.textBaseline = "middle";
  for (const [t, x, y] of [["i", x0, y0], ["u", x1, y0], ["æ", x0, y1], ["ɑ", x1, y1]]) c.fillText(t, x, y + (y === y0 ? -12 : 12) * dpr);

  const base = rows.length ? [rows[0].base, rows[1].base] : [0.5, 0.6];
  const bx = ui ? ui[6] : base[0], by = ui ? ui[7] : base[1];
  const ex = ui ? ui[0] : bx, ey = ui ? ui[1] : by;

  // the mouth, front view, from the engine's lips, jaw, tongue and velum
  if (ui) {
    const [lipAp, lipWidth, lipProt, jaw, , tbY, velum, lipArea, env] = ui.slice(8, 17);
    const unit = Math.min(w, h) / 9; // px per cm
    const cx = w / 2, cy = h * 0.5;
    const width = 4.6 * (0.78 + 0.45 * lipWidth) * (1 - 0.38 * clamp(lipProt, 0, 1)); // cm
    const open = clamp((4 * Math.max(0, lipArea)) / (Math.PI * width), 0, 4); // cm, from the lips' area
    const round = clamp(lipProt, 0, 1);
    const thick = 0.75 + 0.5 * round;
    const lit = clamp(env * 4, 0, 1);
    const glow = 8 + 22 * lit;
    const lips = color("--lips"), hi = color("--lips-hi"), rgb = color("--lips-rgb");
    const rose = (a) => `rgba(${rgb}, ${a})`;
    const outer = () => lens(c, cx, cy, (width + 1.4) * unit, (open / 2 + thick) * unit, (open / 2 + thick * 1.1) * unit, round);
    const mouth = (more = false) => lens(c, cx, cy, width * unit, (open / 2) * unit, (open / 2) * unit, round, more);
    const top = cy - (open / 2 + thick) * unit, bottom = cy + (open / 2 + thick * 1.1) * unit;
    c.lineJoin = "round";
    // the lips: a ring of soft rose, fuller in the middle of each lip, glowing with the sound
    const body = c.createLinearGradient(0, top, 0, bottom);
    body.addColorStop(0, rose(0.1 + 0.08 * lit));
    body.addColorStop(0.3, rose(0.28 + 0.12 * lit));
    body.addColorStop(0.5, rose(0.06));
    body.addColorStop(0.72, rose(0.32 + 0.14 * lit));
    body.addColorStop(1, rose(0.1 + 0.08 * lit));
    c.shadowColor = lips;
    c.shadowBlur = glow * dpr;
    outer();
    if (open > 0.02) mouth(true);
    c.fillStyle = body;
    c.fill("evenodd");
    c.strokeStyle = lips;
    c.lineWidth = 1.6 * dpr;
    outer();
    c.stroke();
    if (open > 0.02) {
      // inside: a deep wine dark, the teeth in faint bone, the tongue in rose
      c.shadowBlur = 0;
      mouth();
      c.fillStyle = "#14060b";
      c.fill();
      c.save();
      mouth();
      c.clip();
      const tx = cx - width * 0.32 * unit, ty = cy - (open / 2) * unit - 2, tw = width * 0.64 * unit, th = 0.45 * unit + 2;
      c.fillStyle = "rgba(255, 240, 235, 0.1)";
      c.fillRect(tx, ty, tw, th);
      c.strokeStyle = "rgba(255, 240, 235, 0.45)";
      c.lineWidth = dpr;
      c.strokeRect(tx, ty, tw, th);
      const tongueTop = cy + (open / 2) * unit - clamp((tbY - 6.2) * 0.9 + 0.3 + jaw * 0.25, 0.1, 2.2) * unit;
      c.beginPath();
      c.ellipse(cx, tongueTop + 1.4 * unit, width * 0.38 * unit, 1.4 * unit, 0, Math.PI, 2 * Math.PI);
      c.fillStyle = rose(0.3 + 0.15 * lit);
      c.fill();
      c.strokeStyle = lips;
      c.lineWidth = 1.2 * dpr;
      c.stroke();
      c.restore();
      c.strokeStyle = lips;
      c.lineWidth = 1.3 * dpr;
      mouth();
      c.stroke();
    } else {
      c.shadowBlur = 0;
      c.strokeStyle = lips;
      c.lineWidth = 1.4 * dpr;
      c.beginPath();
      c.moveTo(cx - (width / 2) * unit, cy);
      c.quadraticCurveTo(cx, cy + 0.12 * unit, cx + (width / 2) * unit, cy);
      c.stroke();
    }
    // gloss: a light streak on the lower lip and two on the upper lip's bow
    c.shadowColor = hi;
    c.shadowBlur = 6 * dpr;
    c.strokeStyle = hi;
    c.lineCap = "round";
    c.globalAlpha = 0.45 + 0.35 * lit;
    c.lineWidth = 1.6 * dpr;
    c.beginPath();
    c.ellipse(cx + 0.1 * width * unit, cy + (open / 2 + thick * 0.62) * unit, width * 0.2 * unit, thick * 0.28 * unit, 0, 1.2 * Math.PI, 1.75 * Math.PI);
    c.stroke();
    c.lineWidth = 1.1 * dpr;
    c.globalAlpha *= 0.7;
    for (const sx of [-1, 1]) {
      c.beginPath();
      c.ellipse(cx + sx * width * 0.16 * unit, cy - (open / 2 + thick * 0.5) * unit, width * 0.08 * unit, thick * 0.18 * unit, 0, 1.15 * Math.PI, 1.8 * Math.PI);
      c.stroke();
    }
    c.lineCap = "butt";
    // nostrils light up as the velum opens
    const nas = clamp((velum + 0.1) / 0.6, 0, 1);
    c.shadowColor = lips;
    c.shadowBlur = (4 + 14 * nas) * dpr;
    c.strokeStyle = lips;
    c.globalAlpha = 0.3 + 0.6 * nas;
    c.lineWidth = 1.2 * dpr;
    for (const s of [-1, 1]) {
      c.beginPath();
      c.ellipse(cx + s * 0.55 * unit, cy - (open / 2 + thick + 1.6) * unit, 0.32 * unit, 0.18 * unit, s * 0.3, 0, 2 * Math.PI);
      c.stroke();
    }
    c.globalAlpha = 1;
    c.shadowBlur = 0;
  }

  // the pad's points: the base (where you set it) and the moving value (Motion)
  const px = toPad(bx) * w, py = toPad(by) * h, qx = toPad(ex) * w, qy = toPad(ey) * h;
  const beyond = bx < 0 || bx > 1 || by < 0 || by > 1;
  c.globalAlpha = textOn ? 0.4 : 1;
  c.strokeStyle = beyond ? color("--sound") : color("--ink");
  c.lineWidth = 1.2 * dpr;
  c.beginPath();
  c.moveTo(px - 7 * dpr, py);
  c.lineTo(px + 7 * dpr, py);
  c.moveTo(px, py - 7 * dpr);
  c.lineTo(px, py + 7 * dpr);
  c.stroke();
  if (Math.hypot(qx - px, qy - py) > 1) {
    c.strokeStyle = color("--beam-line");
    c.beginPath();
    c.moveTo(px, py);
    c.lineTo(qx, qy);
    c.stroke();
  }
  c.fillStyle = color("--beam");
  c.shadowColor = color("--beam");
  c.shadowBlur = 8 * dpr;
  c.beginPath();
  c.arc(qx, qy, 3.5 * dpr, 0, 2 * Math.PI);
  c.fill();
  c.shadowBlur = 0;
  c.globalAlpha = 1;
}
function padAt(e) {
  const r = mouthEl.getBoundingClientRect();
  const x = PAD_LO + clamp((e.clientX - r.left) / r.width, 0, 1) * (PAD_HI - PAD_LO);
  const y = PAD_LO + clamp((e.clientY - r.top) / r.height, 0, 1) * (PAD_HI - PAD_LO);
  setBase(rows[0], x);
  setBase(rows[1], y);
}
mouthEl.addEventListener("pointerdown", (e) => {
  mouthEl.setPointerCapture(e.pointerId);
  if (view === "horn") {
    bringHornIn();
    // a hand-drawn change: no longer the wave as built (it still wraps if it was one)
    hornWave.shape = -1;
    showHornWave();
    hornLast = -1;
    hornAt(e);
  } else padAt(e);
});
mouthEl.addEventListener("pointermove", (e) => e.buttons && (view === "horn" ? hornAt(e) : padAt(e)));
mouthEl.addEventListener("dblclick", () => view === "horn" && resetHorn());
new ResizeObserver(drawMouth).observe(mouthEl);

let peakHold = 0;
function frameLoop() {
  requestAnimationFrame(frameLoop);
  drawMouth();
  if (ui) {
    // live markers on the Motion rows
    for (const r of rows) {
      const live = ui[r.t < 6 ? r.t : 14 + r.t]; // the horn's three come after the first 20 values
      r.marker.hidden = r.depth === 0;
      r.marker.style.left = 100 * clamp(baseNorm(r, live), 0, 1) + "%";
      const a = baseNorm(r, r.base) * 100, b = clamp(baseNorm(r, live), 0, 1) * 100;
      r.input.style.setProperty("--lo", Math.min(a, b) + "%");
      r.input.style.setProperty("--hi", Math.max(a, b) + "%");
    }
  }
  if (analyser) {
    const t = new Float32Array(analyser.fftSize);
    analyser.getFloatTimeDomainData(t);
    let p = 0;
    for (const v of t) p = Math.max(p, Math.abs(v));
    peakHold = Math.max(p, peakHold * 0.97);
    const db = 20 * Math.log10(peakHold + 1e-9);
    $("peak").textContent = (db < -90 ? "−∞" : db.toFixed(1)) + " dB";
    $("peak").style.color = peakHold > 0.99 ? color("--warn") : "";
  }
}
requestAnimationFrame(frameLoop);

// ---------------------------------------------------------------- audio graph
async function start() {
  if (ctx) return;
  status("loading…", true);
  ctx = new AudioContext({ latencyHint: "interactive" });
  const [mod] = await Promise.all([WebAssembly.compileStreaming(fetch("suu.wasm")), ctx.audioWorklet.addModule("worklet.js")]);
  node = new AudioWorkletNode(ctx, "suu", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2], processorOptions: { module: mod } });
  node.port.onmessage = (e) => onWorklet(e.data);
  buildSpace();
  analyser = ctx.createAnalyser();
  analyser.fftSize = 2048;
  fx.master.connect(analyser);
  analyser.connect(ctx.destination);
  window.__suu = { ctx, analyser }; // for automated checks
  pushAll();
  await setSource(current);
  status("loading words…", true);
  const gz = await fetch("cmudict.txt.gz");
  const bytes = new Uint8Array(await new Response(gz.body.pipeThrough(new DecompressionStream("gzip"))).arrayBuffer());
  post({ type: "dict", bytes }, [bytes.buffer]);
}
// a browser starts audio only after a gesture: the first click, touch or key anywhere starts it
for (const ev of ["pointerdown", "keydown"]) addEventListener(ev, () => start(), { capture: true, once: true });
let statusTimer = 0;
/** A short message in the bar that fades after a few seconds. */
function status(t, keep = false) {
  const el = $("status");
  el.textContent = t;
  el.classList.remove("fade");
  clearTimeout(statusTimer);
  if (!keep) statusTimer = setTimeout(() => el.classList.add("fade"), 4000);
}

function onWorklet(m) {
  if (m.type === "dict") status("ready");
  else if (m.type === "text") {
    textOn = m.syllables > 0;
    $("text-toggle").textContent = textOn ? `TEXT · ${m.syllables}` : "TEXT";
    $("singtext").textContent = textOn ? "■ STOP" : "▶ SING";
    $("singtext").setAttribute("aria-pressed", String(textOn));
  } else if (m.type === "ui") {
    ui = m.ui;
    showSeq(ui[19] > 0.5, ui[17]);
  }
}

function pushAll() {
  for (const [key, c] of controls) {
    if (c.target === "space") continue;
    send(c.target, state[key]);
    if (SYNCABLE[key]?.id) send(SYNCABLE[key].id, c.sync ? divBeats(c.sync) : 0);
  }
  for (const r of rows) {
    send(r.param, r.base);
    sendMod(r);
  }
  send(19, bpm());
  send(1, lastNote);
  sendHorn();
  gate();
  sendTextRate();
  setSpace();
  steps.forEach((_, i) => sendStep(i));
  sendSeq();
}

function buildSpace() {
  const m = (fx.master = ctx.createGain());
  node.connect(m);
  // ping-pong delay: L → R → (feedback, darkened) → L
  const dl = ctx.createDelay(2), dr = ctx.createDelay(2);
  const fb = ctx.createGain(), tone = ctx.createBiquadFilter();
  tone.type = "lowpass";
  tone.frequency.value = 3500;
  const merge = ctx.createChannelMerger(2);
  const dmix = ctx.createGain();
  node.connect(dl);
  dl.connect(dr);
  dr.connect(tone);
  tone.connect(fb);
  fb.connect(dl);
  dl.connect(merge, 0, 0);
  dr.connect(merge, 0, 1);
  merge.connect(dmix);
  dmix.connect(m);
  // reverb: a generated stereo impulse, decaying noise that darkens as it fades
  const conv = ctx.createConvolver();
  const rmix = ctx.createGain();
  node.connect(conv);
  dmix.connect(conv);
  conv.connect(rmix);
  rmix.connect(m);
  Object.assign(fx, { dl, dr, fb, dmix, conv, rmix, irLen: 0 });
}

function makeIR(len) {
  const n = Math.floor(ctx.sampleRate * len);
  const ir = ctx.createBuffer(2, n, ctx.sampleRate);
  for (let ch = 0; ch < 2; ch++) {
    const d = ir.getChannelData(ch);
    let lp = 0;
    for (let i = 0; i < n; i++) {
      const t = i / n;
      lp += (0.85 - 0.8 * t) * (Math.random() * 2 - 1 - lp);
      d[i] = lp * Math.pow(1 - t, 2.2) * Math.min(1, i / (0.01 * ctx.sampleRate));
    }
  }
  fx.conv.buffer = ir;
  fx.irLen = len;
}

function setSpace() {
  // not before the graph is built: the first click starts the audio and may change a setting
  // in the same moment (pushAll sends the settings once it is)
  if (!ctx || !fx.dl) return;
  const on = spaceOn;
  const t = ctx.currentTime;
  fx.dl.delayTime.setTargetAtTime(eff("delayTime"), t, 0.05);
  fx.dr.delayTime.setTargetAtTime(eff("delayTime"), t, 0.05);
  fx.fb.gain.setTargetAtTime(state.delayFb, t, 0.02);
  fx.dmix.gain.setTargetAtTime(on ? state.delayMix : 0, t, 0.02);
  fx.rmix.gain.setTargetAtTime(on ? state.revMix : 0, t, 0.02);
  if (Math.abs(fx.irLen - state.revSize) > 0.05) makeIR(state.revSize);
}
let spaceOn = true;
$("spaceon").onclick = () => {
  spaceOn = !spaceOn;
  $("spaceon").setAttribute("aria-pressed", String(spaceOn));
  $("spaceon").textContent = spaceOn ? "ON" : "OFF";
  setSpace();
};

// ---------------------------------------------------------------- sources
let current = "2", fileName = "", autoHold = false;
async function setSource(src) {
  // keys gate an outside sound as they gate the synth: hold opens it at once (and keys then sing
  // the text's syllables over it); back on the synth, a hold this turned on goes off again
  const outside = (x) => x === "file" || x === "mic";
  if (outside(src) && !outside(current) && !held) {
    setHold(true);
    autoHold = true;
  } else if (!outside(src) && outside(current) && autoHold) {
    setHold(false);
    autoHold = false;
  }
  current = src;
  document.querySelectorAll("[data-src]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.src === src)));
  const internal = src !== "file" && src !== "mic";
  if (src === "file" && fileName) status(fileName);
  if (!ctx) return;
  srcNode?.disconnect();
  try {
    srcNode?.stop?.();
  } catch {}
  srcNode = null;
  send(0, internal ? +src : 0);
  if (src === "file" && fileBuf) {
    srcNode = ctx.createBufferSource();
    srcNode.buffer = fileBuf;
    srcNode.loop = true;
    srcNode.connect(node);
    srcNode.start();
  } else if (src === "mic") {
    try {
      micStream ??= await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false } });
      srcNode = ctx.createMediaStreamSource(micStream);
      srcNode.connect(node);
      status("mic on · use headphones");
    } catch (e) {
      status("no microphone: " + e.message);
    }
  }
}
document.querySelectorAll("[data-src]").forEach((b) => {
  b.onclick = async () => {
    if (b.dataset.src === "file" && (!fileBuf || current === "file")) return $("file").click();
    await start();
    setSource(b.dataset.src);
  };
});
async function loadFile(f) {
  await start();
  try {
    fileBuf = await ctx.decodeAudioData(await f.arrayBuffer());
    fileName = f.name;
    await setSource("file");
    status(f.name);
  } catch {
    status(`could not decode ${f.name}`);
  }
}
$("file").onchange = (e) => e.target.files[0] && loadFile(e.target.files[0]);
addEventListener("dragover", (e) => e.preventDefault());
addEventListener("drop", (e) => {
  e.preventDefault();
  const f = e.dataTransfer.files[0];
  if (f) /\.json$/i.test(f.name) ? openPreset(f) : loadFile(f);
});

// ---------------------------------------------------------------- keyboard
const KEYS = "awsedftgyhujk";
const NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
let octave = 0, held = false, lastNote = 45;
const down = new Map(); // key index → midi
const noteName = (m) => NAMES[((m % 12) + 12) % 12] + (Math.floor(m / 12) - 1);
function note(midi) {
  lastNote = clamp(midi, 12, 108);
  send(1, lastNote);
  $("notename").textContent = noteName(lastNote);
}
function gate() {
  send(2, down.size || held ? 1 : 0);
}
function shiftOctave(d) {
  octave = clamp(octave + d, -3, 3);
  note(lastNote + 12 * d);
  for (const [i, m] of down) down.set(i, m + 12 * d);
}
const typing = (e) => e.target.tagName === "SELECT" || (e.target.tagName === "INPUT" && (e.target.type === "text" || e.target.type === "number"));
/** Every key up: on blur, a hidden tab, Escape, or Cmd released (macOS sends no keyup for keys
 *  released while Cmd is down, which left notes sounding). */
function allKeysUp() {
  if (!down.size) return;
  down.clear();
  gate();
}
addEventListener("keydown", async (e) => {
  if (e.key === "Escape") return allKeysUp();
  if (typing(e) || e.metaKey || e.ctrlKey || e.altKey) return;
  const key = e.key.toLowerCase();
  if (e.code === "Space") {
    e.preventDefault();
    if (!e.repeat) {
      await start();
      setPlaying(!playing);
    }
    return;
  }
  if (key === "z" || key === "x") {
    if (!e.repeat) shiftOctave(key === "z" ? -1 : 1);
    return;
  }
  const i = KEYS.indexOf(key);
  if (i < 0 || e.repeat) return;
  // the key is down before any await: a keyup during the first start (audio still loading)
  // must find it, or the note stays on
  const midi = 48 + 12 * octave + i;
  const legato = down.size > 0;
  down.set(i, midi);
  await start();
  if (!down.has(i)) return; // released while starting
  note(midi);
  // a key while the gate is already open (another key, or hold) is still a new note for the text
  if (legato || held) post({ type: "retrigger" });
  gate();
});
addEventListener("keyup", (e) => {
  if (e.key === "Meta" || e.key === "Control" || e.key === "Alt") return allKeysUp();
  const i = KEYS.indexOf(e.key.toLowerCase());
  if (i < 0 || !down.has(i)) return;
  down.delete(i);
  if (down.size) note([...down.values()].pop()); // back to the key still held
  gate();
});
addEventListener("blur", allKeysUp);
document.addEventListener("visibilitychange", () => document.hidden && allKeysUp());
function setHold(on) {
  held = on;
  autoHold = false;
  $("hold").setAttribute("aria-pressed", String(held));
  gate();
}
$("hold").onclick = async () => {
  await start();
  setHold(!held);
};
$("octdown").onclick = () => shiftOctave(-1);
$("octup").onclick = () => shiftOctave(1);
$("bpm").onchange = () => {
  send(19, bpm());
  for (const key of Object.keys(SYNCABLE)) update(key, state[key]);
  for (const r of rows) if (r.sync) sendMod(r);
  setSpace();
  if (textOn) sing($("text").value);
};

// ---------------------------------------------------------------- sequencer
const VOWELS = ["·", "ɑ", "e", "i", "o", "u"]; // index 0 = leave the mouth (−1)
const steps = [
  [1, 0, 0, 0], [0, 0, 0, -1], [1, 12, 0, 4], [1, 0, 0, 2], [1, 0, 1, 0], [1, 7, 0, 3], [0, 0, 0, -1], [1, 10, 0, 1],
  [1, 0, 0, 0], [0, 0, 0, -1], [1, 12, 0, 4], [1, 0, 0, 2], [1, 3, 0, 3], [1, 5, 0, 0], [1, 7, 0, 1], [0, 0, 0, -1],
].map(([on, n, tie, vowel], i) => ({ on: !!on, note: n, tie: !!tie, vowel, accent: i % 8 === 0, len: tie ? 1 : 0.6 }));
let playing = false;
const cells = [];

function sendStep(i) {
  const s = steps[i];
  post({ type: "step", i, on: s.on, note: s.note, tie: s.tie, vowel: s.vowel, accent: s.accent, len: s.len });
}
let seqGate = 1; // × every step's length
function sendSeq() {
  post({ type: "seq", len: clamp(+$("seqlen").value || 16, 1, 16), div: +$("rate").value, gate: seqGate });
  $("seqgate").textContent = Math.round(seqGate * 100) + "%";
  $("seqgate").classList.toggle("on", seqGate !== 1);
  renderSteps();
}
dragValue($("seqgate"), () => seqGate, (v) => {
  seqGate = Math.round(clamp(v, 0.1, 2) * 100) / 100;
  sendSeq();
}, 1 / 200);
$("seqgate").ondblclick = () => {
  seqGate = 1;
  sendSeq();
};
function renderSteps() {
  const len = clamp(+$("seqlen").value || 16, 1, 16);
  steps.forEach((s, i) => {
    const c = cells[i];
    c.el.classList.toggle("on", s.on);
    c.el.classList.toggle("tie", s.on && s.tie);
    c.el.classList.toggle("accent", s.on && s.accent);
    c.el.style.setProperty("--len", s.len);
    c.el.style.setProperty("--glen", Math.min(1, s.len * seqGate));
    c.el.classList.toggle("off-len", i >= len);
    c.label.textContent = s.on ? (s.note > 0 ? "+" + s.note : s.note < 0 ? "−" + -s.note : "0") : "–";
    c.vowel.textContent = VOWELS[s.vowel + 1];
    c.vowel.classList.toggle("set", s.vowel >= 0);
  });
}
for (let i = 0; i < 16; i++) {
  const el = document.createElement("div");
  el.className = "step";
  el.innerHTML = `<div class="note"><span></span><i class="acc" title="Accent">◆</i><div class="len" title="Length"><b></b></div></div><div class="vowel"></div><i class="led"></i>`;
  $("steps").append(el);
  const noteEl = el.querySelector(".note"), vowelEl = el.querySelector(".vowel");
  cells.push({ el, note: noteEl, label: noteEl.querySelector("span"), vowel: vowelEl });
  const accEl = el.querySelector(".acc"), lenEl = el.querySelector(".len");
  accEl.addEventListener("pointerdown", (e) => e.stopPropagation());
  accEl.addEventListener("click", () => {
    steps[i].accent = !steps[i].accent;
    steps[i].on = true;
    sendStep(i);
    renderSteps();
  });
  // length: drag across the bar
  lenEl.addEventListener("pointerdown", (e) => {
    e.stopPropagation();
    lenEl.setPointerCapture(e.pointerId);
    const set = (ev) => {
      const r = lenEl.getBoundingClientRect();
      // the bar shows the length after the global gate, so the cursor sets that
      const at = clamp((ev.clientX - r.left) / r.width, 0, 1) / seqGate;
      steps[i].len = clamp(Math.round(at * 20) / 20, 0.05, 1);
      sendStep(i);
      renderSteps();
    };
    set(e);
    lenEl.onpointermove = (ev) => ev.buttons && set(ev);
  });
  let drag = null;
  noteEl.addEventListener("pointerdown", (e) => {
    noteEl.setPointerCapture(e.pointerId);
    drag = { y: e.clientY, note: steps[i].note, moved: false, shift: e.shiftKey };
  });
  noteEl.addEventListener("pointermove", (e) => {
    if (!drag) return;
    const d = Math.round((drag.y - e.clientY) / 8);
    if (d !== 0 || drag.moved) {
      drag.moved = true;
      steps[i].note = clamp(drag.note + d, -24, 24);
      steps[i].on = true;
      sendStep(i);
      renderSteps();
    }
  });
  noteEl.addEventListener("pointerup", () => {
    if (drag && !drag.moved) {
      if (drag.shift) steps[i].tie = !steps[i].tie;
      else steps[i].on = !steps[i].on;
      sendStep(i);
      renderSteps();
    }
    drag = null;
  });
  noteEl.addEventListener("dblclick", () => {
    steps[i].note = 0;
    steps[i].on = true;
    sendStep(i);
    renderSteps();
  });
  vowelEl.addEventListener("click", () => {
    steps[i].vowel = steps[i].vowel >= 4 ? -1 : steps[i].vowel + 1;
    sendStep(i);
    renderSteps();
  });
}
sendSeq();

function setPlaying(on) {
  playing = on;
  post({ type: "play", on });
  $("play").textContent = on ? "■ STOP" : "▶ PLAY";
  $("play").setAttribute("aria-pressed", String(on));
  if (!on) showSeq(false, 0);
}
$("play").onclick = async () => {
  await start();
  setPlaying(!playing);
};
let shownStep = -1;
function showSeq(isPlaying, step) {
  const s = isPlaying ? step : -1;
  if (s === shownStep) return;
  if (shownStep >= 0) cells[shownStep]?.el.classList.remove("now");
  if (s >= 0) cells[s]?.el.classList.add("now");
  shownStep = s;
}
for (const id of ["rate", "seqlen"]) $(id).onchange = sendSeq;

// ---------------------------------------------------------------- text
let textMode = 1;
function sing(text) {
  if (!node) return;
  const bytes = new TextEncoder().encode(text);
  post({ type: "text", bytes, mode: textMode }, [bytes.buffer]);
}
$("text-toggle").onclick = () => {
  const open = $("text-panel").hidden;
  $("text-panel").hidden = !open;
  $("text-toggle").setAttribute("aria-expanded", String(open));
  if (open) $("text").focus();
};
$("text-form").onsubmit = async (e) => {
  e.preventDefault();
  await start();
  sing($("text").value);
  $("text").blur(); // so the keys play again
};
$("singtext").onclick = async () => {
  await start();
  sing(textOn ? "" : $("text").value);
};
// while it sings, typing applies the text once it pauses; Enter sings, Escape hands the keys back
let typingTimer = 0;
$("text").addEventListener("input", () => {
  clearTimeout(typingTimer);
  typingTimer = setTimeout(() => node && textOn && sing($("text").value), 300);
});
$("text").addEventListener("keydown", (e) => {
  if (e.key === "Escape") $("text").blur();
});
function setTextMode(m) {
  textMode = m;
  document.querySelectorAll("[data-tmode]").forEach((x) => x.setAttribute("aria-pressed", String(+x.dataset.tmode === m)));
  $("looprate").hidden = m !== 0;
}
document.querySelectorAll("[data-tmode]").forEach((b) => {
  b.onclick = () => {
    setTextMode(+b.dataset.tmode);
    if (textOn) sing($("text").value);
  };
});
// the loop's own rate: free (syllables per second) or a note value per syllable
const loop = { rate: 2, sync: "1/4" };
function sendTextRate(final = true) {
  send(22, loop.rate);
  send(23, loop.sync ? divBeats(loop.sync) : 0);
  $("loophz").textContent = loop.rate.toFixed(loop.rate < 10 ? 1 : 0) + "/s";
  $("loophz").hidden = !!loop.sync;
  $("loopdiv").hidden = !loop.sync;
  if (loop.sync) $("loopdiv").value = loop.sync;
  $("loopsync").setAttribute("aria-pressed", String(!!loop.sync));
  // the loop plans a pass ahead: sing again so a new rate is heard now
  if (final && textOn && textMode === 0) sing($("text").value);
}
$("loopdiv").innerHTML = DIVS.map(([n]) => `<option>${n}</option>`).join("");
dragValue($("loophz"), () => Math.log(loop.rate), (lv, final) => {
  loop.rate = clamp(Math.exp(lv), 0.25, 16);
  sendTextRate(final);
}, 1 / 60);
$("loopsync").onclick = () => {
  loop.sync = loop.sync ? null : "1/4";
  sendTextRate();
};
$("loopdiv").onchange = () => {
  loop.sync = $("loopdiv").value;
  sendTextRate();
};
setTextMode(textMode);
sendTextRate(false);

// ---------------------------------------------------------------- focus
// a focus ring only while moving by keys: Tab turns it on, any pointer turns it off
addEventListener("keydown", (e) => e.key === "Tab" && (document.documentElement.dataset.nav = "keys"), true);
addEventListener("pointerdown", () => delete document.documentElement.dataset.nav, true);

// ---------------------------------------------------------------- help
function showInfo(open) {
  $("info").hidden = !open;
  $("info-toggle").setAttribute("aria-expanded", String(open));
}
$("info-toggle").onclick = (e) => {
  e.stopPropagation();
  showInfo($("info").hidden);
};
addEventListener("pointerdown", (e) => {
  if (!$("info").hidden && !$("info").contains(e.target) && e.target !== $("info-toggle")) showInfo(false);
});
addEventListener("keydown", (e) => e.key === "Escape" && showInfo(false));

// ---------------------------------------------------------------- saved state: autosave, A/B, files
// One JSON state holds every setting. The web app keeps it (two slots, A and B) in this browser's
// storage, saved every second; SAVE / OPEN write and read it as a file (a preset). The plug-in
// will keep the same JSON in the host's saved state instead, never in shared browser storage.
// Reading is tolerant: a missing field takes its default, an unknown one is ignored, numbers are
// clamped, and a wrong structure refuses the whole state before anything changes.
const STATE_VERSION = 1;
const MOTION_NAMES = ["vowelX", "vowelY", "round", "nasal", "inject", "damping", "morph", "shift", "open"];
const INTERNAL = ["1", "2", "3"];

function getState() {
  const knobs = {};
  for (const [key, c] of controls) knobs[key] = c.sync ? { v: state[key], sync: c.sync } : state[key];
  const motion = {};
  rows.forEach((r, i) => (motion[MOTION_NAMES[i]] = { base: r.base, depth: r.depth, rate: r.rate, sync: r.sync, shape: r.shape }));
  return {
    suu: STATE_VERSION,
    source: INTERNAL.includes(current) ? +current : undefined,
    note: lastNote,
    octave,
    bpm: bpm(),
    space: spaceOn,
    knobs,
    motion,
    seq: { rate: +$("rate").value, length: clamp(+$("seqlen").value || 16, 1, 16), gate: seqGate, steps: steps.map((s) => ({ ...s })) },
    text: { words: $("text").value, mode: textMode, rate: loop.rate, sync: loop.sync },
    horn: Array.from(horn, (a) => Math.round(a * 1000) / 1000),
    hornWrap,
    hornWave: { ...hornWave },
  };
}

const DEFAULTS = getState();

/** A complete, clamped state from `raw` and the defaults; throws on a wrong structure. */
function readState(raw) {
  const obj = (x, what) => {
    if (x === undefined) return {};
    if (x === null || typeof x !== "object" || Array.isArray(x)) throw new Error(`${what} is not an object`);
    return x;
  };
  const num = (x, d, lo, hi) => (Number.isFinite(x) ? clamp(x, lo, hi) : d);
  const bool = (x, d) => (typeof x === "boolean" ? x : d);
  const div = (x, d) => (x === null ? null : DIVS.some((q) => q[0] === x) ? x : d);
  const r = obj(raw, "the state");
  if (!Number.isFinite(r.suu)) throw new Error("not a Suu state");
  const d = DEFAULTS;
  const out = {
    suu: STATE_VERSION,
    source: INTERNAL.includes(String(r.source)) ? +r.source : d.source,
    note: Math.round(num(r.note, d.note, 12, 108)),
    octave: Math.round(num(r.octave, d.octave, -3, 3)),
    bpm: num(r.bpm, d.bpm, 30, 300),
    space: bool(r.space, d.space),
    knobs: {},
    motion: {},
  };
  const knobs = obj(r.knobs, "knobs");
  for (const [key, c] of controls) {
    const k = knobs[key], dk = d.knobs[key];
    const dv = typeof dk === "object" ? dk.v : dk;
    if (k !== null && typeof k === "object") {
      const v = num(k.v, dv, c.min, c.max);
      out.knobs[key] = SYNCABLE[key] && div(k.sync, null) ? { v, sync: div(k.sync, null) } : v;
    } else out.knobs[key] = num(k, dv, c.min, c.max);
  }
  const motion = obj(r.motion, "motion");
  rows.forEach((row, i) => {
    const name = MOTION_NAMES[i], m = obj(motion[name], `motion ${name}`), dm = d.motion[name];
    out.motion[name] = { base: num(m.base, dm.base, row.lo, row.hi), depth: num(m.depth, dm.depth, -1, 1), rate: num(m.rate, dm.rate, 0.02, 20), sync: div(m.sync, dm.sync), shape: Math.round(num(m.shape, dm.shape, 0, SHAPES.length - 1)) };
  });
  const seq = obj(r.seq, "seq");
  if (seq.steps !== undefined && !Array.isArray(seq.steps)) throw new Error("steps is not a list");
  out.seq = {
    rate: [1, 2, 4, 8].includes(seq.rate) ? seq.rate : d.seq.rate,
    length: Math.round(num(seq.length, d.seq.length, 1, 16)),
    gate: num(seq.gate, d.seq.gate, 0.1, 2),
    steps: d.seq.steps.map((ds, i) => {
      const s = obj(seq.steps?.[i], `step ${i + 1}`);
      return { on: bool(s.on, ds.on), note: Math.round(num(s.note, ds.note, -24, 24)), tie: bool(s.tie, ds.tie), vowel: Math.round(num(s.vowel, ds.vowel, -1, 4)), accent: bool(s.accent, ds.accent), len: num(s.len, ds.len, 0.05, 1) };
    }),
  };
  const text = obj(r.text, "text");
  if (r.horn !== undefined && !Array.isArray(r.horn)) throw new Error("horn is not a list");
  out.horn = d.horn.map((da, i) => num(r.horn?.[i], da, 0, HORN_MAX));
  out.hornWrap = bool(r.hornWrap, d.hornWrap);
  const hw = obj(r.hornWave, "hornWave");
  out.hornWave = { shape: Math.round(num(hw.shape, d.hornWave.shape, -1, 4)), cycles: Math.round(num(hw.cycles, d.hornWave.cycles, 1, 8)), depth: num(hw.depth, d.hornWave.depth, 0, 1) };
  out.text = { words: typeof text.words === "string" ? text.words.slice(0, 400) : d.text.words, mode: [0, 1, 2].includes(text.mode) ? text.mode : d.text.mode, rate: num(text.rate, d.text.rate, 0.25, 16), sync: div(text.sync, d.text.sync) };
  return out;
}

/** Put a state read by `readState` on every control (and into the engine, if it runs). */
function applyState(st) {
  $("bpm").value = st.bpm;
  send(19, bpm());
  octave = st.octave;
  note(st.note);
  for (const [key, c] of controls) {
    const k = st.knobs[key];
    c.setSync?.(typeof k === "object" ? k.sync : null);
    update(key, typeof k === "object" ? k.v : k);
  }
  rows.forEach((r, i) => {
    const m = st.motion[MOTION_NAMES[i]];
    Object.assign(r, { depth: m.depth, rate: m.rate, sync: m.sync, shape: m.shape });
    r.shapeSel.value = String(m.shape);
    r.rateSel.value = m.sync ?? "1/4";
    setBase(r, m.base);
    sendMod(r);
  });
  $("rate").value = String(st.seq.rate);
  $("seqlen").value = String(st.seq.length);
  seqGate = st.seq.gate;
  st.seq.steps.forEach((s, i) => {
    steps[i] = { ...s };
    sendStep(i);
  });
  sendSeq();
  spaceOn = st.space;
  $("spaceon").setAttribute("aria-pressed", String(spaceOn));
  $("spaceon").textContent = spaceOn ? "ON" : "OFF";
  setSpace();
  if (st.source !== undefined && current !== "file" && current !== "mic") setSource(String(st.source));
  horn = Float32Array.from(st.horn);
  hornWrap = st.hornWrap;
  Object.assign(hornWave, st.hornWave);
  showHornWave();
  sendHorn();
  $("text").value = st.text.words;
  setTextMode(st.text.mode);
  Object.assign(loop, { rate: st.text.rate, sync: st.text.sync });
  sendTextRate(false);
  if (textOn) sing($("text").value);
}

// browser storage: the plug-in replaces this with the host's state
const store = {
  key: "suu.session.v1",
  load() {
    try {
      return JSON.parse(localStorage.getItem(this.key) ?? "null");
    } catch {
      return null;
    }
  },
  save(text) {
    try {
      localStorage.setItem(this.key, text);
    } catch {}
  },
};

const slots = { active: "A", A: null, B: null };
let savedText = "";
function save() {
  slots[slots.active] = getState();
  const text = JSON.stringify(slots);
  if (text !== savedText) store.save((savedText = text));
}
function showSlots() {
  for (const k of ["A", "B"]) $("slot" + k).setAttribute("aria-pressed", String(slots.active === k));
  const other = slots.active === "A" ? "B" : "A";
  $("copyab").textContent = `${slots.active}→${other}`;
  $("copyab").title = `Copy ${slots.active} to ${other}`;
}
function load(st, what) {
  try {
    applyState(readState(st));
    return true;
  } catch (e) {
    status(`${what} not loaded: ${e.message}`);
    return false;
  }
}
for (const k of ["A", "B"]) {
  $("slot" + k).onclick = () => {
    if (slots.active === k) return;
    save();
    slots.active = k;
    // a slot used for the first time starts as a copy of the other
    if (slots[k]) load(slots[k], k);
    showSlots();
    save();
  };
}
$("init").onclick = () => {
  applyState(DEFAULTS);
  save();
  status(`${slots.active}: initial settings`);
};
$("copyab").onclick = () => {
  const other = slots.active === "A" ? "B" : "A";
  slots[other] = getState();
  save();
  status(`${slots.active} copied to ${other}`);
};
$("savefile").onclick = () => {
  const blob = new Blob([JSON.stringify(getState(), null, 1)], { type: "application/json" });
  const a = Object.assign(document.createElement("a"), { href: URL.createObjectURL(blob), download: "suu-preset.json" });
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
};
$("openfile").onclick = () => $("presetfile").click();
async function openPreset(f) {
  let raw;
  try {
    raw = JSON.parse(await f.text());
  } catch {
    return status(`${f.name}: not a preset`);
  }
  if (load(raw, f.name)) {
    status(`${f.name} → ${slots.active}`);
    save();
  }
}
$("presetfile").onchange = (e) => {
  if (e.target.files[0]) openPreset(e.target.files[0]);
  e.target.value = "";
};

{
  const saved = store.load();
  if (saved && typeof saved === "object") {
    slots.active = saved.active === "B" ? "B" : "A";
    slots.A = saved.A ?? null;
    slots.B = saved.B ?? null;
    if (slots[slots.active]) load(slots[slots.active], "the saved settings");
  }
  showSlots();
  setInterval(save, 1000);
  addEventListener("pagehide", save);
}
