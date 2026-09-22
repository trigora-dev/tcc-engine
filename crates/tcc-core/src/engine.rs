use std::collections::HashMap;

use tcc_ir::{
    analyze_program, validate, Artifact, ConstValue, EngineCaps, FuncId, FunctionLiveness,
    Instruction, Pc, ENGINE_FORMAT_VERSION, LANGUAGE_SEMANTICS_PY, LANGUAGE_SEMANTICS_TS,
    MAX_JOIN_BRANCHES,
};
use tcc_state::{
    absorb_continuation, absorb_value, export_value, gc_heap, persist_intent, structural_eq,
    BranchOp, BranchPhase, Continuation, ContinuationStatus, HeapCell, JoinBranch, JoinKind,
    JoinReentry, JoinState, JoinStatus, PendingOp, PersistKind, Value, WaitKind,
};

use crate::error::CoreError;
use crate::protocol::{
    ChildSpec, EffectRecord, EffectStatus, HostRequest, HostResponse, WaitRegistration,
};

/// Checkpoint requests carry a continuation delta, so this enum stays large.
#[allow(clippy::large_enum_variant)]
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
    /// Last continuation confirmed by `persist_confirmed`. Unacked persists do not update this.
    confirmed: Option<Continuation>,
    deltas_since_snapshot: u32,
    liveness: HashMap<FuncId, FunctionLiveness>,
}

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
        match continuation.status {
            ContinuationStatus::Completed => return Err(CoreError::Terminal("completed")),
            ContinuationStatus::Failed => return Err(CoreError::Terminal("failed")),
            ContinuationStatus::Cancelled => return Err(CoreError::Terminal("cancelled")),
            _ => {}
        }
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
                HostRequest::PersistCheckpoint { kind, .. },
                HostResponse::PersistConfirmed { revision },
            ) => {
                self.continuation.revision = revision;
                match kind {
                    PersistKind::Snapshot => self.deltas_since_snapshot = 0,
                    PersistKind::Delta => self.deltas_since_snapshot += 1,
                }
                if self
                    .continuation
                    .join
                    .as_ref()
                    .is_some_and(|join| join.state == JoinStatus::Failed)
                {
                    self.confirmed = Some(self.continuation.clone());
                    self.throw_join_failure()?;
                    return Ok(());
                }
                if self.continuation.join.as_ref().is_some_and(|join| {
                    join.kind == JoinKind::Any && join.state == JoinStatus::Succeeded
                }) {
                    self.confirmed = Some(self.continuation.clone());
                    self.publish_race_success()?;
                    return Ok(());
                }
                self.continuation.status = self.status_after_checkpoint();
                self.confirmed = Some(self.continuation.clone());
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
                let value = absorb_value(&mut self.continuation.heap, value);
                self.continuation.stack.push(value);
                self.advance_pc()?;
                self.continuation.pending = None;
                self.outstanding = Some(HostRequest::PersistEffect {
                    record: EffectRecord {
                        key,
                        idempotency_key,
                        status: EffectStatus::Completed,
                        result: self.continuation.stack.last().cloned().map(|value| {
                            export_value(&self.continuation.heap, &value).unwrap_or(value)
                        }),
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
                if self.race_is_active() {
                    self.advance_pc()?;
                }
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
                    if self.join_is_active() {
                        if self.race_is_active() {
                            self.record_race_branch_failure(message);
                            self.after_race_branch_progress();
                        } else {
                            self.fail_active_branch(message);
                            self.queue_persist();
                        }
                        return Ok(());
                    }
                    return self.throw_value(Value::String(message)).map(|_| ());
                }
                if self.join_is_active() {
                    let value = self.pop()?;
                    self.complete_planned_branch(value)?;
                    if self.race_is_active() {
                        self.after_race_branch_progress();
                    } else {
                        self.continuation.status = self.status_after_checkpoint();
                        self.queue_persist();
                    }
                    return Ok(());
                }
                self.queue_persist();
                Ok(())
            }
            (
                HostRequest::RegisterTimer { .. }
                | HostRequest::RegisterWait { .. }
                | HostRequest::CreateChild { .. },
                HostResponse::Ack,
            ) => {
                if self.join_is_active() {
                    self.note_branch_registered();
                    self.continuation.pending = None;
                    if self.race_is_active() {
                        self.after_race_branch_progress();
                    } else {
                        self.continuation.status = self.status_after_checkpoint();
                        self.queue_persist();
                    }
                    return Ok(());
                }
                self.continuation.status = ContinuationStatus::Suspended;
                self.queue_persist();
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
        if self.continuation.join.is_some() {
            return self.apply_join_wake(response);
        }
        if response_branch(&response).is_some() {
            return Err(CoreError::TypeError(
                "branch is only valid for a concurrent group".into(),
            ));
        }
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
                HostResponse::EventPayload { value, .. },
            ) => {
                let value = absorb_value(&mut self.continuation.heap, value);
                self.continuation.stack.push(value);
                self.clear_wait_and_persist();
                Ok(())
            }
            (
                ContinuationStatus::Suspended,
                Some(PendingOp::Wait {
                    kind: WaitKind::Timer { .. },
                }),
                HostResponse::TimerFired { .. },
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
                HostResponse::ChildResult { value, .. },
            ) => {
                let value = absorb_value(&mut self.continuation.heap, value);
                self.continuation.stack.push(value);
                self.clear_wait_and_persist();
                Ok(())
            }
            (_, _, HostResponse::Cancel) => {
                self.continuation.status = ContinuationStatus::Cancelled;
                self.continuation.pending = None;
                self.queue_persist();
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
        self.queue_persist();
    }

    fn queue_persist(&mut self) {
        self.compact_dead_locals();
        absorb_continuation(&mut self.continuation);
        gc_heap(&mut self.continuation);
        let intent = persist_intent(
            self.confirmed.as_ref(),
            &self.continuation,
            self.deltas_since_snapshot,
        );
        self.outstanding = Some(HostRequest::PersistCheckpoint {
            revision: self.continuation.revision + 1,
            kind: intent.kind,
            base_revision: self.confirmed.as_ref().map(|c| c.revision).unwrap_or(0),
            materialize: intent.materialize,
            delta: intent.delta,
        });
    }

    fn compact_dead_locals(&mut self) {
        for frame in &mut self.continuation.frames {
            let Some(live) = self.liveness.get(&FuncId(frame.func_id)) else {
                continue;
            };
            for (slot, value) in frame.locals.iter_mut().enumerate() {
                if !live.is_live_at(frame.pc, slot as u32) {
                    *value = Value::Undefined;
                }
            }
        }
    }

    pub fn run_until_host(&mut self, budget: u32) -> EngineOutcome {
        if let Some(request) = &self.outstanding {
            return EngineOutcome::Host(request.clone());
        }

        if self
            .continuation
            .join
            .as_ref()
            .is_some_and(|join| join.state == JoinStatus::Failed)
        {
            if let Err(err) = self.throw_join_failure() {
                self.continuation.status = ContinuationStatus::Failed;
                return EngineOutcome::Failed {
                    message: err.to_string(),
                };
            }
            if let Some(request) = self.outstanding.clone() {
                return EngineOutcome::Host(request);
            }
        }
        if self
            .continuation
            .join
            .as_ref()
            .is_some_and(|join| join.kind == JoinKind::Any && join.state == JoinStatus::Succeeded)
        {
            if let Err(err) = self.publish_race_success() {
                self.continuation.status = ContinuationStatus::Failed;
                return EngineOutcome::Failed {
                    message: err.to_string(),
                };
            }
            if let Some(request) = self.outstanding.clone() {
                return EngineOutcome::Host(request);
            }
        }

        match self.continuation.status {
            ContinuationStatus::Completed => {
                let raw = self.continuation.result.clone().unwrap_or(Value::Undefined);
                let result = export_value(&self.continuation.heap, &raw).unwrap_or(raw);
                return EngineOutcome::Completed { result };
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
                let id = self.alloc_cell(HeapCell::Object(std::collections::BTreeMap::new()));
                self.continuation.stack.push(Value::Ref(id));
                self.advance_pc().map(|()| None)
            }
            Instruction::SetProp { key } => {
                let value = self.pop()?;
                let object = self.pop()?;
                let id = self.expect_ref(object, "SetProp requires an object")?;
                self.guard_structure(id)?;
                match self.continuation.heap.get_mut(id as usize) {
                    Some(HeapCell::Object(fields)) => {
                        fields.insert(key.clone(), value);
                    }
                    _ => {
                        return Err(CoreError::TypeError(
                            "SetProp requires an object".to_string(),
                        ))
                    }
                }
                self.continuation.stack.push(Value::Ref(id));
                self.advance_pc().map(|()| None)
            }
            Instruction::GetProp { key } => {
                let object = self.pop()?;
                let id = self.expect_ref(object, "GetProp requires an object")?;
                let value = match self.continuation.heap.get(id as usize) {
                    Some(HeapCell::Object(fields)) => {
                        fields.get(&key).cloned().unwrap_or(Value::Undefined)
                    }
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
                let id = self.alloc_cell(HeapCell::Array(Vec::new()));
                self.continuation.stack.push(Value::Ref(id));
                self.advance_pc().map(|()| None)
            }
            Instruction::ArrayPush => {
                let value = self.pop()?;
                let array = self.pop()?;
                let id = self.expect_ref(array, "ArrayPush requires an array")?;
                self.guard_structure(id)?;
                match self.continuation.heap.get_mut(id as usize) {
                    Some(HeapCell::Array(items)) => items.push(value),
                    _ => {
                        return Err(CoreError::TypeError(
                            "ArrayPush requires an array".to_string(),
                        ))
                    }
                }
                self.continuation.stack.push(Value::Ref(id));
                self.advance_pc().map(|()| None)
            }
            Instruction::StrictEq => self.compare_equal(false),
            Instruction::StrictNeq => self.compare_equal(true),
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
                    self.queue_persist();
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
            Instruction::Invoke { arg_count } => self.yield_invoke(arg_count),
            Instruction::Fork { count, join_pc } => self.begin_join(count, join_pc),
            Instruction::JoinAll => self.finish_join(),
            Instruction::JoinAny => self.finish_any(),
            Instruction::ArrayIndex { index } => self.array_index(index),
            Instruction::Add => self.apply_bin(crate::compute::Arith::Add),
            Instruction::Sub => self.apply_bin(crate::compute::Arith::Sub),
            Instruction::Mul => self.apply_bin(crate::compute::Arith::Mul),
            Instruction::Div => self.apply_bin(crate::compute::Arith::Div),
            Instruction::Rem => self.apply_bin(crate::compute::Arith::Rem),
            Instruction::Neg => self.apply_bin(crate::compute::Arith::Neg),
            Instruction::GetIndex => self.get_index(),
            Instruction::SetIndex => self.set_index(),
            Instruction::Length => self.collection_length(),
            Instruction::WatchIter => self.watch_iter(),
            Instruction::UnwatchIter => self.unwatch_iter(),
            Instruction::Same => {
                let right = self.pop()?;
                let left = self.pop()?;
                self.continuation
                    .stack
                    .push(Value::Bool(js_equal(&left, &right)));
                self.advance_pc().map(|()| None)
            }
            Instruction::Call { func, argc } => self.call(func, argc),
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
        };
        self.outstanding = Some(request.clone());
        Ok(Some(EngineOutcome::Host(request)))
    }

    fn yield_sleep(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
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

    fn yield_wait(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
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

    fn yield_invoke(&mut self, arg_count: u32) -> Result<Option<EngineOutcome>, CoreError> {
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

    fn throw_value(&mut self, value: Value) -> Result<Option<EngineOutcome>, CoreError> {
        if let Some(handler) = self.continuation.try_stack.pop() {
            self.continuation.stack.truncate(handler.stack_len as usize);
            self.continuation.stack.push(value);
            self.set_pc(handler.catch)?;
            return Ok(None);
        }
        self.continuation.status = ContinuationStatus::Failed;
        self.continuation.result = Some(value);
        self.queue_persist();
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

    fn begin_join(&mut self, count: u32, join_pc: Pc) -> Result<Option<EngineOutcome>, CoreError> {
        if self.continuation.join.is_some() {
            return Err(CoreError::TypeError(
                "nested concurrent groups are not supported".into(),
            ));
        }
        if count == 0 || count as usize > MAX_JOIN_BRANCHES {
            return Err(CoreError::TypeError(format!(
                "join branch count {count} exceeds {MAX_JOIN_BRANCHES}"
            )));
        }
        let kind = self.join_kind_at(join_pc)?;
        let site = self.frame()?.pc;
        let reentry = self
            .continuation
            .reentries
            .iter()
            .find(|entry| entry.site == site)
            .map(|entry| entry.next)
            .unwrap_or(0);
        let execution_id = self.continuation.execution_id.clone();
        let instance = format!("{execution_id}:{site}:{reentry}");
        let branches = (0..count)
            .map(|index| JoinBranch {
                index,
                branch_id: format!("{instance}:{index}"),
                op: None,
                phase: BranchPhase::Planned,
                result: None,
                error: None,
            })
            .collect();
        self.continuation.join = Some(JoinState {
            kind,
            site,
            reentry,
            join_pc: join_pc.0,
            state: JoinStatus::Active,
            winner_branch: None,
            failure_branch: None,
            branches,
        });
        self.advance_pc().map(|()| None)
    }

    fn finish_join(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let join = self
            .continuation
            .join
            .clone()
            .ok_or_else(|| CoreError::TypeError("JoinAll without Fork".into()))?;
        if join.kind != JoinKind::All {
            return Err(CoreError::TypeError("JoinAll on a race".into()));
        }
        if join.state != JoinStatus::Active
            || join
                .branches
                .iter()
                .any(|branch| branch.phase != BranchPhase::Completed)
        {
            self.continuation.status = ContinuationStatus::Suspended;
            return Ok(Some(EngineOutcome::Suspended));
        }
        let items = join
            .branches
            .iter()
            .map(|branch| branch.result.clone().unwrap_or(Value::Undefined))
            .collect();
        self.record_reentry(join.site, join.reentry);
        if let Some(join) = self.continuation.join.as_mut() {
            join.state = JoinStatus::Succeeded;
        }
        self.continuation.join = None;
        let id = self.alloc_cell(HeapCell::Array(items));
        self.continuation.stack.push(Value::Ref(id));
        self.advance_pc().map(|()| None)
    }

    fn finish_any(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let join = self
            .continuation
            .join
            .clone()
            .ok_or_else(|| CoreError::TypeError("JoinAny without Fork".into()))?;
        if join.kind != JoinKind::Any {
            return Err(CoreError::TypeError("JoinAny on an all-join".into()));
        }
        if join.state == JoinStatus::Succeeded {
            self.publish_race_success()?;
            return Ok(None);
        }
        if join.state == JoinStatus::Failed {
            self.throw_join_failure()?;
            if let Some(request) = self.outstanding.clone() {
                return Ok(Some(EngineOutcome::Host(request)));
            }
            return Ok(None);
        }
        if join
            .branches
            .iter()
            .any(|branch| branch.phase == BranchPhase::Planned)
        {
            return Err(CoreError::TypeError(
                "JoinAny before every branch is registered".into(),
            ));
        }
        if join
            .branches
            .iter()
            .any(|branch| matches!(branch.phase, BranchPhase::Completed | BranchPhase::Failed))
        {
            self.settle_race();
            self.queue_persist();
            return Ok(Some(EngineOutcome::Host(
                self.outstanding.clone().expect("just queued"),
            )));
        }
        self.continuation.status = ContinuationStatus::Suspended;
        Ok(Some(EngineOutcome::Suspended))
    }

    fn join_kind_at(&self, join_pc: Pc) -> Result<JoinKind, CoreError> {
        let frame = self.frame()?;
        let function = self
            .artifact
            .function(FuncId(frame.func_id))
            .ok_or(CoreError::NoFrame)?;
        match function.instructions.get(join_pc.0 as usize) {
            Some(Instruction::JoinAll) => Ok(JoinKind::All),
            Some(Instruction::JoinAny) => Ok(JoinKind::Any),
            _ => Err(CoreError::TypeError(
                "join_pc must be JoinAll or JoinAny".into(),
            )),
        }
    }

    fn race_is_active(&self) -> bool {
        self.continuation
            .join
            .as_ref()
            .is_some_and(|join| join.kind == JoinKind::Any && join.state == JoinStatus::Active)
    }

    fn after_race_branch_progress(&mut self) {
        let planned = self.continuation.join.as_ref().is_some_and(|join| {
            join.branches
                .iter()
                .any(|branch| branch.phase == BranchPhase::Planned)
        });
        if planned {
            self.continuation.status = ContinuationStatus::Runnable;
            self.queue_persist();
            return;
        }
        let ready = self.continuation.join.as_ref().is_some_and(|join| {
            join.branches
                .iter()
                .any(|branch| matches!(branch.phase, BranchPhase::Completed | BranchPhase::Failed))
        });
        if ready {
            self.settle_race();
            self.queue_persist();
            return;
        }
        self.continuation.status = ContinuationStatus::Suspended;
        self.queue_persist();
    }

    fn settle_race(&mut self) {
        let Some(join) = self.continuation.join.as_mut() else {
            return;
        };
        if join.kind != JoinKind::Any || join.state != JoinStatus::Active {
            return;
        }
        let winner = join
            .branches
            .iter()
            .filter(|branch| matches!(branch.phase, BranchPhase::Completed | BranchPhase::Failed))
            .map(|branch| branch.index)
            .min();
        let Some(index) = winner else {
            return;
        };
        let failed = join
            .branches
            .iter()
            .any(|branch| branch.index == index && branch.phase == BranchPhase::Failed);
        for branch in &mut join.branches {
            if branch.index != index
                && matches!(branch.phase, BranchPhase::Planned | BranchPhase::Registered)
            {
                branch.phase = BranchPhase::Detached;
            }
        }
        join.winner_branch = Some(index);
        if failed {
            join.state = JoinStatus::Failed;
            join.failure_branch = Some(index);
        } else {
            join.state = JoinStatus::Succeeded;
            join.failure_branch = None;
        }
        self.continuation.pending = None;
        self.continuation.status = ContinuationStatus::Runnable;
    }

    fn publish_race_success(&mut self) -> Result<(), CoreError> {
        let join = self
            .continuation
            .join
            .clone()
            .ok_or_else(|| CoreError::TypeError("no race winner".into()))?;
        let value = join
            .winner_branch
            .and_then(|index| {
                join.branches
                    .iter()
                    .find(|branch| branch.index == index)
                    .and_then(|branch| branch.result.clone())
            })
            .unwrap_or(Value::Undefined);
        self.record_reentry(join.site, join.reentry);
        self.continuation.join = None;
        self.continuation.stack.push(value);
        self.continuation.status = ContinuationStatus::Runnable;
        if self.frame()?.pc == join.join_pc {
            self.advance_pc()?;
        }
        Ok(())
    }

    fn record_race_branch_failure(&mut self, message: String) {
        let Some(join) = self.continuation.join.as_mut() else {
            return;
        };
        if let Some(branch) = join
            .branches
            .iter_mut()
            .find(|branch| branch.phase == BranchPhase::Planned && branch.op.is_some())
        {
            branch.phase = BranchPhase::Failed;
            branch.error = Some(message);
        }
    }

    fn array_index(&mut self, index: u32) -> Result<Option<EngineOutcome>, CoreError> {
        let top = self
            .continuation
            .stack
            .last()
            .cloned()
            .ok_or(CoreError::StackUnderflow)?;
        let id = self.expect_ref(top, "ArrayIndex requires an array")?;
        let value = match self.continuation.heap.get(id as usize) {
            Some(HeapCell::Array(items)) => items
                .get(index as usize)
                .cloned()
                .unwrap_or(Value::Undefined),
            _ => {
                return Err(CoreError::TypeError(
                    "ArrayIndex requires an array".to_string(),
                ))
            }
        };
        self.continuation.stack.push(value);
        self.advance_pc().map(|()| None)
    }

    fn alloc_cell(&mut self, cell: HeapCell) -> u32 {
        let id = self.continuation.heap.len() as u32;
        self.continuation.heap.push(cell);
        id
    }

    fn expect_ref(&self, value: Value, message: &str) -> Result<u32, CoreError> {
        match value {
            Value::Ref(id) => Ok(id),
            _ => Err(CoreError::TypeError(message.to_string())),
        }
    }

    fn guard_structure(&self, id: u32) -> Result<(), CoreError> {
        if self.continuation.iterating.contains(&id) {
            Err(CoreError::TypeError(
                "cannot change a collection while it is being iterated".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    fn compare_equal(&mut self, negate: bool) -> Result<Option<EngineOutcome>, CoreError> {
        let right = self.pop()?;
        let left = self.pop()?;
        let equal = if self.continuation.language_semantics_version == LANGUAGE_SEMANTICS_PY {
            structural_eq(&self.continuation.heap, &left, &right)
                .map_err(|err| CoreError::TypeError(err.to_string()))?
        } else {
            js_equal(&left, &right)
        };
        self.continuation
            .stack
            .push(Value::Bool(if negate { !equal } else { equal }));
        self.advance_pc().map(|()| None)
    }

    fn apply_bin(&mut self, op: crate::compute::Arith) -> Result<Option<EngineOutcome>, CoreError> {
        let (left, right) = if matches!(op, crate::compute::Arith::Neg) {
            (self.pop()?, None)
        } else {
            let right = self.pop()?;
            (self.pop()?, Some(right))
        };
        let lang = self.continuation.language_semantics_version.clone();
        match crate::compute::apply_arith(&lang, op, left, right) {
            Ok(value) => {
                self.continuation.stack.push(value);
                self.advance_pc().map(|()| None)
            }
            Err(crate::compute::ArithFail::Type(message)) => {
                Err(CoreError::TypeError(message.to_string()))
            }
            Err(crate::compute::ArithFail::Raise(message)) => {
                self.throw_value(Value::String(message.to_string()))
            }
        }
    }

    fn get_index(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let index_value = self.pop()?;
        let collection = self.pop()?;
        let id = self.expect_ref(collection, "index requires a collection")?;
        let lang = self.continuation.language_semantics_version.clone();
        let index = match crate::compute::index_number(&lang, index_value) {
            Ok(index) => index,
            Err(crate::compute::ArithFail::Type(message)) => {
                return Err(CoreError::TypeError(message.to_string()))
            }
            Err(crate::compute::ArithFail::Raise(message)) => {
                return self.throw_value(Value::String(message.to_string()))
            }
        };
        let len = match self.continuation.heap.get(id as usize) {
            Some(HeapCell::Array(items)) => items.len(),
            _ => {
                return Err(CoreError::TypeError(
                    "index requires a list or array".to_string(),
                ))
            }
        };
        let resolved = if lang == LANGUAGE_SEMANTICS_PY && index < 0 {
            len as i64 + index
        } else {
            index
        };
        if resolved < 0 || resolved as usize >= len {
            if lang == LANGUAGE_SEMANTICS_PY {
                return self.throw_value(Value::String("IndexError".into()));
            }
            self.continuation.stack.push(Value::Undefined);
        } else {
            let value = match self.continuation.heap.get(id as usize) {
                Some(HeapCell::Array(items)) => items[resolved as usize].clone(),
                _ => Value::Undefined,
            };
            self.continuation.stack.push(value);
        }
        self.advance_pc().map(|()| None)
    }

    fn set_index(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let value = self.pop()?;
        let index_value = self.pop()?;
        let collection = self.pop()?;
        let id = self.expect_ref(collection, "index assignment requires a collection")?;
        self.guard_structure(id)?;
        let lang = self.continuation.language_semantics_version.clone();
        let index = match crate::compute::index_number(&lang, index_value) {
            Ok(index) => index,
            Err(crate::compute::ArithFail::Type(message)) => {
                return Err(CoreError::TypeError(message.to_string()))
            }
            Err(crate::compute::ArithFail::Raise(message)) => {
                return self.throw_value(Value::String(message.to_string()))
            }
        };
        let len = match self.continuation.heap.get(id as usize) {
            Some(HeapCell::Array(items)) => items.len(),
            _ => {
                return Err(CoreError::TypeError(
                    "index assignment requires a list or array".to_string(),
                ))
            }
        };
        let resolved = if lang == LANGUAGE_SEMANTICS_PY && index < 0 {
            len as i64 + index
        } else {
            index
        };
        if resolved < 0 || resolved as usize >= len {
            return Err(CoreError::TypeError(
                "index assignment is out of range".to_string(),
            ));
        }
        match self.continuation.heap.get_mut(id as usize) {
            Some(HeapCell::Array(items)) => items[resolved as usize] = value,
            _ => {
                return Err(CoreError::TypeError(
                    "index assignment requires a list or array".to_string(),
                ))
            }
        }
        self.continuation.stack.push(Value::Ref(id));
        self.advance_pc().map(|()| None)
    }

    fn collection_length(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let collection = self.pop()?;
        let id = self.expect_ref(collection, "length requires a collection")?;
        let len = match self.continuation.heap.get(id as usize) {
            Some(HeapCell::Array(items)) => items.len(),
            _ => {
                return Err(CoreError::TypeError(
                    "length requires a list or array".to_string(),
                ))
            }
        };
        self.continuation.stack.push(Value::Number(len as f64));
        self.advance_pc().map(|()| None)
    }

    fn watch_iter(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let top = self
            .continuation
            .stack
            .last()
            .cloned()
            .ok_or(CoreError::StackUnderflow)?;
        let id = self.expect_ref(top, "for requires a list or array")?;
        if !matches!(
            self.continuation.heap.get(id as usize),
            Some(HeapCell::Array(_))
        ) {
            return Err(CoreError::TypeError(
                "for requires a list or array".to_string(),
            ));
        }
        self.continuation.iterating.push(id);
        self.advance_pc().map(|()| None)
    }

    fn unwatch_iter(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        if self.continuation.iterating.pop().is_none() {
            return Err(CoreError::TypeError(
                "unwatch without an active for".to_string(),
            ));
        }
        self.advance_pc().map(|()| None)
    }

    fn call(&mut self, func: FuncId, argc: u32) -> Result<Option<EngineOutcome>, CoreError> {
        let mut args = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            args.push(self.pop()?);
        }
        args.reverse();
        let function = self
            .artifact
            .function(func)
            .ok_or_else(|| CoreError::TypeError(format!("missing function {}", func.0)))?;
        if function.local_count as usize > 4096 {
            return Err(CoreError::TypeError("too many locals".into()));
        }
        let mut locals = vec![Value::Undefined; function.local_count as usize];
        bind_program_args(
            &self.continuation.language_semantics_version,
            function.param_count,
            &function.param_defaults,
            &args,
            &mut locals,
        )?;
        let func_id = function.id.0;
        self.advance_pc()?;
        self.continuation.frames.push(tcc_state::Frame {
            func_id,
            pc: 0,
            locals,
        });
        Ok(None)
    }

    fn join_is_active(&self) -> bool {
        self.continuation
            .join
            .as_ref()
            .is_some_and(|join| join.state == JoinStatus::Active)
    }

    fn next_planned_branch_id(&self) -> Result<String, CoreError> {
        let join = self
            .continuation
            .join
            .as_ref()
            .ok_or_else(|| CoreError::TypeError("durable op outside a join".into()))?;
        join.branches
            .iter()
            .find(|branch| branch.phase == BranchPhase::Planned && branch.op.is_none())
            .map(|branch| branch.branch_id.clone())
            .ok_or_else(|| CoreError::TypeError("no planned join branch".into()))
    }

    fn bind_planned_branch(&mut self, op: BranchOp) -> Result<String, CoreError> {
        let branch_id = self.next_planned_branch_id()?;
        let join = self.continuation.join.as_mut().expect("join");
        let branch = join
            .branches
            .iter_mut()
            .find(|branch| branch.branch_id == branch_id)
            .expect("planned branch");
        branch.op = Some(op);
        Ok(branch_id)
    }

    fn note_branch_registered(&mut self) {
        let Some(join) = self.continuation.join.as_mut() else {
            return;
        };
        if let Some(branch) = join
            .branches
            .iter_mut()
            .find(|branch| branch.phase == BranchPhase::Planned && branch.op.is_some())
        {
            branch.phase = BranchPhase::Registered;
        }
    }

    fn complete_planned_branch(&mut self, value: Value) -> Result<(), CoreError> {
        let join = self
            .continuation
            .join
            .as_mut()
            .ok_or_else(|| CoreError::TypeError("no join".into()))?;
        let branch = join
            .branches
            .iter_mut()
            .find(|branch| branch.phase == BranchPhase::Planned && branch.op.is_some())
            .ok_or_else(|| CoreError::TypeError("no planned branch to complete".into()))?;
        branch.phase = BranchPhase::Completed;
        branch.result = Some(value);
        Ok(())
    }

    fn fail_active_branch(&mut self, message: String) {
        let Some(join) = self.continuation.join.as_mut() else {
            return;
        };
        let index = join
            .branches
            .iter()
            .find(|branch| branch.phase == BranchPhase::Planned && branch.op.is_some())
            .map(|branch| branch.index);
        if let Some(index) = index {
            if let Some(branch) = join
                .branches
                .iter_mut()
                .find(|branch| branch.index == index)
            {
                branch.phase = BranchPhase::Failed;
                branch.error = Some(message);
            }
            join.state = JoinStatus::Failed;
            join.failure_branch = Some(index);
            for branch in &mut join.branches {
                if branch.index != index
                    && matches!(branch.phase, BranchPhase::Planned | BranchPhase::Registered)
                {
                    branch.phase = BranchPhase::Detached;
                }
            }
        }
        self.continuation.pending = None;
        self.continuation.status = ContinuationStatus::Runnable;
    }

    fn status_after_checkpoint(&self) -> ContinuationStatus {
        if matches!(
            self.continuation.status,
            ContinuationStatus::Completed
                | ContinuationStatus::Failed
                | ContinuationStatus::Cancelled
        ) {
            return self.continuation.status;
        }
        if self.continuation.pending.is_some() {
            return ContinuationStatus::Suspended;
        }
        if let Some(join) = &self.continuation.join {
            if join.state == JoinStatus::Active {
                let planned = join
                    .branches
                    .iter()
                    .any(|branch| branch.phase == BranchPhase::Planned);
                let waiting = join
                    .branches
                    .iter()
                    .any(|branch| branch.phase == BranchPhase::Registered);
                if waiting && !planned {
                    return ContinuationStatus::Suspended;
                }
            }
        }
        ContinuationStatus::Runnable
    }

    fn record_reentry(&mut self, site: u32, reentry: u32) {
        if let Some(entry) = self
            .continuation
            .reentries
            .iter_mut()
            .find(|entry| entry.site == site)
        {
            entry.next = reentry.saturating_add(1);
        } else {
            self.continuation.reentries.push(JoinReentry {
                site,
                next: reentry.saturating_add(1),
            });
        }
    }

    fn throw_join_failure(&mut self) -> Result<(), CoreError> {
        let join = self
            .continuation
            .join
            .clone()
            .ok_or_else(|| CoreError::TypeError("no failed join".into()))?;
        let message = join
            .failure_branch
            .and_then(|index| {
                join.branches
                    .iter()
                    .find(|branch| branch.index == index)
                    .and_then(|branch| branch.error.clone())
            })
            .unwrap_or_else(|| "branch failed".to_string());
        self.record_reentry(join.site, join.reentry);
        self.continuation.join = None;
        let handler = !self.continuation.try_stack.is_empty();
        self.throw_value(Value::String(message))?;
        if handler && self.outstanding.is_none() {
            self.queue_persist();
        }
        Ok(())
    }

    fn apply_join_wake(&mut self, response: HostResponse) -> Result<(), CoreError> {
        if matches!(response, HostResponse::Cancel) {
            self.continuation.status = ContinuationStatus::Cancelled;
            self.continuation.pending = None;
            self.continuation.join = None;
            self.queue_persist();
            return Ok(());
        }
        let active = self
            .continuation
            .join
            .as_ref()
            .is_some_and(|join| join.state == JoinStatus::Active);
        let branch = response_branch(&response);
        let Some(branch_id) = branch else {
            if active {
                return Err(CoreError::TypeError("join delivery requires branch".into()));
            }
            return Ok(());
        };
        if !active {
            return Ok(());
        }
        let value = match response {
            HostResponse::EventPayload { value, .. } => value,
            HostResponse::TimerFired { .. } => Value::Undefined,
            HostResponse::ChildResult { value, .. } => value,
            other => {
                return Err(CoreError::UnexpectedHostResponse {
                    expected: "join wake",
                    got: other.kind_name(),
                });
            }
        };
        let value = absorb_value(&mut self.continuation.heap, value);
        self.complete_registered_branch(&branch_id, value)
    }

    fn complete_registered_branch(
        &mut self,
        branch_id: &str,
        value: Value,
    ) -> Result<(), CoreError> {
        let join = self
            .continuation
            .join
            .as_mut()
            .ok_or_else(|| CoreError::TypeError("no join".into()))?;
        let Some(branch) = join
            .branches
            .iter_mut()
            .find(|branch| branch.branch_id == branch_id)
        else {
            return Err(CoreError::TypeError(format!(
                "unknown branch `{branch_id}`"
            )));
        };
        if branch.phase != BranchPhase::Registered {
            return Ok(());
        }
        branch.phase = BranchPhase::Completed;
        branch.result = Some(value);
        self.continuation.pending = None;
        if self
            .continuation
            .join
            .as_ref()
            .is_some_and(|join| join.kind == JoinKind::Any && join.state == JoinStatus::Active)
        {
            self.after_race_branch_progress();
            return Ok(());
        }
        self.continuation.status = ContinuationStatus::Runnable;
        let waiting = self.continuation.join.as_ref().is_some_and(|join| {
            join.branches
                .iter()
                .any(|branch| branch.phase == BranchPhase::Registered)
        });
        if waiting {
            self.continuation.status = ContinuationStatus::Suspended;
        }
        self.queue_persist();
        Ok(())
    }
}

fn response_branch(response: &HostResponse) -> Option<String> {
    match response {
        HostResponse::EventPayload { branch, .. }
        | HostResponse::TimerFired { branch }
        | HostResponse::ChildResult { branch, .. } => branch.clone(),
        _ => None,
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

/// The only language dispatch for program arguments. After this returns, slots are ordinary locals.
fn bind_program_args(
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
        LANGUAGE_SEMANTICS_PY => {
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

fn js_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => tcc_state::numbers_strict_eq(*left, *right),
        _ => left == right,
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => *number != 0.0 && !number.is_nan(),
        Value::String(text) => !text.is_empty(),
        Value::Object(_) | Value::Array(_) | Value::Ref(_) => true,
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
                    param_defaults: Vec::new(),
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

    #[test]
    fn resume_rejects_pc_out_of_range() {
        let artifact = Artifact::minimal_return("hash-pc");
        let mut continuation = tcc_state::Continuation::start(
            "exec-pc",
            artifact.envelope.artifact_hash.clone(),
            ENGINE_FORMAT_VERSION,
            artifact.envelope.language_semantics_version.clone(),
            0,
            0,
        );
        continuation.frames[0].pc = 99;
        let err = Engine::resume(artifact, continuation, &caps()).unwrap_err();
        assert!(matches!(err, CoreError::InvalidContinuation(message) if message.contains("pc")));
    }

    #[test]
    fn resume_rejects_language_semantics_mismatch() {
        let artifact = Artifact::minimal_return("hash-sem");
        let continuation = tcc_state::Continuation::start(
            "exec-sem",
            artifact.envelope.artifact_hash.clone(),
            ENGINE_FORMAT_VERSION,
            "py.subset.v1",
            0,
            0,
        );
        let err = Engine::resume(artifact, continuation, &caps()).unwrap_err();
        assert!(matches!(err, CoreError::LanguageSemanticsMismatch { .. }));
    }

    #[test]
    fn start_rejects_oob_local_without_unknown_instruction() {
        let mut artifact = Artifact::minimal_return("hash-oob");
        artifact.program.functions[0].instructions = vec![
            Instruction::LoadLocal {
                local: tcc_ir::LocalId(0),
            },
            Instruction::Return,
        ];
        artifact.program.functions[0].spans = vec![None, None];
        let err = Engine::start(artifact, "exec-oob", &caps()).unwrap_err();
        assert!(matches!(
            err,
            CoreError::Ir(tcc_ir::IrError::LocalOutOfRange { .. })
        ));
    }
}
