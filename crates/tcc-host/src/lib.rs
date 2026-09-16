//! Host protocol driver. Persistence, effects, and scheduling stay in the host.

#![allow(clippy::derive_partial_eq_without_eq)]

use std::fmt;

use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    Message(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Message(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for HostError {}

pub trait Host {
    fn handle(&mut self, request: HostRequest) -> Result<HostResponse, HostError>;
}

/// In-memory host used by core tests and local experimentation. Not a storage adapter.
#[derive(Debug, Default)]
pub struct MemoryHost {
    pub revision: u64,
}

impl Host for MemoryHost {
    fn handle(&mut self, request: HostRequest) -> Result<HostResponse, HostError> {
        match request {
            HostRequest::PersistCheckpoint { .. } => {
                self.revision += 1;
                Ok(HostResponse::PersistConfirmed {
                    revision: self.revision,
                })
            }
            HostRequest::PersistEffect { .. }
            | HostRequest::RegisterTimer { .. }
            | HostRequest::RegisterWait { .. }
            | HostRequest::CreateChild { .. } => Ok(HostResponse::Ack),
            HostRequest::RunEffect { .. } => Err(HostError::Message(
                "MemoryHost does not execute external effects".to_string(),
            )),
            HostRequest::FetchArtifact { hash } => Ok(HostResponse::Artifact { hash }),
        }
    }
}

pub struct Driver<H> {
    engine: Engine,
    host: H,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DriverOutcome {
    Completed { result: tcc_state::Value },
    Failed { message: String },
    BudgetExhausted,
    Suspended,
}

impl<H: Host> Driver<H> {
    pub fn new(engine: Engine, host: H) -> Self {
        Self { engine, host }
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub fn run(&mut self, budget: u32) -> Result<DriverOutcome, HostError> {
        loop {
            match self.engine.run_until_host(budget) {
                EngineOutcome::Completed { result } => {
                    return Ok(DriverOutcome::Completed { result });
                }
                EngineOutcome::Failed { message } => return Ok(DriverOutcome::Failed { message }),
                EngineOutcome::BudgetExhausted => return Ok(DriverOutcome::BudgetExhausted),
                EngineOutcome::Host(request) => {
                    let suspend = matches!(
                        request,
                        HostRequest::RegisterTimer { .. }
                            | HostRequest::RegisterWait { .. }
                            | HostRequest::CreateChild { .. }
                    );
                    let response = self.host.handle(request)?;
                    self.engine
                        .apply_host_response(response)
                        .map_err(|err| HostError::Message(err.to_string()))?;
                    if suspend {
                        return Ok(DriverOutcome::Suspended);
                    }
                    if self.engine.continuation().status == tcc_state::ContinuationStatus::Completed
                    {
                        return Ok(DriverOutcome::Completed {
                            result: self
                                .engine
                                .continuation()
                                .result
                                .clone()
                                .unwrap_or(tcc_state::Value::Undefined),
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tcc_core::Engine;
    use tcc_ir::{Artifact, EngineCaps};

    use super::*;

    #[test]
    fn memory_host_drives_minimal_program_to_completion() {
        let artifact = Artifact::minimal_return("hash-host");
        let engine = Engine::start(artifact, "exec-host", &EngineCaps::current()).unwrap();
        let mut driver = Driver::new(engine, MemoryHost::default());
        let outcome = driver.run(32).unwrap();
        assert_eq!(
            outcome,
            DriverOutcome::Completed {
                result: tcc_state::Value::Undefined
            }
        );
    }
}
