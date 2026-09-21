import type { ContinuationDelta, ContinuationJson } from "./reconstruct.ts";

export type DriverLanguage = "typescript" | "python";

/** Opaque to the runner. Drivers store workspace paths inside. */
export type ConformanceHandle = {
  executionId: string;
  workspace: string;
};

export type ConformanceRunResult = {
  status: string;
  result: unknown;
  handle: ConformanceHandle;
};

export type StartInput = {
  artifactJson: string;
  executionId?: string;
  effects?: Record<string, unknown>;
  eventPayload?: unknown;
  autoDeliverEvent?: boolean;
  cancel?: boolean;
};

export type CrashInput = StartInput & {
  crashAt: string;
};

export type ResumeInput = {
  handle: ConformanceHandle;
  artifactJson?: string;
  effects?: Record<string, unknown>;
  eventPayload?: unknown;
  autoDeliverEvent?: boolean;
  cancel?: boolean;
};

/**
 * Semantic host-conformance-v1 driver. No SQLite schema, exec_head, WAL, or packing knobs.
 */
export type HostConformanceDriver = {
  language: DriverLanguage;
  compile(source: string, filename: string): string;
  applyDelta(
    base: ContinuationJson,
    delta: ContinuationDelta,
  ): ContinuationJson;
  start(input: StartInput): Promise<ConformanceRunResult>;
  crashAt(input: CrashInput): Promise<ConformanceHandle>;
  resume(input: ResumeInput): Promise<ConformanceRunResult>;
  readContinuation(handle: ConformanceHandle): Promise<ContinuationJson>;
  effectLog(handle: ConformanceHandle): Promise<string[]>;
};
