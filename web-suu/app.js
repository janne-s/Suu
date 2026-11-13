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
// time knobs that can lock to the tempo, and the division they start from
const SYNCABLE = { delayTime: "1/8." };

const state = {};
const controls = new Map();
const toNorm = (c, v) => (c.curve === "log" ? Math.log(v / c.min) / Math.log(c.max / c.min) : (v - c.min) / (c.max - c.min));
const fromNorm = (c, n) => (c.curve === "log" ? c.min * Math.pow(c.max / c.min, n) : c.min + n * (c.max - c.min));

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
  } else send(c.target, v);
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
    btn.onclick = () => {
      if (c.sync) {
        const secs = eff(key);
        c.sync = null;
        update(key, secs);
      } else {
        c.sync = SYNCABLE[key];
        box.value = c.sync;
        update(key, state[key]);
      }
      btn.setAttribute("aria-pressed", String(!!c.sync));
      input.hidden = !!c.sync;
      box.hidden = !c.sync;
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
function lens(c, cx, cy, w, hTop, hBot, round) {
  const n = 2 + 2.5 * round; // 2 = pointed corners like a lens … 4.5 = an O
  c.beginPath();
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

function drawMouth() {
  const c = mouthEl.getContext("2d");
  const [w, h, dpr] = fit(mouthEl);
  c.clearRect(0, 0, w, h);
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
    const glow = 6 + 18 * clamp(env * 4, 0, 1);
    const beam = color("--beam"), struct = color("--struct");
    c.lineJoin = "round";
    c.shadowColor = beam;
    // outer lips
    c.shadowBlur = glow * dpr;
    c.strokeStyle = beam;
    c.lineWidth = 1.8 * dpr;
    lens(c, cx, cy, (width + 1.4) * unit, (open / 2 + thick) * unit, (open / 2 + thick * 1.1) * unit, round);
    c.stroke();
    // the opening
    if (open > 0.02) {
      c.lineWidth = 1.4 * dpr;
      lens(c, cx, cy, width * unit, (open / 2) * unit, (open / 2) * unit, round);
      c.stroke();
      c.save();
      lens(c, cx, cy, width * unit, (open / 2) * unit, (open / 2) * unit, round);
      c.clip();
      c.shadowBlur = 0;
      // upper teeth, and the tongue rising with the tongue body
      c.strokeStyle = struct;
      c.lineWidth = dpr;
      c.strokeRect(cx - width * 0.32 * unit, cy - (open / 2) * unit - 2, width * 0.64 * unit, 0.45 * unit + 2);
      const tongueTop = cy + (open / 2) * unit - clamp((tbY - 6.2) * 0.9 + 0.3 + jaw * 0.25, 0.1, 2.2) * unit;
      c.strokeStyle = beam;
      c.lineWidth = 1.4 * dpr;
      c.beginPath();
      c.ellipse(cx, tongueTop + 1.4 * unit, width * 0.38 * unit, 1.4 * unit, 0, Math.PI, 2 * Math.PI);
      c.stroke();
      c.restore();
    } else {
      c.beginPath();
      c.moveTo(cx - (width / 2) * unit, cy);
      c.lineTo(cx + (width / 2) * unit, cy);
      c.stroke();
    }
    // nostrils light up as the velum opens
    const nas = clamp((velum + 0.1) / 0.6, 0, 1);
    c.shadowBlur = (4 + 14 * nas) * dpr;
    c.strokeStyle = struct;
    c.globalAlpha = 0.35 + 0.65 * nas;
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
  padAt(e);
});
mouthEl.addEventListener("pointermove", (e) => e.buttons && padAt(e));
$("pad-reset").onclick = () => {
  setBase(rows[0], 0.5);
  setBase(rows[1], 0.6);
};
new ResizeObserver(drawMouth).observe(mouthEl);

let peakHold = 0;
function frameLoop() {
  requestAnimationFrame(frameLoop);
  drawMouth();
  if (ui) {
    // live markers on the Motion rows
    for (const r of rows) {
      r.marker.hidden = r.depth === 0;
      r.marker.style.left = 100 * clamp(baseNorm(r, ui[r.t]), 0, 1) + "%";
      const a = baseNorm(r, r.base) * 100, b = clamp(baseNorm(r, ui[r.t]), 0, 1) * 100;
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
  $("start").textContent = "● ON";
  $("start").setAttribute("aria-pressed", "true");
  status("loading words…", true);
  const gz = await fetch("cmudict.txt.gz");
  const bytes = new Uint8Array(await new Response(gz.body.pipeThrough(new DecompressionStream("gzip"))).arrayBuffer());
  post({ type: "dict", bytes }, [bytes.buffer]);
}
$("start").onclick = start;
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
  } else if (m.type === "ui") {
    ui = m.ui;
    showSeq(ui[19] > 0.5, ui[17]);
  }
}

function pushAll() {
  for (const [key, c] of controls) if (c.target !== "space") send(c.target, state[key]);
  for (const r of rows) {
    send(r.param, r.base);
    sendMod(r);
  }
  send(19, bpm());
  send(1, lastNote);
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
  if (!ctx) return;
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
let current = "2", fileName = "";
async function setSource(src) {
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
  if (f) loadFile(f);
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
addEventListener("keydown", async (e) => {
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
  await start();
  const midi = 48 + 12 * octave + i;
  const legato = down.size > 0;
  down.set(i, midi);
  note(midi);
  if (legato) post({ type: "retrigger" });
  gate();
});
addEventListener("keyup", (e) => {
  const i = KEYS.indexOf(e.key.toLowerCase());
  if (i < 0 || !down.has(i)) return;
  down.delete(i);
  if (down.size) note([...down.values()].pop()); // back to the key still held
  gate();
});
addEventListener("blur", () => {
  down.clear();
  gate();
});
$("hold").onclick = async () => {
  await start();
  held = !held;
  $("hold").setAttribute("aria-pressed", String(held));
  gate();
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
function sendSeq() {
  post({ type: "seq", mode: 0, len: clamp(+$("seqlen").value || 16, 1, 16), div: +$("rate").value, gate: 0.6, order: 0, octaves: 1 });
  renderSteps();
}
function renderSteps() {
  const len = clamp(+$("seqlen").value || 16, 1, 16);
  steps.forEach((s, i) => {
    const c = cells[i];
    c.el.classList.toggle("on", s.on);
    c.el.classList.toggle("tie", s.on && s.tie);
    c.el.classList.toggle("accent", s.on && s.accent);
    c.el.style.setProperty("--len", s.len);
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
      steps[i].len = clamp(Math.round(((ev.clientX - r.left) / r.width) * 20) / 20, 0.05, 1);
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
renderSteps();

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
$("stoptext").onclick = () => sing("");
// typing applies the text by itself once it pauses; Enter or Escape hands the keys back to playing
let typingTimer = 0;
$("text").addEventListener("input", () => {
  clearTimeout(typingTimer);
  typingTimer = setTimeout(() => node && sing($("text").value), 300);
});
$("text").addEventListener("keydown", (e) => {
  if (e.key === "Escape") $("text").blur();
});
document.querySelectorAll("[data-tmode]").forEach((b) => {
  b.onclick = () => {
    textMode = +b.dataset.tmode;
    document.querySelectorAll("[data-tmode]").forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
    if (textOn) sing($("text").value);
  };
});

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
