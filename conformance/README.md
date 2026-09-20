# Host conformance v1

Named kit for hosts that speak TCC host protocol version 1. Spec: [`spec/host-conformance-v1.md`](../spec/host-conformance-v1.md). Case index: [`cases.json`](cases.json).

Do not duplicate persist goldens here. Reconstruct cases point at [`spec/fixtures/persist/`](../spec/fixtures/persist/).

## Run

```text
node --experimental-sqlite --experimental-strip-types --test conformance/run-node.ts
python conformance/run_python.py
```

These wrap the existing Node and Python `assertConformance` / persist reconstruct suites plus artifact pinning.

## Driver

| Language | Implementation |
|---|---|
| TypeScript | `hosts/node/src/conformance.ts` (`start` / `crashAt` / `resume` / continuation / effect log) and `hosts/node/src/reconstruct.ts` (`applyDelta`) |
| Python | `hosts/python/conformance.py` and `tcc_engine.persist.apply_delta` |
