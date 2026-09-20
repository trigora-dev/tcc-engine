use tcc_core::{Engine, EngineOutcome, HostRequest, HostResponse};
use tcc_ir::gen::{generate, seed_count};
use tcc_ir::{validate, EngineCaps};
use tcc_state::{apply_continuation_delta, Continuation, Value};

fn drive(engine: &mut Engine) -> Value {
    let mut last_confirmed: Option<Continuation> = None;
    loop {
        match engine.run_until_host(256) {
            EngineOutcome::Host(HostRequest::RunEffect { key, .. }) => {
                let value = match key.as_str() {
                    "skipped" => Value::Number(99.0),
                    "taken" => Value::Number(42.0),
                    _ => Value::Number(42.0),
                };
                engine
                    .apply_host_response(HostResponse::EffectResult { value })
                    .unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistEffect { .. })
            | EngineOutcome::Host(HostRequest::RegisterWait { .. })
            | EngineOutcome::Host(HostRequest::RegisterTimer { .. })
            | EngineOutcome::Host(HostRequest::CreateChild { .. }) => {
                engine.apply_host_response(HostResponse::Ack).unwrap();
            }
            EngineOutcome::Host(HostRequest::PersistCheckpoint {
                revision, delta, ..
            }) => {
                if let (Some(base), Some(delta)) = (last_confirmed.as_ref(), delta.as_ref()) {
                    let mut reconstructed = apply_continuation_delta(base.clone(), delta).unwrap();
                    reconstructed.revision = engine.continuation().revision;
                    assert_eq!(
                        reconstructed,
                        *engine.continuation(),
                        "snapshot reconstruct != delta reconstruct at revision {revision}"
                    );
                }
                engine
                    .apply_host_response(HostResponse::PersistConfirmed { revision })
                    .unwrap();
                last_confirmed = Some(engine.continuation().clone());
            }
            EngineOutcome::Suspended => match &engine.continuation().pending {
                Some(tcc_state::PendingOp::Wait {
                    kind: tcc_state::WaitKind::Timer { .. },
                }) => engine
                    .apply_host_response(HostResponse::TimerFired)
                    .unwrap(),
                Some(tcc_state::PendingOp::Wait {
                    kind: tcc_state::WaitKind::Child { .. },
                }) => engine
                    .apply_host_response(HostResponse::ChildResult {
                        value: Value::Number(7.0),
                    })
                    .unwrap(),
                _ => engine
                    .apply_host_response(HostResponse::EventPayload {
                        value: Value::String("ok".into()),
                    })
                    .unwrap(),
            },
            EngineOutcome::Completed { result } => return result,
            EngineOutcome::Cancelled => panic!("cancelled"),
            EngineOutcome::Failed { message } => panic!("failed: {message}"),
            EngineOutcome::BudgetExhausted => panic!("budget"),
            EngineOutcome::Host(other) => panic!("unexpected host request {other:?}"),
        }
    }
}

#[test]
fn generated_programs_complete_and_round_trip_deltas() {
    for seed in 0..seed_count() {
        let artifact = generate(seed);
        validate(&artifact, &EngineCaps::current()).unwrap();
        let mut engine = Engine::start(artifact, format!("gen-{seed}"), &EngineCaps::current())
            .unwrap_or_else(|err| panic!("start seed {seed}: {err}"));
        let _ = drive(&mut engine);
    }
}

#[test]
fn generated_native_matches_encoded_round_trip() {
    for seed in 0..seed_count() {
        let artifact = generate(seed);
        let json = tcc_ir::encode_artifact(&artifact).unwrap();
        let decoded = tcc_ir::decode_artifact(&json).unwrap();
        let mut left =
            Engine::start(artifact, format!("a-{seed}"), &EngineCaps::current()).unwrap();
        let mut right =
            Engine::start(decoded, format!("a-{seed}"), &EngineCaps::current()).unwrap();
        let left_result = drive(&mut left);
        let right_result = drive(&mut right);
        assert_eq!(left_result, right_result, "seed {seed}");
        assert_eq!(
            left.continuation().frames,
            right.continuation().frames,
            "seed {seed}"
        );
    }
}
