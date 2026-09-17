use std::collections::BTreeMap;

use crate::continuation::{
    ArtifactId, Continuation, ContinuationStatus, Frame, PendingOp, TryHandler, WaitKind,
};
use crate::error::StateError;
use crate::json::Json;
use crate::value::Value;

pub fn encode_value(value: &Value) -> Result<Vec<u8>, StateError> {
    Ok(value_to_json(value).stringify().into_bytes())
}

pub fn decode_value(bytes: &[u8]) -> Result<Value, StateError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| StateError::InvalidEncoding("value is not utf-8".to_string()))?;
    json_to_value(&Json::parse(text)?)
}

pub fn encode_continuation(continuation: &Continuation) -> Result<Vec<u8>, StateError> {
    Ok(continuation_to_json(continuation).stringify().into_bytes())
}

pub fn decode_continuation(bytes: &[u8]) -> Result<Continuation, StateError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| StateError::InvalidEncoding("continuation is not utf-8".to_string()))?;
    json_to_continuation(&Json::parse(text)?)
}

fn value_to_json(value: &Value) -> Json {
    let mut map = BTreeMap::new();
    match value {
        Value::Undefined => {
            map.insert("t".to_string(), Json::String("undefined".to_string()));
        }
        Value::Null => {
            map.insert("t".to_string(), Json::String("null".to_string()));
        }
        Value::Bool(flag) => {
            map.insert("t".to_string(), Json::String("bool".to_string()));
            map.insert("v".to_string(), Json::Bool(*flag));
        }
        Value::Number(number) => {
            map.insert("t".to_string(), Json::String("number".to_string()));
            map.insert("v".to_string(), Json::Number(*number));
        }
        Value::String(text) => {
            map.insert("t".to_string(), Json::String("string".to_string()));
            map.insert("v".to_string(), Json::String(text.clone()));
        }
        Value::Object(fields) => {
            map.insert("t".to_string(), Json::String("object".to_string()));
            let mut object = BTreeMap::new();
            for (key, value) in fields {
                object.insert(key.clone(), value_to_json(value));
            }
            map.insert("v".to_string(), Json::Object(object));
        }
        Value::Array(items) => {
            map.insert("t".to_string(), Json::String("array".to_string()));
            map.insert(
                "v".to_string(),
                Json::Array(items.iter().map(value_to_json).collect()),
            );
        }
    }
    Json::Object(map)
}

fn json_to_value(json: &Json) -> Result<Value, StateError> {
    let map = json.as_object()?;
    match Json::get(map, "t")?.as_str()? {
        "undefined" => Ok(Value::Undefined),
        "null" => Ok(Value::Null),
        "bool" => match Json::get(map, "v")? {
            Json::Bool(flag) => Ok(Value::Bool(*flag)),
            _ => Err(StateError::InvalidEncoding("bool value".to_string())),
        },
        "number" => match Json::get(map, "v")? {
            Json::Number(number) => Ok(Value::Number(*number)),
            _ => Err(StateError::InvalidEncoding("number value".to_string())),
        },
        "string" => Ok(Value::String(Json::get(map, "v")?.as_str()?.to_string())),
        "object" => {
            let fields = Json::get(map, "v")?.as_object()?;
            let mut object = BTreeMap::new();
            for (key, value) in fields {
                object.insert(key.clone(), json_to_value(value)?);
            }
            Ok(Value::Object(object))
        }
        "array" => {
            let items = Json::get(map, "v")?.as_array()?;
            let mut array = Vec::new();
            for item in items {
                array.push(json_to_value(item)?);
            }
            Ok(Value::Array(array))
        }
        _ => Err(StateError::UnsupportedValue),
    }
}

fn continuation_to_json(continuation: &Continuation) -> Json {
    let mut map = BTreeMap::new();
    map.insert(
        "execution_id".to_string(),
        Json::String(continuation.execution_id.clone()),
    );
    map.insert(
        "artifact_hash".to_string(),
        Json::String(continuation.artifact.hash.clone()),
    );
    map.insert(
        "engine_format_version".to_string(),
        Json::Number(continuation.engine_format_version as f64),
    );
    map.insert(
        "language_semantics_version".to_string(),
        Json::String(continuation.language_semantics_version.clone()),
    );
    map.insert(
        "revision".to_string(),
        Json::Number(continuation.revision as f64),
    );
    map.insert(
        "status".to_string(),
        Json::String(status_name(continuation.status).to_string()),
    );
    map.insert(
        "frames".to_string(),
        Json::Array(continuation.frames.iter().map(frame_to_json).collect()),
    );
    map.insert(
        "stack".to_string(),
        Json::Array(continuation.stack.iter().map(value_to_json).collect()),
    );
    map.insert(
        "pending".to_string(),
        match &continuation.pending {
            Some(pending) => pending_to_json(pending),
            None => Json::Null,
        },
    );
    map.insert(
        "result".to_string(),
        match &continuation.result {
            Some(value) => value_to_json(value),
            None => Json::Null,
        },
    );
    map.insert(
        "try_stack".to_string(),
        Json::Array(
            continuation
                .try_stack
                .iter()
                .map(try_handler_to_json)
                .collect(),
        ),
    );
    Json::Object(map)
}

fn json_to_continuation(json: &Json) -> Result<Continuation, StateError> {
    let map = json.as_object()?;
    Ok(Continuation {
        execution_id: Json::get(map, "execution_id")?.as_str()?.to_string(),
        artifact: ArtifactId {
            hash: Json::get(map, "artifact_hash")?.as_str()?.to_string(),
        },
        engine_format_version: Json::get(map, "engine_format_version")?.as_u32()?,
        language_semantics_version: Json::get(map, "language_semantics_version")?
            .as_str()?
            .to_string(),
        revision: Json::get(map, "revision")?.as_u64()?,
        status: parse_status(Json::get(map, "status")?.as_str()?)?,
        frames: Json::get(map, "frames")?
            .as_array()?
            .iter()
            .map(json_to_frame)
            .collect::<Result<_, _>>()?,
        stack: Json::get(map, "stack")?
            .as_array()?
            .iter()
            .map(json_to_value)
            .collect::<Result<_, _>>()?,
        pending: match Json::get(map, "pending")? {
            Json::Null => None,
            other => Some(json_to_pending(other)?),
        },
        result: match Json::get(map, "result")? {
            Json::Null => None,
            other => Some(json_to_value(other)?),
        },
        try_stack: match map.get("try_stack") {
            None | Some(Json::Null) => Vec::new(),
            Some(other) => other
                .as_array()?
                .iter()
                .map(json_to_try_handler)
                .collect::<Result<_, _>>()?,
        },
    })
}

fn try_handler_to_json(handler: &TryHandler) -> Json {
    let mut map = BTreeMap::new();
    map.insert("catch".to_string(), Json::Number(handler.catch as f64));
    map.insert(
        "finally".to_string(),
        match handler.finally {
            Some(pc) => Json::Number(pc as f64),
            None => Json::Null,
        },
    );
    map.insert(
        "stack_len".to_string(),
        Json::Number(handler.stack_len as f64),
    );
    Json::Object(map)
}

fn json_to_try_handler(json: &Json) -> Result<TryHandler, StateError> {
    let map = json.as_object()?;
    Ok(TryHandler {
        catch: Json::get(map, "catch")?.as_u32()?,
        finally: match Json::get(map, "finally")? {
            Json::Null => None,
            other => Some(other.as_u32()?),
        },
        stack_len: Json::get(map, "stack_len")?.as_u32()?,
    })
}

fn frame_to_json(frame: &Frame) -> Json {
    let mut map = BTreeMap::new();
    map.insert("func_id".to_string(), Json::Number(frame.func_id as f64));
    map.insert("pc".to_string(), Json::Number(frame.pc as f64));
    map.insert(
        "locals".to_string(),
        Json::Array(frame.locals.iter().map(value_to_json).collect()),
    );
    Json::Object(map)
}

fn json_to_frame(json: &Json) -> Result<Frame, StateError> {
    let map = json.as_object()?;
    Ok(Frame {
        func_id: Json::get(map, "func_id")?.as_u32()?,
        pc: Json::get(map, "pc")?.as_u32()?,
        locals: Json::get(map, "locals")?
            .as_array()?
            .iter()
            .map(json_to_value)
            .collect::<Result<_, _>>()?,
    })
}

fn pending_to_json(pending: &PendingOp) -> Json {
    let mut map = BTreeMap::new();
    match pending {
        PendingOp::Effect {
            key,
            idempotency_key,
        } => {
            map.insert("type".to_string(), Json::String("effect".to_string()));
            map.insert("key".to_string(), Json::String(key.clone()));
            map.insert(
                "idempotency_key".to_string(),
                Json::String(idempotency_key.clone()),
            );
        }
        PendingOp::Wait { kind } => {
            map.insert("type".to_string(), Json::String("wait".to_string()));
            map.insert("kind".to_string(), wait_to_json(kind));
        }
    }
    Json::Object(map)
}

fn json_to_pending(json: &Json) -> Result<PendingOp, StateError> {
    let map = json.as_object()?;
    match Json::get(map, "type")?.as_str()? {
        "effect" => Ok(PendingOp::Effect {
            key: Json::get(map, "key")?.as_str()?.to_string(),
            idempotency_key: Json::get(map, "idempotency_key")?.as_str()?.to_string(),
        }),
        "wait" => Ok(PendingOp::Wait {
            kind: json_to_wait(Json::get(map, "kind")?)?,
        }),
        _ => Err(StateError::InvalidEncoding(
            "unknown pending op".to_string(),
        )),
    }
}

fn wait_to_json(kind: &WaitKind) -> Json {
    let mut map = BTreeMap::new();
    match kind {
        WaitKind::Timer { resume_at_ms } => {
            map.insert("type".to_string(), Json::String("timer".to_string()));
            map.insert(
                "resume_at_ms".to_string(),
                Json::Number(*resume_at_ms as f64),
            );
        }
        WaitKind::Event {
            wait_id,
            event_name,
            correlation_key,
        } => {
            map.insert("type".to_string(), Json::String("event".to_string()));
            map.insert("wait_id".to_string(), Json::String(wait_id.clone()));
            map.insert("event_name".to_string(), Json::String(event_name.clone()));
            map.insert(
                "correlation_key".to_string(),
                match correlation_key {
                    Some(key) => Json::String(key.clone()),
                    None => Json::Null,
                },
            );
        }
        WaitKind::Child {
            invoke_id,
            child_execution_id,
        } => {
            map.insert("type".to_string(), Json::String("child".to_string()));
            map.insert("invoke_id".to_string(), Json::String(invoke_id.clone()));
            map.insert(
                "child_execution_id".to_string(),
                Json::String(child_execution_id.clone()),
            );
        }
    }
    Json::Object(map)
}

fn json_to_wait(json: &Json) -> Result<WaitKind, StateError> {
    let map = json.as_object()?;
    match Json::get(map, "type")?.as_str()? {
        "timer" => Ok(WaitKind::Timer {
            resume_at_ms: Json::get(map, "resume_at_ms")?.as_u64()?,
        }),
        "event" => Ok(WaitKind::Event {
            wait_id: Json::get(map, "wait_id")?.as_str()?.to_string(),
            event_name: Json::get(map, "event_name")?.as_str()?.to_string(),
            correlation_key: match Json::get(map, "correlation_key")? {
                Json::Null => None,
                other => Some(other.as_str()?.to_string()),
            },
        }),
        "child" => Ok(WaitKind::Child {
            invoke_id: Json::get(map, "invoke_id")?.as_str()?.to_string(),
            child_execution_id: Json::get(map, "child_execution_id")?.as_str()?.to_string(),
        }),
        _ => Err(StateError::InvalidEncoding("unknown wait kind".to_string())),
    }
}

fn status_name(status: ContinuationStatus) -> &'static str {
    match status {
        ContinuationStatus::Runnable => "runnable",
        ContinuationStatus::Running => "running",
        ContinuationStatus::Suspended => "suspended",
        ContinuationStatus::Completed => "completed",
        ContinuationStatus::Failed => "failed",
        ContinuationStatus::Cancelled => "cancelled",
    }
}

fn parse_status(name: &str) -> Result<ContinuationStatus, StateError> {
    match name {
        "runnable" => Ok(ContinuationStatus::Runnable),
        "running" => Ok(ContinuationStatus::Running),
        "suspended" => Ok(ContinuationStatus::Suspended),
        "completed" => Ok(ContinuationStatus::Completed),
        "failed" => Ok(ContinuationStatus::Failed),
        "cancelled" => Ok(ContinuationStatus::Cancelled),
        _ => Err(StateError::InvalidEncoding("unknown status".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::continuation::Continuation;

    #[test]
    fn value_round_trip() {
        let values = [
            Value::Undefined,
            Value::Null,
            Value::Bool(true),
            Value::Number(1.5),
            Value::String("ok".to_string()),
            Value::Object(BTreeMap::from([(
                "result".to_string(),
                Value::Number(42.0),
            )])),
            Value::Array(vec![Value::Number(1.0), Value::Bool(true)]),
        ];
        for value in values {
            let bytes = encode_value(&value).unwrap();
            assert_eq!(decode_value(&bytes).unwrap(), value);
        }
    }

    #[test]
    fn rejects_unknown_value_tag() {
        let err = decode_value(br#"{"t":"bigint","v":"1"}"#).unwrap_err();
        assert_eq!(err, StateError::UnsupportedValue);
    }

    #[test]
    fn continuation_round_trip() {
        let continuation = Continuation::start("exec-1", "hash-1", 1, "ts.subset.v1", 0, 2);
        let bytes = encode_continuation(&continuation).unwrap();
        assert_eq!(decode_continuation(&bytes).unwrap(), continuation);
    }
}
