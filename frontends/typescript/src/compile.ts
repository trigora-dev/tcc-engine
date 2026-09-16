import {
  ENGINE_FORMAT_VERSION,
  FRONTEND_ID,
  FRONTEND_VERSION,
  LANGUAGE_SEMANTICS_VERSION,
  type Artifact,
  type CompileOptions,
} from "./types.ts";

export class CompileError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CompileError";
  }
}

export class CompileNotImplementedError extends CompileError {
  constructor() {
    super(
      "TypeScript lowering is not implemented yet. This package currently exports the artifact envelope types only.",
    );
    this.name = "CompileNotImplementedError";
  }
}

/** Compile supported TypeScript into a TCC artifact. Not implemented in this scaffold. */
export function compile(_source: string, _options: CompileOptions = {}): Artifact {
  void ENGINE_FORMAT_VERSION;
  void FRONTEND_ID;
  void FRONTEND_VERSION;
  void LANGUAGE_SEMANTICS_VERSION;
  throw new CompileNotImplementedError();
}
