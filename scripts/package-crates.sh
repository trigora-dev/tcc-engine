#!/usr/bin/env bash
# Copyright (c) 2026 Trigora, Inc.
# SPDX-License-Identifier: BUSL-1.1
# See LICENSE for full terms.

# Package the publishable TCC crates.
# Path dependencies are patched only for this command. The packaged manifests
# keep version requirements and are checked for leftover path dependencies.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
export CARGO_TARGET_DIR="${root}/target"

package() {
  local name="$1"
  shift
  local dir="crates/${name}"
  if [[ "$name" == "tcc-rust-frontend" ]]; then
    dir="frontends/rust"
  fi
  cp "$root/LICENSE" "$dir/LICENSE"
  cargo package --no-verify --allow-dirty -p "$name" "$@" || {
    rm -f "$dir/LICENSE"
    exit 1
  }
  rm -f "$dir/LICENSE"
  local version
  version="$(awk -F '"' '/^version = / { print $2; exit }' "$dir/Cargo.toml")"
  local crate="${CARGO_TARGET_DIR}/package/${name}-${version}.crate"
  if [[ ! -f "$crate" ]]; then
    echo "missing packaged crate ${crate}" >&2
    exit 1
  fi
  local toml
  toml="$(tar -tzf "$crate" | grep -E '/Cargo.toml$' | head -n 1)"
  if tar -xOf "$crate" "$toml" | grep -E 'path[[:space:]]*=[[:space:]]*"\.\.'; then
    echo "path dependency remains in ${crate}" >&2
    exit 1
  fi
  if ! tar -tzf "$crate" | grep -q '/LICENSE$'; then
    echo "LICENSE is missing from ${crate}" >&2
    exit 1
  fi
  if tar -tzf "$crate" | grep -q 'THIRD_PARTY_LICENSES'; then
    echo "THIRD_PARTY_LICENSES must not be packaged in ${crate}" >&2
    exit 1
  fi
  echo "packaged ${name}"
}

package tcc-ir
package tcc-state
package tcc-core \
  --config 'patch.crates-io.tcc-ir.path="crates/tcc-ir"' \
  --config 'patch.crates-io.tcc-state.path="crates/tcc-state"'
package tcc-host \
  --config 'patch.crates-io.tcc-core.path="crates/tcc-core"' \
  --config 'patch.crates-io.tcc-ir.path="crates/tcc-ir"' \
  --config 'patch.crates-io.tcc-state.path="crates/tcc-state"'
package tcc-host-sqlite \
  --config 'patch.crates-io.tcc-core.path="crates/tcc-core"' \
  --config 'patch.crates-io.tcc-host.path="crates/tcc-host"' \
  --config 'patch.crates-io.tcc-ir.path="crates/tcc-ir"' \
  --config 'patch.crates-io.tcc-state.path="crates/tcc-state"'
package tcc-rust-frontend \
  --config 'patch.crates-io.tcc-ir.path="crates/tcc-ir"' \
  --config 'patch.crates-io.tcc-core.path="crates/tcc-core"' \
  --config 'patch.crates-io.tcc-state.path="crates/tcc-state"'
