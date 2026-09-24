use super::*;

impl Engine {
    pub(super) fn clear_wait_and_persist(&mut self) {
        self.continuation.pending = None;
        self.continuation.status = ContinuationStatus::Runnable;
        self.queue_persist();
    }

    pub(super) fn queue_persist(&mut self) {
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
            delta: intent.delta.map(Box::new),
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

    pub(super) fn status_after_checkpoint(&self) -> ContinuationStatus {
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
}
