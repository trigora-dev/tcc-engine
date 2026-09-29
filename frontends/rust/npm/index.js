// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

export const PACKAGE_VERSION = "26.10.0";
export const FRONTEND_IDENTITY = "rust";
export const FRONTEND_ID = FRONTEND_IDENTITY;
export const FRONTEND_VERSION = PACKAGE_VERSION;
export const LANGUAGE_SEMANTICS_VERSION = "rust.subset.v1";
export const ENGINE_FORMAT_VERSION = 1;

export function compile(source) {
  const dir = mkdtempSync(path.join(tmpdir(), "tcc-rust-"));
  const file = path.join(dir, "input.rs");
  writeFileSync(file, source);
  const bin = path.join(here, "vendor", "tcc-rust-compile");
  const result = spawnSync(bin, [file], { encoding: "utf8" });
  if (result.status !== 0) {
    throw new Error(result.stderr || result.stdout || `tcc-rust-compile failed (${bin})`);
  }
  return JSON.parse(result.stdout);
}
