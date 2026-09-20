# Persistence benchmarks

These benchmarks characterize the portable TCC engine on the local host. They are not the research-prototype measurements and are not intended to model production Cloud latency.

## Public suite (canonical: Node / WASM)

Build the release WASM once, then run. Benches do not rebuild it; they fail if the artifact is missing.

```bash
pnpm build:wasm
pnpm bench                 # full suite
pnpm bench:persistence     # healthy path + group commit + replay baseline
pnpm bench:recovery        # history depth + live-state + WAL isolation
```

Optional targeted aliases (contributors / CI):

```bash
pnpm bench:persistence:healthy
pnpm bench:persistence:group
pnpm bench:persistence:replay
pnpm bench:recovery:history
pnpm bench:recovery:live-state
pnpm bench:recovery:wal
```

Quick smoke (smaller warmup/iters; skips 10k/100k foreign WAL):

```bash
TCC_BENCH_SCALE=quick pnpm bench
```

Output is a human summary plus machine-readable JSON (`meta` + `results`).

| Scenario | What it measures |
|---|---|
| `healthy_path_persistence` | Naive vs optimized throughput for mutation shapes A–D |
| `concurrency_group_commit` | Shared-store waves at concurrency 1…32; commit count and batch occupancy |
| `healthy_path_replay_baseline` | Replay/history persist vs TCC optimized, sequential A–D and coordinated waves on A |
| `recovery_history_depth` | Reconstruct vs target revision depth (bounded suffix) |
| `live_state_scaling` | Reconstruct vs live continuation size |
| `wal_isolation` | Reconstruct with foreign WAL 0…100k and **target suffix = 10** |

In `concurrency_group_commit`, `concurrency` is the number of in-flight executions in a coordinated wave, not parallel workers. Each execution pauses at a checkpoint; the host commits the wave before confirming any of those checkpoints. One execution creates four checkpoints, so a wave of one has four commits with batch occupancy one.

Healthy-path workloads: A has one effect-result local with no reassignment; B creates eight locals with no reassignment; C creates four locals and reassigns each once; D creates one local and reassigns it three times. `deltaPayloadShare` is delta payload bytes divided by all persisted checkpoint payload bytes. It is not a live-value mutation fraction. `executionsPerSec` counts completed executions, and persistence accounting excludes warmup.

`healthy_path_replay_baseline` is a matched local-host comparison of replay/history persistence against TCC optimized continuation persistence. It is not the research-prototype ~10% number. `overheadVsReplay` is `(tccMedianMs - replayMedianMs) / replayMedianMs` on optimized rows so a single GC/scheduler pause cannot dominate. `executionsPerSec` remains mean throughput and is not the comparison metric. Sequential rows use `coordination: "single"`; waves use `runBatchOnStore` for both models so group commit is not TCC-only. Do not fold this table into `healthy_path_persistence`.

## Python mirror

Same scenario names / JSON fields; secondary to Node. Recovery here covers live-state and WAL isolation (history depth is Node-canonical).

```bash
TCC_BENCH_SCALE=quick python bench/python/run.py
python bench/python/run.py persistence
python bench/python/run.py persistence:replay
python bench/python/run.py recovery
```
