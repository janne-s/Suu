# Suu

A talking filter: sound through a physically modelled vocal tract. One Rust core, two shells: a
web page (wasm in an AudioWorklet) and a VST3/CLAP plug-in (nih-plug) that shows the same page
in a webview.

## Layout

- `crates/chimera/`: the vocal-tract engine, from CHIMERA (third party). Suu uses it as a library.
- `crates/suu/`: Suu's core. `crates/suu-wasm/`: its C ABI for the worklet.
- `plugin/`: a separate Cargo workspace: shared `src/`, the effect `fx/`, the instrument `synth/`.
- `web/`: the page. The plug-in embeds these files with `include_bytes!`.
- `data/`: dictionary and fonts. `tools/`: web build, local server, e2e checks.

## Commands

```sh
bun run test          # cargo tests: engine, core, plug-in
bun run typecheck
bun run web           # build and serve the page at :5318 (PORT=… for another)
bun run e2e           # headless Chrome, against the running page
bun run build:plugin  # VST3 and CLAP in plugin/target/bundled/
cargo run --release -p suu --example bench   # compare with the previous commit
```

## Rules

- No comments in code, except short section labels (`// ---- sequencer`). Say it in names and
  tests.
- The audio thread never allocates, locks or makes system calls, and output does not depend on
  the block size. Math goes through `chimera::math`. Tests enforce these.
- Changes to `crates/chimera/` are opt-in and keep its golden tests passing. Re-bless only a
  deliberate change: `CHIMERA_BLESS=1 cargo test --workspace --no-fail-fast`, run twice.
- `*.generated.json` files are generated, never edited.
- Plug-in parameter IDs never change (`plugin/tests/param_ids.txt`; append new ones with
  `SUU_BLESS_IDS=1`).
- A change in `web/` also changes the plug-in's window: run both e2e checks. Web-only parts stay
  hidden in the plug-in.
- Plug-in state comes only from the host, never from browser storage.
- Keep each fixed bug's reproduction as a test.
- No copyrighted recordings or scores in git.
- Commit messages are one short subject line.
