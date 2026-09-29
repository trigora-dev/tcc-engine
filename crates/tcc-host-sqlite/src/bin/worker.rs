// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use serde::Deserialize;
use tcc_host_sqlite::{read_continuation, resume_execution, start_execution, Crash, SqliteHost};
use tcc_state::Value;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let mode = env::var("TCC_MODE").unwrap_or_else(|_| "start".to_string());
    let db = env::var("TCC_DB_PATH").map_err(|_| "missing TCC_DB_PATH".to_string())?;
    let execution_id = env::var("TCC_EXECUTION_ID").unwrap_or_else(|_| "first".to_string());
    if mode == "read" {
        let body = read_continuation(PathBuf::from(db).as_path(), &execution_id)
            .map_err(|err| err.to_string())?;
        println!("{body}");
        return Ok(());
    }
    let config = read_config()?;
    let mut host = SqliteHost::open(PathBuf::from(&db).as_path(), &execution_id)
        .map_err(|err| err.to_string())?;
    host.crash = Crash::from_env();
    host.auto_deliver = auto_deliver(&config);
    host.event_payload = event_payload(&config)?;
    host.completion_reverse = config.completion_order.as_deref() == Some("reverse");
    host.cancel =
        config.cancel.unwrap_or(false) || env::var("TCC_CANCEL").ok().as_deref() == Some("1");
    host.effects = effects_from(&config)?;
    host.fail_counts = config.fail_counts.unwrap_or_default();
    host.effect_log = env::var("TCC_EFFECT_LOG_PATH").ok().map(PathBuf::from);
    host.child_artifacts = config.child_artifacts.unwrap_or_default();
    let artifact = env::var("TCC_ARTIFACT_JSON").ok();
    let result = if mode == "resume" {
        resume_execution(&mut host, artifact.as_deref(), &execution_id)
    } else {
        let artifact = artifact.ok_or_else(|| "missing TCC_ARTIFACT_JSON".to_string())?;
        start_execution(&mut host, &artifact, &execution_id)
    }
    .map_err(|err| err.to_string())?;
    let text = serde_json::to_string(&result_json(&result).map_err(|err| err.to_string())?)
        .map_err(|err| err.to_string())?;
    println!("{text}");
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
struct WorkerConfig {
    #[serde(default)]
    effects: Option<HashMap<String, serde_json::Value>>,
    #[serde(default, rename = "failCounts")]
    fail_counts: Option<HashMap<String, u32>>,
    #[serde(default, rename = "eventPayload")]
    event_payload: Option<serde_json::Value>,
    #[serde(default, rename = "autoDeliverEvent")]
    auto_deliver_event: Option<bool>,
    #[serde(default, rename = "completionOrder")]
    completion_order: Option<String>,
    #[serde(default, rename = "childArtifacts")]
    child_artifacts: Option<HashMap<String, String>>,
    #[serde(default)]
    cancel: Option<bool>,
}

fn read_config() -> Result<WorkerConfig, String> {
    let Ok(path) = env::var("TCC_CONFIG_PATH") else {
        return Ok(WorkerConfig::default());
    };
    let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str(&text).map_err(|err| err.to_string())
}

fn auto_deliver(config: &WorkerConfig) -> bool {
    if let Ok(flag) = env::var("TCC_AUTO_EVENT") {
        return flag != "0";
    }
    config.auto_deliver_event.unwrap_or(true)
}

fn event_payload(config: &WorkerConfig) -> Result<Option<Value>, String> {
    if let Ok(text) = env::var("TCC_EVENT_PAYLOAD") {
        let parsed = serde_json::from_str(&text).map_err(|err| err.to_string())?;
        return Ok(Some(plain_to_value(&parsed)));
    }
    Ok(config.event_payload.as_ref().map(plain_to_value))
}

fn effects_from(config: &WorkerConfig) -> Result<HashMap<String, Value>, String> {
    let parsed = if let Ok(text) = env::var("TCC_EFFECTS") {
        serde_json::from_str(&text).map_err(|err| err.to_string())?
    } else {
        config.effects.clone().unwrap_or_else(|| {
            let mut effects = HashMap::new();
            effects.insert("generate".to_string(), serde_json::json!(42));
            effects
        })
    };
    Ok(parsed
        .into_iter()
        .map(|(key, value)| (key, plain_to_value(&value)))
        .collect())
}

fn plain_to_value(value: &serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(flag) => Value::Bool(*flag),
        serde_json::Value::Number(number) => Value::Number(number.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(text) => Value::String(text.clone()),
        serde_json::Value::Array(items) => Value::Array(items.iter().map(plain_to_value).collect()),
        serde_json::Value::Object(map) => {
            let mut fields = std::collections::BTreeMap::new();
            for (key, item) in map {
                fields.insert(key.clone(), plain_to_value(item));
            }
            Value::Object(fields)
        }
    }
}

fn result_json(result: &tcc_host_sqlite::RunResult) -> Result<serde_json::Value, String> {
    let value = if result.status == "failed" {
        serde_json::Value::String(result.error.clone().unwrap_or_default())
    } else if let Some(value) = &result.result {
        let text = tcc_state::encode_value(value)
            .map_err(|err| err.to_string())
            .and_then(|bytes| String::from_utf8(bytes).map_err(|err| err.to_string()))?;
        serde_json::from_str(&text).map_err(|err| err.to_string())?
    } else {
        serde_json::Value::Null
    };
    Ok(serde_json::json!({
        "status": result.status,
        "result": value,
        "continuationJson": result.continuation_json,
        "revision": result.revision,
    }))
}
