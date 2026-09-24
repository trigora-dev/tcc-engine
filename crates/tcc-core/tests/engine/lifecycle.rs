use tcc_core::{CoreError, Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{Envelope, FuncId, Function, Instruction, Program};
use tcc_state::{ContinuationStatus, Value};

use super::caps;

#[test]
fn returns_undefined_from_minimal_program() {
    let artifact = tcc_ir::Artifact::minimal_return("hash-a");
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
fn budget_zero_does_not_run() {
    let artifact = tcc_ir::Artifact {
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
fn start_rejects_oob_local_without_unknown_instruction() {
    let mut artifact = tcc_ir::Artifact::minimal_return("hash-oob");
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
