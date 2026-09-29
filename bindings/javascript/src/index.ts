// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

type WasmExports = {
  memory: WebAssembly.Memory;
  tcc_engine_format_version: () => number;
  tcc_alloc: (len: number) => number;
  tcc_free: (ptr: number, len: number) => void;
  tcc_json_ptr: () => number;
  tcc_json_len: () => number;
  tcc_start: (aPtr: number, aLen: number, ePtr: number, eLen: number) => number;
  tcc_start_with_args: (
    aPtr: number,
    aLen: number,
    ePtr: number,
    eLen: number,
    argsPtr: number,
    argsLen: number,
  ) => number;
  tcc_resume: (aPtr: number, aLen: number, cPtr: number, cLen: number) => number;
  tcc_run_until_host: (budget: number) => number;
  tcc_apply_response: (ptr: number, len: number) => number;
  tcc_continuation: () => number;
};

export const HOST_PROTOCOL_VERSION = 1;

export function stampHostProtocolVersion(
  message: Record<string, unknown>,
): Record<string, unknown> {
  if (!Object.prototype.hasOwnProperty.call(message, "host_protocol_version")) {
    return { host_protocol_version: HOST_PROTOCOL_VERSION, ...message };
  }
  return message;
}

export type Outcome =
  | { type: "host"; request: Record<string, unknown> }
  | { type: "completed"; result: unknown }
  | { type: "failed"; message: string }
  | { type: "cancelled" }
  | { type: "budget_exhausted" }
  | { type: "suspended" };

let wasm: WasmExports | undefined;
let wasmModule: WebAssembly.Module | undefined;

export type EngineSource = string | BufferSource | WebAssembly.Module | WebAssembly.Instance;

function isBufferSource(value: unknown): value is BufferSource {
  return value instanceof ArrayBuffer || ArrayBuffer.isView(value);
}

async function instantiateEngine(source?: EngineSource): Promise<WebAssembly.Instance> {
  if (source instanceof WebAssembly.Instance) {
    wasmModule = undefined;
    return source;
  }

  if (source instanceof WebAssembly.Module) {
    wasmModule = source;
    return WebAssembly.instantiate(source, {});
  }

  if (isBufferSource(source)) {
    wasmModule = await WebAssembly.compile(source);
    return WebAssembly.instantiate(wasmModule, {});
  }

  const { readFile } = await import("node:fs/promises");
  const { fileURLToPath } = await import("node:url");
  const resolved = source ?? fileURLToPath(new URL("./tcc_wasm.wasm", import.meta.url));
  const bytes = await readFile(resolved);
  wasmModule = await WebAssembly.compile(bytes);
  return WebAssembly.instantiate(wasmModule, {});
}

export async function loadEngine(source?: EngineSource): Promise<void> {
  const instance = await instantiateEngine(source);
  wasm = instance.exports as unknown as WasmExports;
}

export function engineFormatVersion(): number {
  return api().tcc_engine_format_version();
}

export class EngineBinding {
  private exports: WasmExports;

  constructor(artifactJson: string, executionId: string, isolated = false, argsJson?: string) {
    if (isolated && !wasmModule) throw new Error("isolated engine requires a loaded WASM module");
    this.exports = isolated ? new WebAssembly.Instance(wasmModule!, {}).exports as unknown as WasmExports : api();
    const artifact = writeString(this.exports, artifactJson);
    const exec = writeString(this.exports, executionId);
    const args = argsJson === undefined ? undefined : writeString(this.exports, argsJson);
    const code = args
      ? this.exports.tcc_start_with_args(
          artifact.ptr,
          artifact.len,
          exec.ptr,
          exec.len,
          args.ptr,
          args.len,
        )
      : this.exports.tcc_start(artifact.ptr, artifact.len, exec.ptr, exec.len);
    this.exports.tcc_free(artifact.ptr, artifact.len);
    this.exports.tcc_free(exec.ptr, exec.len);
    if (args) {
      this.exports.tcc_free(args.ptr, args.len);
    }
    if (code !== 0) {
      throw new Error(readLast(this.exports));
    }
  }

  static resume(artifactJson: string, continuationJson: string): EngineBinding {
    const binding = Object.create(EngineBinding.prototype) as EngineBinding;
    binding.exports = api();
    const artifact = writeString(binding.exports, artifactJson);
    const continuation = writeString(binding.exports, continuationJson);
    const code = binding.exports.tcc_resume(
      artifact.ptr,
      artifact.len,
      continuation.ptr,
      continuation.len,
    );
    binding.exports.tcc_free(artifact.ptr, artifact.len);
    binding.exports.tcc_free(continuation.ptr, continuation.len);
    if (code !== 0) {
      throw new Error(readLast(binding.exports));
    }
    return binding;
  }

  runUntilHost(budget = 256): Outcome {
    const code = this.exports.tcc_run_until_host(budget);
    const json = readLast(this.exports);
    if (code !== 0) {
      throw new Error(json);
    }
    return JSON.parse(json) as Outcome;
  }

  applyHostResponse(response: Record<string, unknown>): void {
    const payload = writeString(this.exports, JSON.stringify(stampHostProtocolVersion(response)));
    const code = this.exports.tcc_apply_response(payload.ptr, payload.len);
    this.exports.tcc_free(payload.ptr, payload.len);
    if (code !== 0) {
      throw new Error(readLast(this.exports));
    }
  }

  continuationJson(): string {
    const code = this.exports.tcc_continuation();
    const json = readLast(this.exports);
    if (code !== 0) {
      throw new Error(json);
    }
    return json;
  }
}

function api(): WasmExports {
  if (!wasm) {
    throw new Error("WASM engine is not loaded");
  }
  return wasm;
}

function writeString(exports: WasmExports, text: string): { ptr: number; len: number } {
  const bytes = encoder.encode(text);
  const ptr = exports.tcc_alloc(bytes.length);
  new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
  return { ptr, len: bytes.length };
}

function readLast(exports: WasmExports): string {
  const ptr = exports.tcc_json_ptr();
  const len = exports.tcc_json_len();
  return decoder.decode(new Uint8Array(exports.memory.buffer, ptr, len));
}
