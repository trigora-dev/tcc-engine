/** Compiler intrinsics. The TypeScript frontend lowers these; the engine never runs them. */

function notRuntime(): never {
  throw new Error(
    "TCC durable operations are compiler intrinsics; compile the program instead of calling them at runtime",
  );
}

export function effect<T>(_key: string, _fn: () => T | Promise<T>): Promise<T> {
  notRuntime();
}

export function waitForEvent(_name: string): Promise<unknown> {
  notRuntime();
}

export function sleep(_ms: number): Promise<void> {
  notRuntime();
}

export function invoke(_name: string): Promise<unknown> {
  notRuntime();
}
