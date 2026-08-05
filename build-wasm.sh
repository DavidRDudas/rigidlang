#!/usr/bin/env bash
# Build the browser playground's WebAssembly module.
# Needs: rustup target add wasm32-unknown-unknown
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown --lib
cp target/wasm32-unknown-unknown/release/rigid.wasm site/rigid.wasm
printf 'site/rigid.wasm  %s bytes (%s gzipped)\n' \
  "$(wc -c < site/rigid.wasm | tr -d ' ')" \
  "$(gzip -c site/rigid.wasm | wc -c | tr -d ' ')"
