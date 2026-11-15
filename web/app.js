const $ = (id) => document.getElementById(id);
const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
const css = getComputedStyle(document.documentElement);
const colors = new Map();
const color = (n) => colors.get(n) ?? (colors.set(n, css.getPropertyValue(n).trim()), colors.get(n));
let dirty = true;

const inPlugin = typeof window.sendToPlugin === "function";
const plugin = { ready: false, quiet: false, host: {}, words: null, horn: "", ui: "", frame: 0, typing: false };
let ctx, node, srcNode, fileBuf, micStream, analyser, fx = {};
const wasmModule = inPlugin ? null : WebAssembly.compileStreaming(fetch("suu.wasm"));
function post(m, transfer) {
  if (inPlugin) return pluginPost(m);
  node?.port.postMessage(m, transfer ?? []);
}
function send(id, v) {
  post({ type: "param", id, v });
}
const bpm = () => clamp(+$("bpm").value || 100, 30, 300);

const DIVS = [["1/128", 1 / 32], ["1/64T", 1 / 24], ["1/64", 1 / 16], ["1/32T", 1 / 12], ["1/32", 0.125], ["1/16T", 1 / 6], ["1/16", 0.25], ["1/16.", 0.375], ["1/8T", 1 / 3], ["1/8", 0.5], ["1/8.", 0.75], ["1/4T", 2 / 3], ["1/4", 1], ["1/4.", 1.5], ["1/2", 2], ["1/2.", 3], ["1/1", 4], ["2/1", 8], ["4/1", 16]];
const divBeats = (n) => DIVS.find((d) => d[0] === n)[1];

// ---------------------------------------------------------------- knobs
const pct = (v) => Math.round(v * 100) + "%";
const times = (v) => "×" + v.toFixed(2);
const dbOff = (v) => (v <= -59.95 ? "off" : (v > 0 ? "+" : "") + v.toFixed(1) + " dB");
const secsFmt = (v) => (v < 1 ? (v * 1000).toFixed(0) + " ms" : v.toFixed(2) + " s");
const shortSecs = (v) => (v < 0.01 ? (v * 1000).toFixed(1) + "ms" : v < 1 ? (v * 1000).toFixed(0) + "ms" : v.toFixed(v < 10 ? 2 : 1) + "s");
const INT = new Set(["coarse", "stereoCanon"]);
const KNOBS = {
  "k-mouth": [
    ["size", "SIZE", 0.4, 4, 1, times, "log", 18, [0.7, 1.25]],
    ["breath", "BREATH", 0, 6, 1, times, null, 5, [0, 2]],
    ["follow", "FOLLOW", 0, 1, 0.7, pct, null, 6],
  ],
  "k-source": [
    ["osc", "OSC", -60, 12, 0, dbOff, null, 36],
    ["input", "INPUT", -60, 12, 0, dbOff, null, 37],
    ["folds", "FOLDS", 0, 2, 0, pct, null, 13, [0, 1]],
    ["reson", "RESON", 0, 1, 0, (v) => (v < 0.005 ? "off" : pct(v)), null, 24],
  ],
  "k-air": [
    ["aspiration", "ASPIR", 0, 10, 1, pct, "sq", 65],
    ["frication", "FRIC", 0, 10, 1, pct, "sq", 66],
    ["sibilance", "SIBIL", 0, 10, 1, pct, "sq", 67],
    ["whistle", "WHISTLE", 0, 1, 1, pct, null, 78],
    ["move", "MOVE", 0, 8, 1, pct, "sq", 79],
  ],
  "k-pitch": [
    ["glide", "GLIDE", 0, 2, 0.03, (v) => (v < 0.0005 ? "off" : v < 1 ? (v * 1000).toFixed(0) + " ms" : v.toFixed(2) + " s"), "sq", 20],
    ["coarse", "TUNE", -48, 48, 0, (v) => (Math.round(v) > 0 ? "+" : "") + Math.round(v) + " st", null, 53],
    ["steady", "STEADY", 0, 4, 0, (v) => (v < 0.0005 ? "off" : shortSecs(v)), "sq", 54],
  ],
  "k-gate": [
    ["envThresh", "THRESH", -80, 0, -40, (v) => v.toFixed(0) + " dB", null, 40],
    ["envRelease", "HANG", 0.005, 4, 0.15, secsFmt, "log", 41],
  ],
  "k-env": [
    ["attack", "A", 0.001, 10, 0.005, shortSecs, "log", 45],
    ["decay", "D", 0.001, 10, 0.2, shortSecs, "log", 47],
    ["sustain", "S", 0, 1, 1, pct, null, 49],
    ["release", "R", 0.001, 10, 0.08, shortSecs, "log", 50],
  ],
  "k-drive": [
    ["drive", "DRIVE", -40, 40, 0, (v) => v.toFixed(1) + " dB", null, 3],
    ["tilt", "TILT", 0, 2000, 150, (v) => (v < 1 ? "off" : v.toFixed(0) + " Hz"), null, 4],
    ["sub", "SUB", 0, 1, 0, pct, null, 15],
    ["subhz", "SUB HZ", 30, 400, 120, (v) => v.toFixed(0) + " Hz", "log", 14],
  ],
  "k-stereo": [
    ["stereoSpread", "SPREAD", 0.7, 1.4, 1.12, times, null, 75],
    ["stereoVowel", "VOWEL", -0.5, 0.5, 0.12, (v) => (v > 0.005 ? "+" : "") + Math.round(v * 100) + "%", null, 76],
    ["stereoWidth", "WIDTH", 0, 1, 1, pct, null, 77],
    ["stereoPhase", "PHASE", 0, 1, 0, (v) => (v < 0.005 ? "off" : Math.round(v * 360) + "°"), null, 81],
    ["stereoCanon", "CANON", 0, 16, 0, (v) => (v < 0.5 ? "off" : "+" + Math.round(v)), null, 80],
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
const KNOB_TIPS = {
  size: "Size\nThe mouth's size: small is bright and high, large dark and low.",
  osc: "Osc\nThe oscillator's level into the mouth.",
  input: "Input\nThe file's or mic's level into the mouth (in the plug-in: the track).",
  folds: "Folds\nThe model's own vocal folds at the played note.",
  stereoSpread: "Spread\nThe right mouth's size against the left's.",
  stereoVowel: "Vowel\nThe right mouth's vowel, further back (+) or front (−).",
  stereoWidth: "Width\nHow far apart the two mouths sound. 0%: both in the middle.",
  aspiration: "Aspiration\nThe breath noise at the folds: h and a breathy edge.",
  frication: "Frication\nThe noise at the mouth's narrowest point: f, sh, ch.",
  sibilance: "Sibilance\nThe noise where the air hits the teeth: s and z.",
  move: "Move\nThe sound of the mouth changing shape: the air its moving walls push. 0%: silent moves.",
  stereoPhase: "Phase\nThe right mouth's Motion runs this far ahead in its cycle.",
  stereoCanon: "Canon\nThe right mouth says the text this many parts behind the left.",
  whistle: "Whistle\nThe ringing of a mouth closed at the tongue and the lips. Lower tames it.",
  breath: "Breath\nLung pressure: the air in the consonants and the folds.",
  follow: "Follow\nHow far the breath follows the sound's level (0: steady air).",
  reson: "Reson\nA resonator tuned to the played note: gives a file or mic pitch. 0: off.",
  glide: "Glide\nHow long pitch and vowel changes take. 0: at once.",
  drive: "Drive\nLevel into the mouth.",
  tilt: "Tilt\nDarkens the source before the mouth. 0: off.",
  sub: "Sub\nAdds the dry source's low end.",
  subhz: "Sub Hz\nWhere the sub's low-pass sits.",
  delayTime: "Time\nDelay time.",
  delayFb: "Feedback\nDelay repeats.",
  delayMix: "Delay\nDelay level.",
  revMix: "Reverb\nReverb level.",
  revSize: "Length\nReverb length.",
  mix: "Mix\nThe dry source … the mouth.",
  envThresh: "Threshold\nWith ENV on: the input's level that opens the sound.",
  envRelease: "Hang\nWith ENV on: how long the sound stays open after the input falls below the threshold.",
  attack: "Attack\nHow long the sound takes to open, from where it is. ♪: a note value.",
  decay: "Decay\nHow long it takes from the top down to Sustain. ♪: a note value.",
  sustain: "Sustain\nThe level it holds while the note is on.",
  release: "Release\nHow long it takes to fall silent after the note ends. ♪: a note value.",
  coarse: "Tune (coarse)\nTransposes the pitch in semitones: the note's, or the input's (PITCH IN).",
  steady: "Steady\nWith PITCH IN: a new pitch is taken once the input has stayed on it this long. Quick changes and slides between notes are skipped. ♪: a note value.",
  out: "Output\nOutput level.",
};
const SYNCABLE = { delayTime: { div: "1/8." }, glide: { div: "1/16", id: 21 }, envRelease: { div: "1/8", id: 42 }, attack: { div: "1/16", id: 46 }, decay: { div: "1/8", id: 48 }, release: { div: "1/8", id: 51 }, steady: { div: "1/16", id: 55 } };

const state = {};
const controls = new Map();
const toNorm = (c, v) =>
  c.curve === "log" ? Math.log(v / c.min) / Math.log(c.max / c.min) : c.curve === "sq" ? Math.sqrt((v - c.min) / (c.max - c.min)) : (v - c.min) / (c.max - c.min);
const fromNorm = (c, n) => (c.curve === "log" ? c.min * Math.pow(c.max / c.min, n) : c.curve === "sq" ? c.min + n * n * (c.max - c.min) : c.min + n * (c.max - c.min));

function eff(key) {
  const c = controls.get(key);
  return c.sync ? clamp((divBeats(c.sync) * 60) / bpm(), 0.01, 2) : state[key];
}

let sizeSent = 0;
function update(key, v, final = true) {
  const c = controls.get(key);
  v = clamp(v, c.min, c.max);
  if (INT.has(key)) v = Math.round(v);
  state[key] = v;
  const n = toNorm(c, v);
  c.input.value = n;
  c.input.style.setProperty("--turn", `${270 * n}deg`);
  c.output.textContent = c.fmt(c.sync ? eff(key) : v);
  c.cell.classList.toggle("beyond", !!c.anat && (v < c.anat[0] - 1e-9 || v > c.anat[1] + 1e-9));
  c.cell.dataset.tip = KNOB_TIPS[key] ?? c.label;
  if (c.target === "space") setSpace();
  else if (c.target === 18) {
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
  output.dataset.tip = "Value\nClick to type one.";
  output.onclick = () => {
    const raw = prompt(`${label} (${min} … ${max})`, String(+state[key].toFixed(3)));
    if (raw !== null && Number.isFinite(+raw)) update(key, +raw);
  };
  if (SYNCABLE[key]) {
    const btn = document.createElement("button");
    btn.className = "sync";
    btn.textContent = "♪";
    btn.dataset.tip = "Sync\nA note value at the tempo instead of a time.";
    btn.setAttribute("aria-pressed", "false");
    const box = document.createElement("select");
    box.className = "field syncbox";
    box.hidden = true;
    box.setAttribute("aria-label", `${label} as a note value`);
    box.innerHTML = DIVS.filter((d) => d[1] <= 4).map(([n]) => `<option>${n}</option>`).join("");
    input.after(box);
    cell.classList.add("syncable");
    cell.append(btn);
    if (key === "glide") {
      const dir = document.createElement("button");
      dir.id = "glidedir";
      dir.className = "sync dir";
      dir.addEventListener("click", () => setGlideDir((glideDir + 1) % 3));
      cell.append(dir);
    }
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
let glideDir = 0;
function setGlideDir(d) {
  glideDir = d;
  send(35, d);
  const b = $("glidedir");
  b.textContent = ["↕", "↑", "↓"][d];
  const said = ["Glide both ways", "Glide up only (down jumps)", "Glide down only (up jumps)"][d];
  b.dataset.tip = `Direction\n${said}. Click to change.`;
  b.setAttribute("aria-label", said);
  b.setAttribute("aria-pressed", String(d !== 0));
}
for (const [parent, list] of Object.entries(KNOBS)) for (const k of list) knob(parent, k);
setGlideDir(0);

// ---------------------------------------------------------------- Motion
const MOTION = [
  [0, "VOWEL X", 7, -0.3, 1.3, 0.5, (v) => (v < 0.15 ? "front" : v > 0.85 ? "back" : pct(v))],
  [1, "VOWEL Y", 8, -0.3, 1.3, 0.6, (v) => (v < 0.15 ? "close" : v > 0.85 ? "open" : pct(v))],
  [2, "ROUND", 9, -1, 1, 0, (v) => (v < 0 ? "spread " + pct(-v) : pct(v))],
  [3, "NASAL", 10, 0, 1.5, 0, pct],
  [4, "INJECT", 11, 0, 1, 0, (v) => (v < 0.02 ? "throat" : v > 0.98 ? "lips" : pct(v))],
  [5, "DAMPING", 12, 0.1, 20, 1, times, true],
  [6, "MORPH", 25, 0, 1, 0, (v) => (v < 0.005 ? "mouth" : v > 0.995 ? "horn" : pct(v))],
  [7, "SHIFT", 26, -0.5, 0.5, 0, (v) => (Math.abs(v) < 0.005 ? "0" : (v > 0 ? "lips " : "throat ") + pct(Math.abs(v)))],
  [8, "OPEN", 27, 0.25, 4, 1, times, true],
];
const SHAPES = ["SIN", "TRI", "SAW", "SQR", "RND", "ENV", "ADSR"];
const MOTION_TIPS = [
  "Vowel X\nThe vowel from front to back (i … u).",
  "Vowel Y\nThe vowel from close to open (i … a).",
  "Round\nThe lips from spread to rounded and pushed out.",
  "Nasal\nOpens the way into the nose: a nasal tone.",
  "Inject\nWhere the sound enters: the throat filters fully, near the lips less.",
  "Damping\nThe resonances: low rings, high muffles.",
  "Morph\nFrom the mouth (the pad's vowel) to the drawn horn.",
  "Shift\nSlides the horn's shape along the tract (a wave wraps round).",
  "Open\nWidens or narrows the whole horn.",
];
const rows = [];
const baseNorm = (r, v) => (r.log ? Math.log(v / r.lo) / Math.log(r.hi / r.lo) : (v - r.lo) / (r.hi - r.lo));
const baseFrom = (r, n) => (r.log ? r.lo * Math.pow(r.hi / r.lo, n) : r.lo + n * (r.hi - r.lo));

function setBase(r, v, sendIt = true) {
  dirty = true;
  r.base = clamp(v, r.lo, r.hi);
  r.input.value = baseNorm(r, r.base);
  r.out.textContent = r.fmt(r.base);
  if (sendIt) send(r.param, r.base);
}
function sendMod(r) {
  const env = r.shape === 5;
  post({ type: "mod", target: r.t, depth: r.depth, rate: r.rate, sync: r.sync ? divBeats(r.sync) : 0, shape: r.shape, smooth: r.smooth });
  r.smoothBox.textContent = Math.round(r.smooth * 100) + "%";
  r.smoothBox.classList.toggle("idle", ![2, 3, 4].includes(r.shape));
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
    <span class="rate"><span class="field drag val" tabindex="0" aria-label="${label} rate"></span><select class="field" hidden aria-label="${label} rate as a note value">${DIVS.map(([n]) => `<option>${n}</option>`).join("")}</select><button data-tip="Sync\nThe rate as a note value at the tempo.">♪</button></span>
    <select class="field shape" aria-label="${label} shape">${SHAPES.map((s, i) => `<option value="${i}">${s}</option>`).join("")}</select>
    <span class="field drag smooth" tabindex="0" aria-label="${label} smooth" data-tip="Smooth\nRandom, square and saw: from sharp jumps (0) to glides (100%). Double-click: 0."></span>`;
  $("motion").append(el);
  el.querySelector(".name").dataset.tip = MOTION_TIPS[t];
  el.querySelector(".base").dataset.tip = "Base\nWhere it rests; Motion moves around it. Double-click: default.";
  el.querySelector(".depth").dataset.tip = "Depth\nHow far it swings, either way. Drag; double-click: 0.";
  el.querySelector(".rate .val").dataset.tip = "Rate\nCycles per second. Drag.";
  el.querySelector(".rate select").dataset.tip = "Rate\nOne cycle per this note value.";
  el.querySelector("select.shape").dataset.tip = "Shape\nSine, triangle, saw, square, random, the sound's envelope (ENV), or the ADSR: each note moves it.";
  const r = {
    t, param, lo, hi, fmt, log, el, def, base: def, depth: 0, rate: 0.5, sync: null, shape: 0, smooth: 0,
    smoothBox: el.querySelector(".smooth"),
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
  dragValue(r.smoothBox, () => r.smooth, (v) => {
    r.smooth = Math.round(clamp(v, 0, 1) * 100) / 100;
    sendMod(r);
  }, 1 / 150);
  r.smoothBox.ondblclick = () => {
    r.smooth = 0;
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

const narrow = matchMedia("(max-width: 900px)");
function showFold() {
  const open = !narrow.matches || !$("motion-panel").classList.contains("folded");
  $("motion-fold").setAttribute("aria-expanded", String(open));
  $("motion-fold").tabIndex = narrow.matches ? 0 : -1;
}
$("motion-fold").onclick = () => {
  $("motion-panel").classList.toggle("folded");
  showFold();
};
narrow.addEventListener("change", showFold);
showFold();

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

function lens(c, cx, cy, w, hTop, hBot, round, more = false) {
  const n = 2 + 2.5 * round;
  if (!more) c.beginPath();
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
const HORN_POINTS = 32, HORN_MAX = 20, UI_AREAS = 48;
const defaultHorn = () => Float32Array.from({ length: HORN_POINTS }, (_, i) => {
  const u = i / (HORN_POINTS - 1);
  return 0.6 + 1.5 * Math.exp(-(((u - 0.3) / 0.12) ** 2)) + 9 * u ** 3;
});
let horn = defaultHorn(), view = "pad";
let hornWrap = false;
const hornWave = { shape: -1, cycles: 1, depth: 0.6 };
function sendHorn() {
  dirty = true;
  post({ type: "profile", areas: horn.slice(), wrap: hornWrap });
}
const hornU = (i) => i / (hornWrap ? HORN_POINTS : HORN_POINTS - 1);
let rndSteps = [];
const RND_STEPS = 8;
let snapVals = [];
let drawn = { mean: 3, w: new Array(HORN_POINTS).fill(0) };
function captureDrawn() {
  if (hornWave.depth === 0) hornWave.depth = 1;
  const mean = horn.reduce((a, b) => a + b, 0) / horn.length;
  drawn = { mean, w: Array.from(horn, (a) => (mean > 0 ? Math.round(((a - mean) / (mean * hornWave.depth)) * 1e4) / 1e4 : 0)) };
  hornWave.cycles = 1;
}
function buildWave(newRandom = false) {
  const { shape, cycles, depth } = hornWave;
  if (shape < 0) {
    horn = Float32Array.from({ length: HORN_POINTS }, (_, i) => {
      const f = ((cycles * i) / HORN_POINTS) % 1;
      return clamp(drawn.mean * (1 + depth * drawn.w[Math.floor(f * HORN_POINTS)]), 0, HORN_MAX);
    });
    return sendHorn();
  }
  if (newRandom || rndSteps.length !== RND_STEPS) rndSteps = Array.from({ length: RND_STEPS }, () => Math.random() * 2 - 1);
  const steps = rndSteps;
  horn = Float32Array.from({ length: HORN_POINTS }, (_, i) => {
    const ph = (cycles * i) / HORN_POINTS, f = ph % 1;
    const w = [Math.sin(2 * Math.PI * ph), 1 - 4 * Math.abs(f - 0.5), 2 * f - 1, f < 0.5 ? 1 : -1, steps[Math.floor(f * RND_STEPS)], snapVals[Math.floor(f * HORN_POINTS)] ?? 0][shape];
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
  captureDrawn();
  showHornWave();
  sendHorn();
}
function bringHornIn() {
  if (rows[6] && rows[6].base === 0) setBase(rows[6], 1);
}
$("horn-reset").onclick = resetHorn;
$("horn-shape").onchange = () => {
  hornWave.shape = +$("horn-shape").value;
  if (hornWave.shape < 0) captureDrawn();
  if (hornWave.shape === 5 && !snapVals.length) return post({ type: "snapshot" });
  if (hornWave.shape >= 0) buildWave(true);
  showHornWave();
};
$("horn-snap").onclick = async () => {
  await start();
  post({ type: "snapshot" });
};
function onSnapshot(values) {
  if (!values) {
    status("snapshot: no sound (play a note, or start the file)");
    if (hornWave.shape === 5 && !snapVals.length) hornWave.shape = -1;
    return showHornWave();
  }
  snapVals = Array.from(values, (v) => Math.round(v * 1000) / 1000);
  hornWave.shape = 5;
  buildWave();
  showHornWave();
}
dragValue($("horn-cycles"), () => hornWave.cycles, (v) => {
  const c = Math.round(clamp(v, 1, 8));
  if (c === hornWave.cycles) return;
  hornWave.cycles = c;
  buildWave();
  showHornWave();
}, 1 / 25);
dragValue($("horn-depth"), () => hornWave.depth, (v) => {
  const d = Math.round(clamp(v, 0, 2) * 100) / 100;
  if (d === hornWave.depth) return;
  hornWave.depth = d;
  buildWave();
  showHornWave();
}, 1 / 150);
captureDrawn();
showHornWave();
function hornFrame(w, h) {
  const x0 = w * 0.07, x1 = w * 0.93, cy = h * 0.5;
  const perCm = (h * 0.44) / Math.sqrt(HORN_MAX / Math.PI);
  return { x0, x1, cy, x: (u) => x0 + u * (x1 - x0), r: (a) => Math.sqrt(Math.max(0, a) / Math.PI) * perCm, perCm };
}
function drawHorn(c, w, h, dpr) {
  const f = hornFrame(w, h);
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
    if (hornWrap) c.lineTo(f.x(1), f.cy + side * f.r(horn[0]));
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
  const i = hornWrap ? Math.min(HORN_POINTS - 1, Math.round(u * HORN_POINTS)) : Math.round(u * (HORN_POINTS - 1));
  const cm = Math.abs(y - f.cy) / f.perCm;
  const a = clamp(Math.PI * cm * cm, 0, HORN_MAX);
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
  dirty = true;
  view = v;
  $("view-pad").setAttribute("aria-pressed", String(v === "pad"));
  $("view-horn").setAttribute("aria-pressed", String(v === "horn"));
  mouthEl.style.cursor = v === "horn" ? "cell" : "";
  $("horn-bar").hidden = v !== "horn";
  $("text-panel").hidden = true;
  $("text-toggle").setAttribute("aria-expanded", "false");
}
$("view-pad").onclick = () => setView("pad");
$("view-horn").onclick = () => setView("horn");

function drawMouth() {
  const c = mouthEl.getContext("2d");
  const [w, h, dpr] = fit(mouthEl);
  c.clearRect(0, 0, w, h);
  if (view === "horn") return drawHorn(c, w, h, dpr);
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

  if (ui) {
    const [lipAp, lipWidth, lipProt, jaw, , tbY, velum, lipArea, env] = ui.slice(8, 17);
    const unit = Math.min(w, h) / 9;
    const cx = w / 2, cy = h * 0.5;
    const width = 4.6 * (0.78 + 0.45 * lipWidth) * (1 - 0.38 * clamp(lipProt, 0, 1));
    const open = clamp((4 * Math.max(0, lipArea)) / (Math.PI * width), 0, 4);
    const round = clamp(lipProt, 0, 1);
    const thick = 0.75 + 0.5 * round;
    const lit = clamp(env * 4, 0, 1);
    const glow = 8 + 22 * lit;
    const lips = color("--lips"), rgb = color("--lips-rgb");
    const rose = (a) => `rgba(${rgb}, ${a})`;
    const outer = () => lens(c, cx, cy, (width + 1.4) * unit, (open / 2 + thick) * unit, (open / 2 + thick * 1.1) * unit, round);
    const mouth = (more = false) => lens(c, cx, cy, width * unit, (open / 2) * unit, (open / 2) * unit, round, more);
    const top = cy - (open / 2 + thick) * unit, bottom = cy + (open / 2 + thick * 1.1) * unit;
    c.lineJoin = "round";
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
    c.save();
    c.shadowBlur = 0;
    outer();
    if (open > 0.02) mouth(true);
    c.clip("evenodd");
    const sheen = (y, rx, ry, a) => {
      c.save();
      c.translate(cx, y);
      c.scale(1, ry / rx);
      const g = c.createRadialGradient(0, 0, 0, 0, 0, rx);
      g.addColorStop(0, `rgba(255, 228, 236, ${a})`);
      g.addColorStop(1, "rgba(255, 228, 236, 0)");
      c.fillStyle = g;
      c.beginPath();
      c.arc(0, 0, rx, 0, 2 * Math.PI);
      c.fill();
      c.restore();
    };
    sheen(cy + (open / 2 + thick * 0.5) * unit, width * 0.32 * unit, thick * 0.45 * unit, 0.13 + 0.07 * lit);
    sheen(cy - (open / 2 + thick * 0.55) * unit, width * 0.26 * unit, thick * 0.35 * unit, 0.06 + 0.04 * lit);
    c.restore();
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
let stroke = false;
mouthEl.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  stroke = true;
  mouthEl.setPointerCapture(e.pointerId);
  if (view === "horn") {
    bringHornIn();
    hornWave.shape = -1;
    showHornWave();
    hornLast = -1;
    hornAt(e);
  } else padAt(e);
});
mouthEl.addEventListener("pointermove", (e) => {
  if (stroke && e.buttons & 1) view === "horn" ? hornAt(e) : padAt(e);
});
function endStroke() {
  if (!stroke) return;
  stroke = false;
  if (view === "horn" && hornLast >= 0) {
    captureDrawn();
    showHornWave();
  }
  hornLast = -1;
}
for (const ev of ["pointerup", "pointercancel", "lostpointercapture"]) mouthEl.addEventListener(ev, endStroke);
mouthEl.addEventListener("dblclick", () => view === "horn" && resetHorn());
new ResizeObserver(() => {
  dirty = true;
  drawMouth();
}).observe(mouthEl);

let peakHold = 0, scope = null;
function frameLoop() {
  requestAnimationFrame(frameLoop);
  if (inPlugin) pluginSync();
  if (!dirty) return;
  dirty = false;
  drawMouth();
  if (ui) {
    for (const r of rows) {
      const live = ui[r.t < 6 ? r.t : 14 + r.t];
      const a = baseNorm(r, r.base) * 100, b = clamp(baseNorm(r, live), 0, 1) * 100;
      const key = `${r.depth === 0} ${b} ${a}`;
      if (key === r.shown) continue;
      r.shown = key;
      r.marker.hidden = r.depth === 0;
      r.marker.style.left = b + "%";
      r.input.style.setProperty("--lo", Math.min(a, b) + "%");
      r.input.style.setProperty("--hi", Math.max(a, b) + "%");
    }
  }
  if (analyser) {
    const t = (scope ??= new Float32Array(analyser.fftSize));
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
let booting = null;
function start() {
  if (inPlugin) return Promise.resolve();
  wakeAudio();
  return (booting ??= boot());
}
let silentLoop = null;
function silentWav() {
  const n = 4800, b = new DataView(new ArrayBuffer(44 + 2 * n));
  const text = (o, s) => [...s].forEach((c, i) => b.setUint8(o + i, c.charCodeAt(0)));
  text(0, "RIFF"); b.setUint32(4, 36 + 2 * n, true); text(8, "WAVEfmt ");
  b.setUint32(16, 16, true); b.setUint16(20, 1, true); b.setUint16(22, 1, true);
  b.setUint32(24, 48000, true); b.setUint32(28, 96000, true); b.setUint16(32, 2, true); b.setUint16(34, 16, true);
  text(36, "data"); b.setUint32(40, 2 * n, true);
  return URL.createObjectURL(new Blob([b], { type: "audio/wav" }));
}
function wakeAudio() {
  if (navigator.audioSession) navigator.audioSession.type = "playback";
  else if (!silentLoop && /iPhone|iPad|iPod/.test(navigator.userAgent)) {
    silentLoop = new Audio(silentWav());
    silentLoop.loop = true;
    silentLoop.setAttribute("playsinline", "");
    silentLoop.play().catch(() => (silentLoop = null));
  }
  if (ctx && ctx.state !== "running") ctx.resume().catch(() => {});
}
async function boot() {
  status("loading…", true);
  ctx = new AudioContext({ latencyHint: "interactive" });
  const [mod] = await Promise.all([wasmModule, ctx.audioWorklet.addModule("worklet.js")]);
  node = new AudioWorkletNode(ctx, "suu", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2], processorOptions: { module: mod } });
  node.port.onmessage = (e) => onWorklet(e.data);
  buildSpace();
  analyser = ctx.createAnalyser();
  analyser.fftSize = 2048;
  fx.master.connect(analyser);
  analyser.connect(ctx.destination);
  window.__suu = { ctx, analyser, node };
  pushAll();
  setOsc(oscWave);
  await setInput(inputKind);
  status("loading words…", true);
  const gz = await fetch("cmudict.txt.gz");
  const bytes = new Uint8Array(await new Response(gz.body.pipeThrough(new DecompressionStream("gzip"))).arrayBuffer());
  post({ type: "dict", bytes }, [bytes.buffer]);
}
for (const ev of ["pointerdown", "pointerup", "touchend", "click", "keydown"]) addEventListener(ev, () => start(), { capture: true });
let statusTimer = 0;
function status(t, keep = false) {
  const el = $("status");
  el.textContent = t;
  el.classList.remove("fade");
  clearTimeout(statusTimer);
  if (!keep) statusTimer = setTimeout(() => el.classList.add("fade"), 4000);
}

let lastSyllables = 0;
function countUnits() {
  textUnits = lastSyllables > 0 ? (tx.unit === 1 ? ($("text").value.match(/[A-Za-z']+/g) ?? []).length : lastSyllables) : 0;
  setCanonRange();
}
function onWorklet(m) {
  if (m.type === "dict") status("ready");
  else if (m.type === "snapshot") onSnapshot(m.values);
  else if (m.type === "text") {
    textOn = m.syllables > 0;
    lastSyllables = m.syllables;
    countUnits();
    $("text-toggle").textContent = textOn ? `TEXT · ${m.syllables}` : "TEXT";
    playLabel($("singtext"), textOn, "SING");
    $("singtext").setAttribute("aria-pressed", String(textOn));
  } else if (m.type === "ui") {
    ui = m.ui;
    dirty = true;
    showSeq(ui[19] > 0.5, ui[17]);
    if (pitchFrom) showRoot();
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
  send(35, glideDir);
  send(39, envOn ? 1 : 0);
  send(43, pitchFrom);
  send(44, vowelIn ? 1 : 0);
  send(52, legato ? 1 : 0);
  send(56, hq ? 1 : 0);
  send(68, lofi ? 1 : 0);
  send(74, stereoOn ? 1 : 0);
  send(82, stereoMirror ? 1 : 0);
  send(57, grid ? divBeats(grid) : 0);
  send(61, gridBreath);
  send(62, gridSmooth);
  send(58, scale);
  send(59, jump);
  send(60, jumpFold ? 1 : 0);
  sendHorn();
  gate();
  sendText(false);
  setSpace();
  steps.forEach((_, i) => sendStep(i));
  sendSeq();
  post({ type: "play", on: playing });
}

function buildSpace() {
  const m = (fx.master = ctx.createGain());
  node.connect(m);
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
let stereoOn = false;
function setStereo(on) {
  stereoOn = on;
  $("stereoon").setAttribute("aria-pressed", String(on));
  $("stereoon").textContent = on ? "ON" : "OFF";
  send(74, on ? 1 : 0);
}
$("stereoon").onclick = () => setStereo(!stereoOn);
let stereoMirror = false;
function setMirror(on) {
  stereoMirror = on;
  $("stereomirror").setAttribute("aria-pressed", String(on));
  send(82, on ? 1 : 0);
}
$("stereomirror").onclick = () => setMirror(!stereoMirror);
let textUnits = 0;
function setCanonRange() {
  const c = controls.get("stereoCanon");
  c.max = textUnits > 1 ? textUnits - 1 : 16;
  update("stereoCanon", state.stereoCanon);
}
let spaceOn = true;
$("spaceon").onclick = () => {
  spaceOn = !spaceOn;
  $("spaceon").setAttribute("aria-pressed", String(spaceOn));
  $("spaceon").textContent = spaceOn ? "ON" : "OFF";
  setSpace();
};

// ---------------------------------------------------------------- sources
let oscWave = "2", inputKind = null, fileName = "", autoHold = false;
function setOsc(w) {
  oscWave = w;
  document.querySelectorAll("[data-osc]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.osc === w)));
  send(0, +w);
}
async function setInput(kind) {
  if (kind && !inputKind && !held) {
    setHold(true);
    autoHold = true;
  } else if (!kind && inputKind && autoHold) {
    setHold(false);
    autoHold = false;
  }
  inputKind = kind;
  document.querySelectorAll("[data-in]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.in === kind)));
  if (kind === "file" && fileName) status(fileName);
  if (!ctx) return;
  srcNode?.disconnect();
  try {
    srcNode?.stop?.();
  } catch {}
  srcNode = null;
  if (kind === "file" && fileBuf) {
    srcNode = ctx.createBufferSource();
    srcNode.buffer = fileBuf;
    srcNode.loop = true;
    srcNode.connect(node);
    srcNode.start();
  } else if (kind === "mic") {
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
document.querySelectorAll("[data-osc]").forEach((b) => {
  b.onclick = async () => {
    await start();
    setOsc(b.dataset.osc);
  };
});
document.querySelectorAll("[data-in]").forEach((b) => {
  b.onclick = async () => {
    if (b.dataset.in === "file" && !fileBuf) return $("file").click();
    await start();
    setInput(inputKind === b.dataset.in ? null : b.dataset.in);
  };
});
async function loadFile(f) {
  await start();
  try {
    fileBuf = await ctx.decodeAudioData(await f.arrayBuffer());
    fileName = f.name;
    const fb = document.querySelector('[data-in="file"]');
    fb.dataset.tip = TIPS['[data-in="file"]'] + "\nLoaded: " + f.name;
    await setInput("file");
    status(f.name);
  } catch {
    status(`could not decode ${f.name}`);
  }
}
$("file").onchange = (e) => {
  const f = e.target.files[0];
  e.target.value = "";
  if (f) loadFile(f);
};
$("pickfile").onclick = () => $("file").click();
addEventListener("dragover", (e) => e.preventDefault());
addEventListener("drop", (e) => {
  e.preventDefault();
  const f = e.dataTransfer.files[0];
  if (f) /\.json$/i.test(f.name) ? openPreset(f) : !inPlugin && loadFile(f);
});

// ---------------------------------------------------------------- keyboard
const KEYS = "awsedftgyhujk";
const NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
let octave = 0, held = false, lastNote = 45;
const down = new Map();
const noteName = (m) => NAMES[((m % 12) + 12) % 12] + (Math.floor(m / 12) - 1);
function note(midi) {
  lastNote = clamp(midi, 12, 108);
  send(1, lastNote);
  showRoot();
}
function showRoot() {
  const t = pitchFrom && ui && ui.length > 23 + UI_AREAS ? ui[23 + UI_AREAS] : 0;
  const m = t > 0 ? Math.round(t) : lastNote;
  $("notename").value = String(((m % 12) + 12) % 12);
  $("rootoct").textContent = String(Math.floor(m / 12) - 1);
}
let pitchFrom = 0;
function setPitchFrom(v) {
  pitchFrom = v;
  $("pitchfrom").value = String(v);
  $("pitchfrom").closest(".keys").classList.toggle("tracking", v > 0);
  send(43, v);
  showRoot();
}
$("pitchfrom").onchange = () => setPitchFrom(+$("pitchfrom").value);
let legato = false;
function setLegato(on) {
  legato = on;
  $("legato").textContent = on ? "LEGATO" : "RETRIG";
  $("legato").setAttribute("aria-pressed", String(on));
  send(52, on ? 1 : 0);
}
$("legato").onclick = () => setLegato(!legato);
let scale = 0xfff, jump = 0, jumpFold = true;
const SCALES = { all: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], maj: [0, 2, 4, 5, 7, 9, 11], min: [0, 2, 3, 5, 7, 8, 10], penta: [0, 2, 4, 7, 9], minpenta: [0, 3, 5, 7, 10] };
NAMES.forEach((n, i) => {
  const b = document.createElement("button");
  b.textContent = n;
  b.dataset.pc = String(i);
  b.onclick = () => setScale(scale ^ (1 << i));
  $("scale-notes").append(b);
});
function setScale(mask) {
  scale = mask & 0xfff;
  for (const b of $("scale-notes").children) b.setAttribute("aria-pressed", String(((scale >> +b.dataset.pc) & 1) === 1));
  send(58, scale);
}
for (const b of document.querySelectorAll("[data-preset]")) {
  b.onclick = () => setScale(SCALES[b.dataset.preset].reduce((m, k) => m | (1 << ((k + lastNote) % 12)), 0));
}
function setJump(st, fold) {
  jump = clamp(Math.round(st), 0, 48);
  jumpFold = fold;
  $("jump").textContent = jump === 0 ? "any" : "±" + jump + " st";
  for (const b of document.querySelectorAll("[data-jump]")) b.setAttribute("aria-pressed", String((b.dataset.jump === "1") === fold));
  send(59, jump);
  send(60, fold ? 1 : 0);
}
dragValue($("jump"), () => jump / 48, (n) => setJump(clamp(n, 0, 1) * 48, jumpFold));
$("jump").ondblclick = () => setJump(0, jumpFold);
for (const b of document.querySelectorAll("[data-jump]")) b.onclick = () => setJump(jump, b.dataset.jump === "1");
setScale(scale);
setJump(jump, jumpFold);
function showPitchPanel(open) {
  const panel = $("pitch-panel");
  if (open) {
    const r = $("pitch-more").getBoundingClientRect();
    panel.style.left = Math.max(8, Math.min(r.left, innerWidth - 320)) + "px";
  }
  panel.hidden = !open;
  $("pitch-more").setAttribute("aria-expanded", String(open));
}
$("pitch-more").onclick = (e) => {
  e.stopPropagation();
  showPitchPanel($("pitch-panel").hidden);
};
addEventListener("pointerdown", (e) => {
  if (!$("pitch-panel").hidden && !$("pitch-panel").contains(e.target) && e.target !== $("pitch-more")) showPitchPanel(false);
});
const EXPR_TARGETS = ["OFF", "VOWEL X", "VOWEL Y", "ROUND", "NASAL", "INJECT", "DAMPING", "MORPH", "SHIFT", "OPEN", "BREATH", "LEVEL"];
const mpe = { pressureTo: 9, pressureDepth: 0.5, slideTo: 1, slideDepth: 0.5, bend: 2 };
for (const id of ["mpe-pressure-to", "mpe-slide-to"]) $(id).innerHTML = EXPR_TARGETS.map((n, i) => `<option value="${i - 1}">${n}</option>`).join("");
function showMpe() {
  $("mpe-pressure-to").value = String(mpe.pressureTo);
  $("mpe-slide-to").value = String(mpe.slideTo);
  $("mpe-pressure-depth").textContent = Math.round(mpe.pressureDepth * 100) + "%";
  $("mpe-slide-depth").textContent = Math.round(mpe.slideDepth * 100) + "%";
  $("mpe-bend").textContent = "±" + mpe.bend + " st";
  send(70, mpe.pressureTo);
  send(71, mpe.pressureDepth);
  send(72, mpe.slideTo);
  send(73, mpe.slideDepth);
}
$("mpe-pressure-to").onchange = () => ((mpe.pressureTo = +$("mpe-pressure-to").value), showMpe());
$("mpe-slide-to").onchange = () => ((mpe.slideTo = +$("mpe-slide-to").value), showMpe());
for (const [id, key] of [["mpe-pressure-depth", "pressureDepth"], ["mpe-slide-depth", "slideDepth"]]) {
  dragValue($(id), () => (mpe[key] + 1) / 2, (v) => ((mpe[key] = Math.round(clamp(v * 2 - 1, -1, 1) * 100) / 100), showMpe()), 1 / 200);
  $(id).ondblclick = () => ((mpe[key] = 0), showMpe());
}
dragValue($("mpe-bend"), () => mpe.bend / 48, (v) => ((mpe.bend = Math.round(clamp(v, 0, 1) * 48)), showMpe()), 1 / 240);
$("mpe-bend").ondblclick = () => ((mpe.bend = 48), showMpe());
function showMpePanel(open) {
  const panel = $("mpe-panel");
  panel.hidden = !open;
  if (open) {
    const r = $("mpe-toggle").getBoundingClientRect();
    const box = panel.offsetParent.getBoundingClientRect();
    panel.style.left = Math.max(8, Math.min(r.left, innerWidth - panel.offsetWidth - 8)) - box.left + "px";
    panel.style.top = r.bottom - box.top + 4 + "px";
  }
  $("mpe-toggle").setAttribute("aria-expanded", String(open));
}
$("mpe-toggle").onclick = (e) => {
  e.stopPropagation();
  showMpePanel($("mpe-panel").hidden);
};
addEventListener("pointerdown", (e) => {
  if (!$("mpe-panel").hidden && !$("mpe-panel").contains(e.target) && !$("mpe-toggle").contains(e.target)) showMpePanel(false);
});
addEventListener("keydown", (e) => e.key === "Escape" && showMpePanel(false));
showMpe();
function showPresetPanel(open) {
  const panel = $("preset-panel");
  panel.hidden = !open;
  if (open) {
    const r = $("preset-toggle").getBoundingClientRect();
    const box = panel.offsetParent.getBoundingClientRect();
    panel.style.left = Math.max(8, Math.min(r.left, innerWidth - panel.offsetWidth - 8)) - box.left + "px";
    panel.style.top = r.bottom - box.top + 4 + "px";
  }
  $("preset-toggle").setAttribute("aria-expanded", String(open));
}
$("preset-toggle").onclick = (e) => {
  e.stopPropagation();
  showPresetPanel($("preset-panel").hidden);
};
addEventListener("pointerdown", (e) => {
  if (!$("preset-panel").hidden && !$("preset-panel").contains(e.target) && !$("preset-toggle").contains(e.target)) showPresetPanel(false);
});
addEventListener("keydown", (e) => e.key === "Escape" && showPresetPanel(false));
let grid = null;
$("grid").add(new Option("FREE", ""));
for (const [n] of DIVS) $("grid").add(new Option(n, n));
let gridBreath = 1, gridSmooth = 0;
function setGrid(div) {
  grid = div || null;
  $("grid").value = grid ?? "";
  send(57, grid ? divBeats(grid) : 0);
  for (const el of document.querySelectorAll(".grid-more")) el.hidden = !grid;
}
function setGridMore(breath, smooth) {
  gridBreath = clamp(breath, 0, 1);
  gridSmooth = clamp(smooth, 0, 1);
  $("gridbreath").textContent = Math.round(gridBreath * 100) + "%";
  $("gridsmooth").textContent = Math.round(gridSmooth * 100) + "%";
  send(61, gridBreath);
  send(62, gridSmooth);
}
dragValue($("gridbreath"), () => gridBreath, (v) => setGridMore(v, gridSmooth));
dragValue($("gridsmooth"), () => gridSmooth, (v) => setGridMore(gridBreath, v));
$("gridbreath").ondblclick = () => setGridMore(1, gridSmooth);
$("gridsmooth").ondblclick = () => setGridMore(gridBreath, 0);
setGridMore(1, 0);
$("grid").onchange = () => setGrid($("grid").value);
let hq = true, lofi = false;
const QUALITY = ["HQ", "LEAN", "LO"];
function setQuality(q) {
  hq = q === 0;
  lofi = q === 2;
  $("hq").textContent = QUALITY[q];
  $("hq").setAttribute("aria-pressed", String(q !== 1));
  send(56, hq ? 1 : 0);
  send(68, lofi ? 1 : 0);
}
const quality = () => (hq ? 0 : lofi ? 2 : 1);
$("hq").onclick = () => setQuality((quality() + 1) % 3);
let vowelIn = false;
function setVowelIn(on) {
  vowelIn = on;
  $("vowelin").setAttribute("aria-pressed", String(on));
  send(44, on ? 1 : 0);
}
$("vowelin").onclick = async () => {
  await start();
  setVowelIn(!vowelIn);
};
NAMES.forEach((n, i) => $("notename").add(new Option(n, String(i))));
showRoot();
$("notename").onchange = () => note(12 * Math.floor(lastNote / 12) + +$("notename").value);
function gate() {
  if (inPlugin) return;
  send(2, down.size || held ? 1 : 0);
}
function shiftOctave(d) {
  octave = clamp(octave + d, -3, 3);
  note(lastNote + 12 * d);
  for (const [i, m] of down) down.set(i, m + 12 * d);
}
const typing = (e) => e.target.tagName === "INPUT" && (e.target.type === "text" || e.target.type === "number");
function allKeysUp() {
  if (!down.size) return;
  down.clear();
  gate();
}
addEventListener("keydown", async (e) => {
  if (e.key === "Escape") return allKeysUp();
  if (inPlugin) return;
  if (typing(e) || e.metaKey || e.ctrlKey || e.altKey) return;
  const key = e.key.toLowerCase();
  if (e.target.tagName === "SELECT" && (KEYS.includes(key) || key === "z" || key === "x" || e.code === "Space")) e.preventDefault();
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
  const midi = 48 + 12 * octave + i;
  const legato = down.size > 0;
  down.set(i, midi);
  await start();
  if (!down.has(i)) return;
  note(midi);
  if (legato || held) post({ type: "retrigger" });
  gate();
});
addEventListener("keyup", (e) => {
  if (e.key === "Meta" || e.key === "Control" || e.key === "Alt") return allKeysUp();
  const i = KEYS.indexOf(e.key.toLowerCase());
  if (i < 0 || !down.has(i)) return;
  down.delete(i);
  if (down.size) note([...down.values()].pop());
  gate();
});
addEventListener("blur", allKeysUp);
addEventListener("pointerdown", (e) => e.target.tagName === "SELECT" && allKeysUp(), true);
addEventListener("change", (e) => e.target.tagName === "SELECT" && e.target.blur(), true);
document.addEventListener("visibilitychange", () => document.hidden && allKeysUp());
function setHold(on) {
  held = on;
  autoHold = false;
  $("hold").setAttribute("aria-pressed", String(held));
  if (on && envOn) setEnv(false);
  gate();
}
let envOn = false;
function setEnv(on) {
  envOn = on;
  $("envgate").setAttribute("aria-pressed", String(on));
  send(39, on ? 1 : 0);
  if (on && held) setHold(false);
}
$("envgate").onclick = async () => {
  await start();
  setEnv(!envOn);
};
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
const VOWELS = ["·", "ɑ", "e", "i", "o", "u"];
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
let seqGate = 1;
function sendSeq() {
  post({ type: "seq", len: clamp(+$("seqlen").value || 16, 1, 16), div: +$("rate").value, gate: seqGate });
  $("seqgate").textContent = Math.round(seqGate * 100) + "%";
  $("seqgate").classList.toggle("on", seqGate !== 1);
  renderSteps();
}
dragValue($("seqgate"), () => seqGate, (v) => {
  seqGate = Math.round(clamp(v, 0, 2) * 100) / 100;
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
  el.innerHTML = `<div class="note"><span></span><i class="acc" data-tip="Accent\nLouder, with more air.">◆</i><div class="len" data-tip="Length\nHow long the step sounds. Drag across."><b></b></div></div><div class="vowel"></div><i class="led"></i>`;
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
  lenEl.addEventListener("pointerdown", (e) => {
    e.stopPropagation();
    lenEl.setPointerCapture(e.pointerId);
    const set = (ev) => {
      const r = lenEl.getBoundingClientRect();
      const at = clamp((ev.clientX - r.left) / r.width, 0, 1) / (seqGate > 0 ? seqGate : 1);
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

const ICON_PLAY = '<svg class="glyph" viewBox="0 0 10 10" aria-hidden="true"><path d="M2 1 9 5 2 9Z" fill="currentColor"/></svg>';
const ICON_STOP = '<svg class="glyph" viewBox="0 0 10 10" aria-hidden="true"><rect x="1.75" y="1.75" width="6.5" height="6.5" rx="0.5" fill="currentColor"/></svg>';
function playLabel(el, on, word) {
  el.innerHTML = (on ? ICON_STOP : ICON_PLAY) + `<span>${on ? "STOP" : word}</span>`;
}
playLabel($("singtext"), false, "SING");
playLabel($("play"), false, "PLAY");
function setPlaying(on) {
  playing = on;
  post({ type: "play", on });
  playLabel($("play"), on, "PLAY");
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
intField($("seqlen"), 1, 16);
intField($("bpm"), 30, 300);

function intField(el, lo, hi) {
  const set = (v) => {
    const n = String(clamp(Math.round(v), lo, hi));
    if (el.value !== n) el.value = n;
  };
  el.addEventListener("change", () => set(+el.value || lo), { capture: true });
  let acc = 0;
  el.addEventListener("wheel", (e) => {
    if (el.readOnly || el.disabled) return;
    e.preventDefault();
    const d = e.deltaMode === 0 ? e.deltaY / 40 : Math.sign(e.deltaY);
    if (d * acc < 0) acc = 0;
    acc += d;
    const n = Math.trunc(acc);
    if (!n) return;
    acc -= n;
    const before = el.value;
    set((+el.value || lo) - n);
    if (el.value !== before) el.dispatchEvent(new Event("change"));
    else acc = 0;
  }, { passive: false });
}

// ---------------------------------------------------------------- text
const tx = { step: 0, unit: 0, say: 0, len: 1, rate: 2, sync: "1/4", seq: true, once: false, speech: 1, gate: 1, round: 0, fit: false, grid: null, resetNote: -1, restartPlay: false };
$("textresetoff").onclick = () => {
  tx.resetNote = -1;
  sendText(false);
};
let learning = false;
function setLearning(on) {
  learning = on;
  $("textlearn").setAttribute("aria-pressed", String(on));
  $("textlearn").textContent = on ? "PLAY A NOTE…" : "LEARN";
}
$("textlearn").onclick = () => {
  setLearning(!learning);
  if (inPlugin) sendToPlugin({ type: "learn", on: learning });
};
$("textplay").onclick = () => {
  tx.restartPlay = !tx.restartPlay;
  sendText(false);
};
$("textgrid").add(new Option("FREE", ""));
$("textgrid").add(new Option("GRID", "GRID"));
for (const [n] of DIVS) $("textgrid").add(new Option(n, n));
$("textgrid").onchange = () => {
  tx.grid = $("textgrid").value || null;
  sendText(false);
};
function sing(text) {
  if (!node && !inPlugin) return;
  const bytes = new TextEncoder().encode(text);
  post({ type: "text", bytes }, [bytes.buffer]);
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
  $("text").blur();
};
$("singtext").onclick = async () => {
  await start();
  sing(textOn ? "" : $("text").value);
};
let typingTimer = 0;
$("text").addEventListener("input", () => {
  clearTimeout(typingTimer);
  typingTimer = setTimeout(() => (node || inPlugin) && textOn && sing($("text").value), 300);
});
$("text").addEventListener("keydown", (e) => {
  if (e.key === "Escape") $("text").blur();
});
function sendText(final = true) {
  send(31, tx.step);
  send(32, tx.unit);
  send(33, tx.say);
  send(34, tx.len);
  send(22, tx.rate);
  send(23, tx.sync ? divBeats(tx.sync) : 0);
  send(28, tx.seq ? 1 : 0);
  send(30, tx.once ? 1 : 0);
  send(29, tx.speech);
  send(38, tx.gate);
  send(64, tx.round);
  send(69, tx.fit ? 1 : 0);
  $("textround").textContent = Math.round(tx.round * 100) + "%";
  $("textround").classList.toggle("on", tx.round > 0);
  $("textfit").setAttribute("aria-pressed", String(tx.fit));
  send(63, tx.grid === "GRID" ? -1 : tx.grid ? divBeats(tx.grid) : 0);
  $("textgrid").value = tx.grid ?? "";
  $("textreset").textContent = tx.resetNote < 0 ? "off" : String(tx.resetNote);
  $("textresetoff").hidden = tx.resetNote < 0;
  $("textplay").setAttribute("aria-pressed", String(tx.restartPlay));
  $("tbreathbox").hidden = !tx.grid;
  $("textgate").textContent = Math.round(tx.gate * 100) + "%";
  $("textgate").classList.toggle("on", tx.gate < 1);
  const press = (attr, v) => document.querySelectorAll(`[${attr}]`).forEach((b) => b.setAttribute("aria-pressed", String(b.getAttribute(attr) === String(v))));
  press("data-tstep", tx.step);
  press("data-tunit", tx.unit);
  press("data-tsay", tx.say);
  press("data-tseq", +tx.seq);
  press("data-tonce", +tx.once);
  $("clockrate").hidden = tx.step !== 1;
  $("lenbox").hidden = !(tx.step === 1 && tx.say === 0);
  $("textlen").textContent = Math.round(tx.len * 100) + "%";
  $("loophz").textContent = tx.rate.toFixed(tx.rate < 10 ? 1 : 0) + "/s";
  $("loophz").hidden = !!tx.sync;
  $("loopdiv").hidden = !tx.sync;
  if (tx.sync) $("loopdiv").value = tx.sync;
  $("loopsync").setAttribute("aria-pressed", String(!!tx.sync));
  $("speech").textContent = "×" + tx.speech.toFixed(2);
  $("speech").classList.toggle("on", tx.speech !== 1);
  countUnits();
  if (final && textOn) sing($("text").value);
}
for (const [attr, key, bool] of [["data-tstep", "step"], ["data-tunit", "unit"], ["data-tsay", "say"], ["data-tseq", "seq", true], ["data-tonce", "once", true]]) {
  document.querySelectorAll(`[${attr}]`).forEach((b) => {
    b.onclick = () => {
      const v = +b.getAttribute(attr);
      tx[key] = bool ? !!v : v;
      sendText();
    };
  });
}
$("loopdiv").innerHTML = DIVS.map(([n]) => `<option>${n}</option>`).join("");
dragValue($("loophz"), () => Math.log(tx.rate), (lv, final) => {
  tx.rate = clamp(Math.exp(lv), 0.25, 16);
  sendText(final);
}, 1 / 60);
$("loopsync").onclick = () => {
  tx.sync = tx.sync ? null : "1/4";
  sendText();
};
$("loopdiv").onchange = () => {
  tx.sync = $("loopdiv").value;
  sendText();
};
dragValue($("textlen"), () => tx.len, (v, final) => {
  tx.len = Math.round(clamp(v, 0.05, 1) * 100) / 100;
  sendText(final);
}, 1 / 150);
$("textlen").ondblclick = () => {
  tx.len = 1;
  sendText();
};
dragValue($("speech"), () => Math.log(tx.speech), (lv, final) => {
  tx.speech = Math.round(clamp(Math.exp(lv), 0.25, 4) * 100) / 100;
  sendText(final);
}, 1 / 80);
$("speech").ondblclick = () => {
  tx.speech = 1;
  sendText();
};
dragValue($("textgate"), () => tx.gate, (v) => {
  tx.gate = Math.round(clamp(v, 0.05, 1) * 100) / 100;
  sendText(false);
}, 1 / 150);
$("textgate").ondblclick = () => {
  tx.gate = 1;
  sendText(false);
};
dragValue($("textround"), () => tx.round, (v) => {
  tx.round = Math.round(clamp(v, 0, 1) * 100) / 100;
  sendText(false);
}, 1 / 150);
$("textround").ondblclick = () => {
  tx.round = 0;
  sendText(false);
};
$("textfit").onclick = () => {
  tx.fit = !tx.fit;
  sendText(false);
};
sendText(false);

// ---------------------------------------------------------------- focus
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
const STATE_VERSION = 1;
const MOTION_NAMES = ["vowelX", "vowelY", "round", "nasal", "inject", "damping", "morph", "shift", "open"];

function getState() {
  const knobs = {};
  for (const [key, c] of controls) knobs[key] = c.sync ? { v: state[key], sync: c.sync } : state[key];
  const motion = {};
  rows.forEach((r, i) => (motion[MOTION_NAMES[i]] = { base: r.base, depth: r.depth, rate: r.rate, sync: r.sync, shape: r.shape, smooth: r.smooth }));
  return {
    suu: STATE_VERSION,
    source: +oscWave,
    note: lastNote,
    octave,
    bpm: bpm(),
    glideDir,
    envGate: envOn,
    pitchFrom,
    legato,
    vowelIn,
    hq,
    lofi,
    mpe: { ...mpe },
    grid,
    gridBreath,
    gridSmooth,
    scale,
    jump,
    jumpFold,
    space: spaceOn,
    stereo: stereoOn,
    stereoMirror,
    knobs,
    motion,
    seq: { rate: +$("rate").value, length: clamp(+$("seqlen").value || 16, 1, 16), gate: seqGate, steps: steps.map((s) => ({ ...s })) },
    text: { words: $("text").value, ...tx },
    horn: Array.from(horn, (a) => Math.round(a * 1000) / 1000),
    hornWrap,
    hornWave: { ...hornWave, snap: snapVals.slice(), drawn: { mean: Math.round(drawn.mean * 1e4) / 1e4, w: drawn.w.slice() } },
  };
}

const DEFAULTS = getState();

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
    source: [0, 1, 2, 3].includes(r.source) ? r.source : d.source,
    note: Math.round(num(r.note, d.note, 12, 108)),
    octave: Math.round(num(r.octave, d.octave, -3, 3)),
    bpm: num(r.bpm, d.bpm, 30, 300),
    space: bool(r.space, d.space),
    envGate: bool(r.envGate, d.envGate),
    pitchFrom: [0, 1, 2].includes(r.pitchFrom) ? r.pitchFrom : d.pitchFrom,
    legato: bool(r.legato, d.legato),
    vowelIn: bool(r.vowelIn, d.vowelIn),
    hq: bool(r.hq, d.hq),
    lofi: bool(r.lofi, d.lofi),
    stereo: bool(r.stereo, d.stereo),
    stereoMirror: bool(r.stereoMirror, d.stereoMirror),
    mpe: (() => {
      const m = obj(r.mpe, "mpe"), to = (x, dv) => (Number.isInteger(x) && x >= -1 && x <= 10 ? x : dv);
      return { pressureTo: to(m.pressureTo, d.mpe.pressureTo), pressureDepth: num(m.pressureDepth, d.mpe.pressureDepth, -1, 1), slideTo: to(m.slideTo, d.mpe.slideTo), slideDepth: num(m.slideDepth, d.mpe.slideDepth, -1, 1), bend: Math.round(num(m.bend, d.mpe.bend, 0, 48)) };
    })(),
    grid: div(r.grid, d.grid),
    gridBreath: num(r.gridBreath, d.gridBreath, 0, 1),
    gridSmooth: num(r.gridSmooth, d.gridSmooth, 0, 1),
    scale: Number.isInteger(r.scale) ? r.scale & 0xfff : d.scale,
    jump: Math.round(num(r.jump, d.jump, 0, 48)),
    jumpFold: bool(r.jumpFold, d.jumpFold),
    glideDir: [0, 1, 2].includes(r.glideDir) ? r.glideDir : d.glideDir,
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
    out.motion[name] = { base: num(m.base, dm.base, row.lo, row.hi), depth: num(m.depth, dm.depth, -1, 1), rate: num(m.rate, dm.rate, 0.02, 20), sync: div(m.sync, dm.sync), shape: Math.round(num(m.shape, dm.shape, 0, SHAPES.length - 1)), smooth: num(m.smooth, dm.smooth, 0, 1) };
  });
  const seq = obj(r.seq, "seq");
  if (seq.steps !== undefined && !Array.isArray(seq.steps)) throw new Error("steps is not a list");
  out.seq = {
    rate: [1, 2, 4, 8, 16, 32].includes(seq.rate) ? seq.rate : d.seq.rate,
    length: Math.round(num(seq.length, d.seq.length, 1, 16)),
    gate: num(seq.gate, d.seq.gate, 0, 2),
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
  if (hw.snap !== undefined && !Array.isArray(hw.snap)) throw new Error("hornWave.snap is not a list");
  const snap = Array.isArray(hw.snap) && hw.snap.length === HORN_POINTS ? hw.snap.map((v) => num(v, 0, -1, 1)) : [];
  const shape = Math.round(num(hw.shape, d.hornWave.shape, -1, 5));
  const dr = obj(hw.drawn, "hornWave.drawn");
  if (dr.w !== undefined && !Array.isArray(dr.w)) throw new Error("hornWave.drawn.w is not a list");
  const drawnOk = Array.isArray(dr.w) && dr.w.length === HORN_POINTS;
  out.hornWave = {
    shape: shape === 5 && !snap.length ? -1 : shape,
    cycles: Math.round(num(hw.cycles, d.hornWave.cycles, 1, 8)),
    depth: num(hw.depth, d.hornWave.depth, 0, 2),
    snap,
    drawn: drawnOk ? { mean: num(dr.mean, 3, 0, HORN_MAX), w: dr.w.map((v) => num(v, 0, -50, 50)) } : null,
  };
  const old = [[1, 0, 0], [0, 0, 0], [0, 1, 1]][text.mode] ?? null;
  const pick = (v, dv, n) => (Number.isInteger(v) && v >= 0 && v < n ? v : dv);
  out.text = {
    words: typeof text.words === "string" ? text.words.slice(0, 400) : d.text.words,
    step: pick(text.step, old ? old[0] : d.text.step, 2),
    unit: pick(text.unit, old ? old[1] : d.text.unit, 2),
    say: pick(text.say, old ? old[2] : d.text.say, 2),
    len: num(text.len, d.text.len, 0.05, 1),
    rate: num(text.rate, d.text.rate, 0.25, 16),
    sync: div(text.sync, d.text.sync),
    seq: bool(text.seq, d.text.seq),
    once: bool(text.once, d.text.once),
    speech: num(text.speech, d.text.speech, 0.25, 4),
    gate: num(text.gate, d.text.gate, 0.05, 1),
    round: num(text.round, d.text.round, 0, 1),
    fit: bool(text.fit, d.text.fit),
    grid: text.grid === "GRID" ? "GRID" : div(text.grid, d.text.grid),
    resetNote: Math.round(num(text.resetNote, d.text.resetNote, -1, 127)),
    restartPlay: bool(text.restartPlay, d.text.restartPlay),
  };
  return out;
}

function applyState(st) {
  $("bpm").value = st.bpm;
  send(19, bpm());
  octave = st.octave;
  note(st.note);
  setGlideDir(st.glideDir);
  setEnv(st.envGate);
  setPitchFrom(st.pitchFrom);
  setLegato(st.legato);
  setVowelIn(st.vowelIn);
  setQuality(st.hq ? 0 : st.lofi ? 2 : 1);
  Object.assign(mpe, st.mpe);
  setStereo(st.stereo);
  setMirror(st.stereoMirror);
  showMpe();
  setGrid(st.grid);
  setGridMore(st.gridBreath, st.gridSmooth);
  setScale(st.scale);
  setJump(st.jump, st.jumpFold);
  for (const [key, c] of controls) {
    const k = st.knobs[key];
    c.setSync?.(typeof k === "object" ? k.sync : null);
    update(key, typeof k === "object" ? k.v : k);
  }
  rows.forEach((r, i) => {
    const m = st.motion[MOTION_NAMES[i]];
    Object.assign(r, { depth: m.depth, rate: m.rate, sync: m.sync, shape: m.shape, smooth: m.smooth });
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
  setOsc(String(st.source));
  horn = Float32Array.from(st.horn);
  hornWrap = st.hornWrap;
  snapVals = st.hornWave.snap.slice();
  Object.assign(hornWave, { shape: st.hornWave.shape, cycles: st.hornWave.cycles, depth: st.hornWave.depth });
  if (st.hornWave.drawn) drawn = { mean: st.hornWave.drawn.mean, w: st.hornWave.drawn.w.slice() };
  else if (hornWave.shape < 0) captureDrawn();
  showHornWave();
  sendHorn();
  $("text").value = st.text.words;
  const { words, ...settings } = st.text;
  Object.assign(tx, settings);
  sendText(false);
  if (textOn) sing($("text").value);
}

const store = {
  key: "suu.session.v1",
  load() {
    if (inPlugin) return null;
    try {
      return JSON.parse(localStorage.getItem(this.key) ?? "null");
    } catch {
      return null;
    }
  },
  save(text) {
    if (inPlugin) return;
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
  $("preset-slot").textContent = " " + slots.active;
  $("copyab").textContent = `${slots.active}→${other}`;
  $("copyab").dataset.tip = `Copy ${slots.active} → ${other}\nThis side's settings onto the other.`;
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
    if (slots[k]) load(slots[k], k);
    showSlots();
    save();
  };
}
$("init").onclick = () => {
  if (inPlugin) return sendToPlugin({ type: "reset" });
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
  if (inPlugin) return sendToPlugin({ type: "save", name: "suu-preset.json", text: JSON.stringify(getState(), null, 1) });
  const blob = new Blob([JSON.stringify(getState(), null, 1)], { type: "application/json" });
  const a = Object.assign(document.createElement("a"), { href: URL.createObjectURL(blob), download: "suu-preset.json" });
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
};
$("openfile").onclick = () => (inPlugin ? sendToPlugin({ type: "open" }) : $("presetfile").click());
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

// ---------------------------------------------------------------- the mouth before audio starts
(async () => {
  if (inPlugin) return;
  try {
    const { exports: x } = await WebAssembly.instantiate(await wasmModule, {});
    if (ui) return;
    const h = x.suu_new(48000);
    for (const r of rows) x.suu_set(h, r.param, r.base);
    x.suu_set(h, 18, state.size);
    const bytes = new Uint8Array(horn.buffer.slice(horn.byteOffset, horn.byteOffset + horn.byteLength));
    const p = x.suu_alloc(bytes.length);
    new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
    x.suu_set_profile(h, p, horn.length, hornWrap ? 1 : 0);
    x.suu_free(p, bytes.length);
    for (let i = 0; i < 150; i++) x.suu_process(h, 128);
    const len = x.suu_ui_update(h);
    if (!ui) {
      ui = new Float32Array(x.memory.buffer, x.suu_ui(h), len).slice();
      dirty = true;
    }
  } catch {}
})();

// ---------------------------------------------------------------- tooltips
const TIPS = {
  '[data-osc="2"]': "Supersaw\nFive detuned saws: the built-in synth, played by keys and steps.",
  '[data-osc="1"]': "Saw\nOne saw: bright, every harmonic.",
  '[data-osc="3"]': "Square\nOdd harmonics only: hollow.",
  '[data-osc="0"]': "Off\nNo oscillator: only the input and the folds.",
  '[data-in="file"]': "File\nAn audio file, looped, mixed with the oscillator. Click again: off. The folder beside it, or a drop anywhere, loads another.",
  "#pickfile": "Choose file\nLoad an audio file (or drop one anywhere).",
  '[data-in="mic"]': "Mic\nThe microphone, mixed with the oscillator. Use headphones.",
  "#envgate": "Env\nThe input's level opens the sound. Each onset says the text's next part.",
  "#hold": "Hold\nKeeps the note on: a drone, or a file or mic passing through. Keys then say the text's next part.",
  "#octdown": "Octave down (Z)",
  "#octup": "Octave up (X)",
  "#notename": "Note\nThe root: the synth's pitch, the resonator's and the sequence's key. Pick it here, its octave with − and +; keys A … K play.",
  "#rootoct": "Octave\nThe root's octave: − and + (Z and X) change it.",
  "#pitch-more": "Scale and jump\nThe input's pitch (PITCH IN): which notes IN ♮ snaps to, and how far it may jump.",
  "#scale-notes button": "Note\nIN ♮ snaps the input's pitch to the notes lit. The presets build a scale on the root.",
  "#jump": "Widest jump\nHow far the input's pitch may move at once, in semitones (any: no limit). Drag; double-click for any.",
  "[data-jump='1']": "Fold\nA wider jump is moved by octaves next to the current pitch.",
  "[data-jump='0']": "Skip\nA wider jump is left out: the pitch stays.",
  "#grid": "Grid (modulation)\nThe mouth moves only on this note value's lines: the pad, Motion, the input's vowel and pitch, step vowels, the horn. FREE: as it comes.",
  "#gridsmooth": "Smooth\nBoth grids: 0% a step on each line; up to 100% each move glides over the line.",
  "#textreset": "Reset note\nThis MIDI note starts the text over and does not sound. Set it with LEARN.",
  "#textresetoff": "No reset note",
  "#textlearn": "Learn\nPress, then play a note: it becomes the reset note.",
  "#textplay": "On play\nThe text starts over when the host's transport starts.",
  "#textgrid": "Grid (text)\nThe speech moves only on this note value's lines, for a stepped, robotic voice. With STEP NOTE each part starts on a line. GRID: the modulation's. FREE: as it comes.",
  "#gridbreath": "Breath\nHow far the text's breath and voicing follow its grid. 0%: as they come.",
  "#hq": "Quality\nHQ: full detail. LEAN: lighter on the CPU. LO: coarser and duller, lightest. Click to step through.",
  "#textround": "Round\nFades each spoken part in and out. 0%: the sound stays on. Double-click: 0%.",
  "#textfit": "Fit\nEach note lasts as long as its syllable or word, even with the key held. On the clock the gaps between parts are silent.",
  "#legato": "Retrig / Legato\nRETRIG: every note starts the envelope again. LEGATO: a note while the sound is open goes on with it (only the pitch moves).",
  "#vowelin": "In\nThe input's vowel sets the pad: sing or speak into it. Motion moves around it.",
  "#pitchfrom": "Pitch\nNOTE: the root and the keys. IN: the input's own pitch. IN ♮: the input's pitch snapped to the scale.",
  "#slotA": "A\nOne of two full settings to compare; switching keeps the other.",
  "#slotB": "B\nOne of two full settings to compare; switching keeps the other.",
  "#init": "Init\nThis side back to the initial settings.",
  "#stereomirror": "Mirror\nThe right mouth's horn turned round, lips to throat.",
  "#stereoon": "Stereo\nA second mouth on the right. Twice the CPU.",
  "#mpe-toggle": "MPE\nWhat a key's pressure and slide move, and the pitch bend range.",
  "#mpe-pressure-to": "Pressure\nWhat pressing into the key moves.",
  "#mpe-slide-to": "Slide\nWhat sliding up and down the key moves.",
  "#mpe-pressure-depth": "Depth\nHow far pressure moves it; negative turns it the other way. Double-click: 0%.",
  "#mpe-slide-depth": "Depth\nHow far slide moves it; negative turns it the other way. Double-click: 0%.",
  "#mpe-bend": "Bend\nPitch bend range in semitones. Double-click: ±48 (MPE).",
  "#preset-toggle": "Preset\nA and B to compare, INIT, and preset files.",
  "#savefile": "Save\nThe settings as a preset file.",
  "#openfile": "Open\nA preset file (or drop one anywhere).",
  "#bpm": "Tempo\nFor the sequencer, the text clock and every ♪.",
  "#info-toggle": "Keys and credits",
  "#help-toggle": "Help\nTooltips on or off.",
  "#view-pad": "Pad\nThe mouth from the front: drag it to set the vowel.",
  "#view-horn": "Horn\nThe tract from the side: draw your own shape (Morph blends it in).",
  "#text-toggle": "Text\nWords for the mouth to say.",
  "#horn-reset": "Reset\nThe original horn.",
  "#horn-shape": "Wave\nBuild the horn from a wave (—: drawn by hand). Shift moves a wave through the tract.",
  "#horn-snap": "Snapshot\nOne period of the sound playing now becomes the horn.",
  "#horn-cycles": "Cycles\nHow many times the shape repeats along the tract.",
  "#horn-depth": "Depth\nHow strongly the shape swells and narrows: 0 is a plain tube.",
  "#text": "Words\nEnglish. Typing while it sings applies it; Enter sings.",
  "#singtext": "Sing\nStart or stop saying the text.",
  '[data-tstep="0"]': "Note\nEach note (a key, hold or step) says the next part.",
  '[data-tstep="1"]': "Clock\nThe text's own clock says it at its rate. It doesn't gate the sound.",
  "#loophz": "Rate\nParts per second. Drag.",
  "#loopdiv": "Rate\nOne part per this note value.",
  "#loopsync": "Sync\nThe rate as a note value at the tempo.",
  '[data-tunit="0"]': "Syllable\nOne syllable at a time.",
  '[data-tunit="1"]': "Word\nOne word at a time.",
  '[data-tsay="0"]': "Hold\nIts vowel holds while the note does, then its ending.",
  '[data-tsay="1"]': "Shot\nSaid through at its own pace, however long the note.",
  "#lenbox": "Length\nOn the clock, how long each held part lasts; 100% joins them. Double-click: 100%.",
  '[data-tseq="0"]': "Free\nCarries on where it was.",
  '[data-tseq="1"]': "Seq\nStarts over each time the sequence does.",
  '[data-tonce="0"]': "Loop\nOver and over.",
  '[data-tonce="1"]': "Once\nOnce per pass of the sequence (or once per Sing).",
  "#speech": "Speed\nHow fast it's spoken: the consonants and moves between sounds. Double-click: ×1.",
  "#textgate": "Gate\nHow much of each part you hear: the sound is cut after this share of a tick (clock) or of its spoken length (notes). Double-click: 100%.",
  "#motion-fold": "Motion\nShow or hide the rows.",
  "#github": "GitHub\nSuu's source code and the plug-in's releases.",
  "#motion-reset": "Reset\nEvery depth back to 0.",
  "#play": "Play (Space)\nStart or stop the sequencer.",
  "#rate": "Rate\nThe steps' note value.",
  "#seqlen": "Steps\nHow many steps the pattern has.",
  "#seqgate": "Gate\nScales every step's length alike; 0 is silent. Double-click: 100%.",
  "#spaceon": "Space\nWeb only: delay and reverb. Off is what the plug-in does.",
  "#peak": "Peak\nThe output's peak; red is clipping.",
};
for (const [sel, t] of Object.entries(TIPS)) document.querySelectorAll(sel).forEach((el) => (el.dataset.tip = t));
cells.forEach((c) => {
  c.note.dataset.tip = "Step\nDrag up or down: its note. Click: on or off. Shift-click: tie to the next. Double-click: the root.";
  c.vowel.dataset.tip = "Vowel\nClick: the vowel this step moves the mouth to (· leaves it).";
});
function mouthTip() {
  mouthEl.dataset.tip = view === "horn" ? "Horn\nThe tract, throat to lips, as it sounds now. Drag to draw; double-click: the original." : "Vowel pad\nDrag: front to back, close to open.";
}
mouthTip();
for (const id of ["view-pad", "view-horn"]) $(id).addEventListener("click", mouthTip);
document.querySelectorAll("[title]").forEach((el) => {
  el.dataset.tip ??= el.title;
  el.removeAttribute("title");
});

let tipsOn = true;
try {
  tipsOn = localStorage.getItem("suu.tips") !== "off";
} catch {}
const tipEl = $("tip");
let tipTimer = 0, tipFor = null;
function hideTip() {
  clearTimeout(tipTimer);
  tipEl.hidden = true;
  tipFor = null;
}
function showTip(el) {
  const [name, ...rest] = el.dataset.tip.split("\n");
  tipEl.replaceChildren();
  const b = document.createElement("b");
  b.textContent = name;
  tipEl.append(b);
  if (rest.length) tipEl.append(rest.join(" "));
  tipEl.hidden = false;
  const r = el.getBoundingClientRect(), t = tipEl.getBoundingClientRect();
  let x = clamp(r.left + r.width / 2 - t.width / 2, 6, innerWidth - t.width - 6);
  let y = r.bottom + 6;
  if (y + t.height > innerHeight - 6) y = r.top - t.height - 6;
  tipEl.style.left = x + "px";
  tipEl.style.top = Math.max(6, y) + "px";
}
addEventListener("pointerover", (e) => {
  const el = e.target.closest?.("[data-tip]");
  if (el === tipFor) return;
  hideTip();
  if (!tipsOn || !el || e.pointerType === "touch") return;
  tipFor = el;
  tipTimer = setTimeout(() => showTip(el), 600);
});
addEventListener("pointerdown", hideTip, true);
addEventListener("keydown", hideTip, true);
function setTips(on) {
  tipsOn = on;
  $("help-toggle").setAttribute("aria-pressed", String(on));
  try {
    localStorage.setItem("suu.tips", on ? "on" : "off");
  } catch {}
  hideTip();
}
$("help-toggle").onclick = () => setTips(!tipsOn);
setTips(tipsOn);

// ---------------------------------------------------------------- plug-in
const PDIVS = [null, ...DIVS.map((d) => d[0])];
const divIndex = (name) => Math.max(0, PDIVS.indexOf(name ?? null));
const knobVal = (k) => (typeof k === "object" ? k.v : k);
const PMAP = [];
{
  const add = (id, get, set) => PMAP.push({ id, get, set });
  const knob = (id, key) => add(id, (st) => knobVal(st.knobs[key]), (st, v) => {
    const k = st.knobs[key];
    if (typeof k === "object") k.v = v;
    else st.knobs[key] = v;
  });
  add("wave", (st) => st.source, (st, v) => (st.source = Math.round(v)));
  for (const [id, key] of [["env_threshold", "envThresh"], ["env_release", "envRelease"], ["attack", "attack"], ["decay", "decay"], ["sustain", "sustain"], ["release", "release"], ["coarse", "coarse"], ["steady", "steady"], ["osc", "osc"], ["input", "input"], ["folds", "folds"], ["reson", "reson"], ["glide", "glide"], ["size", "size"], ["breath", "breath"], ["follow", "follow"], ["drive", "drive"], ["tilt", "tilt"], ["sub", "sub"], ["sub_hz", "subhz"], ["mix", "mix"], ["output", "out"], ["aspiration", "aspiration"], ["frication", "frication"], ["sibilance", "sibilance"], ["whistle", "whistle"], ["stereo_spread", "stereoSpread"], ["stereo_vowel", "stereoVowel"], ["stereo_width", "stereoWidth"], ["move", "move"], ["stereo_canon", "stereoCanon"], ["stereo_phase", "stereoPhase"]]) knob(id, key);
  add("glide_sync", (st) => (typeof st.knobs.glide === "object" ? divIndex(st.knobs.glide.sync) : 0), (st, v) => {
    const val = knobVal(st.knobs.glide);
    st.knobs.glide = v > 0 ? { v: val, sync: PDIVS[Math.round(v)] } : val;
  });
  for (const [id, key] of [["env_release_sync", "envRelease"], ["steady_sync", "steady"], ["attack_sync", "attack"], ["decay_sync", "decay"], ["release_sync", "release"]])
    add(id, (st) => (typeof st.knobs[key] === "object" ? divIndex(st.knobs[key].sync) : 0), (st, v) => {
      const val = knobVal(st.knobs[key]);
      st.knobs[key] = v > 0 ? { v: val, sync: PDIVS[Math.round(v)] } : val;
    });
  add("legato", (st) => +st.legato, (st, v) => (st.legato = v > 0.5));
  add("vowel_in", (st) => +st.vowelIn, (st, v) => (st.vowelIn = v > 0.5));
  add("hq", (st) => +st.hq, (st, v) => (st.hq = v > 0.5));
  add("lofi", (st) => +st.lofi, (st, v) => (st.lofi = v > 0.5));
  add("stereo", (st) => +st.stereo, (st, v) => (st.stereo = v > 0.5));
  add("stereo_mirror", (st) => +st.stereoMirror, (st, v) => (st.stereoMirror = v > 0.5));
  add("pressure_to", (st) => st.mpe.pressureTo + 1, (st, v) => (st.mpe.pressureTo = Math.round(v) - 1));
  add("pressure_depth", (st) => st.mpe.pressureDepth, (st, v) => (st.mpe.pressureDepth = v));
  add("slide_to", (st) => st.mpe.slideTo + 1, (st, v) => (st.mpe.slideTo = Math.round(v) - 1));
  add("slide_depth", (st) => st.mpe.slideDepth, (st, v) => (st.mpe.slideDepth = v));
  add("bend_range", (st) => st.mpe.bend, (st, v) => (st.mpe.bend = Math.round(v)));
  add("grid", (st) => divIndex(st.grid), (st, v) => (st.grid = PDIVS[Math.round(v)]));
  add("grid_breath", (st) => st.gridBreath, (st, v) => (st.gridBreath = v));
  add("grid_smooth", (st) => st.gridSmooth, (st, v) => (st.gridSmooth = v));
  for (let i = 0; i < 12; i++) add(`scale_${i + 1}`, (st) => (st.scale >> i) & 1, (st, v) => (st.scale = v > 0.5 ? st.scale | (1 << i) : st.scale & ~(1 << i)));
  add("jump", (st) => st.jump, (st, v) => (st.jump = Math.round(v)));
  add("jump_mode", (st) => (st.jumpFold ? 0 : 1), (st, v) => (st.jumpFold = Math.round(v) === 0));
  add("glide_dir", (st) => st.glideDir, (st, v) => (st.glideDir = Math.round(v)));
  add("pitch", (st) => st.pitchFrom, (st, v) => (st.pitchFrom = Math.round(v)));
  add("root", (st) => st.note, (st, v) => (st.note = Math.round(v)));
  const PREFIX = ["vowel_x", "vowel_y", "round", "nasal", "inject", "damping", "morph", "shift", "open"];
  MOTION_NAMES.forEach((name, i) => {
    const p = PREFIX[i], m = (st) => st.motion[name];
    add(p + "_base", (st) => m(st).base, (st, v) => (m(st).base = v));
    add(p + "_depth", (st) => m(st).depth, (st, v) => (m(st).depth = v));
    add(p + "_rate", (st) => m(st).rate, (st, v) => (m(st).rate = v));
    add(p + "_sync", (st) => divIndex(m(st).sync), (st, v) => (m(st).sync = PDIVS[Math.round(v)]));
    add(p + "_shape", (st) => m(st).shape, (st, v) => (m(st).shape = Math.round(v)));
    add(p + "_smooth", (st) => m(st).smooth, (st, v) => (m(st).smooth = v));
  });
  const text = (id, key, kind) => add(id, (st) => (kind === "bool" ? +st.text[key] : kind === "sync" ? divIndex(st.text[key]) : st.text[key]),
    (st, v) => (st.text[key] = kind === "bool" ? v > 0.5 : kind === "sync" ? PDIVS[Math.round(v)] : kind === "int" ? Math.round(v) : v));
  text("text_step", "step", "int");
  text("text_unit", "unit", "int");
  text("text_say", "say", "int");
  text("text_rate", "rate");
  text("text_sync", "sync", "sync");
  text("text_len", "len");
  text("text_gate", "gate");
  text("text_round", "round");
  text("text_fit", "fit", "bool");
  text("text_seq", "seq", "bool");
  text("text_once", "once", "bool");
  text("speech", "speech");
  text("text_reset_note", "resetNote", "int");
  text("text_restart_play", "restartPlay", "bool");
  add("text_grid_main", (st) => +(st.text.grid === "GRID"), (st, v) => {
    if (v > 0.5) st.text.grid = "GRID";
    else if (st.text.grid === "GRID") st.text.grid = null;
  });
  add("text_grid", (st) => (st.text.grid && st.text.grid !== "GRID" ? divIndex(st.text.grid) : 0), (st, v) => {
    if (st.text.grid !== "GRID") st.text.grid = PDIVS[Math.round(v)];
  });

  add("seq_rate", (st) => Math.round(Math.log2(st.seq.rate)), (st, v) => (st.seq.rate = 2 ** Math.round(v)));
  add("seq_length", (st) => st.seq.length, (st, v) => (st.seq.length = Math.round(v)));
  add("seq_gate", (st) => st.seq.gate, (st, v) => (st.seq.gate = v));
  for (let i = 0; i < 16; i++) {
    const s = (st) => st.seq.steps[i], n = i + 1;
    add(`step_on_${n}`, (st) => +s(st).on, (st, v) => (s(st).on = v > 0.5));
    add(`step_note_${n}`, (st) => s(st).note, (st, v) => (s(st).note = Math.round(v)));
    add(`step_tie_${n}`, (st) => +s(st).tie, (st, v) => (s(st).tie = v > 0.5));
    add(`step_vowel_${n}`, (st) => s(st).vowel + 1, (st, v) => (s(st).vowel = Math.round(v) - 1));
    add(`step_accent_${n}`, (st) => +s(st).accent, (st, v) => (s(st).accent = v > 0.5));
    add(`step_len_${n}`, (st) => s(st).len, (st, v) => (s(st).len = v));
  }
  add("gate", (st) => (st.envGate ? 0 : held ? 1 : 2), (st, v) => (st.envGate = Math.round(v) === 0));
  add("seq", () => +playing, null);
}

function hornKey(st) {
  return JSON.stringify([st.horn, st.hornWrap]);
}

function applyHost(st, values) {
  for (const m of PMAP) if (m.set && m.id in values) m.set(st, values[m.id]);
  plugin.quiet = true;
  try {
    applyState(readState(st));
    if ("gate" in values) setHold(Math.round(values.gate) === 1);
    if ("seq" in values && (values.seq > 0.5) !== playing) setPlaying(values.seq > 0.5);
  } finally {
    plugin.quiet = false;
  }
  const now = getState();
  for (const m of PMAP) plugin.host[m.id] = m.get(now);
  plugin.horn = hornKey(now);
}

const textField = (el) => el?.tagName === "INPUT" && (el.type === "text" || el.type === "number") && !el.readOnly;
function pluginSync() {
  const typing = textField(document.activeElement);
  if (typing !== plugin.typing) {
    plugin.typing = typing;
    sendToPlugin({ type: "typing", on: typing });
  }
  if (!plugin.ready || plugin.quiet) return;
  const st = getState();
  for (const m of PMAP) {
    const v = m.get(st);
    if (!(Math.abs(v - plugin.host[m.id]) <= 1e-6)) {
      plugin.host[m.id] = v;
      sendToPlugin({ type: "set", id: m.id, value: v });
    }
  }
  if (st.text.words !== plugin.words) {
    plugin.words = st.text.words;
    sendToPlugin({ type: "words", text: plugin.words });
  }
  const hk = hornKey(st);
  if (hk !== plugin.horn) {
    plugin.horn = hk;
    sendToPlugin({ type: "horn", areas: st.horn, wrap: st.hornWrap });
  }
  if (++plugin.frame % 60 === 0) {
    const json = JSON.stringify(st);
    if (json !== plugin.ui) sendToPlugin({ type: "ui", json: (plugin.ui = json) });
  }
}

function pluginPost(m) {
  if (plugin.quiet) return;
  if (m.type === "text") {
    const text = new TextDecoder().decode(m.bytes);
    if (text.trim()) plugin.words = text;
    sendToPlugin({ type: "text", text });
  } else if (m.type === "snapshot") sendToPlugin({ type: "snapshot" });
}

if (inPlugin) {
  window.__suuPlugin = { ids: PMAP.map((m) => m.id) };
  document.body.classList.add("in-plugin");
  $("bpm").readOnly = true;
  TIPS["#bpm"] = "Tempo\nThe host's.";
  $("bpm").dataset.tip = TIPS["#bpm"];
  addEventListener("focusin", (e) => !textField(e.target) && e.target.tagName !== "SELECT" && e.target.blur?.(), true);
  addEventListener("change", (e) => e.target.tagName === "SELECT" && e.target.blur(), true);
  addEventListener("pointerup", (e) => !textField(e.target) && e.target.tagName !== "SELECT" && sendToPlugin({ type: "keys" }), true);
  addEventListener("change", (e) => e.target.tagName === "SELECT" && sendToPlugin({ type: "keys" }), true);
  TIPS["#rootoct"] = "Octave\nThe root's octave: − and + change it.";
  $("rootoct").dataset.tip = TIPS["#rootoct"];
  addEventListener("error", (e) => sendToPlugin({ type: "log", text: `${e.message} ${e.filename}:${e.lineno}` }));
  addEventListener("unhandledrejection", (e) => sendToPlugin({ type: "log", text: String(e.reason?.stack ?? e.reason) }));
  window.onPluginMessage = (m) => {
    if (m.type === "init") {
      let base = DEFAULTS;
      if (m.ui) {
        try {
          base = readState(JSON.parse(m.ui));
        } catch {}
      }
      const st = structuredClone(base);
      if (typeof m.words === "string") st.text.words = m.words;
      if (m.horn) {
        st.horn = m.horn;
        st.hornWrap = m.hornWrap;
      }
      plugin.words = st.text.words;
      applyHost(st, m.values);
      plugin.ui = JSON.stringify(getState());
      plugin.ready = true;
      status(m.synth ? "Suu Synth" : "Suu");
    } else if (m.type === "params") {
      applyHost(getState(), m.values);
    } else if (m.type === "ui") {
      if (Math.abs(+$("bpm").value - m.bpm) > 0.005) $("bpm").value = Math.round(m.bpm * 100) / 100;
      onWorklet({ type: "ui", ui: Float32Array.from(m.ui) });
    } else if (m.type === "text") {
      plugin.words = m.words;
      if (document.activeElement !== $("text")) $("text").value = m.words;
      onWorklet({ type: "text", syllables: m.syllables });
    } else if (m.type === "snapshot") {
      onSnapshot(m.values);
    } else if (m.type === "learned") {
      setLearning(false);
      status(`reset note: ${m.note}`);
    } else if (m.type === "opened") {
      let raw;
      try {
        raw = JSON.parse(m.text);
      } catch {
        return status(`${m.name ?? "the file"}: not a preset`);
      }
      if (load(raw, m.name ?? "the preset")) status(`${m.name ?? "preset"} opened`);
    }
  };
  const grip = document.createElement("div");
  grip.className = "plugin-grip";
  document.body.append(grip);
  grip.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    grip.setPointerCapture(e.pointerId);
    const from = [e.screenX, e.screenY, innerWidth, innerHeight];
    const move = (m) => sendToPlugin({ type: "resize", width: Math.round(from[2] + m.screenX - from[0]), height: Math.round(from[3] + m.screenY - from[1]) });
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", () => grip.removeEventListener("pointermove", move), { once: true });
  });
  sendToPlugin({ type: "ready" });
}
