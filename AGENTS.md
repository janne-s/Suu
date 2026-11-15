# Suu

A talking filter: any sound through CHIMERA's physically modelled vocal tract. One Rust core
serves the web app (wasm in an AudioWorklet) and the VST3/CLAP plug-in (nih-plug, with the same
page in a webview). See the README.

## Working here

- Do what was asked, the simplest way. Side work nobody asked for (experiments, tooling, paid
  runs) is a proposal with its cost. If the work grows well past the estimate, stop and say so.
- Before repeating or reversing earlier work, read its history (`git log -- <path>`, `git log -S`).
  CHIMERA's own history is kept in this repository.
- Commit small, with a short, clear subject line and no long body (the owner's preference);
  measurements and reasoning go in the design document.
- Keep each fixed bug's reproduction as a test.
- Code outranks this file: fix the file when they disagree. When something breaks unnoticed, add
  a rule here, with a check.

## Map

- `crates/chimera/`: CHIMERA's engine (airway, anatomy, phonetics, sources, creatures), by
  Jameson Hodge. No I/O. Suu uses it as a library; keep changes to it opt-in.
- `crates/suu/`: Suu's core: the driven airway, the oscillator, Motion, text, the sequencer.
- `crates/suu-wasm/`: the C ABI the web page's worklet calls.
- `plugin/`: the plug-in, its own Cargo workspace. `src/` is shared, `fx/` is the effect,
  `synth/` the instrument, `xtask/` bundles them, `vendor/wry-0.35.2/` is a patched dependency.
- `web/`: the page (`index.html`, `app.js`, `style.css`, `worklet.js`, `assets/`). The plug-in
  embeds the same files with `include_bytes!`, so the page serves both.
- `data/`: the CMU Pronouncing Dictionary and the AVHershey fonts, with their licences. The
  engine's tests, the web build and the plug-in read them from here.
- `tools/`: `build-web.sh` (wasm, dictionary and fonts next to the page), `serve.ts` (local
  server), `e2e-web.ts` and `e2e-plugin.ts` (headless Chrome).
- `docs/rust-engine.md`: the engine's contract, from CHIMERA's port to Rust.

This is not a Node app. Bun runs only the local server and the e2e checks.

## Rules

- **No comments in code.** Only a short label separating sections of a long file
  (`// ---- sequencer`, `/* knobs */`). Explanations, reasons, history, measurements and
  instructions go in names, tests, the design documents or the commit. `crates/chimera/` and
  `plugin/vendor/` are third-party and keep theirs. Check:
  `grep -rnE '^\s*(//|/\*)' crates/suu crates/suu-wasm plugin/src plugin/fx plugin/synth plugin/xtask plugin/tests web tools | grep -vE '// -{3}|/\* [a-zA-Z -]+ \*/'`
  prints nothing.
- **The engine's contract** (`docs/rust-engine.md`): transcendental math only through
  `chimera::math`, no allocation while rendering, output independent of block size. `cargo test`
  enforces each.
- **CHIMERA's existing render path stays bit-exact.** A change to the engine's sound fails the
  golden tests by design. Re-bless only a deliberate change, with
  `CHIMERA_BLESS=1 cargo test --workspace --no-fail-fast`, run twice. See
  `crates/chimera/tests/golden/README.md`.
- **`*.generated.json` tables are never edited.** Their generators (`chimera calibrate`,
  `tools/gen-*`) are in CHIMERA's history, before the commit that made this repository Suu.
- **Parameter IDs are permanent.** Hosts save automation under them. `plugin/tests/param_ids.txt`
  holds them; adding one means appending it (`SUU_BLESS_IDS=1`), removing or renaming one fails.
- **The page is shared.** A change in `web/` changes the plug-in's window too: check both e2e
  runs. Web-only extras (Space, clip export) must stay hidden in the plug-in.
- **Plug-in state comes only from the host.** Never from browser storage, which is shared across
  instances and projects.
- **Nothing copyrighted goes in git**: no third-party recordings, scores or anything derived
  from them.

## Checks

| When you touch | Run |
|---|---|
| anything | `bun run test` and `bun run typecheck` |
| `crates/chimera/` sound | re-bless goldens (above); `cargo run --release -q -p chimera --example bench` against the previous commit |
| `crates/suu/` sound or speed | `cargo run --release -q -p suu --example bench` against the previous commit |
| `crates/suu-wasm/`, `web/` | `bun run web`, then `bun run e2e` |
| `plugin/` | `bun run build:plugin`, then pluginval (and auval for AU) |
