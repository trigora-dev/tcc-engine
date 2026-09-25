use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::{
    Artifact, ConstValue, EngineCaps, Envelope, FuncId, Function, Instruction, Program,
    ENGINE_FORMAT_VERSION,
};
use tcc_state::Value;

fn function(
    id: u32,
    name: &str,
    param_count: u32,
    local_count: u32,
    instructions: Vec<Instruction>,
) -> Function {
    Function {
        id: FuncId(id),
        name: name.into(),
        param_count,
        local_count,
        param_defaults: Vec::new(),
        instructions,
        spans: vec![None; 0],
    }
}

fn artifact(entry: u32, functions: Vec<Function>) -> Artifact {
    let mut functions = functions;
    for function in &mut functions {
        function.spans = vec![None; function.instructions.len()];
    }
    Artifact {
        envelope: Envelope {
            artifact_hash: "entry-identity".into(),
            frontend_id: "typescript".into(),
            frontend_version: "0.0.0".into(),
            language_semantics_version: "ts.subset.v1".into(),
            engine_format_version: ENGINE_FORMAT_VERSION,
            required_engine_features: tcc_ir::FeatureSet::current_engine(),
            required_host_capabilities: tcc_ir::HostCapability::known()
                .iter()
                .map(|id| tcc_ir::HostCapability((*id).to_string()))
                .collect(),
            runtime_modules: Vec::new(),
        },
        program: Program {
            entry: FuncId(entry),
            functions,
        },
    }
}

#[test]
fn nonzero_entry_binds_args_calls_the_helper_and_may_yield() {
    let helper = function(
        0,
        "helper",
        0,
        0,
        vec![
            Instruction::LoadConst {
                value: ConstValue::Number(1.0),
            },
            Instruction::Return,
        ],
    );
    let entry = function(
        1,
        "entry",
        1,
        1,
        vec![
            Instruction::Call {
                func: FuncId(0),
                argc: 0,
            },
            Instruction::Pop,
            Instruction::LoadConst {
                value: ConstValue::String("generate".into()),
            },
            Instruction::Effect { has_input: false },
            Instruction::Return,
        ],
    );
    let mut engine = Engine::start_with_args(
        artifact(1, vec![helper, entry]),
        "entry-identity",
        &EngineCaps::current(),
        &[Value::Number(7.0)],
    )
    .unwrap();
    assert_eq!(engine.continuation().frames[0].func_id, 1);
    assert_eq!(
        engine.continuation().frames[0].locals[0],
        Value::Number(7.0)
    );
    loop {
        match engine.run_until_host(32) {
            EngineOutcome::Host(HostRequest::PersistCheckpoint { revision, .. }) => {
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::RunEffect { key, .. }) => {
                assert_eq!(key, "generate");
                break;
            }
            other => panic!("expected the entry effect, got {other:?}"),
        }
    }
}

#[test]
fn helper_durable_op_is_rejected() {
    let helper = function(
        0,
        "helper",
        0,
        0,
        vec![
            Instruction::LoadConst {
                value: ConstValue::String("nope".into()),
            },
            Instruction::Effect { has_input: false },
            Instruction::Return,
        ],
    );
    let entry = function(
        1,
        "entry",
        0,
        0,
        vec![
            Instruction::Call {
                func: FuncId(0),
                argc: 0,
            },
            Instruction::Return,
        ],
    );
    let err = Engine::start(
        artifact(1, vec![helper, entry]),
        "bad-entry",
        &EngineCaps::current(),
    )
    .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("durable operations are only allowed in the program entry"),
        "{message}"
    );
}
