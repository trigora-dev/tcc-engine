// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use super::lifecycle::bind_program_args;
use super::*;

impl Engine {
    pub(super) fn throw_value(&mut self, value: Value) -> Result<Option<EngineOutcome>, CoreError> {
        if let Some(handler) = self.continuation.try_stack.pop() {
            let depth = handler.frame as usize;
            while self.continuation.frames.len() > depth + 1 {
                self.continuation.frames.pop();
            }
            if self.continuation.frames.len() != depth + 1 {
                return Err(CoreError::TypeError(
                    "try handler frame is no longer active".into(),
                ));
            }
            let live = self.continuation.frames.len() as u32;
            self.continuation.try_stack.retain(|item| item.frame < live);
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

    pub(super) fn new_cell(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
        let value = self.pop()?;
        let id = self.alloc_cell(HeapCell::Cell(value));
        self.continuation.stack.push(Value::Ref(id));
        self.advance_pc().map(|()| None)
    }

    pub(super) fn new_env(&mut self, count: u32) -> Result<Option<EngineOutcome>, CoreError> {
        let mut cells = Vec::with_capacity(count as usize);
        for _ in 0..count {
            cells.push(self.pop()?);
        }
        cells.reverse();
        let mut ids = Vec::with_capacity(cells.len());
        for cell in cells {
            let id = self.expect_ref(cell, "environment slot requires a cell")?;
            if !matches!(
                self.continuation.heap.get(id as usize),
                Some(HeapCell::Cell(_))
            ) {
                return Err(CoreError::TypeError(
                    "environment slot requires a cell".into(),
                ));
            }
            ids.push(id);
        }
        let id = self.alloc_cell(HeapCell::Env(ids));
        self.continuation.stack.push(Value::Ref(id));
        self.advance_pc().map(|()| None)
    }

    pub(super) fn new_closure(&mut self, func: FuncId) -> Result<Option<EngineOutcome>, CoreError> {
        if self.artifact.function(func).is_none() {
            return Err(CoreError::TypeError(format!("missing function {}", func.0)));
        }
        let env = self.pop()?;
        let env_id = self.expect_ref(env, "closure requires an environment")?;
        if !matches!(
            self.continuation.heap.get(env_id as usize),
            Some(HeapCell::Env(_))
        ) {
            return Err(CoreError::TypeError(
                "closure requires an environment".into(),
            ));
        }
        let id = self.alloc_cell(HeapCell::Closure {
            func: func.0,
            env: env_id,
        });
        self.continuation.stack.push(Value::Ref(id));
        self.advance_pc().map(|()| None)
    }

    fn env_cell(&self, env_id: u32, index: u32) -> Result<u32, CoreError> {
        match self.continuation.heap.get(env_id as usize) {
            Some(HeapCell::Env(cells)) => cells.get(index as usize).copied().ok_or_else(|| {
                CoreError::TypeError(format!("environment slot {index} is out of range"))
            }),
            _ => Err(CoreError::TypeError(
                "captured binding requires an environment".into(),
            )),
        }
    }

    pub(super) fn env_get(&mut self, index: u32) -> Result<Option<EngineOutcome>, CoreError> {
        let env = self.pop()?;
        let env_id = self.expect_ref(env, "EnvGet requires an environment")?;
        let cell_id = self.env_cell(env_id, index)?;
        let value = match self.continuation.heap.get(cell_id as usize) {
            Some(HeapCell::Cell(value)) => value.clone(),
            _ => {
                return Err(CoreError::TypeError(
                    "captured binding is not a cell".into(),
                ))
            }
        };
        self.continuation.stack.push(value);
        self.advance_pc().map(|()| None)
    }

    pub(super) fn env_set(&mut self, index: u32) -> Result<Option<EngineOutcome>, CoreError> {
        let value = self.pop()?;
        let env = self.pop()?;
        let env_id = self.expect_ref(env, "EnvSet requires an environment")?;
        let cell_id = self.env_cell(env_id, index)?;
        match self.continuation.heap.get_mut(cell_id as usize) {
            Some(HeapCell::Cell(slot)) => *slot = value,
            _ => {
                return Err(CoreError::TypeError(
                    "captured binding is not a cell".into(),
                ))
            }
        }
        self.advance_pc().map(|()| None)
    }

    pub(super) fn env_slot(&mut self, index: u32) -> Result<Option<EngineOutcome>, CoreError> {
        let env = self.pop()?;
        let env_id = self.expect_ref(env, "EnvSlot requires an environment")?;
        let cell_id = self.env_cell(env_id, index)?;
        self.continuation.stack.push(Value::Ref(cell_id));
        self.advance_pc().map(|()| None)
    }

    pub(super) fn load_func(&mut self, func: FuncId) -> Result<Option<EngineOutcome>, CoreError> {
        if self.artifact.function(func).is_none() {
            return Err(CoreError::TypeError(format!("missing function {}", func.0)));
        }
        let index = func.0 as usize;
        if self.continuation.func_refs.len() <= index {
            self.continuation.func_refs.resize(index + 1, None);
        }
        if let Some(id) = self.continuation.func_refs[index] {
            self.continuation.stack.push(Value::Ref(id));
            return self.advance_pc().map(|()| None);
        }
        let env = self.alloc_cell(HeapCell::Env(Vec::new()));
        let id = self.alloc_cell(HeapCell::Closure { func: func.0, env });
        self.continuation.func_refs[index] = Some(id);
        self.continuation.stack.push(Value::Ref(id));
        self.advance_pc().map(|()| None)
    }

    pub(super) fn call_closure(&mut self, argc: u32) -> Result<Option<EngineOutcome>, CoreError> {
        let mut args = Vec::with_capacity(argc as usize);
        for _ in 0..argc {
            args.push(self.pop()?);
        }
        args.reverse();
        let callee = self.pop()?;
        let closure_id = self.expect_ref(callee, "callback is not a function")?;
        let (func, env) = match self.continuation.heap.get(closure_id as usize) {
            Some(HeapCell::Closure { func, env }) => (*func, *env),
            _ => return Err(CoreError::TypeError("callback is not a function".into())),
        };
        let (param_count, local_count, defaults, func_id) = {
            let function = self
                .artifact
                .function(FuncId(func))
                .ok_or_else(|| CoreError::TypeError(format!("missing function {func}")))?;
            (
                function.param_count,
                function.local_count,
                function.param_defaults.clone(),
                function.id.0,
            )
        };
        if (local_count as usize) < 1 + param_count as usize {
            return Err(CoreError::TypeError(
                "closure locals do not cover its environment and parameters".into(),
            ));
        }
        let mut locals = vec![Value::Undefined; local_count as usize];
        locals[0] = Value::Ref(env);
        bind_program_args(
            &self.continuation.language_semantics_version,
            param_count,
            &defaults,
            &args,
            &mut locals[1..],
        )?;
        self.advance_pc()?;
        self.continuation.frames.push(tcc_state::Frame {
            func_id,
            pc: 0,
            locals,
        });
        Ok(None)
    }

    pub(super) fn call(
        &mut self,
        func: FuncId,
        argc: u32,
    ) -> Result<Option<EngineOutcome>, CoreError> {
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
}
