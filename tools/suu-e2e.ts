/**
 * Headless-Chrome check of the Suu page (web-suu/): audio starts from the first gesture, no stuck
 * notes (a key released while the audio starts; a keyup lost while Cmd is down), the text's
 * sing/stop button and loop rate, the global gate, no selection when dragging, A/B and saved
 * state (kept over a reload; a broken one refused), and one window at three sizes.
 * Usage: bun run suu (in another shell), then bun tools/suu-e2e.ts [url] [outdir]
 */
import { chromium } from "playwright-core";
import { mkdirSync } from "node:fs";
const url = process.argv[2] ?? "http://127.0.0.1:5318/";
const out = process.argv[3] ?? "out/suu-e2e";
mkdirSync(out, { recursive: true });
let failed = 0;
const browser = await chromium.launch({ headless: true, channel: "chrome", args: ["--autoplay-policy=no-user-gesture-required"] });
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
const errors: string[] = [];
const page = await ctx.newPage();
page.on("pageerror", (e) => errors.push(e.message));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
const ok = (name: string, cond: boolean, extra = "") => {
  if (!cond) failed++;
  console.log(`${cond ? "PASS" : "FAIL"} ${name} ${extra}`);
};
const peakDb = () => page.evaluate(async () => {
  const a = (window as any).__suu?.analyser; if (!a) return -999;
  let p = 0; const t = new Float32Array(a.fftSize);
  for (let k = 0; k < 10; k++) { a.getFloatTimeDomainData(t); for (const v of t) p = Math.max(p, Math.abs(v)); await new Promise(r => setTimeout(r, 30)); }
  return 20 * Math.log10(p + 1e-9);
});
await page.goto(url, { waitUntil: "networkidle" });
await page.evaluate(() => localStorage.clear());
await page.reload({ waitUntil: "networkidle" });
await page.screenshot({ path: `${out}/1-idle.png` });
ok("no START/ON button", (await page.locator("#start").count()) === 0);
// the mouth is drawn before any click (a silent engine on the page draws the first frame)
const lipPixels = await page.evaluate(() => {
  const c = document.getElementById("mouth") as HTMLCanvasElement;
  const d = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
  let n = 0;
  for (let i = 0; i < d.length; i += 4) if (d[i] > 150 && d[i + 1] < 140) n++; // the lips' rose
  return n;
});
ok("the mouth shows before audio starts", lipPixels > 500, `${lipPixels} rose pixels`);
// stuck note 1: very first key, released while the audio is still starting
await page.keyboard.down("a"); await page.keyboard.up("a");
await page.waitForTimeout(1500);
ok("audio started from a key", await page.evaluate(() => !!(window as any).__suu));
await page.click("#spaceon"); // no tails: measure the mouth alone
await page.waitForTimeout(800);
let db = await peakDb(); ok("first quick key does not stick", db < -60, `${db.toFixed(1)} dB`);
// a normal note sounds
await page.keyboard.down("a"); await page.waitForTimeout(400); db = await peakDb(); ok("held key sounds", db > -40, `${db.toFixed(1)} dB`);
await page.keyboard.up("a"); await page.waitForTimeout(600); db = await peakDb(); ok("released key stops", db < -60, `${db.toFixed(1)} dB`);
// stuck note 2: Cmd held, the note key's keyup never arrives (macOS), Cmd released
await page.keyboard.down("s");
await page.evaluate(() => dispatchEvent(new KeyboardEvent("keydown", { key: "Meta", code: "MetaLeft", metaKey: true })));
await page.evaluate(() => dispatchEvent(new KeyboardEvent("keyup", { key: "Meta", code: "MetaLeft" })));
await page.waitForTimeout(600); db = await peakDb(); ok("Cmd release frees a key whose keyup was lost", db < -60, `${db.toFixed(1)} dB`);
await page.keyboard.up("s");
// a file: hold opens it at once, keys gate it without hold, and the synth gets hold back off
{
  const n = 48000, buf = Buffer.alloc(44 + 2 * n);
  buf.write("RIFF", 0); buf.writeUInt32LE(36 + 2 * n, 4); buf.write("WAVEfmt ", 8); buf.writeUInt32LE(16, 16);
  buf.writeUInt16LE(1, 20); buf.writeUInt16LE(1, 22); buf.writeUInt32LE(48000, 24); buf.writeUInt32LE(96000, 28);
  buf.writeUInt16LE(2, 32); buf.writeUInt16LE(16, 34); buf.write("data", 36); buf.writeUInt32LE(2 * n, 40);
  for (let i = 0; i < n; i++) buf.writeInt16LE(Math.round(8000 * (((i * 110) / 48000) % 1 - 0.5)), 44 + 2 * i);
  await page.setInputFiles("#file", { name: "saw.wav", mimeType: "audio/wav", buffer: buf });
  await page.waitForTimeout(800);
  ok("a file turns hold on", (await page.getAttribute("#hold", "aria-pressed")) === "true");
  db = await peakDb(); ok("the file sounds at once", db > -40, `${db.toFixed(1)} dB`);
  await page.click("#hold"); await page.waitForTimeout(600);
  db = await peakDb(); ok("hold off closes the file", db < -60, `${db.toFixed(1)} dB`);
  await page.keyboard.down("d"); await page.waitForTimeout(400);
  db = await peakDb(); ok("a key opens the file", db > -40, `${db.toFixed(1)} dB`);
  await page.keyboard.up("d");
  await page.click("#hold");
  await page.click('[data-src="2"]');
  ok("back on the synth, hold is the player's", (await page.getAttribute("#hold", "aria-pressed")) === "true");
  await page.click("#hold");
}
// text: one button
await page.click("#text-toggle");
ok("text button starts as SING", (await page.textContent("#singtext"))!.includes("SING"));
await page.click("#singtext"); await page.waitForTimeout(500);
ok("SING → STOP", (await page.textContent("#singtext"))!.includes("STOP"));
await page.click('[data-tmode="0"]');
ok("loop shows its rate", await page.isVisible("#looprate"));
ok("loop rate synced 1/4 by default", (await page.inputValue("#loopdiv")) === "1/4");
await page.click("#loopsync");
ok("free loop rate shown", await page.isVisible("#loophz"), await page.textContent("#loophz") ?? "");
ok("the loop follows the sequence by default", (await page.textContent("#loopseq")) === "SEQ");
await page.click("#loopseq");
ok("SEQ → FREE", (await page.textContent("#loopseq")) === "FREE");
await page.click("#loopseq");
await page.click("#looponce");
ok("once toggles", (await page.getAttribute("#looponce", "aria-pressed")) === "true");
await page.click("#looponce");
{
  const b = await page.locator("#speech").boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down(); await page.mouse.move(b!.x + 20, b!.y - 60, { steps: 5 }); await page.mouse.up();
  ok("speech speed drags", (await page.textContent("#speech")) !== "×1.00", await page.textContent("#speech") ?? "");
  await page.dblclick("#speech");
}
await page.screenshot({ path: `${out}/2-text.png` });
await page.click("#singtext"); await page.waitForTimeout(300);
ok("STOP → SING", (await page.textContent("#singtext"))!.includes("SING"));
await page.click("#text-toggle");
// global gate drag
const g = await page.locator("#seqgate").boundingBox();
await page.mouse.move(g!.x + 10, g!.y + 10); await page.mouse.down(); await page.mouse.move(g!.x + 10, g!.y - 40, { steps: 5 }); await page.mouse.up();
ok("global gate drags", (await page.textContent("#seqgate")) !== "100%", await page.textContent("#seqgate") ?? "");
await page.mouse.move(g!.x + 10, g!.y + 10); await page.mouse.down(); await page.mouse.move(g!.x + 10, g!.y + 400, { steps: 8 }); await page.mouse.up();
ok("the gate goes down to 0", (await page.textContent("#seqgate")) === "0%", await page.textContent("#seqgate") ?? "");
await page.click("#play"); await page.waitForTimeout(700);
db = await peakDb(); ok("a gate of 0 is silent", db < -60, `${db.toFixed(1)} dB`);
await page.click("#play");
await page.mouse.move(g!.x + 10, g!.y + 10); await page.mouse.down(); await page.mouse.move(g!.x + 10, g!.y - 50, { steps: 5 }); await page.mouse.up();
// no selection when dragging across the page
await page.mouse.move(20, 300); await page.mouse.down(); await page.mouse.move(1400, 880, { steps: 10 }); await page.mouse.up();
ok("dragging selects nothing", (await page.evaluate(() => getSelection()!.toString())) === "");
// focus: nothing after a click, a faint ring after Tab
await page.click("#rate");
await page.keyboard.press("Escape");
const outline = () => page.evaluate(() => getComputedStyle(document.activeElement!).outlineStyle);
ok("no focus ring after a click", (await outline()) === "none", await outline());
await page.keyboard.press("Tab");
ok("a focus ring after Tab", (await outline()) !== "none", await outline());
await page.mouse.click(700, 5);
// glide knob
ok("glide knob", (await page.locator("#glide").count()) === 1, await page.textContent('output[for="glide"]') ?? "");
// A/B
await page.fill("#bpm", "123"); await page.dispatchEvent("#bpm", "change");
await page.click("#slotB");
ok("B starts as a copy of A", (await page.inputValue("#bpm")) === "123");
await page.fill("#bpm", "88"); await page.dispatchEvent("#bpm", "change");
await page.click("#slotA");
ok("A keeps its own", (await page.inputValue("#bpm")) === "123");
await page.click("#slotB");
ok("B keeps its own", (await page.inputValue("#bpm")) === "88");
await page.waitForTimeout(1200);
await page.reload({ waitUntil: "networkidle" });
ok("reload keeps B active and its tempo", (await page.inputValue("#bpm")) === "88" && (await page.getAttribute("#slotB", "aria-pressed")) === "true");
ok("reload keeps the gate", (await page.textContent("#seqgate")) !== "100%", await page.textContent("#seqgate") ?? "");
// HORN: draw, and the morph comes in
await page.click("#view-horn");
await page.keyboard.down("a");
const m = await page.locator("#mouth").boundingBox();
await page.mouse.move(m!.x + m!.width * 0.2, m!.y + m!.height * 0.5 - 60);
await page.mouse.down();
await page.mouse.move(m!.x + m!.width * 0.8, m!.y + m!.height * 0.5 - 20, { steps: 12 });
await page.mouse.up();
await page.waitForTimeout(500);
db = await peakDb(); ok("a drawn horn sounds", db > -50, `${db.toFixed(1)} dB`);
await page.keyboard.up("a");
ok("drawing brings MORPH in", (await page.locator(".motion-row").nth(6).locator(".base output").textContent()) === "horn");
await page.screenshot({ path: `${out}/4-horn.png` });
await page.waitForTimeout(1200);
await page.reload({ waitUntil: "networkidle" });
const hornSaved = await page.evaluate(() => JSON.parse(localStorage.getItem("suu.session.v1")!).B.horn as number[]);
ok("the drawn horn is saved", hornSaved.length === 32 && Math.abs(hornSaved[10] - hornSaved[25]) > 0.1, JSON.stringify(hornSaved.slice(8, 12)));
// a wave horn: built from SIN, the bar's reset icon brings the original back
await page.click("#view-horn");
await page.selectOption("#horn-shape", "0");
let saved = async () => { await page.waitForTimeout(1200); return await page.evaluate(() => JSON.parse(localStorage.getItem("suu.session.v1")!).B); };
let st = await saved();
ok("SIN builds a wrapping wave horn", st.hornWrap === true && st.hornWave.shape === 0 && Math.abs(st.horn[8] - st.horn[24]) > 2, JSON.stringify(st.horn.slice(0, 6)));
await page.screenshot({ path: `${out}/5-wave.png` });
// snapshot: the synth's waveform (a supersaw) becomes the horn
await page.keyboard.down("a");
await page.waitForTimeout(300);
await page.click("#horn-snap");
await page.keyboard.up("a");
st = await saved();
ok("the camera snapshots the sound into the horn", st.hornWave.shape === 5 && st.hornWave.snap.length === 32 && st.hornWrap === true, JSON.stringify(st.hornWave.snap.slice(0, 4)));
await page.click("#horn-reset");
st = await saved();
ok("reset icon restores the original horn", st.hornWrap === false && st.hornWave.shape === -1 && Math.abs(st.horn[0] - 0.6) < 0.05, JSON.stringify(st.horn.slice(0, 3)));
// dragging Depth, then moving over the horn, draws nothing (the owner's spike)
await page.selectOption("#horn-shape", "0");
let before = (await saved()).horn as number[];
{
  const b = await page.locator("#horn-depth").boundingBox();
  const m = await page.locator("#mouth").boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down();
  await page.mouse.move(m!.x + m!.width * 0.6, m!.y + 20, { steps: 8 }); // up over the horn
  await page.mouse.move(b!.x + 20, b!.y + 10, { steps: 8 }); // and back: depth unchanged
  await page.mouse.up();
  // a press on the bar's edge (no control there) moved over the horn: 17 points drew before
  const bar = await page.locator("#horn-bar").boundingBox();
  await page.mouse.move(bar!.x + 3, bar!.y + 3); await page.mouse.down();
  await page.mouse.move(m!.x + m!.width * 0.5, m!.y + 30, { steps: 8 }); await page.mouse.up();
}
let after = (await saved()).horn as number[];
ok("a drag that began elsewhere draws nothing on the horn", after.every((a, i) => Math.abs(a - before[i]) < 1e-6), JSON.stringify(after.map((a, i) => +(a - before[i]).toFixed(2)).filter((d) => d)));
await page.click("#horn-reset");
// Depth and Cycles work on a drawn horn too, and keep it drawn ("—")
await page.click("#view-horn");
for (const [id, dy] of [["#horn-depth", -50], ["#horn-cycles", -60]] as const) {
  const b = await page.locator(id).boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down(); await page.mouse.move(b!.x + 20, b!.y + dy, { steps: 6 }); await page.mouse.up();
}
st = await saved();
ok("depth and cycles keep the drawn horn", st.hornWave.shape === -1 && st.hornWave.cycles > 1 && st.hornWave.depth > 0.6 && Math.abs(st.horn[0] - 0.6) > 0.05, JSON.stringify({ c: st.hornWave.cycles, d: st.hornWave.depth, h0: st.horn[0] }));
await page.click("#horn-reset");
await page.click("#view-pad");
// Motion SMOOTH: a field per row, saved with the state
{
  const box = page.locator(".motion-row").nth(1).locator(".smooth");
  const b = await box.boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down(); await page.mouse.move(b!.x + 20, b!.y - 60, { steps: 5 }); await page.mouse.up();
  const st = await (async () => { await page.waitForTimeout(1200); return await page.evaluate(() => JSON.parse(localStorage.getItem("suu.session.v1")!).B); })();
  ok("smooth drags and is saved", st.motion.vowelY.smooth > 0.2, String(st.motion.vowelY.smooth));
  ok("Motion reset is an icon", (await page.locator("#motion-reset svg").count()) === 1);
}
// INIT: this side back to the initial settings
await page.click("#init");
ok("INIT restores the defaults", (await page.inputValue("#bpm")) === "100" && (await page.textContent("#seqgate")) === "100%");
ok("resonator knob, off by default", (await page.textContent('output[for="reson"]')) === "off");
ok("credits name the author and the repo", (await page.locator('#info a[href="https://github.com/janne-s/Suu"]').count()) === 1);
// a broken saved state is refused without breaking the page
await page.evaluate(() => {
  localStorage.setItem("suu.session.v1", JSON.stringify({ active: "A", A: { suu: 1, bpm: 140, seq: { steps: 5 } } }));
  Storage.prototype.setItem = () => {}; // this page's last save must not overwrite it
});
await page.reload({ waitUntil: "networkidle" });
ok("broken state refused, defaults kept", (await page.inputValue("#bpm")) === "100", await page.textContent("#status") ?? "");
for (const [w, h] of [[1440, 900], [1280, 720], [1024, 640]]) {
  await page.setViewportSize({ width: w, height: h });
  await page.waitForTimeout(200);
  const scroll = await page.evaluate(() => [document.documentElement.scrollWidth > innerWidth, document.documentElement.scrollHeight > innerHeight]);
  ok(`${w}x${h} one window`, !scroll[0] && !scroll[1], JSON.stringify(scroll));
  await page.screenshot({ path: `${out}/3-${w}.png` });
}
ok("no console errors", errors.length === 0, errors.join(" | "));
await browser.close();
process.exit(failed ? 1 : 0);
