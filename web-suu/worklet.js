// Suu's AudioWorklet: one wasm instance, mono in → mono out (copied to both channels).
// From the page: {type:"param", id, v} · {type:"dict", bytes} · {type:"text", bytes}
//   · {type:"step", i, on, note, tie, vowel, accent, len} · {type:"seq", len, div, gate}
//   · {type:"play", on} · {type:"retrigger"} · {type:"profile", areas: Float32Array, wrap}
//   · {type:"mod", target, depth, rate, sync, shape, smooth}
// To the page: ready · dict · text {syllables} · applied {id} · ui {ui} (see Suu::ui_state)
//   · snapshot {values: Float32Array(32) | null} (after {type:"snapshot"})

const FRAME_EVERY = 1 / 30; // s

class SuuProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.x = new WebAssembly.Instance(options.processorOptions.module, {}).exports;
    this.h = this.x.suu_new(sampleRate);
    this.views();
    this.untilFrame = 0;
    this.port.onmessage = (e) => this.message(e.data);
    this.port.postMessage({ type: "ready" });
  }

  views() {
    const buf = this.x.memory.buffer;
    this.mem = buf;
    this.inp = new Float32Array(buf, this.x.suu_input(this.h), 256);
    this.out = new Float32Array(buf, this.x.suu_output(this.h), 256);
  }

  bytes(bytes) {
    const p = this.x.suu_alloc(bytes.length);
    new Uint8Array(this.x.memory.buffer, p, bytes.length).set(bytes);
    return p;
  }

  message(m) {
    const x = this.x, h = this.h;
    switch (m.type) {
      case "param":
        x.suu_set(h, m.id, m.v);
        if (m.ack) this.port.postMessage({ type: "applied", id: m.id });
        break;
      case "dict": {
        const p = this.bytes(m.bytes);
        x.suu_load_dict(h, p, m.bytes.length);
        x.suu_free(p, m.bytes.length);
        this.port.postMessage({ type: "dict" });
        break;
      }
      case "text": {
        const p = this.bytes(m.bytes);
        const n = x.suu_set_text(h, p, m.bytes.length);
        x.suu_free(p, m.bytes.length);
        this.port.postMessage({ type: "text", syllables: n });
        break;
      }
      case "step":
        x.suu_set_step(h, m.i, m.on ? 1 : 0, m.note, m.tie ? 1 : 0, m.vowel, m.accent ? 1 : 0, m.len);
        break;
      case "seq":
        x.suu_set_seq(h, m.len, m.div, m.gate);
        break;
      case "profile": {
        const bytes = new Uint8Array(m.areas.buffer, m.areas.byteOffset, m.areas.byteLength);
        const p = this.bytes(bytes);
        x.suu_set_profile(h, p, m.areas.length, m.wrap ? 1 : 0);
        x.suu_free(p, bytes.length);
        break;
      }
      case "snapshot": {
        const p = x.suu_snapshot(h);
        const values = p ? new Float32Array(this.x.memory.buffer, p, 32).slice() : null;
        this.port.postMessage({ type: "snapshot", values });
        break;
      }
      case "play":
        x.suu_play(h, m.on ? 1 : 0);
        break;
      case "mod":
        x.suu_set_mod(h, m.target, m.depth, m.rate, m.sync, m.shape, m.smooth ?? 0);
        break;
      case "retrigger":
        x.suu_retrigger(h);
        break;
    }
    if (this.x.memory.buffer !== this.mem) this.views();
  }

  process(inputs, outputs) {
    if (this.x.memory.buffer !== this.mem) this.views();
    const out = outputs[0];
    const n = out[0].length;
    const inp = inputs[0];
    if (inp && inp.length) {
      const a = inp[0], b = inp[1];
      for (let i = 0; i < n; i++) this.inp[i] = b ? 0.5 * (a[i] + b[i]) : a[i];
    } else {
      this.inp.fill(0, 0, n);
    }
    this.x.suu_process(this.h, n);
    for (const ch of out) ch.set(this.out.subarray(0, n));
    this.untilFrame -= n / sampleRate;
    if (this.untilFrame <= 0) {
      this.untilFrame += FRAME_EVERY;
      const len = this.x.suu_ui_update(this.h);
      const ui = new Float32Array(this.x.memory.buffer, this.x.suu_ui(this.h), len).slice();
      this.port.postMessage({ type: "ui", ui }, [ui.buffer]);
    }
    return true;
  }
}

registerProcessor("suu", SuuProcessor);
