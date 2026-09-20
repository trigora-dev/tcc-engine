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
  tcc_resume: (aPtr: number, aLen: number, cPtr: number, cLen: number) => number;
  tcc_run_until_host: (budget: number) => number;
  tcc_apply_response: (ptr: number, len: number) => number;
  tcc_continuation: () => number;
};

export type Outcome =
  | { type: "host"; request: Record<string, unknown> }
  | { type: "completed"; result: unknown }
  | { type: "failed"; message: string }
  | { type: "cancelled" }
  | { type: "budget_exhausted" }
  | { type: "suspended" };

let wasm: WasmExports | undefined;

export async function loadEngine(wasmPath?: string): Promise<void> {
  const { readFile } = await import("node:fs/promises");
  const { fileURLToPath } = await import("node:url");
  const resolved = wasmPath ?? fileURLToPath(new URL("./tcc_wasm.wasm", import.meta.url));
  const bytes = await readFile(resolved);
  const result = await WebAssembly.instantiate(bytes, {});
  wasm = result.instance.exports as unknown as WasmExports;
}

export function engineFormatVersion(): number {
  return api().tcc_engine_format_version();
}

export class EngineBinding {
  constructor(artifactJson: string, executionId: string) {
    const artifact = writeString(artifactJson);
    const exec = writeString(executionId);
    const code = api().tcc_start(artifact.ptr, artifact.len, exec.ptr, exec.len);
    api().tcc_free(artifact.ptr, artifact.len);
    api().tcc_free(exec.ptr, exec.len);
    if (code !== 0) {
      throw new Error(readLast());
    }
  }

  static resume(artifactJson: string, continuationJson: string): EngineBinding {
    const binding = Object.create(EngineBinding.prototype) as EngineBinding;
    const artifact = writeString(artifactJson);
    const continuation = writeString(continuationJson);
    const code = api().tcc_resume(
      artifact.ptr,
      artifact.len,
      continuation.ptr,
      continuation.len,
    );
    api().tcc_free(artifact.ptr, artifact.len);
    api().tcc_free(continuation.ptr, continuation.len);
    if (code !== 0) {
      throw new Error(readLast());
    }
    return binding;
  }

  runUntilHost(budget = 256): Outcome {
    const code = api().tcc_run_until_host(budget);
    const json = readLast();
    if (code !== 0) {
      throw new Error(json);
    }
    return JSON.parse(json) as Outcome;
  }

  applyHostResponse(response: Record<string, unknown>): void {
    const payload = writeString(JSON.stringify(response));
    const code = api().tcc_apply_response(payload.ptr, payload.len);
    api().tcc_free(payload.ptr, payload.len);
    if (code !== 0) {
      throw new Error(readLast());
    }
  }

  continuationJson(): string {
    const code = api().tcc_continuation();
    const json = readLast();
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

function writeString(text: string): { ptr: number; len: number } {
  const bytes = encoder.encode(text);
  const ptr = api().tcc_alloc(bytes.length);
  new Uint8Array(api().memory.buffer, ptr, bytes.length).set(bytes);
  return { ptr, len: bytes.length };
}

function readLast(): string {
  const ptr = api().tcc_json_ptr();
  const len = api().tcc_json_len();
  return decoder.decode(new Uint8Array(api().memory.buffer, ptr, len));
}
