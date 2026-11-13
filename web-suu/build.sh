#!/bin/sh
# Build Suu's wasm and copy it and the pronouncing dictionary next to the page.
set -e
cd "$(dirname "$0")/.."
cargo build -p suu-wasm --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/suu_wasm.wasm web-suu/suu.wasm
cp public/data/cmudict.txt.gz web-suu/cmudict.txt.gz
cp public/data/CMUDICT-LICENSE.txt web-suu/CMUDICT-LICENSE.txt
mkdir -p web-suu/fonts
cp public/fonts/AVHersheySimplexLight.otf public/fonts/AVHersheySimplexMedium.otf public/fonts/AVHERSHEY-LICENSE.txt web-suu/fonts/
