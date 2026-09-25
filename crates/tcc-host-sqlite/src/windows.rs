use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tcc_core::{ChildSpec, EffectRecord, EffectStatus, HostRequest, HostResponse};
use tcc_host::Host;
use tcc_state::{Continuation, ContinuationStatus, PersistKind, Value};

use crate::host::SqliteHost;
use crate::store::{CompletedChild, NewChild, Snapshot, Store};

fn db_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "tcc-host-{label}-{}-{nanos}.db",
        std::process::id()
    ))
}

fn snapshot(execution_id: &str, revision: u64, body: &str) -> Snapshot {
    Snapshot {
        execution_id: execution_id.to_string(),
        revision,
        status: "suspended".to_string(),
        body: body.to_string(),
        child: None,
        completed_child: None,
    }
}

fn reopen(path: &Path) -> Store {
    Store::open(path).expect("reopen")
}

#[test]
fn checkpoint_commit_survives_reopen_and_uncommitted_write_does_not() {
    let path = db_path("checkpoint");
    {
        let mut store = Store::open(&path).unwrap();
        store.create_execution("ex", "hash").unwrap();
        store
            .rollback_snapshot(&snapshot("ex", 1, r#"{"revision":1}"#))
            .unwrap();
        assert!(store.checkpoint("ex").unwrap().is_none());
    }
    assert!(reopen(&path).checkpoint("ex").unwrap().is_none());
    {
        let mut store = reopen(&path);
        store
            .commit_snapshot(&snapshot(
                "ex",
                2,
                r#"{"revision":2,"artifact_hash":"hash"}"#,
            ))
            .unwrap();
    }
    let saved = reopen(&path).checkpoint("ex").unwrap().expect("committed");
    assert_eq!(saved.revision, 2);
    assert!(saved.body.contains("artifact_hash"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn crash_before_checkpoint_rolls_back_and_after_commit_is_visible() {
    let path = db_path("crash-checkpoint");
    {
        let mut host = SqliteHost::open(&path, "ex").unwrap();
        host.store.create_execution("ex", "hash").unwrap();
        stage(&mut host, ContinuationStatus::Suspended);
        host.crash.want = Some("before_persist_checkpoint".to_string());
        let err = host.handle(checkpoint_request(1)).unwrap_err();
        assert!(err.to_string().contains("before_persist_checkpoint"));
        assert!(host.store.checkpoint("ex").unwrap().is_none());
    }
    assert!(reopen(&path).checkpoint("ex").unwrap().is_none());
    {
        let mut host = SqliteHost::open(&path, "ex").unwrap();
        stage(&mut host, ContinuationStatus::Suspended);
        host.crash.want = Some("after_persist_checkpoint".to_string());
        let err = host.handle(checkpoint_request(4)).unwrap_err();
        assert!(err.to_string().contains("after_persist_checkpoint"));
        let saved = host.store.checkpoint("ex").unwrap().expect("committed");
        assert_eq!(saved.revision, 4);
        assert!(saved.body.contains("\"revision\":4"));
    }
    let saved = reopen(&path).checkpoint("ex").unwrap().expect("reopen");
    assert_eq!(saved.revision, 4);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn effect_journal_hit_mismatch_and_commit_windows() {
    let path = db_path("effect");
    {
        let mut store = Store::open(&path).unwrap();
        store
            .mark_effect_started("ex", "generate", "ex:generate", r#"{"t":"object","v":{}}"#)
            .unwrap();
        store
            .rollback_effect(
                "ex",
                "generate",
                "ex:generate",
                r#"{"t":"object","v":{}}"#,
                r#"{"t":"number","v":42}"#,
            )
            .unwrap();
        let row = store.effect("ex", "generate").unwrap().unwrap();
        assert_eq!(row.status, "started");
        assert!(row.result.is_none());
    }
    {
        let mut store = reopen(&path);
        assert_eq!(
            store.effect("ex", "generate").unwrap().unwrap().status,
            "started"
        );
        store
            .complete_effect(
                "ex",
                "generate",
                "ex:generate",
                r#"{"t":"object","v":{}}"#,
                r#"{"t":"number","v":42}"#,
            )
            .unwrap();
    }
    assert_eq!(
        reopen(&path)
            .effect("ex", "generate")
            .unwrap()
            .unwrap()
            .status,
        "completed"
    );

    let path = db_path("effect-host");
    let mut host = SqliteHost::open(&path, "ex").unwrap();
    host.effects
        .insert("generate".to_string(), Value::Number(42.0));
    let first = host.handle(effect_request(object_input(1.0))).unwrap();
    assert!(matches!(first, HostResponse::EffectResult { .. }));
    host.crash.want = Some("before_persist_effect".to_string());
    let err = host
        .handle(effect_record(EffectStatus::Completed))
        .unwrap_err();
    assert!(err.to_string().contains("before_persist_effect"));
    assert_eq!(
        host.store.effect("ex", "generate").unwrap().unwrap().status,
        "started"
    );
    host.crash.want = None;
    assert!(matches!(
        host.handle(effect_record(EffectStatus::Completed)).unwrap(),
        HostResponse::Ack
    ));
    host.effects.remove("generate");
    let hit = host.handle(effect_request(object_input(1.0))).unwrap();
    match hit {
        HostResponse::EffectResult { value } => assert_eq!(value, Value::Number(42.0)),
        other => panic!("expected journal hit, got {other:?}"),
    }
    let mismatch = host.handle(effect_request(object_input(2.0))).unwrap_err();
    assert!(mismatch.to_string().contains("input mismatch"));
    let blank = host
        .store
        .mark_effect_started("ex", "other", "ex:other", "");
    assert!(blank.unwrap_err().to_string().contains("input_json"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn child_row_commits_with_the_checkpoint_and_not_on_ack() {
    let path = db_path("child");
    let mut host = SqliteHost::open(&path, "parent").unwrap();
    host.store
        .create_execution("parent", "parent-hash")
        .unwrap();
    host.child_artifacts
        .insert("child".to_string(), child_artifact());
    host.crash.want = Some("after_create_child".to_string());
    let err = host.handle(create_child()).unwrap_err();
    assert!(err.to_string().contains("after_create_child"));
    assert!(host.store.child("invoke-1").unwrap().is_none());
    host.crash.want = None;
    assert!(matches!(
        host.handle(create_child()).unwrap(),
        HostResponse::Ack
    ));
    assert!(host.store.child("invoke-1").unwrap().is_none());
    stage(&mut host, ContinuationStatus::Suspended);
    assert!(matches!(
        host.handle(checkpoint_request(1)).unwrap(),
        HostResponse::PersistConfirmed { revision: 1 }
    ));
    let row = host.store.child("invoke-1").unwrap().unwrap();
    assert_eq!(row.status, "ready");
    assert!(host
        .store
        .execution(&row.child_execution_id)
        .unwrap()
        .is_none());
    drop(host);
    let store = reopen(&path);
    assert_eq!(store.child("invoke-1").unwrap().unwrap().status, "ready");
    assert!(store.ready_children("parent").unwrap().len() == 1);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn child_insert_before_commit_is_invisible() {
    let path = db_path("child-rollback");
    {
        let mut store = Store::open(&path).unwrap();
        store.create_execution("parent", "hash").unwrap();
        let mut snap = snapshot("parent", 1, "{}");
        snap.child = Some(sample_child());
        store.rollback_snapshot(&snap).unwrap();
        assert!(store.child("invoke-1").unwrap().is_none());
        store.commit_snapshot(&snap).unwrap();
    }
    assert_eq!(
        reopen(&path).child("invoke-1").unwrap().unwrap().status,
        "ready"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn timer_due_query_and_resolution_windows() {
    let path = db_path("timer");
    {
        let mut store = Store::open(&path).unwrap();
        store.upsert_timer("ex", "", 10).unwrap();
        assert!(store.due_timers(9).unwrap().is_empty());
        assert_eq!(store.due_timers(10).unwrap().len(), 1);
        store.rollback_timer("ex", "").unwrap();
        assert_eq!(store.timer("ex", "").unwrap().unwrap().status, "pending");
    }
    {
        let mut store = reopen(&path);
        assert_eq!(store.timer("ex", "").unwrap().unwrap().status, "pending");
        store.resolve_timer("ex", "").unwrap();
    }
    let row = reopen(&path).timer("ex", "").unwrap().unwrap();
    assert_eq!(row.status, "resolved");
    assert!(reopen(&path).due_timers(10).unwrap().is_empty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn event_delivery_persists_payload_only_on_commit() {
    let path = db_path("event");
    {
        let mut store = Store::open(&path).unwrap();
        store.upsert_wait("wait-1", "ex", "approval").unwrap();
        store
            .rollback_wait("wait-1", r#"{"t":"string","v":"ok"}"#)
            .unwrap();
        let row = store.wait("wait-1").unwrap().unwrap();
        assert_eq!(row.status, "pending");
        assert!(row.payload.is_none());
        store
            .resolve_wait("wait-1", r#"{"t":"string","v":"ok"}"#)
            .unwrap();
    }
    let row = reopen(&path).wait("wait-1").unwrap().unwrap();
    assert_eq!(row.status, "resolved");
    assert_eq!(row.payload.as_deref(), Some(r#"{"t":"string","v":"ok"}"#));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn completed_child_result_commits_with_the_child_snapshot() {
    let path = db_path("child-result");
    {
        let mut store = Store::open(&path).unwrap();
        store.create_execution("parent", "hash").unwrap();
        store
            .create_execution("child:invoke-1", "child-hash")
            .unwrap();
        let mut created = snapshot("parent", 1, "{}");
        created.child = Some(sample_child());
        store.commit_snapshot(&created).unwrap();
        let mut done = snapshot("child:invoke-1", 2, r#"{"status":"completed"}"#);
        done.status = "completed".to_string();
        done.completed_child = Some(CompletedChild {
            child_execution_id: "child:invoke-1".to_string(),
            result_json: r#"{"t":"number","v":7}"#.to_string(),
        });
        store.rollback_snapshot(&done).unwrap();
        assert_eq!(store.child("invoke-1").unwrap().unwrap().status, "ready");
        store.commit_snapshot(&done).unwrap();
    }
    let row = reopen(&path).child("invoke-1").unwrap().unwrap();
    assert_eq!(row.status, "completed");
    assert_eq!(row.result_json.as_deref(), Some(r#"{"t":"number","v":7}"#));
    let _ = std::fs::remove_file(&path);
}

fn stage(host: &mut SqliteHost, status: ContinuationStatus) {
    let mut continuation = Continuation::start("ex", "hash", 1, "ts", 0, 0);
    continuation.execution_id = host.execution_id.clone();
    continuation.status = status;
    host.stage_continuation(&continuation).unwrap();
}

fn checkpoint_request(revision: u64) -> HostRequest {
    HostRequest::PersistCheckpoint {
        revision,
        kind: PersistKind::Delta,
        base_revision: 0,
        materialize: false,
        delta: None,
    }
}

fn object_input(n: f64) -> Value {
    let mut fields = BTreeMap::new();
    fields.insert("n".to_string(), Value::Number(n));
    Value::Object(fields)
}

fn effect_request(input: Value) -> HostRequest {
    HostRequest::RunEffect {
        key: "generate".to_string(),
        idempotency_key: "ex:generate".to_string(),
        input,
    }
}

fn effect_record(status: EffectStatus) -> HostRequest {
    HostRequest::PersistEffect {
        record: EffectRecord {
            key: "generate".to_string(),
            idempotency_key: "ex:generate".to_string(),
            status,
            result: Some(Value::Number(42.0)),
        },
    }
}

fn create_child() -> HostRequest {
    HostRequest::CreateChild {
        child: ChildSpec {
            invoke_id: "invoke-1".to_string(),
            child_execution_id: "child:invoke-1".to_string(),
            program_name: "child".to_string(),
            args: Vec::new(),
        },
    }
}

fn sample_child() -> NewChild {
    NewChild {
        invoke_id: "invoke-1".to_string(),
        parent_execution_id: "parent".to_string(),
        child_execution_id: "child:invoke-1".to_string(),
        program_name: "child".to_string(),
        artifact_hash: "child-hash".to_string(),
        artifact_body: "{}".to_string(),
        args_json: None,
    }
}

fn child_artifact() -> String {
    r#"{"envelope":{"artifact_hash":"child-hash","engine_format_version":1,"language_semantics_version":"ts"},"program":{"entry":0,"functions":[]}}"#.to_string()
}
