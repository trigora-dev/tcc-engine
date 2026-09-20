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
        } => {
            map.insert("type".into(), Json::String("run_effect".into()));
            map.insert("key".into(), Json::String(key.clone()));
            map.insert(
                "idempotency_key".into(),
                Json::String(idempotency_key.clone()),
            );
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
        HostRequest::RegisterTimer { resume_at_ms } => {
            map.insert("type".into(), Json::String("register_timer".into()));
            map.insert("resume_at_ms".into(), Json::Number(*resume_at_ms as f64));
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
            map.insert(
                "input".into(),
                match &child.input {
                    Some(value) => value_json(value)?,
                    None => Json::Null,
                },
            );
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
        }),
        "timer_fired" => Ok(HostResponse::TimerFired),
        "child_result" => Ok(HostResponse::ChildResult {
            value: json_value(Json::get(map, "value").map_err(core_state)?)?,
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
    fn encode_outcome_declares_protocol_version() {
        let json = encode_outcome(&EngineOutcome::Cancelled).unwrap();
        assert!(json.contains("\"host_protocol_version\":1"));
        assert!(json.contains("\"type\":\"cancelled\""));
    }
}
