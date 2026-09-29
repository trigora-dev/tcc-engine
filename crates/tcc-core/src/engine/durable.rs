// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use super::*;

impl Engine {
    pub(super) fn yield_effect(
        &mut self,
        has_input: bool,
    ) -> Result<Option<EngineOutcome>, CoreError> {
        let key = expect_string(self.pop()?)?;
        let input = if has_input {
            let value = self.pop()?;
            export_value(&self.continuation.heap, &value).unwrap_or(value)
        } else {
            Value::Object(std::collections::BTreeMap::new())
        };
        if self.join_is_active() {
            self.bind_planned_branch(BranchOp::Effect { key: key.clone() })?;
        }
        let execution_id = self.continuation.execution_id.clone();
        let idempotency_key = format!("{execution_id}:{key}");
        self.continuation.pending = Some(PendingOp::Effect {
            key: key.clone(),
            idempotency_key: idempotency_key.clone(),
        });
        let request = HostRequest::RunEffect {
            key,
            idempotency_key,
            input,
        };
        self.outstanding = Some(request.clone());
        Ok(Some(EngineOutcome::Host(request)))
    }

    pub(super) fn yield_sleep(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let duration = expect_number(self.pop()?)?;
        let resume_at_ms = duration.max(0.0) as u64;
        let branch = if self.join_is_active() {
            Some(self.bind_planned_branch(BranchOp::Timer { resume_at_ms })?)
        } else {
            None
        };
        self.advance_pc()?;
        self.continuation.pending = Some(PendingOp::Wait {
            kind: WaitKind::Timer { resume_at_ms },
        });
        let request = HostRequest::RegisterTimer {
            resume_at_ms,
            branch,
        };
        self.outstanding = Some(request.clone());
        Ok(Some(EngineOutcome::Host(request)))
    }

    pub(super) fn yield_wait(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let event_name = expect_string(self.pop()?)?;
        let wait_id = if self.join_is_active() {
            self.bind_planned_branch(BranchOp::Event {
                event_name: event_name.clone(),
            })?
        } else {
            event_wait_id(
                &self.continuation.execution_id,
                &event_name,
                None,
                self.frame()?.pc,
            )
        };
        self.advance_pc()?;
        self.continuation.pending = Some(PendingOp::Wait {
            kind: WaitKind::Event {
                wait_id: wait_id.clone(),
                event_name: event_name.clone(),
                correlation_key: None,
            },
        });
        let request = HostRequest::RegisterWait {
            wait: WaitRegistration {
                wait_id,
                event_name,
                correlation_key: None,
                timeout_ms: None,
            },
        };
        self.outstanding = Some(request.clone());
        Ok(Some(EngineOutcome::Host(request)))
    }

    pub(super) fn yield_invoke(
        &mut self,
        arg_count: u32,
    ) -> Result<Option<EngineOutcome>, CoreError> {
        let program_name = expect_string(self.pop()?)?;
        let mut args = Vec::with_capacity(arg_count as usize);
        for _ in 0..arg_count {
            args.push(self.pop()?);
        }
        args.reverse();
        let args = args
            .into_iter()
            .map(|value| export_value(&self.continuation.heap, &value).unwrap_or(value))
            .collect();
        let (invoke_id, child_execution_id) = if self.join_is_active() {
            let invoke_id = self.next_planned_branch_id()?;
            let child_execution_id = format!("child:{invoke_id}");
            self.bind_planned_branch(BranchOp::Child {
                program_name: program_name.clone(),
                invoke_id: invoke_id.clone(),
                child_execution_id: child_execution_id.clone(),
            })?;
            (invoke_id, child_execution_id)
        } else {
            let pc = self.frame()?.pc;
            let invoke_id = format!("{}:invoke:{}", self.continuation.execution_id, pc);
            let child_execution_id = format!("child:{invoke_id}");
            (invoke_id, child_execution_id)
        };
        self.advance_pc()?;
        self.continuation.pending = Some(PendingOp::Wait {
            kind: WaitKind::Child {
                invoke_id: invoke_id.clone(),
                child_execution_id: child_execution_id.clone(),
            },
        });
        let request = HostRequest::CreateChild {
            child: ChildSpec {
                invoke_id,
                child_execution_id,
                program_name,
                args,
            },
        };
        self.outstanding = Some(request.clone());
        Ok(Some(EngineOutcome::Host(request)))
    }

    pub(super) fn require_entry_frame(&self) -> Result<(), CoreError> {
        let entry = self.artifact.program.entry.0;
        match self.continuation.frames.as_slice() {
            [frame] if frame.func_id == entry => Ok(()),
            _ => Err(CoreError::TypeError(
                "a durable boundary may execute only while the program entry frame is active"
                    .into(),
            )),
        }
    }
}

fn expect_string(value: Value) -> Result<String, CoreError> {
    match value {
        Value::String(text) => Ok(text),
        _ => Err(CoreError::UnknownInstruction { func: 0, pc: 0 }),
    }
}

fn expect_number(value: Value) -> Result<f64, CoreError> {
    match value {
        Value::Number(number) => Ok(number),
        _ => Err(CoreError::UnknownInstruction { func: 0, pc: 0 }),
    }
}

fn event_wait_id(
    execution_id: &str,
    event_name: &str,
    correlation_key: Option<&str>,
    pc: u32,
) -> String {
    format!(
        "{}:{}:{}:{}",
        execution_id,
        event_name,
        correlation_key.unwrap_or(""),
        pc
    )
}
