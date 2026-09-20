import { readFileSync } from "node:fs";

export const HOST_ENGINE_VERSION = (
  JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")) as { version: string }
).version;

export type HostEventType =
  | "checkpoint.persisted"
  | "checkpoint.materialized"
  | "continuation.restored"
  | "effect.journal_hit"
  | "batch.committed"
  | "child.created"
  | "child.completed"
  | "runtime.error";

export type HostEvent = {
  type: HostEventType;
  executionId?: string;
  revision?: number;
  durationMs?: number;
  bytes?: number;
  kind?: "snapshot" | "delta";
  batchSize?: number;
  suffixLength?: number;
  engineVersion: string;
  message?: string;
};

export type HostObserver = (event: HostEvent) => void;

export type HostEventInput = Omit<HostEvent, "engineVersion">;

export function emitHostEvent(observer: HostObserver | undefined, event: HostEventInput): void {
  if (!observer) {
    return;
  }
  try {
    observer({ ...event, engineVersion: HOST_ENGINE_VERSION });
  } catch {
    // Observers must not fail persist or confirm.
  }
}
