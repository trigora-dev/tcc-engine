/** Mirrors `tcc-ir` engine features. Unknown required features are rejected by the engine. */
export type EngineFeature =
  | "ts.control_flow"
  | "durable.effect"
  | "durable.sleep"
  | "durable.wait_for_event"
  | "durable.invoke"
  | "ts.exceptions"
  | (string & {});

export type HostCapability =
  | "host.persist_checkpoint"
  | "host.effect"
  | "host.timer"
  | "host.event"
  | "host.child"
  | "host.fetch_artifact"
  | (string & {});

export type ConstValue =
  | { t: "undefined" }
  | { t: "null" }
  | { t: "bool"; v: boolean }
  | { t: "number"; v: number }
  | { t: "string"; v: string };

export type Instruction =
  | { op: "Nop" }
  | { op: "Jump"; target: number }
  | { op: "JumpIfTrue"; target: number }
  | { op: "JumpIfFalse"; target: number }
  | { op: "LoadLocal"; local: number }
  | { op: "StoreLocal"; local: number }
  | { op: "LoadConst"; value: ConstValue }
  | { op: "Pop" }
  | { op: "Return" }
  | { op: "Call"; func: number; argc: number }
  | { op: "Effect" }
  | { op: "Sleep" }
  | { op: "WaitForEvent" }
  | { op: "Invoke" }
  | { op: "Throw" }
  | { op: "PushTry"; catch: number; finally: number | null }
  | { op: "PopTry" };

export type SourceSpan = {
  file: string;
  start_line: number;
  start_column: number;
  end_line: number;
  end_column: number;
};

export type RuntimeModule = {
  id: string;
  kind: string;
};

export type Envelope = {
  artifact_hash: string;
  frontend_id: string;
  frontend_version: string;
  language_semantics_version: string;
  engine_format_version: number;
  required_engine_features: EngineFeature[];
  required_host_capabilities: HostCapability[];
  runtime_modules: RuntimeModule[];
};

export type FunctionDecl = {
  id: number;
  name: string;
  param_count: number;
  local_count: number;
  instructions: Instruction[];
  spans: Array<SourceSpan | null>;
};

export type Program = {
  entry: number;
  functions: FunctionDecl[];
};

export type Artifact = {
  envelope: Envelope;
  program: Program;
};

export const ENGINE_FORMAT_VERSION = 1;
export const FRONTEND_ID = "typescript";
export const LANGUAGE_SEMANTICS_VERSION = "ts.subset.v1";
export const FRONTEND_VERSION = "0.0.0";

export type CompileOptions = {
  filename?: string;
};
