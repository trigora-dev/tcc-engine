// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

/** Mirrors `tcc-ir` engine features. Unknown required features are rejected by the engine. */
export type EngineFeature =
  | "ts.control_flow"
  | "durable.effect"
  | "durable.sleep"
  | "durable.wait_for_event"
  | "durable.invoke"
  | "durable.concurrent_group"
  | "ts.exceptions"
  | "lang.compute"
  | (string & {});

export type HostCapability =
  | "host.persist_checkpoint"
  | "host.persist_checkpoint_delta"
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
  | { op: "NewObject" }
  | { op: "SetProp"; key: string }
  | { op: "GetProp"; key: string }
  | { op: "NewArray" }
  | { op: "ArrayPush" }
  | { op: "StrictEq" }
  | { op: "StrictNeq" }
  | { op: "Lt" }
  | { op: "Le" }
  | { op: "Gt" }
  | { op: "Ge" }
  | { op: "Not" }
  | { op: "Return" }
  | { op: "Call"; func: number; argc: number }
  | { op: "Effect" }
  | { op: "Sleep" }
  | { op: "WaitForEvent" }
  | { op: "Invoke"; arg_count?: number }
  | { op: "Fork"; count: number; join_pc: number }
  | { op: "JoinAll" }
  | { op: "JoinAny" }
  | { op: "ArrayIndex"; index: number }
  | { op: "Add" }
  | { op: "Sub" }
  | { op: "Mul" }
  | { op: "Div" }
  | { op: "Rem" }
  | { op: "Pow" }
  | { op: "Neg" }
  | { op: "GetIndex" }
  | { op: "SetIndex" }
  | { op: "Length" }
  | { op: "WatchIter" }
  | { op: "UnwatchIter" }
  | { op: "Same" }
  | { op: "NewCell" }
  | { op: "NewEnv"; count: number }
  | { op: "NewClosure"; func: number }
  | { op: "EnvGet"; index: number }
  | { op: "EnvSet"; index: number }
  | { op: "EnvSlot"; index: number }
  | { op: "CallClosure"; argc: number }
  | { op: "LoadFunc"; func: number }
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

export const PACKAGE_VERSION = "26.10.0";
export const ENGINE_FORMAT_VERSION = 1;
export const FRONTEND_IDENTITY = "typescript";
export const FRONTEND_ID = FRONTEND_IDENTITY;
export const LANGUAGE_SEMANTICS_VERSION = "ts.subset.v1";
/** Frontend build that produced the artifact. Not the language-semantics version. */
export const FRONTEND_VERSION = PACKAGE_VERSION;

export type CompileOptions = {
  filename?: string;
};
