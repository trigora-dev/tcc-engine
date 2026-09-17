use tcc_ir::{
    validate, Artifact, ConstValue, EngineCaps, FuncId, Instruction, ENGINE_FORMAT_VERSION,
};
use tcc_state::{Continuation, ContinuationStatus, PendingOp, Value, WaitKind};

use crate::error::CoreError;
use crate::protocol::{
    ChildSpec, EffectRecord, EffectStatus, HostRequest, HostResponse, WaitRegistration,
};

#[derive(Debug, Clone, PartialEq)]
pub enum EngineOutcome {
    Host(HostRequest),
    Completed { result: Value },
    Failed { message: String },
    Cancelled,
    BudgetExhausted,
    Suspended,
}

#[derive(Debug)]
pub struct Engine {
    artifact: Artifact,
    continuation: Continuation,
    outstanding: Option<HostRequest>,
}

impl Engine {
    pub fn start(
        artifact: Artifact,
        execution_id: impl Into<String>,
        caps: &EngineCaps,
    ) -> Result<Self, CoreError> {
        validate(&artifact, caps)?;
        let entry = artifact
            .function(artifact.program.entry)
            .expect("validated artifact has an entry function");
        let continuation = Continuation::start(
            execution_id,
            artifact.envelope.artifact_hash.clone(),
            artifact.envelope.engine_format_version,
            artifact.envelope.language_semantics_version.clone(),
            entry.id.0,
            entry.local_count,
        );
        Ok(Self {
            artifact,
            continuation,
            outstanding: None,
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
        match continuation.status {
            ContinuationStatus::Completed => return Err(CoreError::Terminal("completed")),
            ContinuationStatus::Failed => return Err(CoreError::Terminal("failed")),
            ContinuationStatus::Cancelled => return Err(CoreError::Terminal("cancelled")),
            _ => {}
        }
        Ok(Self {
            artifact,
            continuation,
            outstanding: None,
        })
    }

    pub fn continuation(&self) -> &Continuation {
        &self.continuation
    }

    pub fn artifact_hash(&self) -> &str {
        &self.artifact.envelope.artifact_hash
    }

    pub fn apply_host_response(&mut self, response: HostResponse) -> Result<(), CoreError> {
        if self.outstanding.is_none() {
            return self.apply_wake(response);
        }
        let request = self
            .outstanding
            .take()
            .ok_or(CoreError::UnexpectedHostResponse {
                expected: "none",
                got: response.kind_name(),
            })?;
        match (request, response) {
            (
                HostRequest::PersistCheckpoint { .. },
                HostResponse::PersistConfirmed { revision },
            ) => {
                self.continuation.revision = revision;
                self.continuation.status = match self.continuation.pending {
                    Some(_) => ContinuationStatus::Suspended,
                    None if matches!(
                        self.continuation.status,
                        ContinuationStatus::Completed
                            | ContinuationStatus::Failed
                            | ContinuationStatus::Cancelled
                    ) =>
                    {
                        self.continuation.status
                    }
                    None => ContinuationStatus::Runnable,
                };
                Ok(())
            }
            (HostRequest::PersistCheckpoint { .. }, HostResponse::Ack) => Ok(()),
            (
                HostRequest::RunEffect {
                    key,
                    idempotency_key,
                },
                HostResponse::EffectResult { value },
            ) => {
                self.continuation.stack.push(value);
                self.advance_pc()?;
                self.continuation.pending = None;
                self.outstanding = Some(HostRequest::PersistEffect {
                    record: EffectRecord {
                        key,
                        idempotency_key,
                        status: EffectStatus::Completed,
                        result: self.continuation.stack.last().cloned(),
                    },
                });
                Ok(())
            }
            (
                HostRequest::RunEffect {
                    key,
                    idempotency_key,
                },
                HostResponse::EffectFailed { message },
            ) => {
                self.continuation.pending = None;
                self.outstanding = Some(HostRequest::PersistEffect {
                    record: EffectRecord {
                        key,
                        idempotency_key,
                        status: EffectStatus::Failed,
                        result: Some(Value::String(message)),
                    },
                });
                Ok(())
            }
            (HostRequest::PersistEffect { record }, HostResponse::Ack) => {
                if record.status == EffectStatus::Failed {
                    let message = match record.result {
                        Some(Value::String(text)) => text,
                        _ => "effect failed".to_string(),
                    };
                    return self.throw_value(Value::String(message)).map(|_| ());
                }
                self.outstanding = Some(HostRequest::PersistCheckpoint {
                    revision: self.continuation.revision + 1,
                });
                Ok(())
            }
            (
                HostRequest::RegisterTimer { .. }
                | HostRequest::RegisterWait { .. }
                | HostRequest::CreateChild { .. },
                HostResponse::Ack,
            ) => {
                self.continuation.status = ContinuationStatus::Suspended;
                self.outstanding = Some(HostRequest::PersistCheckpoint {
                    revision: self.continuation.revision + 1,
                });
                Ok(())
            }
            (request, response) => {
                self.outstanding = Some(request);
                Err(CoreError::UnexpectedHostResponse {
                    expected: self
                        .outstanding
                        .as_ref()
                        .map(HostRequest::kind_name)
                        .unwrap_or("none"),
                    got: response.kind_name(),
                })
            }
        }
    }

    fn apply_wake(&mut self, response: HostResponse) -> Result<(), CoreError> {
        match (
            &self.continuation.status,
            &self.continuation.pending,
            response,
        ) {
            (
                ContinuationStatus::Suspended,
                Some(PendingOp::Wait {
                    kind: WaitKind::Event { .. },
                }),
                HostResponse::EventPayload { value },
            ) => {
                self.continuation.stack.push(value);
                self.clear_wait_and_persist();
                Ok(())
            }
            (
                ContinuationStatus::Suspended,
                Some(PendingOp::Wait {
                    kind: WaitKind::Timer { .. },
                }),
                HostResponse::TimerFired,
            ) => {
                self.continuation.stack.push(Value::Undefined);
                self.clear_wait_and_persist();
                Ok(())
            }
            (
                ContinuationStatus::Suspended,
                Some(PendingOp::Wait {
                    kind: WaitKind::Child { .. },
                }),
                HostResponse::ChildResult { value },
            ) => {
                self.continuation.stack.push(value);
                self.clear_wait_and_persist();
                Ok(())
            }
            (_, _, HostResponse::Cancel) => {
                self.continuation.status = ContinuationStatus::Cancelled;
                self.continuation.pending = None;
                self.outstanding = Some(HostRequest::PersistCheckpoint {
                    revision: self.continuation.revision + 1,
                });
                Ok(())
            }
            (_, _, response) => Err(CoreError::UnexpectedHostResponse {
                expected: "wake",
                got: response.kind_name(),
            }),
        }
    }

    fn clear_wait_and_persist(&mut self) {
        self.continuation.pending = None;
        self.continuation.status = ContinuationStatus::Runnable;
        self.outstanding = Some(HostRequest::PersistCheckpoint {
            revision: self.continuation.revision + 1,
        });
    }

    pub fn run_until_host(&mut self, budget: u32) -> EngineOutcome {
        if let Some(request) = &self.outstanding {
            return EngineOutcome::Host(request.clone());
        }

        match self.continuation.status {
            ContinuationStatus::Completed => {
                return EngineOutcome::Completed {
                    result: self.continuation.result.clone().unwrap_or(Value::Undefined),
                };
            }
            ContinuationStatus::Failed => {
                return EngineOutcome::Failed {
                    message: match &self.continuation.result {
                        Some(Value::String(text)) => text.clone(),
                        _ => "execution failed".to_string(),
                    },
                };
            }
            ContinuationStatus::Cancelled => {
                return EngineOutcome::Cancelled;
            }
            ContinuationStatus::Suspended => {
                return EngineOutcome::Suspended;
            }
            ContinuationStatus::Runnable | ContinuationStatus::Running => {
                self.continuation.status = ContinuationStatus::Running;
            }
        }

        for _ in 0..budget {
            match self.step() {
                Ok(None) => {}
                Ok(Some(outcome)) => return outcome,
                Err(err) => {
                    self.continuation.status = ContinuationStatus::Failed;
                    return EngineOutcome::Failed {
                        message: err.to_string(),
                    };
                }
            }
        }

        EngineOutcome::BudgetExhausted
    }

    fn step(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let instruction = self.current_instruction()?.clone();
        match instruction {
            Instruction::Nop => self.advance_pc().map(|()| None),
            Instruction::Jump { target } => self.set_pc(target.0).map(|()| None),
            Instruction::JumpIfTrue { target } => {
                let value = self.pop()?;
                if is_truthy(&value) {
                    self.set_pc(target.0)?;
                } else {
                    self.advance_pc()?;
                }
                Ok(None)
            }
            Instruction::JumpIfFalse { target } => {
                let value = self.pop()?;
                if is_truthy(&value) {
                    self.advance_pc()?;
                } else {
                    self.set_pc(target.0)?;
                }
                Ok(None)
            }
            Instruction::LoadLocal { local } => {
                let value = self.frame()?.locals.get(local.0 as usize).cloned().ok_or(
                    CoreError::UnknownInstruction {
                        func: self.frame()?.func_id,
                        pc: self.frame()?.pc,
                    },
                )?;
                self.continuation.stack.push(value);
                self.advance_pc().map(|()| None)
            }
            Instruction::StoreLocal { local } => {
                let value = self.pop()?;
                let frame = self.frame_mut()?;
                let slot = frame.locals.get_mut(local.0 as usize).ok_or(
                    CoreError::UnknownInstruction {
                        func: frame.func_id,
                        pc: frame.pc,
                    },
                )?;
                *slot = value;
                self.advance_pc().map(|()| None)
            }
            Instruction::LoadConst { value } => {
                self.continuation.stack.push(const_to_value(&value));
                self.advance_pc().map(|()| None)
            }
            Instruction::Pop => {
                self.pop()?;
                self.advance_pc().map(|()| None)
            }
            Instruction::NewObject => {
                self.continuation
                    .stack
                    .push(Value::Object(std::collections::BTreeMap::new()));
                self.advance_pc().map(|()| None)
            }
            Instruction::SetProp { key } => {
                let value = self.pop()?;
                let mut object = self.pop()?;
                match &mut object {
                    Value::Object(fields) => {
                        fields.insert(key, value);
                    }
                    _ => {
                        return Err(CoreError::TypeError(
                            "SetProp requires an object".to_string(),
                        ))
                    }
                }
                self.continuation.stack.push(object);
                self.advance_pc().map(|()| None)
            }
            Instruction::GetProp { key } => {
                let object = self.pop()?;
                let value = match object {
                    Value::Object(fields) => fields.get(&key).cloned().unwrap_or(Value::Undefined),
                    _ => {
                        return Err(CoreError::TypeError(
                            "GetProp requires an object".to_string(),
                        ))
                    }
                };
                self.continuation.stack.push(value);
                self.advance_pc().map(|()| None)
            }
            Instruction::NewArray => {
                self.continuation.stack.push(Value::Array(Vec::new()));
                self.advance_pc().map(|()| None)
            }
            Instruction::ArrayPush => {
                let value = self.pop()?;
                let mut array = self.pop()?;
                match &mut array {
                    Value::Array(items) => items.push(value),
                    _ => {
                        return Err(CoreError::TypeError(
                            "ArrayPush requires an array".to_string(),
                        ))
                    }
                }
                self.continuation.stack.push(array);
                self.advance_pc().map(|()| None)
            }
            Instruction::StrictEq => self.binary(|left, right| Ok(Value::Bool(left == right))),
            Instruction::StrictNeq => self.binary(|left, right| Ok(Value::Bool(left != right))),
            Instruction::Lt => self.numeric_cmp(|a, b| a < b),
            Instruction::Le => self.numeric_cmp(|a, b| a <= b),
            Instruction::Gt => self.numeric_cmp(|a, b| a > b),
            Instruction::Ge => self.numeric_cmp(|a, b| a >= b),
            Instruction::Not => {
                let value = self.pop()?;
                self.continuation
                    .stack
                    .push(Value::Bool(!is_truthy(&value)));
                self.advance_pc().map(|()| None)
            }
            Instruction::Return => {
                let result = if self.continuation.stack.is_empty() {
                    Value::Undefined
                } else {
                    self.pop()?
                };
                self.continuation.frames.pop();
                if self.continuation.frames.is_empty() {
                    self.continuation.status = ContinuationStatus::Completed;
                    self.continuation.result = Some(result);
                    self.outstanding = Some(HostRequest::PersistCheckpoint {
                        revision: self.continuation.revision + 1,
                    });
                    Ok(Some(EngineOutcome::Host(
                        self.outstanding.clone().expect("just queued"),
                    )))
                } else {
                    self.continuation.stack.push(result);
                    Ok(None)
                }
            }
            Instruction::Effect => self.yield_effect(),
            Instruction::Sleep => self.yield_sleep(),
            Instruction::WaitForEvent => self.yield_wait(),
            Instruction::Invoke => self.yield_invoke(),
            Instruction::Call { .. } => {
                let frame = self.frame()?;
                Err(CoreError::UnknownInstruction {
                    func: frame.func_id,
                    pc: frame.pc,
                })
            }
            Instruction::Throw => {
                let value = self.pop()?;
                self.throw_value(value)
            }
            Instruction::PushTry { catch, finally } => {
                let stack_len = self.continuation.stack.len() as u32;
                self.continuation.try_stack.push(tcc_state::TryHandler {
                    catch: catch.0,
                    finally: finally.map(|pc| pc.0),
                    stack_len,
                });
                self.advance_pc().map(|()| None)
            }
            Instruction::PopTry => {
                self.continuation.try_stack.pop();
                self.advance_pc().map(|()| None)
            }
        }
    }

    fn yield_effect(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let key = expect_string(self.pop()?)?;
        let execution_id = self.continuation.execution_id.clone();
        let idempotency_key = format!("{execution_id}:{key}");
        self.continuation.pending = Some(PendingOp::Effect {
            key: key.clone(),
            idempotency_key: idempotency_key.clone(),
        });
        let request = HostRequest::RunEffect {
            key,
            idempotency_key,
        };
        self.outstanding = Some(request.clone());
        Ok(Some(EngineOutcome::Host(request)))
    }

    fn yield_sleep(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let duration = expect_number(self.pop()?)?;
        let resume_at_ms = duration.max(0.0) as u64;
        self.advance_pc()?;
        self.continuation.pending = Some(PendingOp::Wait {
            kind: WaitKind::Timer { resume_at_ms },
        });
        let request = HostRequest::RegisterTimer { resume_at_ms };
        self.outstanding = Some(request.clone());
        Ok(Some(EngineOutcome::Host(request)))
    }

    fn yield_wait(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let event_name = expect_string(self.pop()?)?;
        let wait_id = event_wait_id(
            &self.continuation.execution_id,
            &event_name,
            None,
            self.frame()?.pc,
        );
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

    fn yield_invoke(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let flow_name = expect_string(self.pop()?)?;
        let pc = self.frame()?.pc;
        let invoke_id = format!("{}:invoke:{}", self.continuation.execution_id, pc);
        let child_execution_id = format!("child:{invoke_id}");
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
                flow_name,
                input: None,
            },
        };
        self.outstanding = Some(request.clone());
        Ok(Some(EngineOutcome::Host(request)))
    }

    fn throw_value(&mut self, value: Value) -> Result<Option<EngineOutcome>, CoreError> {
        if let Some(handler) = self.continuation.try_stack.pop() {
            self.continuation.stack.truncate(handler.stack_len as usize);
            self.continuation.stack.push(value);
            self.set_pc(handler.catch)?;
            return Ok(None);
        }
        self.continuation.status = ContinuationStatus::Failed;
        self.continuation.result = Some(value);
        self.outstanding = Some(HostRequest::PersistCheckpoint {
            revision: self.continuation.revision + 1,
        });
        Ok(Some(EngineOutcome::Host(
            self.outstanding.clone().expect("just queued"),
        )))
    }

    fn binary(
        &mut self,
        op: impl FnOnce(Value, Value) -> Result<Value, CoreError>,
    ) -> Result<Option<EngineOutcome>, CoreError> {
        let right = self.pop()?;
        let left = self.pop()?;
        self.continuation.stack.push(op(left, right)?);
        self.advance_pc().map(|()| None)
    }

    fn numeric_cmp(
        &mut self,
        op: impl FnOnce(f64, f64) -> bool,
    ) -> Result<Option<EngineOutcome>, CoreError> {
        self.binary(|left, right| match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Bool(op(a, b))),
            _ => Err(CoreError::TypeError(
                "numeric compare requires numbers".to_string(),
            )),
        })
    }

    fn current_instruction(&self) -> Result<&Instruction, CoreError> {
        let frame = self.frame()?;
        let function = self
            .artifact
            .function(FuncId(frame.func_id))
            .ok_or(CoreError::NoFrame)?;
        function
            .instructions
            .get(frame.pc as usize)
            .ok_or(CoreError::UnknownInstruction {
                func: frame.func_id,
                pc: frame.pc,
            })
    }

    fn frame(&self) -> Result<&tcc_state::Frame, CoreError> {
        self.continuation.frames.last().ok_or(CoreError::NoFrame)
    }

    fn frame_mut(&mut self) -> Result<&mut tcc_state::Frame, CoreError> {
        self.continuation
            .frames
            .last_mut()
            .ok_or(CoreError::NoFrame)
    }

    fn pop(&mut self) -> Result<Value, CoreError> {
        self.continuation
            .stack
            .pop()
            .ok_or(CoreError::StackUnderflow)
    }

    fn advance_pc(&mut self) -> Result<(), CoreError> {
        let frame = self.frame_mut()?;
        frame.pc += 1;
        Ok(())
    }

    fn set_pc(&mut self, pc: u32) -> Result<(), CoreError> {
        self.frame_mut()?.pc = pc;
        Ok(())
    }
}

fn const_to_value(value: &ConstValue) -> Value {
    match value {
        ConstValue::Undefined => Value::Undefined,
        ConstValue::Null => Value::Null,
        ConstValue::Bool(flag) => Value::Bool(*flag),
        ConstValue::Number(number) => Value::Number(*number),
        ConstValue::String(text) => Value::String(text.clone()),
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => *number != 0.0 && !number.is_nan(),
        Value::String(text) => !text.is_empty(),
        Value::Object(_) | Value::Array(_) => true,
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

#[cfg(test)]
mod tests {
    use super::*;
    use tcc_ir::{Envelope, FuncId, Function, Instruction, Program};

    fn caps() -> EngineCaps {
        EngineCaps::current()
    }

    #[test]
    fn returns_undefined_from_minimal_program() {
        let artifact = Artifact::minimal_return("hash-a");
        let mut engine = Engine::start(artifact, "exec-a", &caps()).unwrap();
        let outcome = engine.run_until_host(16);
        assert!(matches!(
            outcome,
            EngineOutcome::Host(HostRequest::PersistCheckpoint { .. })
        ));
        engine
            .apply_host_response(HostResponse::PersistConfirmed { revision: 1 })
            .unwrap();
        assert_eq!(engine.continuation().status, ContinuationStatus::Completed);
        assert_eq!(engine.continuation().result, Some(Value::Undefined));
    }

    #[test]
    fn resume_rejects_artifact_mismatch() {
        let artifact = Artifact::minimal_return("hash-a");
        let engine = Engine::start(artifact, "exec-a", &caps()).unwrap();
        let continuation = engine.continuation().clone();
        let other = Artifact::minimal_return("hash-b");
        let err = Engine::resume(other, continuation, &caps()).unwrap_err();
        assert!(matches!(err, CoreError::ArtifactMismatch { .. }));
    }

    #[test]
    fn budget_zero_does_not_run() {
        let artifact = Artifact {
            envelope: Envelope::typescript_v1("hash-c"),
            program: Program {
                entry: FuncId(0),
                functions: vec![Function {
                    id: FuncId(0),
                    name: "main".into(),
                    param_count: 0,
                    local_count: 0,
                    instructions: vec![Instruction::Nop, Instruction::Return],
                    spans: vec![None, None],
                }],
            },
        };
        let mut engine = Engine::start(artifact, "exec-c", &caps()).unwrap();
        assert_eq!(engine.run_until_host(0), EngineOutcome::BudgetExhausted);
        assert_eq!(engine.continuation().frames[0].pc, 0);
        assert_eq!(engine.continuation().revision, 0);
    }

    #[test]
    fn persist_ack_does_not_commit_revision() {
        let artifact = Artifact::minimal_return("hash-ack");
        let mut engine = Engine::start(artifact, "exec-ack", &caps()).unwrap();
        assert!(matches!(
            engine.run_until_host(16),
            EngineOutcome::Host(HostRequest::PersistCheckpoint { .. })
        ));
        engine.apply_host_response(HostResponse::Ack).unwrap();
        assert_eq!(engine.continuation().revision, 0);
        assert_eq!(engine.continuation().status, ContinuationStatus::Completed);
    }

    #[test]
    fn persist_confirmed_sets_revision() {
        let artifact = Artifact::minimal_return("hash-confirm");
        let mut engine = Engine::start(artifact, "exec-confirm", &caps()).unwrap();
        engine.run_until_host(16);
        engine
            .apply_host_response(HostResponse::PersistConfirmed { revision: 7 })
            .unwrap();
        assert_eq!(engine.continuation().revision, 7);
        assert_eq!(engine.continuation().status, ContinuationStatus::Completed);
    }
}
