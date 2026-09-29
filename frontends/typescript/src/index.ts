// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

export { compile, CompileError, type DiagnosticSpan } from "./compile.ts";
export {
  ENGINE_FORMAT_VERSION,
  FRONTEND_ID,
  FRONTEND_IDENTITY,
  FRONTEND_VERSION,
  LANGUAGE_SEMANTICS_VERSION,
  PACKAGE_VERSION,
  type Artifact,
  type CompileOptions,
  type ConstValue,
  type Envelope,
  type EngineFeature,
  type FunctionDecl,
  type HostCapability,
  type Instruction,
  type Program,
  type RuntimeModule,
  type SourceSpan,
} from "./types.ts";
