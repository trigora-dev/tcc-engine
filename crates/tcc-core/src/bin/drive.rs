// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

//! Drive one artifact JSON blob the way the ordinary-corpus scorer does.
//! Stdin is `{ "artifact", "args", "effects", "events", "child" }`. Stdout is the tagged result.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::process::ExitCode;

use tcc_core::{
    decode_args_array, decode_response, stamp_host_protocol_version, Engine, EngineOutcome,
    HostRequest,
};
use tcc_ir::Artifact;
use tcc_ir::{decode_artifact, EngineCaps};
use tcc_state::json::Json;
use tcc_state::{decode_continuation, encode_continuation, Value};

fn main() -> ExitCode {
    match run() {
        Ok(result) => {
            let _ = io::stdout().write_all(result.as_bytes());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<String, String> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|err| err.to_string())?;
    let message = Json::parse(&input).map_err(|err| err.to_string())?;
    let map = message.as_object().map_err(|err| err.to_string())?;
    let artifact_json = json_str(map, "artifact")?;
    let artifact = decode_artifact(&artifact_json).map_err(|err| err.to_string())?;
    let args_json = map
        .get("args")
        .cloned()
        .unwrap_or(Json::Array(Vec::new()))
        .stringify();
    let args = decode_args_array(&args_json).map_err(|err| err.to_string())?;
    let effects = tagged_map(map.get("effects"))?;
    let events = tagged_map(map.get("events"))?;
    let child = tagged_value(map.get("child"))?;
    let mut engine =
        Engine::start_with_args(artifact.clone(), "ordinary", &EngineCaps::current(), &args)
            .map_err(|err| err.to_string())?;
    let result = drive(&mut engine, &effects, &events, child.as_ref())?;
    if !events.is_empty() || !effects.is_empty() || child.is_some() {
        let mut again = Engine::start_with_args(
            artifact.clone(),
            "ordinary-resume",
            &EngineCaps::current(),
            &args,
        )
        .map_err(|err| err.to_string())?;
        resume_once(&mut again, &artifact, &effects, &events, child.as_ref())?;
    }
    Ok(result.stringify())
}

fn drive(
    engine: &mut Engine,
    effects: &BTreeMap<String, Value>,
    events: &BTreeMap<String, Value>,
    child: Option<&Value>,
) -> Result<Json, String> {
    for _ in 0..10_000 {
        match engine.run_until_host(100_000) {
            EngineOutcome::Completed { result } => {
                return Json::parse(&encode_value_json(&result)?).map_err(|err| err.to_string());
            }
            EngineOutcome::Failed { message } => return Err(message),
            EngineOutcome::Suspended => apply(engine, &wake(events, child)?)?,
            EngineOutcome::Host(request) => apply_request(engine, &request, effects)?,
            other => return Err(format!("unexpected outcome {other:?}")),
        }
    }
    Err("drive did not finish".to_string())
}

fn resume_once(
    engine: &mut Engine,
    artifact: &Artifact,
    effects: &BTreeMap<String, Value>,
    events: &BTreeMap<String, Value>,
    child: Option<&Value>,
) -> Result<(), String> {
    for _ in 0..10_000 {
        match engine.run_until_host(100_000) {
            EngineOutcome::Suspended => {
                require_single_frame(engine)?;
                let saved = continuation_json(engine)?;
                return resume_from(artifact, &saved, effects, events, child, true);
            }
            EngineOutcome::Completed { .. } => return Ok(()),
            EngineOutcome::Failed { message } => return Err(message),
            EngineOutcome::Host(HostRequest::PersistEffect { .. })
                if events.is_empty() && child.is_none() =>
            {
                apply(engine, &Json::Object(ack()))?;
                require_single_frame(engine)?;
                let saved = continuation_json(engine)?;
                return resume_from(artifact, &saved, effects, events, child, false);
            }
            EngineOutcome::Host(request) => apply_request(engine, &request, effects)?,
            other => return Err(format!("resume setup {other:?}")),
        }
    }
    Err("resume did not reach a durable boundary".to_string())
}

fn resume_from(
    artifact: &Artifact,
    saved: &str,
    effects: &BTreeMap<String, Value>,
    events: &BTreeMap<String, Value>,
    child: Option<&Value>,
    deliver_event: bool,
) -> Result<(), String> {
    let continuation = decode_continuation(saved.as_bytes()).map_err(|err| err.to_string())?;
    let mut resumed = Engine::resume(artifact.clone(), continuation, &EngineCaps::current())
        .map_err(|err| err.to_string())?;
    if deliver_event {
        apply(&mut resumed, &wake(events, child)?)?;
    }
    drive(&mut resumed, effects, events, child).map(|_| ())
}

fn require_single_frame(engine: &Engine) -> Result<(), String> {
    if engine.continuation().frames.len() == 1 {
        Ok(())
    } else {
        Err("resumed continuation has a helper frame".to_string())
    }
}

fn continuation_json(engine: &Engine) -> Result<String, String> {
    let bytes = encode_continuation(engine.continuation()).map_err(|err| err.to_string())?;
    String::from_utf8(bytes).map_err(|err| err.to_string())
}

fn apply_request(
    engine: &mut Engine,
    request: &HostRequest,
    effects: &BTreeMap<String, Value>,
) -> Result<(), String> {
    let response = match request {
        HostRequest::RunEffect { key, .. } => {
            let value = effects
                .get(key)
                .cloned()
                .ok_or_else(|| format!("missing effect {key}"))?;
            let mut map = BTreeMap::new();
            map.insert("type".into(), Json::String("effect_result".into()));
            map.insert(
                "value".into(),
                Json::parse(&encode_value_json(&value)?).map_err(|err| err.to_string())?,
            );
            Json::Object(map)
        }
        HostRequest::PersistCheckpoint { revision, .. } => {
            let mut map = BTreeMap::new();
            map.insert("type".into(), Json::String("persist_confirmed".into()));
            map.insert("revision".into(), Json::Number(*revision as f64));
            Json::Object(map)
        }
        HostRequest::PersistEffect { .. }
        | HostRequest::RegisterWait { .. }
        | HostRequest::RegisterTimer { .. }
        | HostRequest::CreateChild { .. } => Json::Object(ack()),
        other => return Err(format!("unexpected host request {other:?}")),
    };
    apply(engine, &response)
}

fn apply(engine: &mut Engine, response: &Json) -> Result<(), String> {
    let stamped =
        stamp_host_protocol_version(&response.stringify()).map_err(|err| err.to_string())?;
    let decoded = decode_response(&stamped).map_err(|err| err.to_string())?;
    engine
        .apply_host_response(decoded)
        .map_err(|err| err.to_string())
}

fn ack() -> BTreeMap<String, Json> {
    let mut map = BTreeMap::new();
    map.insert("type".into(), Json::String("ack".into()));
    map
}

fn wake(events: &BTreeMap<String, Value>, child: Option<&Value>) -> Result<Json, String> {
    if let Some(value) = child {
        return tagged_response("child_result", value);
    }
    let value = events
        .values()
        .next()
        .ok_or_else(|| "suspended without an event".to_string())?;
    tagged_response("event_payload", value)
}

fn tagged_response(kind: &str, value: &Value) -> Result<Json, String> {
    let mut map = BTreeMap::new();
    map.insert("type".into(), Json::String(kind.into()));
    map.insert(
        "value".into(),
        Json::parse(&encode_value_json(value)?).map_err(|err| err.to_string())?,
    );
    Ok(Json::Object(map))
}

fn encode_value_json(value: &Value) -> Result<String, String> {
    let bytes = tcc_state::encode_value(value).map_err(|err| err.to_string())?;
    String::from_utf8(bytes).map_err(|err| err.to_string())
}

fn json_str(map: &BTreeMap<String, Json>, key: &str) -> Result<String, String> {
    Json::get(map, key)
        .and_then(|value| value.as_str().map(|text| text.to_string()))
        .map_err(|err| err.to_string())
}

fn tagged_value(value: Option<&Json>) -> Result<Option<Value>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if matches!(value, Json::Null) {
        return Ok(None);
    }
    let bytes = value.stringify();
    tcc_state::decode_value(bytes.as_bytes())
        .map(Some)
        .map_err(|err| err.to_string())
}

fn tagged_map(value: Option<&Json>) -> Result<BTreeMap<String, Value>, String> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    if matches!(value, Json::Null) {
        return Ok(BTreeMap::new());
    }
    let object = value.as_object().map_err(|err| err.to_string())?;
    let mut out = BTreeMap::new();
    for (key, item) in object {
        let bytes = item.stringify();
        let decoded = tcc_state::decode_value(bytes.as_bytes()).map_err(|err| err.to_string())?;
        out.insert(key.clone(), decoded);
    }
    Ok(out)
}
