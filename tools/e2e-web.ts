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
const lipPixels = await page.evaluate(() => {
  const c = document.getElementById("mouth") as HTMLCanvasElement;
  const d = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
  let n = 0;
  for (let i = 0; i < d.length; i += 4) if (d[i] > 150 && d[i + 1] < 140) n++;
  return n;
});
ok("the mouth shows before audio starts", lipPixels > 500, `${lipPixels} rose pixels`);
await page.keyboard.down("a"); await page.keyboard.up("a");
await page.waitForTimeout(1500);
ok("audio started from a key", await page.evaluate(() => !!(window as any).__suu));
await page.click("#spaceon");
await page.waitForTimeout(800);
let db = await peakDb(); ok("first quick key does not stick", db < -60, `${db.toFixed(1)} dB`);
await page.keyboard.down("a"); await page.waitForTimeout(400); db = await peakDb(); ok("held key sounds", db > -40, `${db.toFixed(1)} dB`);
await page.keyboard.up("a"); await page.waitForTimeout(600); db = await peakDb(); ok("released key stops", db < -60, `${db.toFixed(1)} dB`);
await page.keyboard.down("s");
await page.evaluate(() => dispatchEvent(new KeyboardEvent("keydown", { key: "Meta", code: "MetaLeft", metaKey: true })));
await page.evaluate(() => dispatchEvent(new KeyboardEvent("keyup", { key: "Meta", code: "MetaLeft" })));
await page.waitForTimeout(600); db = await peakDb(); ok("Cmd release frees a key whose keyup was lost", db < -60, `${db.toFixed(1)} dB`);
await page.keyboard.up("s");
{
  const sel = page.locator(".motion-row select.shape").first();
  const before = await sel.inputValue();
  await page.keyboard.down("d"); await page.waitForTimeout(300);
  await sel.dispatchEvent("pointerdown");
  await sel.focus();
  await page.waitForTimeout(600); db = await peakDb();
  ok("opening a list frees a held key", db < -60, `${db.toFixed(1)} dB`);
  await page.keyboard.down("a"); await page.keyboard.up("a");
  ok("a note key on a focused list picks nothing", (await sel.inputValue()) === before, `${before} → ${await sel.inputValue()}`);
  await sel.selectOption("1");
  ok("the pick lands", (await sel.inputValue()) === "1");
  await page.keyboard.up("d"); await page.waitForTimeout(600);
  await page.keyboard.down("f"); await page.waitForTimeout(400); db = await peakDb();
  ok("keys play after a pick", db > -40, `${db.toFixed(1)} dB`);
  await page.keyboard.up("f"); await page.waitForTimeout(600); db = await peakDb();
  ok("and stop", db < -60, `${db.toFixed(1)} dB`);
  await sel.selectOption(before);
  await page.waitForTimeout(600);
}
{
  await page.click('[data-osc="0"]');
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
  await page.click('[data-in="file"]');
  ok("the file off, hold is the player's", (await page.getAttribute("#hold", "aria-pressed")) === "true" && (await page.getAttribute('[data-in="file"]', "aria-pressed")) === "false");
  await page.waitForTimeout(500);
  db = await peakDb(); ok("with no oscillator and no input, hold is silent", db < -60, `${db.toFixed(1)} dB`);
  await page.click('[data-in="file"]'); await page.click('[data-osc="2"]'); await page.waitForTimeout(500);
  db = await peakDb(); ok("oscillator and file mix", db > -40, `${db.toFixed(1)} dB`);
  await page.click('[data-in="file"]');
  await page.click("#hold");
  for (const name of ["other.wav", "other.wav"]) {
    const [chooser] = await Promise.all([page.waitForEvent("filechooser", { timeout: 2000 }), page.click("#pickfile")]);
    await chooser.setFiles({ name, mimeType: "audio/wav", buffer: buf });
    await page.waitForTimeout(500);
  }
  ok("the folder loads another file", (await page.getAttribute('[data-in="file"]', "aria-pressed")) === "true" && (await page.getAttribute('[data-in="file"]', "data-tip"))!.includes("Loaded: other.wav"));
  await page.click('[data-in="file"]');
  ok("FILE still turns the file off", (await page.getAttribute('[data-in="file"]', "aria-pressed")) === "false");
  await page.click("#hold");
}
await page.click("#text-toggle");
ok("text button starts as SING", (await page.textContent("#singtext"))!.includes("SING"));
await page.click("#singtext"); await page.waitForTimeout(500);
ok("SING → STOP", (await page.textContent("#singtext"))!.includes("STOP"));
{
  const words = ((await page.inputValue("#text")).match(/[A-Za-z']+/g) ?? []).length;
  page.once("dialog", (d) => d.accept("40"));
  await page.click('output[for="stereoCanon"]');
  ok("CANON reaches no further than the text has parts", (await page.textContent('output[for="stereoCanon"]')) === `+${words - 1}`, `${await page.textContent('output[for="stereoCanon"]')} with ${words} words`);
  page.once("dialog", (d) => d.accept("0"));
  await page.click('output[for="stereoCanon"]');
}
const pressed = async (sel: string) => (await page.getAttribute(sel, "aria-pressed")) === "true";
ok("defaults: note, syllable, hold, seq, loop", (await pressed('[data-tstep="0"]')) && (await pressed('[data-tunit="0"]')) && (await pressed('[data-tsay="0"]')) && (await pressed('[data-tseq="1"]')) && (await pressed('[data-tonce="0"]')));
ok("no clock rate or length on notes", !(await page.isVisible("#clockrate")) && !(await page.isVisible("#lenbox")));
await page.click('[data-tstep="1"]');
ok("the clock shows its rate and length", (await page.isVisible("#clockrate")) && (await page.isVisible("#lenbox")));
ok("clock rate synced 1/4 by default", (await page.inputValue("#loopdiv")) === "1/4");
await page.click("#loopsync");
ok("free clock rate shown", await page.isVisible("#loophz"), await page.textContent("#loophz") ?? "");
await page.click("#loopsync");
{
  const b = await page.locator("#textlen").boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down(); await page.mouse.move(b!.x + 20, b!.y + 40, { steps: 5 }); await page.mouse.up();
  ok("length drags", (await page.textContent("#textlen")) !== "100%", await page.textContent("#textlen") ?? "");
}
await page.click('[data-tsay="1"]');
ok("a shot has no length", !(await page.isVisible("#lenbox")));
await page.click('[data-tunit="1"]'); await page.click('[data-tonce="1"]'); await page.click('[data-tseq="0"]');
ok("word, once, free", (await pressed('[data-tunit="1"]')) && (await pressed('[data-tonce="1"]')) && (await pressed('[data-tseq="0"]')));
await page.screenshot({ path: `${out}/2-text.png` });
await page.click('[data-tstep="0"]'); await page.click('[data-tunit="0"]'); await page.click('[data-tsay="0"]'); await page.click('[data-tonce="0"]'); await page.click('[data-tseq="1"]');
{
  const b = await page.locator("#speech").boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down(); await page.mouse.move(b!.x + 20, b!.y - 60, { steps: 5 }); await page.mouse.up();
  ok("speech speed drags", (await page.textContent("#speech")) !== "×1.00", await page.textContent("#speech") ?? "");
  const gb = await page.locator("#textgate").boundingBox();
  await page.mouse.move(gb!.x + 20, gb!.y + 10); await page.mouse.down(); await page.mouse.move(gb!.x + 20, gb!.y + 60, { steps: 5 }); await page.mouse.up();
  ok("text gate drags down", (await page.textContent("#textgate")) !== "100%", await page.textContent("#textgate") ?? "");
  await page.dblclick("#textgate");
  ok("and back to 100%", (await page.textContent("#textgate")) === "100%");
  await page.dblclick("#speech");
}
await page.click("#singtext"); await page.waitForTimeout(300);
ok("STOP → SING", (await page.textContent("#singtext"))!.includes("SING"));
await page.click("#text-toggle");
const g = await page.locator("#seqgate").boundingBox();
await page.mouse.move(g!.x + 10, g!.y + 10); await page.mouse.down(); await page.mouse.move(g!.x + 10, g!.y - 40, { steps: 5 }); await page.mouse.up();
ok("global gate drags", (await page.textContent("#seqgate")) !== "100%", await page.textContent("#seqgate") ?? "");
await page.mouse.move(g!.x + 10, g!.y + 10); await page.mouse.down(); await page.mouse.move(g!.x + 10, g!.y + 400, { steps: 8 }); await page.mouse.up();
ok("the gate goes down to 0", (await page.textContent("#seqgate")) === "0%", await page.textContent("#seqgate") ?? "");
await page.click("#play"); await page.waitForTimeout(700);
db = await peakDb(); ok("a gate of 0 is silent", db < -60, `${db.toFixed(1)} dB`);
await page.click("#play");
await page.mouse.move(g!.x + 10, g!.y + 10); await page.mouse.down(); await page.mouse.move(g!.x + 10, g!.y - 50, { steps: 5 }); await page.mouse.up();
await page.mouse.move(20, 300); await page.mouse.down(); await page.mouse.move(1400, 880, { steps: 10 }); await page.mouse.up();
ok("dragging selects nothing", (await page.evaluate(() => getSelection()!.toString())) === "");
await page.click("#rate");
await page.keyboard.press("Escape");
const outline = () => page.evaluate(() => getComputedStyle(document.activeElement!).outlineStyle);
ok("no focus ring after a click", (await outline()) === "none", await outline());
await page.keyboard.press("Tab");
ok("a focus ring after Tab", (await outline()) !== "none", await outline());
await page.mouse.click(700, 5);
await page.click("#glidedir");
ok("glide direction cycles", (await page.textContent("#glidedir")) === "↑");
await page.click("#glidedir"); await page.click("#glidedir");
ok("and back to both ways", (await page.textContent("#glidedir")) === "↕");
ok("sync down to 1/128", (await page.locator(".motion-row select.field[hidden], .motion-row .rate select").first().locator('option', { hasText: "1/128" }).count()) === 1);
ok("the sequencer runs at 1/128 too", (await page.locator('#rate option[value="32"]').count()) === 1);
ok("glide knob", (await page.locator("#glide").count()) === 1, await page.textContent('output[for="glide"]') ?? "");
const preset = async (sel: string) => {
  if (!(await page.isVisible("#preset-panel"))) await page.click("#preset-toggle");
  await page.click(sel);
};
await page.fill("#bpm", "123"); await page.dispatchEvent("#bpm", "change");
await preset("#slotB");
ok("the PRESET button names the side", (await page.textContent("#preset-toggle")) === "PRESET B");
ok("B starts as a copy of A", (await page.inputValue("#bpm")) === "123");
await page.fill("#bpm", "88"); await page.dispatchEvent("#bpm", "change");
await preset("#slotA");
ok("A keeps its own", (await page.inputValue("#bpm")) === "123");
await preset("#slotB");
ok("B keeps its own", (await page.inputValue("#bpm")) === "88");
await page.waitForTimeout(1200);
await page.reload({ waitUntil: "networkidle" });
ok("reload keeps B active and its tempo", (await page.inputValue("#bpm")) === "88" && (await page.getAttribute("#slotB", "aria-pressed")) === "true");
ok("reload keeps the gate", (await page.textContent("#seqgate")) !== "100%", await page.textContent("#seqgate") ?? "");
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
await page.click("#view-horn");
await page.selectOption("#horn-shape", "0");
let saved = async () => { await page.waitForTimeout(1200); return await page.evaluate(() => JSON.parse(localStorage.getItem("suu.session.v1")!).B); };
let st = await saved();
ok("SIN builds a wrapping wave horn", st.hornWrap === true && st.hornWave.shape === 0 && Math.abs(st.horn[8] - st.horn[24]) > 2, JSON.stringify(st.horn.slice(0, 6)));
await page.selectOption("#horn-shape", "4");
st = await saved();
ok("RND builds an uneven horn at once, with one cycle", st.hornWave.shape === 4 && st.hornWave.cycles === 1 && new Set(st.horn.map((a: number) => a.toFixed(3))).size >= 4, JSON.stringify(st.horn.slice(0, 8)));
await page.selectOption("#horn-shape", "0");
st = await saved();
await page.screenshot({ path: `${out}/5-wave.png` });
await page.keyboard.down("a");
await page.waitForTimeout(300);
await page.click("#horn-snap");
await page.keyboard.up("a");
st = await saved();
ok("the camera snapshots the sound into the horn", st.hornWave.shape === 5 && st.hornWave.snap.length === 32 && st.hornWrap === true, JSON.stringify(st.hornWave.snap.slice(0, 4)));
await page.click("#horn-reset");
st = await saved();
ok("reset icon restores the original horn", st.hornWrap === false && st.hornWave.shape === -1 && Math.abs(st.horn[0] - 0.6) < 0.05, JSON.stringify(st.horn.slice(0, 3)));
await page.selectOption("#horn-shape", "0");
let before = (await saved()).horn as number[];
{
  const b = await page.locator("#horn-depth").boundingBox();
  const m = await page.locator("#mouth").boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down();
  await page.mouse.move(m!.x + m!.width * 0.6, m!.y + 20, { steps: 8 });
  await page.mouse.move(b!.x + 20, b!.y + 10, { steps: 8 });
  await page.mouse.up();
  const bar = await page.locator("#horn-bar").boundingBox();
  await page.mouse.move(bar!.x + 3, bar!.y + 3); await page.mouse.down();
  await page.mouse.move(m!.x + m!.width * 0.5, m!.y + 30, { steps: 8 }); await page.mouse.up();
}
let after = (await saved()).horn as number[];
ok("a drag that began elsewhere draws nothing on the horn", after.every((a, i) => Math.abs(a - before[i]) < 1e-6), JSON.stringify(after.map((a, i) => +(a - before[i]).toFixed(2)).filter((d) => d)));
await page.click("#horn-reset");
await page.selectOption("#horn-shape", "0");
before = (await saved()).horn as number[];
{
  const m = await page.locator("#mouth").boundingBox();
  await page.mouse.move(m!.x + m!.width * 0.92, m!.y + m!.height * 0.15); await page.mouse.down();
  await page.mouse.move(m!.x + m!.width * 0.93, m!.y + m!.height * 0.12, { steps: 3 }); await page.mouse.up();
}
after = (await saved()).horn as number[];
const moved = after.map((a, i) => (Math.abs(a - before[i]) > 1e-6 ? i : -1)).filter((i) => i >= 0);
ok("drawing at the lips' end stays there", moved.length > 0 && moved.every((i) => i > 24), JSON.stringify(moved));
await page.click("#horn-reset");
await page.click("#view-horn");
for (const [id, dy] of [["#horn-depth", -50], ["#horn-cycles", -60]] as const) {
  const b = await page.locator(id).boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down(); await page.mouse.move(b!.x + 20, b!.y + dy, { steps: 6 }); await page.mouse.up();
}
st = await saved();
ok("depth and cycles keep the drawn horn", st.hornWave.shape === -1 && st.hornWave.cycles > 1 && st.hornWave.depth > 0.6 && Math.abs(st.horn[0] - 0.6) > 0.05, JSON.stringify({ c: st.hornWave.cycles, d: st.hornWave.depth, h0: st.horn[0] }));
await page.click("#horn-reset");
await page.click("#view-pad");
{
  const box = page.locator(".motion-row").nth(1).locator(".smooth");
  const b = await box.boundingBox();
  await page.mouse.move(b!.x + 20, b!.y + 10); await page.mouse.down(); await page.mouse.move(b!.x + 20, b!.y - 60, { steps: 5 }); await page.mouse.up();
  const st = await (async () => { await page.waitForTimeout(1200); return await page.evaluate(() => JSON.parse(localStorage.getItem("suu.session.v1")!).B); })();
  ok("smooth drags and is saved", st.motion.vowelY.smooth > 0.2, String(st.motion.vowelY.smooth));
  ok("Motion reset is an icon", (await page.locator("#motion-reset svg").count()) === 1);
}
{
  await page.mouse.move(5, 300);
  await page.hover("#hold"); await page.waitForTimeout(800);
  ok("a tooltip names what a control does", (await page.isVisible("#tip")) && (await page.textContent("#tip"))!.startsWith("Hold"), await page.textContent("#tip") ?? "");
  await page.click("#help-toggle"); await page.mouse.move(5, 300);
  await page.hover("#hold"); await page.waitForTimeout(800);
  ok("? turns tooltips off", !(await page.isVisible("#tip")));
  await page.reload({ waitUntil: "networkidle" });
  ok("and it is remembered", (await page.getAttribute("#help-toggle", "aria-pressed")) === "false");
  await page.click("#help-toggle");
  const bare = await page.evaluate(() => [...document.querySelectorAll("button, select, input, .field.drag, .knob")].filter((el) => (el as HTMLElement).offsetParent && !el.closest("[data-tip]") && !el.closest(".popover")).map((el) => el.id || el.className || el.tagName));
  ok("every visible control has a tip", bare.length === 0, JSON.stringify(bare));
}
{
  await page.fill("#seqlen", "20");
  await page.press("#seqlen", "Enter");
  ok("STEPS typed 20 shows 16", (await page.inputValue("#seqlen")) === "16", await page.inputValue("#seqlen"));
  const sl = await page.locator("#seqlen").boundingBox();
  await page.mouse.move(sl!.x + sl!.width / 2, sl!.y + sl!.height / 2);
  await page.mouse.wheel(0, -100);
  await page.waitForTimeout(50);
  const up = await page.inputValue("#seqlen");
  await page.mouse.wheel(0, 120);
  await page.waitForTimeout(50);
  const down = await page.inputValue("#seqlen");
  ok("STEPS wheel stays at 16 up, 3 steps down for 120 px", up === "16" && down === "13", `${up} ${down}`);
  await page.fill("#seqlen", "0");
  await page.press("#seqlen", "Enter");
  ok("STEPS typed 0 shows 1", (await page.inputValue("#seqlen")) === "1");
  await page.fill("#seqlen", "16");
  await page.press("#seqlen", "Enter");
  await page.fill("#bpm", "500");
  await page.press("#bpm", "Enter");
  ok("BPM typed 500 shows 300", (await page.inputValue("#bpm")) === "300", await page.inputValue("#bpm"));
}
await preset("#init");
ok("INIT restores the defaults", (await page.inputValue("#bpm")) === "100" && (await page.textContent("#seqgate")) === "100%");
await page.keyboard.press("Escape");
await page.click("#stereoon");
await page.keyboard.down("a");
await page.waitForTimeout(1200);
const stereoPeak = await peakDb();
await page.keyboard.up("a");
ok("STEREO on and it still sounds", (await page.textContent("#stereoon")) === "ON" && stereoPeak > -40, stereoPeak.toFixed(1) + " dB");
await page.click("#stereoon");
ok("no MPE button on the web", !(await page.isVisible("#mpe-toggle")));
ok("PRESET closes", !(await page.isVisible("#preset-panel")));
ok("the version in the info", /^Version \d+\.\d+\.\d+$/.test((await page.textContent("#version")) ?? ""), (await page.textContent("#version")) ?? "");
await page.click("#pulse-more");
ok("▾ opens PULSE", await page.isVisible("#pulse-panel") && (await page.textContent('output[for="pulseWidth"]')) === "50%");
await page.mouse.click(5, 300);
ok("PULSE closes", !(await page.isVisible("#pulse-panel")));
await page.selectOption("#seqopen", "2");
ok("OPEN + KEYS kept in the state", await page.evaluate(() => JSON.parse(localStorage.getItem("suu.session.v1") ?? "{}") !== null) && (await page.inputValue("#seqopen")) === "2");
await page.selectOption("#seqopen", "0");
ok("ASPIR, FRIC and SIBIL at 100%", (await Promise.all(["aspiration", "frication", "sibilance"].map((k) => page.textContent(`output[for="${k}"]`)))).every((t) => t === "100%"));
await page.click("#hq");
ok("quality steps to LEAN", (await page.textContent("#hq")) === "LEAN");
await page.click("#hq");
ok("then LO", (await page.textContent("#hq")) === "LO");
await page.click("#hq");
ok("and back to HQ", (await page.textContent("#hq")) === "HQ");
ok("ROUND 0% and FIT off by default", (await page.textContent("#textround")) === "0%" && (await page.getAttribute("#textfit", "aria-pressed")) === "false");
{
  const glide = page.locator("#glidedir").locator("..");
  if ((await glide.locator("select").getAttribute("hidden")) !== null) await glide.locator(".sync").first().click();
  const free = await page.evaluate(() => {
    const dir = document.getElementById("glidedir")!;
    const box = dir.parentElement!.querySelector("select")!.getBoundingClientRect();
    return [...dir.parentElement!.querySelectorAll(".sync")].every((b) => {
      const r = b.getBoundingClientRect();
      return r.left >= box.right && document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2) === b;
    });
  });
  ok("glide's ♪ and direction sit beside the note value, not over it", free);
  await glide.locator(".sync").first().click();
}
ok("resonator knob, off by default", (await page.textContent('output[for="reson"]')) === "off");
ok("GitHub link at the header's right end", await page.evaluate(() => {
  const a = document.getElementById("github") as HTMLAnchorElement, bar = document.querySelector(".bar")!;
  return a.href === "https://github.com/janne-s/Suu" && bar.lastElementChild === a && a.getBoundingClientRect().right > innerWidth - 40;
}));
ok("credits name the author and the repo", (await page.locator('#info a[href="https://github.com/janne-s/Suu"]').count()) === 1);
await page.evaluate(() => {
  localStorage.setItem("suu.session.v1", JSON.stringify({ active: "A", A: { suu: 1, bpm: 140, seq: { steps: 5 } } }));
  Storage.prototype.setItem = () => {};
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
{
  const first = await ctx.newPage();
  first.on("pageerror", (e) => errors.push(e.message));
  await first.goto(url, { waitUntil: "networkidle" });
  await first.click("#play");
  await first.waitForTimeout(1500);
  ok("PLAY as the first gesture runs the sequencer", (await first.locator(".step.now").count()) === 1);
  await first.setViewportSize({ width: 390, height: 844 });
  await first.waitForTimeout(200);
  const shown = () => first.locator(".motion-head").isVisible();
  const wide = () => first.evaluate(() => document.documentElement.scrollWidth);
  ok("phone: Motion folds to its title row", !(await shown()) && (await first.isVisible("#grid")));
  ok("phone: no sideways scroll", (await wide()) <= 390, String(await wide()));
  await first.click("#motion-fold");
  ok("phone: its title opens it", await shown());
  ok("phone: no sideways scroll with Motion open", (await wide()) <= 390, String(await wide()));
  await first.screenshot({ path: `${out}/6-phone.png`, fullPage: true });
  await first.close();
}
ok("no console errors", errors.length === 0, errors.join(" | "));
await browser.close();
process.exit(failed ? 1 : 0);
