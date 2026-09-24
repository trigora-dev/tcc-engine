use super::*;

impl Engine {
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
                    ..
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
                    ..
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
}

pub(super) fn response_branch(response: &HostResponse) -> Option<String> {
    match response {
        HostResponse::EventPayload { branch, .. }
        | HostResponse::TimerFired { branch }
        | HostResponse::ChildResult { branch, .. } => branch.clone(),
        _ => None,
    }
}
