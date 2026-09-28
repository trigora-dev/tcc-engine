# Compatibility

## Versions

| Version | Scope |
|---|---|
| `engine_format_version` | Artifact envelope and instruction encoding |
| `host_protocol_version` | Host request/outcome/response JSON. Required; missing is not `1` |
| `language_semantics_version` | Language rules used by the instructions. Allowlist: `ts.subset.v1`, `py.subset.v1`, and `rust.subset.v1` |
| `frontend_id` | Frontend identity. Must be non-empty; ids are not allowlisted |
| `frontend_version` | Frontend that produced the artifact. Identity only, not a reject key |
| continuation `revision` | Host-confirmed checkpoint number |

The engine rejects an artifact whose `engine_format_version` it does not implement (`IrError::UnsupportedFormat` / `CoreError::FormatMismatch`). It rejects unknown `language_semantics_version` values and an empty `frontend_id`. It rejects required engine features and host capabilities it does not implement. It does not define behavior for unknown instructions or value tags.

Host JSON without `host_protocol_version` is `MissingHostProtocolVersion`. A present value other than `1` is `UnsupportedHostProtocol { got, supported }`. There is no “missing means 1” rule.

Protocol v1 is the finalized shape in [host-protocol.md](host-protocol.md). It includes ordinary single-op delivery and correlated join delivery. There is no protocol v2 and no decoder for the pre-correlation payloads (`event_payload` / `timer_fired` / `child_result` that cannot name a branch when the execution is in a join).

| Artifact | Host protocol v1 | Runs |
|---|---|---|
| Non-join, `branch` omitted on deliveries | finalized v1 | yes |
| Concurrent group, deliveries carry `branch` | finalized v1 | yes |
| Pre-correlation wire shape (join delivery without `branch`, or a second dialect) | — | no |

`engine_format_version` stays `1`. Format v1 includes `Fork`, `JoinAll`, and `JoinAny`. `durable.concurrent_group` is the feature an engine checks before running those instructions. It does not preserve a pre-join reading of the same bytes. A join object without `kind` fails decode.

Package versions on npm/PyPI use CalVer `YY.MM.MICRO` (first public cut `26.10.0`) and are **not** these identifiers. `YY` and `MM` identify the release month; `MICRO` is a release counter within that month. Releases within the same year keep supported public package APIs backward compatible; breaking package API changes may occur when the year component changes. A later package release may still emit `engine_format_version` 1 and `ts.subset.v1` / `py.subset.v1` / `rust.subset.v1` when the change is packaging, diagnostics, or a compatible feature. Frontends export `PACKAGE_VERSION`, `FRONTEND_IDENTITY`, `LANGUAGE_SEMANTICS_VERSION`, and `ENGINE_FORMAT_VERSION` as distinct constants. Envelope `frontend_version` names the frontend build; it is not the language-semantics version. The private `@tcc-engine/frontend-rust` npm package stays `0.1.0-rc.1`; its `frontend_version` stamp follows `PACKAGE_VERSION` (`26.10.0`).

## Artifact identity

A continuation resumes only against the artifact named by `artifact.hash`. A mismatch is `ArtifactMismatch`.

The engine does not migrate a continuation to a different compilation of the program.

## Native and WebAssembly

Native and WASM builds execute the same core. For a given artifact, continuation, and host responses they must produce the same validation result, continuation encoding, and step outcomes.

## Frontends

Every frontend must emit the artifact envelope in [Program format](program-format.md). A frontend may add instructions only by declaring new required engine features. Unsupported source constructs must fail at compile time. TypeScript, Python, and Rust target the same instruction set; they are not required to emit byte-identical artifacts.

`rust.subset.v1` is a supported authoring language. Argument binding for that id is exact arity with no defaults. See [rust-subset.md](rust-subset.md).

Third-party libraries are not executed by the engine unless the frontend compiles them or the host exposes them as a capability.

## Decode

Unknown JSON value tags must fail. They must not be treated as `number` or `string`.
