#!/bin/sh
# Build Suu's wasm and copy it, the pronouncing dictionary and the fonts next to the page (web/).
set -e
cd "$(dirname "$0")/.."
cargo build -p suu-wasm --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/suu_wasm.wasm web/suu.wasm
cp data/cmudict.txt.gz data/CMUDICT-LICENSE.txt web/
mkdir -p web/fonts
cp data/fonts/AVHersheySimplexLight.otf data/fonts/AVHersheySimplexMedium.otf data/fonts/AVHERSHEY-LICENSE.txt web/fonts/
