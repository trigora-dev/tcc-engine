# Compatibility

## Versions

| Version | Scope |
|---|---|
| `engine_format_version` | Artifact envelope and instruction encoding |
| `language_semantics_version` | Language rules used by the instructions |
| `frontend_version` | Frontend that produced the artifact |
| continuation `revision` | Host-confirmed checkpoint number |

The engine rejects an artifact whose `engine_format_version` it does not implement. It rejects required engine features and host capabilities it does not implement. It does not define behavior for unknown instructions or value tags.

## Artifact identity

A continuation resumes only against the artifact named by `artifact.hash`. A mismatch is `ArtifactMismatch`.

The engine does not migrate a continuation to a different compilation of the program.

## Native and WebAssembly

Native and WASM builds execute the same core. For a given artifact, continuation, and host responses they must produce the same validation result, continuation encoding, and step outcomes.

## Frontends

Every frontend must emit the artifact envelope in [Program format](program-format.md). A frontend may add instructions only by declaring new required engine features. Unsupported source constructs must fail at compile time.

Third-party libraries are not executed by the engine unless the frontend compiles them or the host exposes them as a capability.

## Decode

Unknown JSON value tags must fail. They must not be treated as `number` or `string`.
