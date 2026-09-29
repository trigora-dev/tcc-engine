// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use tcc_state::json::Json;
use tcc_state::{continuation_delta_to_json, decode_value, encode_value, Value};

use crate::engine::EngineOutcome;
use crate::error::CoreError;
use crate::protocol::{EffectStatus, HostRequest, HostResponse, HOST_PROTOCOL_VERSION};

pub fn encode_outcome(outcome: &EngineOutcome) -> Result<String, CoreError> {
    let mut map = std::collections::BTreeMap::new();
    map.insert(
        "host_protocol_version".into(),
        Json::Number(HOST_PROTOCOL_VERSION as f64),
    );
    match outcome {
        EngineOutcome::Host(request) => {
            map.insert("type".into(), Json::String("host".into()));
            map.insert("request".into(), request_to_json(request)?);
        }
        EngineOutcome::Completed { result } => {
            map.insert("type".into(), Json::String("completed".into()));
            map.insert("result".into(), value_json(result)?);
        }
        EngineOutcome::Failed { message } => {
            map.insert("type".into(), Json::String("failed".into()));
            map.insert("message".into(), Json::String(message.clone()));
        }
        EngineOutcome::Cancelled => {
            map.insert("type".into(), Json::String("cancelled".into()));
        }
        EngineOutcome::BudgetExhausted => {
            map.insert("type".into(), Json::String("budget_exhausted".into()));
        }
        EngineOutcome::Suspended => {
            map.insert("type".into(), Json::String("suspended".into()));
        }
    }
    Ok(Json::Object(map).stringify())
}

pub fn encode_request(request: &HostRequest) -> Result<String, CoreError> {
    Ok(request_to_json(request)?.stringify())
}

pub fn decode_request(text: &str) -> Result<HostRequest, CoreError> {
    let json = Json::parse(text).map_err(core_state)?;
    let map = json.as_object().map_err(core_state)?;
    match Json::get(map, "type")
        .map_err(core_state)?
        .as_str()
        .map_err(core_state)?
    {
        "run_effect" => {
            let input = match map.get("input") {
                None => {
                    return Err(CoreError::TypeError("missing run_effect.input".into()));
                }
                Some(json) => json_value(json)?,
            };
            Ok(HostRequest::RunEffect {
                key: field_string(map, "key")?,
                idempotency_key: field_string(map, "idempotency_key")?,
                input,
            })
        }
        "create_child" => Ok(HostRequest::CreateChild {
            child: crate::protocol::ChildSpec {
                invoke_id: field_string(map, "invoke_id")?,
                child_execution_id: field_string(map, "child_execution_id")?,
                program_name: field_string(map, "program_name")?,
                args: decode_args_field(map)?,
            },
        }),
        other => Err(CoreError::TypeError(format!(
            "unsupported host request `{other}`"
        ))),
    }
}

/// A JSON array of tagged values. Empty is a valid vector.
pub fn decode_args_array(text: &str) -> Result<Vec<Value>, CoreError> {
    let json = Json::parse(text).map_err(core_state)?;
    match json {
        Json::Array(items) => items.iter().map(json_value).collect(),
        _ => Err(CoreError::TypeError(
            "program args must be an array".to_string(),
        )),
    }
}

fn field_string(
    map: &std::collections::BTreeMap<String, Json>,
    key: &str,
) -> Result<String, CoreError> {
    Ok(Json::get(map, key)
        .map_err(core_state)?
        .as_str()
        .map_err(core_state)?
        .to_string())
}

fn decode_args_field(
    map: &std::collections::BTreeMap<String, Json>,
) -> Result<Vec<Value>, CoreError> {
    match map.get("args") {
        None => Ok(Vec::new()),
        Some(Json::Array(items)) => items.iter().map(json_value).collect(),
        Some(_) => Err(CoreError::TypeError(
            "create_child args must be an array".to_string(),
        )),
    }
}

pub fn decode_response(text: &str) -> Result<HostResponse, CoreError> {
    let json = Json::parse(text).map_err(core_state)?;
    json_to_response(&json)
}

/// Insert `host_protocol_version: 1` when the field is absent. Does not rewrite a present value.
pub fn stamp_host_protocol_version(text: &str) -> Result<String, CoreError> {
    let json = Json::parse(text).map_err(core_state)?;
    let mut map = json.as_object().map_err(core_state)?.clone();
    if !map.contains_key("host_protocol_version") {
        map.insert(
            "host_protocol_version".into(),
            Json::Number(HOST_PROTOCOL_VERSION as f64),
        );
        return Ok(Json::Object(map).stringify());
    }
    Ok(text.to_string())
}

fn request_to_json(request: &HostRequest) -> Result<Json, CoreError> {
    let mut map = std::collections::BTreeMap::new();
    map.insert(
        "host_protocol_version".into(),
        Json::Number(HOST_PROTOCOL_VERSION as f64),
    );
    match request {
        HostRequest::PersistCheckpoint {
            revision,
            kind,
            base_revision,
            materialize,
            delta,
        } => {
            map.insert("type".into(), Json::String("persist_checkpoint".into()));
            map.insert("revision".into(), Json::Number(*revision as f64));
            map.insert("kind".into(), Json::String(kind.as_str().into()));
            map.insert("base_revision".into(), Json::Number(*base_revision as f64));
            map.insert("materialize".into(), Json::Bool(*materialize));
            match delta {
                Some(delta) => {
                    map.insert("delta".into(), continuation_delta_to_json(delta));
                }
                None => {
                    map.insert("delta".into(), Json::Null);
                }
            }
        }
        HostRequest::RunEffect {
            key,
            idempotency_key,
            input,
        } => {
            map.insert("type".into(), Json::String("run_effect".into()));
            map.insert("key".into(), Json::String(key.clone()));
            map.insert(
                "idempotency_key".into(),
                Json::String(idempotency_key.clone()),
            );
            map.insert("input".into(), value_json(input)?);
        }
        HostRequest::PersistEffect { record } => {
            map.insert("type".into(), Json::String("persist_effect".into()));
            map.insert("key".into(), Json::String(record.key.clone()));
            map.insert(
                "idempotency_key".into(),
                Json::String(record.idempotency_key.clone()),
            );
            map.insert(
                "status".into(),
                Json::String(
                    match record.status {
                        EffectStatus::Started => "started",
                        EffectStatus::Completed => "completed",
                        EffectStatus::Failed => "failed",
                    }
                    .into(),
                ),
            );
            map.insert(
                "result".into(),
                match &record.result {
                    Some(value) => value_json(value)?,
                    None => Json::Null,
                },
            );
        }
        HostRequest::RegisterTimer {
            resume_at_ms,
            branch,
        } => {
            map.insert("type".into(), Json::String("register_timer".into()));
            map.insert("resume_at_ms".into(), Json::Number(*resume_at_ms as f64));
            if let Some(branch) = branch {
                map.insert("branch".into(), Json::String(branch.clone()));
            }
        }
        HostRequest::RegisterWait { wait } => {
            map.insert("type".into(), Json::String("register_wait".into()));
            map.insert("wait_id".into(), Json::String(wait.wait_id.clone()));
            map.insert("event_name".into(), Json::String(wait.event_name.clone()));
            map.insert(
                "correlation_key".into(),
                match &wait.correlation_key {
                    Some(key) => Json::String(key.clone()),
                    None => Json::Null,
                },
            );
            map.insert(
                "timeout_ms".into(),
                match wait.timeout_ms {
                    Some(ms) => Json::Number(ms as f64),
                    None => Json::Null,
                },
            );
        }
        HostRequest::CreateChild { child } => {
            map.insert("type".into(), Json::String("create_child".into()));
            map.insert("invoke_id".into(), Json::String(child.invoke_id.clone()));
            map.insert(
                "child_execution_id".into(),
                Json::String(child.child_execution_id.clone()),
            );
            map.insert(
                "program_name".into(),
                Json::String(child.program_name.clone()),
            );
            if !child.args.is_empty() {
                let mut items = Vec::with_capacity(child.args.len());
                for value in &child.args {
                    items.push(value_json(value)?);
                }
                map.insert("args".into(), Json::Array(items));
            }
        }
        HostRequest::FetchArtifact { hash } => {
            map.insert("type".into(), Json::String("fetch_artifact".into()));
            map.insert("hash".into(), Json::String(hash.clone()));
        }
    }
    Ok(Json::Object(map))
}

fn require_host_protocol_version(
    map: &std::collections::BTreeMap<String, Json>,
) -> Result<(), CoreError> {
    match map.get("host_protocol_version") {
        None => Err(CoreError::MissingHostProtocolVersion),
        Some(value) => {
            let got = value.as_u32().map_err(core_state)?;
            if got != HOST_PROTOCOL_VERSION {
                Err(CoreError::UnsupportedHostProtocol {
                    got,
                    supported: HOST_PROTOCOL_VERSION,
                })
            } else {
                Ok(())
            }
        }
    }
}

fn optional_branch(
    map: &std::collections::BTreeMap<String, Json>,
) -> Result<Option<String>, CoreError> {
    match map.get("branch") {
        None | Some(Json::Null) => Ok(None),
        Some(value) => Ok(Some(value.as_str().map_err(core_state)?.to_string())),
    }
}

fn json_to_response(json: &Json) -> Result<HostResponse, CoreError> {
    let map = json.as_object().map_err(core_state)?;
    require_host_protocol_version(map)?;
    match Json::get(map, "type")
        .map_err(core_state)?
        .as_str()
        .map_err(core_state)?
    {
        "ack" => Ok(HostResponse::Ack),
        "persist_confirmed" => Ok(HostResponse::PersistConfirmed {
            revision: Json::get(map, "revision")
                .map_err(core_state)?
                .as_u64()
                .map_err(core_state)?,
        }),
        "effect_result" => Ok(HostResponse::EffectResult {
            value: json_value(Json::get(map, "value").map_err(core_state)?)?,
        }),
        "effect_failed" => Ok(HostResponse::EffectFailed {
            message: Json::get(map, "message")
                .map_err(core_state)?
                .as_str()
                .map_err(core_state)?
                .to_string(),
        }),
        "event_payload" => Ok(HostResponse::EventPayload {
            value: json_value(Json::get(map, "value").map_err(core_state)?)?,
            branch: optional_branch(map)?,
        }),
        "timer_fired" => Ok(HostResponse::TimerFired {
            branch: optional_branch(map)?,
        }),
        "child_result" => Ok(HostResponse::ChildResult {
            value: json_value(Json::get(map, "value").map_err(core_state)?)?,
            branch: optional_branch(map)?,
        }),
        "cancel" => Ok(HostResponse::Cancel),
        "artifact" => Ok(HostResponse::Artifact {
            hash: Json::get(map, "hash")
                .map_err(core_state)?
                .as_str()
                .map_err(core_state)?
                .to_string(),
        }),
        other => Err(CoreError::TypeError(format!(
            "unknown host response `{other}`"
        ))),
    }
}

fn value_json(value: &Value) -> Result<Json, CoreError> {
    let bytes = encode_value(value).map_err(core_state)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| CoreError::TypeError("value json utf-8".into()))?;
    Json::parse(text).map_err(core_state)
}

fn json_value(json: &Json) -> Result<Value, CoreError> {
    decode_value(json.stringify().as_bytes()).map_err(core_state)
}

fn core_state(err: tcc_state::StateError) -> CoreError {
    CoreError::TypeError(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_rejects_missing_host_protocol_version() {
        let err = decode_response(r#"{"type":"ack"}"#).unwrap_err();
        assert!(matches!(err, CoreError::MissingHostProtocolVersion));
    }

    #[test]
    fn decode_rejects_unsupported_host_protocol_version() {
        let err = decode_response(r#"{"host_protocol_version":2,"type":"ack"}"#).unwrap_err();
        assert!(matches!(
            err,
            CoreError::UnsupportedHostProtocol {
                got: 2,
                supported: 1
            }
        ));
    }

    #[test]
    fn decode_accepts_version_one() {
        assert_eq!(
            decode_response(r#"{"host_protocol_version":1,"type":"ack"}"#).unwrap(),
            HostResponse::Ack
        );
    }

    #[test]
    fn stamp_inserts_missing_version() {
        let stamped = stamp_host_protocol_version(r#"{"type":"ack"}"#).unwrap();
        assert_eq!(decode_response(&stamped).unwrap(), HostResponse::Ack);
    }

    #[test]
    fn stamp_does_not_rewrite_present_version() {
        let stamped =
            stamp_host_protocol_version(r#"{"host_protocol_version":2,"type":"ack"}"#).unwrap();
        assert!(matches!(
            decode_response(&stamped).unwrap_err(),
            CoreError::UnsupportedHostProtocol { got: 2, .. }
        ));
    }

    #[test]
    fn run_effect_requires_input_and_writes_an_empty_object() {
        let encoded = encode_request(&HostRequest::RunEffect {
            key: "search".into(),
            idempotency_key: "exec:search".into(),
            input: Value::Object(std::collections::BTreeMap::new()),
        })
        .unwrap();
        assert!(encoded.contains("\"input\""));
        match decode_request(&encoded).unwrap() {
            HostRequest::RunEffect { input, .. } => {
                assert_eq!(input, Value::Object(std::collections::BTreeMap::new()));
            }
            other => panic!("unexpected {other:?}"),
        }
        let missing = decode_request(
            r#"{"idempotency_key":"exec:search","key":"search","type":"run_effect"}"#,
        )
        .unwrap_err();
        assert_eq!(missing.to_string(), "missing run_effect.input");
        let bad_value = decode_request(
            r#"{"idempotency_key":"exec:search","input":1,"key":"search","type":"run_effect"}"#,
        )
        .unwrap_err();
        assert_ne!(bad_value.to_string(), "missing run_effect.input");
    }

    #[test]
    fn create_child_args_are_canonical() {
        let base = r#"{"child_execution_id":"c","invoke_id":"i","program_name":"p","type":"create_child"}"#;
        let missing = decode_request(base).unwrap();
        match &missing {
            HostRequest::CreateChild { child } => assert!(child.args.is_empty()),
            other => panic!("unexpected {other:?}"),
        }
        let encoded = encode_request(&missing).unwrap();
        assert!(!encoded.contains("args"));

        let empty = decode_request(
            r#"{"args":[],"child_execution_id":"c","invoke_id":"i","program_name":"p","type":"create_child"}"#,
        )
        .unwrap();
        assert_eq!(empty, missing);

        let one = decode_request(
            r#"{"args":[{"t":"null"}],"child_execution_id":"c","invoke_id":"i","program_name":"p","type":"create_child"}"#,
        )
        .unwrap();
        match one {
            HostRequest::CreateChild { child } => assert_eq!(child.args, vec![Value::Null]),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn create_child_rejects_malformed_args() {
        for args in ["null", "{}", "\"foo\""] {
            let text = format!(
                r#"{{"args":{args},"child_execution_id":"c","invoke_id":"i","program_name":"p","type":"create_child"}}"#
            );
            let err = decode_request(&text).unwrap_err();
            assert!(
                matches!(err, CoreError::TypeError(ref message) if message.contains("array")),
                "{args}: {err}"
            );
        }
    }

    #[test]
    fn encode_outcome_declares_protocol_version() {
        let json = encode_outcome(&EngineOutcome::Cancelled).unwrap();
        assert!(json.contains("\"host_protocol_version\":1"));
        assert!(json.contains("\"type\":\"cancelled\""));
    }
}
