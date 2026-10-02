#!/usr/bin/env bash
# Copyright (c) 2026 Trigora, Inc.
# SPDX-License-Identifier: BUSL-1.1
# See LICENSE for full terms.

# Build the Python engine and write this platform's license notices before the wheel is packed.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$script_dir/../.." && pwd)"
cd "$root"
# A Git bash /c/... target dir is a different folder for cargo.exe. Leave the
# default relative target/ so both see the same files.
unset CARGO_TARGET_DIR

if ! command -v cargo >/dev/null 2>&1; then
  if [ -n "${HOME:-}" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is not on PATH" >&2
  exit 1
fi

rustup component add llvm-tools-preview
if ! command -v cargo-about >/dev/null 2>&1; then
  cargo install cargo-about --version 0.6.6 --locked
fi

# A release DLL from link.exe keeps the exported Python entry point and drops
# the Rust symbol table that ELF and Mach-O retain. /MAP writes that table
# beside the DLL so the license scan still sees the crates that were linked.
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    cargo rustc -p tcc-python --release -- -C link-arg=/MAP:target/release/_engine.map
    ;;
  *)
    cargo rustc -p tcc-python --release
    ;;
esac

finder="find"
if [ -x /usr/bin/find ]; then
  finder="/usr/bin/find"
fi
lib=""
while IFS= read -r candidate; do
  case "$candidate" in
    */deps/*|*/incremental/*) continue ;;
  esac
  lib="$candidate"
  break
done < <("$finder" target -type f \
  \( -name 'lib_engine.so' -o -name 'lib_engine.dylib' \
     -o -name '_engine.dll' -o -name 'lib_engine.dll' \
     -o -name '_engine.pyd' -o -name 'engine.dll' \) \
  ! -path '*/incremental/*' 2>/dev/null || true)
if [ -z "$lib" ]; then
  while IFS= read -r candidate; do
    lib="$candidate"
    break
  done < <("$finder" target -type f -name '_engine-*.dll' ! -path '*/incremental/*' 2>/dev/null || true)
fi
if [ -z "$lib" ]; then
  echo "engine library not found (uname $(uname -s))" >&2
  "$finder" target -type f \( -name '*.dll' -o -name '*.so' -o -name '*.dylib' -o -name '*.pyd' \) ! -path '*/incremental/*' >&2 || true
  exit 1
fi
echo "engine library: $lib"
bins=(--bin "$lib")
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*)
    map="target/release/_engine.map"
    if [ ! -f "$map" ]; then
      echo "linker map was not written next to the engine DLL" >&2
      exit 1
    fi
    bins+=(--bin "$map")
    echo "engine link map: $map"
    ;;
esac

if command -v python3 >/dev/null 2>&1; then
  py=python3
else
  py=python
fi

stage="$(mktemp -d)"
"$py" scripts/licenses/collect.py \
  --skip-deny \
  --manifest Cargo.toml \
  --exclude tcc-core \
  --exclude tcc-ir \
  --exclude tcc-state \
  --exclude tcc-host \
  --exclude tcc-host-sqlite \
  --exclude tcc-rust-frontend \
  --exclude tcc-python \
  --exclude tcc-wasm \
  --out "$stage" \
  "${bins[@]}"

dest="bindings/python/python/tcc_engine/_licenses"
rm -rf "$dest"
mkdir -p "$dest/licenses"
cp "$stage/THIRD_PARTY_LICENSES" LICENSE "$dest/"
if [ -d "$stage/licenses/third-party" ]; then
  cp -R "$stage/licenses/third-party" "$dest/licenses/third-party"
fi
if grep -q rusqlite "$dest/THIRD_PARTY_LICENSES" || grep -q '^clap ' "$dest/THIRD_PARTY_LICENSES"; then
  echo "the engine wheel notices include a Trigora CLI dependency" >&2
  exit 1
fi
