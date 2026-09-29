// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use super::*;

impl Engine {
    pub fn start(
        artifact: Artifact,
        execution_id: impl Into<String>,
        caps: &EngineCaps,
    ) -> Result<Self, CoreError> {
        Self::start_with_args(artifact, execution_id, caps, &[])
    }

    /// `args` is the ordered program argument vector. An empty slice is no arguments.
    pub fn start_with_args(
        artifact: Artifact,
        execution_id: impl Into<String>,
        caps: &EngineCaps,
        args: &[Value],
    ) -> Result<Self, CoreError> {
        validate(&artifact, caps)?;
        let entry = artifact
            .function(artifact.program.entry)
            .expect("validated artifact has an entry function");
        let mut continuation = Continuation::start(
            execution_id,
            artifact.envelope.artifact_hash.clone(),
            artifact.envelope.engine_format_version,
            artifact.envelope.language_semantics_version.clone(),
            entry.id.0,
            entry.local_count,
        );
        let args: Vec<Value> = args
            .iter()
            .cloned()
            .map(|value| absorb_value(&mut continuation.heap, value))
            .collect();
        bind_program_args(
            &artifact.envelope.language_semantics_version,
            entry.param_count,
            &entry.param_defaults,
            &args,
            &mut continuation.frames[0].locals,
        )?;
        let liveness = analyze_program(&artifact.program);
        Ok(Self {
            artifact,
            continuation,
            outstanding: None,
            confirmed: None,
            deltas_since_snapshot: 0,
            liveness,
        })
    }

    pub fn resume(
        artifact: Artifact,
        continuation: Continuation,
        caps: &EngineCaps,
    ) -> Result<Self, CoreError> {
        validate(&artifact, caps)?;
        if continuation.artifact.hash != artifact.envelope.artifact_hash {
            return Err(CoreError::ArtifactMismatch {
                expected: continuation.artifact.hash,
                found: artifact.envelope.artifact_hash,
            });
        }
        if continuation.engine_format_version != ENGINE_FORMAT_VERSION {
            return Err(CoreError::FormatMismatch {
                found: continuation.engine_format_version,
                supported: ENGINE_FORMAT_VERSION,
            });
        }
        if continuation.language_semantics_version != artifact.envelope.language_semantics_version {
            return Err(CoreError::LanguageSemanticsMismatch {
                expected: artifact.envelope.language_semantics_version.clone(),
                found: continuation.language_semantics_version.clone(),
            });
        }
        let mut continuation = continuation;
        absorb_continuation(&mut continuation);
        validate_resume_frames(&artifact, &continuation)?;
        let liveness = analyze_program(&artifact.program);
        Ok(Self {
            artifact,
            continuation,
            outstanding: None,
            confirmed: None,
            deltas_since_snapshot: 0,
            liveness,
        })
    }
}

pub(super) fn const_to_value(value: &ConstValue) -> Value {
    match value {
        ConstValue::Undefined => Value::Undefined,
        ConstValue::Null => Value::Null,
        ConstValue::Bool(flag) => Value::Bool(*flag),
        ConstValue::Number(number) => Value::Number(*number),
        ConstValue::String(text) => Value::String(text.clone()),
    }
}

/// The only language dispatch for program arguments. After this returns, slots are ordinary locals.
pub(super) fn bind_program_args(
    language_semantics_version: &str,
    param_count: u32,
    defaults: &[Option<tcc_ir::ConstValue>],
    args: &[Value],
    locals: &mut [Value],
) -> Result<(), CoreError> {
    let expected = param_count as usize;
    match language_semantics_version {
        LANGUAGE_SEMANTICS_TS => {
            for (index, value) in args.iter().take(expected).enumerate() {
                locals[index] = value.clone();
            }
            Ok(())
        }
        LANGUAGE_SEMANTICS_PY | LANGUAGE_SEMANTICS_RUST => {
            let given = args.len();
            if given > expected {
                return Err(CoreError::TypeError(format!(
                    "run() takes {expected} positional arguments but {given} were given"
                )));
            }
            for index in 0..expected {
                if index < given {
                    locals[index] = args[index].clone();
                } else if let Some(Some(default)) = defaults.get(index) {
                    locals[index] = const_to_value(default);
                } else {
                    return Err(CoreError::TypeError(format!(
                        "run() missing {} required positional argument(s)",
                        expected - given
                    )));
                }
            }
            Ok(())
        }
        other => Err(CoreError::TypeError(format!(
            "unsupported language semantics `{other}`"
        ))),
    }
}

fn validate_resume_frames(
    artifact: &Artifact,
    continuation: &Continuation,
) -> Result<(), CoreError> {
    for frame in &continuation.frames {
        let function = artifact.function(FuncId(frame.func_id)).ok_or_else(|| {
            CoreError::InvalidContinuation(format!("frame function {} is missing", frame.func_id))
        })?;
        if frame.locals.len() != function.local_count as usize {
            return Err(CoreError::InvalidContinuation(format!(
                "frame locals {} does not match local_count {}",
                frame.locals.len(),
                function.local_count
            )));
        }
        if (frame.pc as usize) >= function.instructions.len() {
            return Err(CoreError::InvalidContinuation(format!(
                "pc {} is out of range in function {} ({} instructions)",
                frame.pc,
                frame.func_id,
                function.instructions.len()
            )));
        }
    }
    Ok(())
}
