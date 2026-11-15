# Suu

Suu is a talking filter: any sound through a simulated human vocal tract.
Shape the mouth on a vowel pad, set it moving, type words for it to sing, or
sequence vowels and notes. In the browser, and as a VST3 and CLAP plug-in.

See it live: [**https://janne-s.github.io/Suu/**](https://janne-s.github.io/Suu/) ·
Plug-in: [**Releases**](https://github.com/janne-s/Suu/releases)

![Suu interface](web/assets/screenshot.jpg)

## How it works

The sound enters the airway as airflow and leaves through the lips, so the
mouth's shape filters it as it would a voice. The airway, articulators and
vocal folds are a physical simulation. Tilt, the sub layer, the resonator,
glide and Space are production extras. Ranges go past human anatomy on
purpose.

## Mouth

- **Vowel pad**: drag the mouth.
  - Left to right: front to back. Top to bottom: close to open.
  - The edge beyond the corner vowels extrapolates.
- **Horn**: the tract from the side, glottis to lips, as it sounds now.
  - Drag to draw your own, build it from a **Wave** (cycles and depth), or
    take one period of the sound with the camera. ↺ restores the original.
  - **Morph** blends the horn in.
  - **Shift** slides it along the tract. A wave wraps, so it can travel
    through.
  - **Open** widens it.
- **Text**: type words, and choose how they are said.
  - **Step**: each note says the next part, or the text's own **Clock** does
    (a rate, or ♪ for a note value).
  - **Unit**: a syllable or a word at a time.
  - **Say**: **Hold** keeps the vowel while the note lasts, **Shot** says the
    part through.
  - **Len**: on the Clock, how much of each tick Hold keeps the vowel. At
    100% the parts join.
  - **Start**: **Free**, or **Seq** to start over with the sequence.
  - **Repeat**: **Loop**, or **1×** once a pass.
  - **Speed**: how fast it is spoken, from ×0.25 to ×4.
  - **Gate**: how much of each part is heard.
- **Motion**: moves Vowel X/Y, Round, Nasal, Inject, Damping, Morph, Shift
  and Open. Each has:
  - **Depth**, and **Rate** or ♪.
  - A shape: sine, triangle, saw, square, random, envelope or ADSR.
  - **Smooth**, from sharp jumps to gliding ones.

## Sequencer

Sixteen steps, each with a note, vowel, accent ◆ and length. Drag a step up or
down for its note, click to turn it on, Shift-click to tie, click the vowel
to change it. **Gate** scales every step's length alike.

## Sound

- **Osc** and **In** — The oscillator (supersaw, saw, square or off) and an
  input (a file or the mic; in the plug-in, the track) sound together, each at
  its level. Keys, Hold and the sequencer gate both.
- **Body** — Size of the mouth (0.4–4×), Breath (lung pressure for the
  consonants and folds) and Follow (how far it follows the sound).
- **Source** — Osc and Input levels, Folds (the model's own vocal folds at the
  played note; 100% is about as loud as Osc at 0 dB), Reson (a resonator tuned
  to the played note, so an input plays in pitch) and Glide (pitch and vowel;
  ♪ syncs it; ↕ ↑ ↓ both ways, only up or only down).
- **Tone** — Drive into the mouth, Tilt (source brightness), and Sub (the dry
  low end).
- **Space** — Delay and reverb, web only. Off is what the plug-in does.

Keys **A W S E D F T G Y H U J K** play, **Z X** change octave, **Space**
plays the sequencer, **Esc** releases all notes. Settings are kept in the
browser with A/B and Init. Save and Open write and read preset files.

## Install the plug-in

macOS 11 or later, Apple silicon. Two plug-ins: **Suu** (an effect: the
track through the mouth, MIDI from another track) and **Suu Synth** (an
instrument: the oscillator through the mouth, a sidechain as input).

Unzip the release and copy the `.vst3` files to `~/Library/Audio/Plug-Ins/VST3/`
(and the `.clap` files to `~/Library/Audio/Plug-Ins/CLAP/`). The build is not
notarized, so clear the quarantine once (the second line for CLAP):

```sh
xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins/VST3/Suu*.vst3
xattr -dr com.apple.quarantine ~/Library/Audio/Plug-Ins/CLAP/Suu*.clap
```

## Run

Rust with the `wasm32-unknown-unknown` target, and Bun (only for the local
server and the browser checks; the page itself has no build step).

```sh
bun install
bun run web            # http://127.0.0.1:5318/
bun run build:plugin   # VST3 and CLAP in plugin/target/bundled/
```

## Tests

```sh
bun run test       # the engine, the core and the plug-in (cargo test)
bun run e2e        # headless Chrome, with bun run web running
```

## Layout

| Folder | What |
|---|---|
| `crates/chimera/` | CHIMERA's vocal-tract engine |
| `crates/suu/` | Suu's core: the driven airway, oscillator, Motion, text, sequencer |
| `crates/suu-wasm/` | the core's interface for the web page |
| `plugin/` | the VST3/CLAP effect and instrument |
| `web/` | the page, which is also the plug-in's window |
| `data/` | pronouncing dictionary and fonts, with their licenses |
| `tools/` | web build, local server, browser checks |

## Limits

English text only. Mono. The page is tested in Chrome and Safari, the
plug-in in Ableton Live on macOS (Apple silicon only for now).

## Credits

Built on [CHIMERA](https://github.com/jamesondh/chimera-vocal-synth) by Jameson
Hodge, not affiliated. Suu keeps CHIMERA's engine (`crates/chimera/`) and its
history, and adds the driven airway, the effect core, the web page and the
plug-in. CHIMERA's own voice app is not part of Suu; it remains in the original
repository. See [AGENTS.md](AGENTS.md) for the engine's rules.

## License

Copyright © 2026 Janne Särkelä. Suu is free software under the
[GNU General Public License v3.0 or later](LICENSE): you may use, change and
share it, also commercially, as long as what you share stays under the same
license with its source.

CHIMERA's engine (`crates/chimera/`) is © 2026 Jameson Hodge under the
[MIT License](crates/chimera/LICENSE). The CMU Pronouncing Dictionary and the
AVHershey fonts keep their own licenses ([data/](data/), [data/fonts/](data/fonts/)).
