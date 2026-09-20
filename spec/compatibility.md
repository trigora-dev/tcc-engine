# Compatibility

## Versions

| Version | Scope |
|---|---|
| `engine_format_version` | Artifact envelope and instruction encoding |
| `host_protocol_version` | Host request/outcome/response JSON. Required; missing is not `1` |
| `language_semantics_version` | Language rules used by the instructions. Allowlist: `ts.subset.v1`, `py.subset.v1` |
| `frontend_id` | Frontend identity. Must be non-empty; ids are not allowlisted |
| `frontend_version` | Frontend that produced the artifact. Identity only, not a reject key |
| continuation `revision` | Host-confirmed checkpoint number |

The engine rejects an artifact whose `engine_format_version` it does not implement (`IrError::UnsupportedFormat` / `CoreError::FormatMismatch`). It rejects unknown `language_semantics_version` values and an empty `frontend_id`. It rejects required engine features and host capabilities it does not implement. It does not define behavior for unknown instructions or value tags.

Host JSON without `host_protocol_version` is `MissingHostProtocolVersion`. A present value other than `1` is `UnsupportedHostProtocol { got, supported }`. There is no “missing means 1” rule.

Package versions on npm/PyPI (`0.1.0-rc.1`, later `0.1.0`, …) are **not** these identifiers. A later package release may still emit `engine_format_version` 1 and `ts.subset.v1` / `py.subset.v1` when the change is packaging, diagnostics, or a bug fix. Frontends export `PACKAGE_VERSION`, `FRONTEND_IDENTITY`, `LANGUAGE_SEMANTICS_VERSION`, and `ENGINE_FORMAT_VERSION` as distinct constants. Envelope `frontend_version` names the frontend build; it is not the language-semantics version.

## Artifact identity

A continuation resumes only against the artifact named by `artifact.hash`. A mismatch is `ArtifactMismatch`.

The engine does not migrate a continuation to a different compilation of the program.

## Native and WebAssembly

Native and WASM builds execute the same core. For a given artifact, continuation, and host responses they must produce the same validation result, continuation encoding, and step outcomes.

## Frontends

Every frontend must emit the artifact envelope in [Program format](program-format.md). A frontend may add instructions only by declaring new required engine features. Unsupported source constructs must fail at compile time. TypeScript and Python both target the same instruction set; they are not required to emit byte-identical artifacts.

Third-party libraries are not executed by the engine unless the frontend compiles them or the host exposes them as a capability.

## Decode

Unknown JSON value tags must fail. They must not be treated as `number` or `string`.
