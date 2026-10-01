# Persistence benchmarks

These benchmarks measure the portable TCC engine on the local reference host.

## Suite (Node / WebAssembly)

Build the release WASM once, then run. Benches do not rebuild it; they fail if the artifact is missing.

```bash
pnpm build:wasm
pnpm bench                 # full suite
pnpm bench:persistence     # healthy path + group commit + replay baseline
pnpm bench:recovery        # history depth + vs-replay + live-state + WAL isolation
```

Targeted runs:

```bash
pnpm bench:persistence:healthy
pnpm bench:persistence:group
pnpm bench:persistence:replay
pnpm bench:recovery:history
pnpm bench:recovery:replay
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
| `recovery_history_replay_baseline` | Matched recovery: TCC reconstruct vs journal prefix replay at fixed live continuation (~4 KB; full also 16/64 KB) |
| `recovery_history_depth` | Reconstruct vs target revision depth (bounded suffix; not the matched-replay chart) |
| `live_state_scaling` | Reconstruct vs live continuation size |
| `wal_isolation` | Reconstruct with foreign WAL 0…100k and **target suffix = 10** |

In `concurrency_group_commit`, `concurrency` is the number of in-flight executions in a coordinated wave, not parallel workers. Each execution pauses at a checkpoint; the host commits the wave before confirming any of those checkpoints. One execution creates four checkpoints, so a wave of one has four commits with batch occupancy one.

Healthy-path workloads: A has one effect-result local with no reassignment; B creates eight locals with no reassignment; C creates four locals and reassigns each once; D creates one local and reassigns it three times. `deltaPayloadShare` is delta payload bytes divided by all persisted checkpoint payload bytes. It is not a live-value mutation fraction. `executionsPerSec` counts completed executions, and persistence accounting excludes warmup.

`healthy_path_replay_baseline` compares replay/history persistence with TCC continuation persistence on the local reference host. `overheadVsReplay` is `(tccMedianMs - replayMedianMs) / replayMedianMs` on optimized rows, so one GC or scheduler pause does not dominate. `executionsPerSec` is mean throughput. Sequential rows use `coordination: "single"`. Waves use `runBatchOnStore` for both models.

`recovery_history_replay_baseline` is the matched recovery comparison. At a measured live continuation (~4 KB primary; 16 KB and 64 KB on full scale) TCC loads/reconstructs the latest committed continuation while the replay baseline starts the engine and replays the committed prefix from the effect journal (`history` WAL, no continuation row). Timed replay recovery must not invoke the effect provider. `liveStateBytes` is actual continuation JSON length, not a local-count proxy. History grows by reassigning one scratch local after the live blob. `overheadVsReplay` uses the same median formula; a negative value means TCC is faster. The primary series is `liveStateTargetBytes = 4096`. Output is the same `meta` + `results` JSON as the rest of the suite.

## Python mirror

Same scenario names / JSON fields; secondary to Node. Recovery here covers live-state and WAL isolation (`recovery:history` and `recovery:replay` are Node-canonical).

```bash
TCC_BENCH_SCALE=quick python bench/python/run.py
python bench/python/run.py persistence
python bench/python/run.py persistence:replay
python bench/python/run.py recovery
```
