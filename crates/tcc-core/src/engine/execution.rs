// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use super::lifecycle::const_to_value;
use super::*;

impl Engine {
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
                let returning = self.continuation.frames.len().saturating_sub(1) as u32;
                self.continuation.frames.pop();
                self.continuation
                    .try_stack
                    .retain(|handler| handler.frame < returning);
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
            Instruction::Effect { has_input } => {
                self.require_entry_frame()?;
                self.yield_effect(has_input)
            }
            Instruction::Sleep => {
                self.require_entry_frame()?;
                self.yield_sleep()
            }
            Instruction::WaitForEvent => {
                self.require_entry_frame()?;
                self.yield_wait()
            }
            Instruction::Invoke { arg_count } => {
                self.require_entry_frame()?;
                self.yield_invoke(arg_count)
            }
            Instruction::Fork { count, join_pc } => {
                self.require_entry_frame()?;
                self.begin_join(count, join_pc)
            }
            Instruction::JoinAll => {
                self.require_entry_frame()?;
                self.finish_join()
            }
            Instruction::JoinAny => {
                self.require_entry_frame()?;
                self.finish_any()
            }
            Instruction::ArrayIndex { index } => self.array_index(index),
            Instruction::Add => self.apply_bin(crate::compute::Arith::Add),
            Instruction::Sub => self.apply_bin(crate::compute::Arith::Sub),
            Instruction::Mul => self.apply_bin(crate::compute::Arith::Mul),
            Instruction::Div => self.apply_bin(crate::compute::Arith::Div),
            Instruction::Rem => self.apply_bin(crate::compute::Arith::Rem),
            Instruction::Neg => self.apply_bin(crate::compute::Arith::Neg),
            Instruction::Pow => self.apply_bin(crate::compute::Arith::Pow),
            Instruction::FloorDiv => self.apply_bin(crate::compute::Arith::FloorDiv),
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
            Instruction::NewCell => self.new_cell(),
            Instruction::NewEnv { count } => self.new_env(count),
            Instruction::NewClosure { func } => self.new_closure(func),
            Instruction::EnvGet { index } => self.env_get(index),
            Instruction::EnvSet { index } => self.env_set(index),
            Instruction::EnvSlot { index } => self.env_slot(index),
            Instruction::CallClosure { argc } => self.call_closure(argc),
            Instruction::LoadFunc { func } => self.load_func(func),
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
                    frame: self.continuation.frames.len().saturating_sub(1) as u32,
                });
                self.advance_pc().map(|()| None)
            }
            Instruction::PopTry => {
                self.continuation.try_stack.pop();
                self.advance_pc().map(|()| None)
            }
        }
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

    pub(super) fn frame(&self) -> Result<&tcc_state::Frame, CoreError> {
        self.continuation.frames.last().ok_or(CoreError::NoFrame)
    }

    fn frame_mut(&mut self) -> Result<&mut tcc_state::Frame, CoreError> {
        self.continuation
            .frames
            .last_mut()
            .ok_or(CoreError::NoFrame)
    }

    pub(super) fn pop(&mut self) -> Result<Value, CoreError> {
        self.continuation
            .stack
            .pop()
            .ok_or(CoreError::StackUnderflow)
    }

    pub(super) fn advance_pc(&mut self) -> Result<(), CoreError> {
        let frame = self.frame_mut()?;
        frame.pc += 1;
        Ok(())
    }

    pub(super) fn set_pc(&mut self, pc: u32) -> Result<(), CoreError> {
        self.frame_mut()?.pc = pc;
        Ok(())
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
