#!/usr/bin/env bash
# Build the Python engine and write this platform's license notices before the wheel is packed.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$script_dir/../.." && pwd)"
cd "$root"
export CARGO_TARGET_DIR="$root/target"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is not on PATH" >&2
  exit 1
fi

rustup component add llvm-tools-preview
if ! command -v cargo-about >/dev/null 2>&1; then
  cargo install cargo-about --version 0.6.6 --locked
fi

cargo build -p tcc-python --release

case "$(uname -s)" in
  Linux)
    lib="$CARGO_TARGET_DIR/release/lib_engine.so"
    ;;
  Darwin)
    lib="$CARGO_TARGET_DIR/release/lib_engine.dylib"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    lib="$CARGO_TARGET_DIR/release/_engine.dll"
    ;;
  *)
    echo "unsupported OS $(uname -s)" >&2
    exit 1
    ;;
esac
test -f "$lib"

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
  --bin "$lib"

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
