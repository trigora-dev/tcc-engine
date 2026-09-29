// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use super::host::response_branch;
use super::*;

impl Engine {
    pub(super) fn begin_join(
        &mut self,
        count: u32,
        join_pc: Pc,
    ) -> Result<Option<EngineOutcome>, CoreError> {
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

    pub(super) fn finish_join(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
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

    pub(super) fn finish_any(&mut self) -> Result<Option<EngineOutcome>, CoreError> {
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

    pub(super) fn race_is_active(&self) -> bool {
        self.continuation
            .join
            .as_ref()
            .is_some_and(|join| join.kind == JoinKind::Any && join.state == JoinStatus::Active)
    }

    pub(super) fn after_race_branch_progress(&mut self) {
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

    pub(super) fn publish_race_success(&mut self) -> Result<(), CoreError> {
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

    pub(super) fn record_race_branch_failure(&mut self, message: String) {
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

    pub(super) fn join_is_active(&self) -> bool {
        self.continuation
            .join
            .as_ref()
            .is_some_and(|join| join.state == JoinStatus::Active)
    }

    pub(super) fn next_planned_branch_id(&self) -> Result<String, CoreError> {
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

    pub(super) fn bind_planned_branch(&mut self, op: BranchOp) -> Result<String, CoreError> {
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

    pub(super) fn note_branch_registered(&mut self) {
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

    pub(super) fn complete_planned_branch(&mut self, value: Value) -> Result<(), CoreError> {
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

    pub(super) fn fail_active_branch(&mut self, message: String) {
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

    pub(super) fn throw_join_failure(&mut self) -> Result<(), CoreError> {
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

    pub(super) fn apply_join_wake(&mut self, response: HostResponse) -> Result<(), CoreError> {
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
