use std::collections::BTreeMap;

use crate::artifact::{
    Artifact, Envelope, FuncId, Function, LocalId, Pc, Program, RuntimeModule, SourceSpan,
};
use crate::error::IrError;
use crate::features::{EngineFeature, HostCapability};
use crate::instruction::{ConstValue, Instruction};
use crate::json::Json;

pub fn encode_artifact(artifact: &Artifact) -> Result<String, IrError> {
    Ok(artifact_to_json(artifact).stringify())
}

pub fn decode_artifact(text: &str) -> Result<Artifact, IrError> {
    json_to_artifact(&Json::parse(text)?)
}

/// Canonical JSON used for hashing: same as encode, but `artifact_hash` is empty.
pub fn canonical_artifact_json(artifact: &Artifact) -> String {
    let mut copy = artifact.clone();
    copy.envelope.artifact_hash.clear();
    artifact_to_json(&copy).stringify()
}

fn artifact_to_json(artifact: &Artifact) -> Json {
    let mut map = BTreeMap::new();
    map.insert("envelope".to_string(), envelope_to_json(&artifact.envelope));
    map.insert("program".to_string(), program_to_json(&artifact.program));
    Json::Object(map)
}

fn envelope_to_json(envelope: &Envelope) -> Json {
    let mut map = BTreeMap::new();
    map.insert(
        "artifact_hash".to_string(),
        Json::String(envelope.artifact_hash.clone()),
    );
    map.insert(
        "frontend_id".to_string(),
        Json::String(envelope.frontend_id.clone()),
    );
    map.insert(
        "frontend_version".to_string(),
        Json::String(envelope.frontend_version.clone()),
    );
    map.insert(
        "language_semantics_version".to_string(),
        Json::String(envelope.language_semantics_version.clone()),
    );
    map.insert(
        "engine_format_version".to_string(),
        Json::Number(envelope.engine_format_version as f64),
    );
    map.insert(
        "required_engine_features".to_string(),
        Json::Array(
            envelope
                .required_engine_features
                .iter()
                .map(|feature| Json::String(feature.0.clone()))
                .collect(),
        ),
    );
    map.insert(
        "required_host_capabilities".to_string(),
        Json::Array(
            envelope
                .required_host_capabilities
                .iter()
                .map(|cap| Json::String(cap.0.clone()))
                .collect(),
        ),
    );
    map.insert(
        "runtime_modules".to_string(),
        Json::Array(
            envelope
                .runtime_modules
                .iter()
                .map(runtime_module_to_json)
                .collect(),
        ),
    );
    Json::Object(map)
}

fn runtime_module_to_json(module: &RuntimeModule) -> Json {
    let mut map = BTreeMap::new();
    map.insert("id".to_string(), Json::String(module.id.clone()));
    map.insert("kind".to_string(), Json::String(module.kind.clone()));
    Json::Object(map)
}

fn program_to_json(program: &Program) -> Json {
    let mut map = BTreeMap::new();
    map.insert("entry".to_string(), Json::Number(program.entry.0 as f64));
    map.insert(
        "functions".to_string(),
        Json::Array(program.functions.iter().map(function_to_json).collect()),
    );
    Json::Object(map)
}

fn function_to_json(function: &Function) -> Json {
    let mut map = BTreeMap::new();
    map.insert("id".to_string(), Json::Number(function.id.0 as f64));
    map.insert("name".to_string(), Json::String(function.name.clone()));
    map.insert(
        "param_count".to_string(),
        Json::Number(function.param_count as f64),
    );
    map.insert(
        "local_count".to_string(),
        Json::Number(function.local_count as f64),
    );
    map.insert(
        "instructions".to_string(),
        Json::Array(
            function
                .instructions
                .iter()
                .map(instruction_to_json)
                .collect(),
        ),
    );
    map.insert(
        "spans".to_string(),
        Json::Array(function.spans.iter().map(span_to_json).collect()),
    );
    Json::Object(map)
}

fn span_to_json(span: &Option<SourceSpan>) -> Json {
    match span {
        None => Json::Null,
        Some(span) => {
            let mut map = BTreeMap::new();
            map.insert("file".to_string(), Json::String(span.file.clone()));
            map.insert(
                "start_line".to_string(),
                Json::Number(span.start_line as f64),
            );
            map.insert(
                "start_column".to_string(),
                Json::Number(span.start_column as f64),
            );
            map.insert("end_line".to_string(), Json::Number(span.end_line as f64));
            map.insert(
                "end_column".to_string(),
                Json::Number(span.end_column as f64),
            );
            Json::Object(map)
        }
    }
}

fn instruction_to_json(instruction: &Instruction) -> Json {
    let mut map = BTreeMap::new();
    match instruction {
        Instruction::Nop => {
            map.insert("op".to_string(), Json::String("Nop".to_string()));
        }
        Instruction::Jump { target } => {
            map.insert("op".to_string(), Json::String("Jump".to_string()));
            map.insert("target".to_string(), Json::Number(target.0 as f64));
        }
        Instruction::JumpIfTrue { target } => {
            map.insert("op".to_string(), Json::String("JumpIfTrue".to_string()));
            map.insert("target".to_string(), Json::Number(target.0 as f64));
        }
        Instruction::JumpIfFalse { target } => {
            map.insert("op".to_string(), Json::String("JumpIfFalse".to_string()));
            map.insert("target".to_string(), Json::Number(target.0 as f64));
        }
        Instruction::LoadLocal { local } => {
            map.insert("op".to_string(), Json::String("LoadLocal".to_string()));
            map.insert("local".to_string(), Json::Number(local.0 as f64));
        }
        Instruction::StoreLocal { local } => {
            map.insert("op".to_string(), Json::String("StoreLocal".to_string()));
            map.insert("local".to_string(), Json::Number(local.0 as f64));
        }
        Instruction::LoadConst { value } => {
            map.insert("op".to_string(), Json::String("LoadConst".to_string()));
            map.insert("value".to_string(), const_to_json(value));
        }
        Instruction::Pop => {
            map.insert("op".to_string(), Json::String("Pop".to_string()));
        }
        Instruction::NewObject => {
            map.insert("op".to_string(), Json::String("NewObject".to_string()));
        }
        Instruction::SetProp { key } => {
            map.insert("op".to_string(), Json::String("SetProp".to_string()));
            map.insert("key".to_string(), Json::String(key.clone()));
        }
        Instruction::Return => {
            map.insert("op".to_string(), Json::String("Return".to_string()));
        }
        Instruction::Call { func, argc } => {
            map.insert("op".to_string(), Json::String("Call".to_string()));
            map.insert("func".to_string(), Json::Number(func.0 as f64));
            map.insert("argc".to_string(), Json::Number(*argc as f64));
        }
        Instruction::Effect => {
            map.insert("op".to_string(), Json::String("Effect".to_string()));
        }
        Instruction::Sleep => {
            map.insert("op".to_string(), Json::String("Sleep".to_string()));
        }
        Instruction::WaitForEvent => {
            map.insert("op".to_string(), Json::String("WaitForEvent".to_string()));
        }
        Instruction::Invoke => {
            map.insert("op".to_string(), Json::String("Invoke".to_string()));
        }
        Instruction::Throw => {
            map.insert("op".to_string(), Json::String("Throw".to_string()));
        }
        Instruction::PushTry { catch, finally } => {
            map.insert("op".to_string(), Json::String("PushTry".to_string()));
            map.insert("catch".to_string(), Json::Number(catch.0 as f64));
            map.insert(
                "finally".to_string(),
                match finally {
                    Some(pc) => Json::Number(pc.0 as f64),
                    None => Json::Null,
                },
            );
        }
        Instruction::PopTry => {
            map.insert("op".to_string(), Json::String("PopTry".to_string()));
        }
    }
    Json::Object(map)
}

fn const_to_json(value: &ConstValue) -> Json {
    let mut map = BTreeMap::new();
    match value {
        ConstValue::Undefined => {
            map.insert("t".to_string(), Json::String("undefined".to_string()));
        }
        ConstValue::Null => {
            map.insert("t".to_string(), Json::String("null".to_string()));
        }
        ConstValue::Bool(flag) => {
            map.insert("t".to_string(), Json::String("bool".to_string()));
            map.insert("v".to_string(), Json::Bool(*flag));
        }
        ConstValue::Number(number) => {
            map.insert("t".to_string(), Json::String("number".to_string()));
            map.insert("v".to_string(), Json::Number(*number));
        }
        ConstValue::String(text) => {
            map.insert("t".to_string(), Json::String("string".to_string()));
            map.insert("v".to_string(), Json::String(text.clone()));
        }
    }
    Json::Object(map)
}

fn json_to_artifact(json: &Json) -> Result<Artifact, IrError> {
    let map = json.as_object()?;
    Ok(Artifact {
        envelope: json_to_envelope(Json::get(map, "envelope")?)?,
        program: json_to_program(Json::get(map, "program")?)?,
    })
}

fn json_to_envelope(json: &Json) -> Result<Envelope, IrError> {
    let map = json.as_object()?;
    Ok(Envelope {
        artifact_hash: Json::get(map, "artifact_hash")?.as_str()?.to_string(),
        frontend_id: Json::get(map, "frontend_id")?.as_str()?.to_string(),
        frontend_version: Json::get(map, "frontend_version")?.as_str()?.to_string(),
        language_semantics_version: Json::get(map, "language_semantics_version")?
            .as_str()?
            .to_string(),
        engine_format_version: Json::get(map, "engine_format_version")?.as_u32()?,
        required_engine_features: Json::get(map, "required_engine_features")?
            .as_array()?
            .iter()
            .map(|item| Ok(EngineFeature(item.as_str()?.to_string())))
            .collect::<Result<_, IrError>>()?,
        required_host_capabilities: Json::get(map, "required_host_capabilities")?
            .as_array()?
            .iter()
            .map(|item| Ok(HostCapability(item.as_str()?.to_string())))
            .collect::<Result<_, IrError>>()?,
        runtime_modules: Json::get(map, "runtime_modules")?
            .as_array()?
            .iter()
            .map(json_to_runtime_module)
            .collect::<Result<_, _>>()?,
    })
}

fn json_to_runtime_module(json: &Json) -> Result<RuntimeModule, IrError> {
    let map = json.as_object()?;
    Ok(RuntimeModule {
        id: Json::get(map, "id")?.as_str()?.to_string(),
        kind: Json::get(map, "kind")?.as_str()?.to_string(),
    })
}

fn json_to_program(json: &Json) -> Result<Program, IrError> {
    let map = json.as_object()?;
    Ok(Program {
        entry: FuncId(Json::get(map, "entry")?.as_u32()?),
        functions: Json::get(map, "functions")?
            .as_array()?
            .iter()
            .map(json_to_function)
            .collect::<Result<_, _>>()?,
    })
}

fn json_to_function(json: &Json) -> Result<Function, IrError> {
    let map = json.as_object()?;
    Ok(Function {
        id: FuncId(Json::get(map, "id")?.as_u32()?),
        name: Json::get(map, "name")?.as_str()?.to_string(),
        param_count: Json::get(map, "param_count")?.as_u32()?,
        local_count: Json::get(map, "local_count")?.as_u32()?,
        instructions: Json::get(map, "instructions")?
            .as_array()?
            .iter()
            .map(json_to_instruction)
            .collect::<Result<_, _>>()?,
        spans: Json::get(map, "spans")?
            .as_array()?
            .iter()
            .map(json_to_span)
            .collect::<Result<_, _>>()?,
    })
}

fn json_to_span(json: &Json) -> Result<Option<SourceSpan>, IrError> {
    if matches!(json, Json::Null) {
        return Ok(None);
    }
    let map = json.as_object()?;
    Ok(Some(SourceSpan {
        file: Json::get(map, "file")?.as_str()?.to_string(),
        start_line: Json::get(map, "start_line")?.as_u32()?,
        start_column: Json::get(map, "start_column")?.as_u32()?,
        end_line: Json::get(map, "end_line")?.as_u32()?,
        end_column: Json::get(map, "end_column")?.as_u32()?,
    }))
}

fn json_to_instruction(json: &Json) -> Result<Instruction, IrError> {
    let map = json.as_object()?;
    match Json::get(map, "op")?.as_str()? {
        "Nop" => Ok(Instruction::Nop),
        "Jump" => Ok(Instruction::Jump {
            target: Pc(Json::get(map, "target")?.as_u32()?),
        }),
        "JumpIfTrue" => Ok(Instruction::JumpIfTrue {
            target: Pc(Json::get(map, "target")?.as_u32()?),
        }),
        "JumpIfFalse" => Ok(Instruction::JumpIfFalse {
            target: Pc(Json::get(map, "target")?.as_u32()?),
        }),
        "LoadLocal" => Ok(Instruction::LoadLocal {
            local: LocalId(Json::get(map, "local")?.as_u32()?),
        }),
        "StoreLocal" => Ok(Instruction::StoreLocal {
            local: LocalId(Json::get(map, "local")?.as_u32()?),
        }),
        "LoadConst" => Ok(Instruction::LoadConst {
            value: json_to_const(Json::get(map, "value")?)?,
        }),
        "Pop" => Ok(Instruction::Pop),
        "NewObject" => Ok(Instruction::NewObject),
        "SetProp" => Ok(Instruction::SetProp {
            key: Json::get(map, "key")?.as_str()?.to_string(),
        }),
        "Return" => Ok(Instruction::Return),
        "Call" => Ok(Instruction::Call {
            func: FuncId(Json::get(map, "func")?.as_u32()?),
            argc: Json::get(map, "argc")?.as_u32()?,
        }),
        "Effect" => Ok(Instruction::Effect),
        "Sleep" => Ok(Instruction::Sleep),
        "WaitForEvent" => Ok(Instruction::WaitForEvent),
        "Invoke" => Ok(Instruction::Invoke),
        "Throw" => Ok(Instruction::Throw),
        "PushTry" => Ok(Instruction::PushTry {
            catch: Pc(Json::get(map, "catch")?.as_u32()?),
            finally: match Json::get(map, "finally")? {
                Json::Null => None,
                other => Some(Pc(other.as_u32()?)),
            },
        }),
        "PopTry" => Ok(Instruction::PopTry),
        other => Err(IrError::InvalidEncoding(format!(
            "unknown instruction `{other}`"
        ))),
    }
}

fn json_to_const(json: &Json) -> Result<ConstValue, IrError> {
    let map = json.as_object()?;
    match Json::get(map, "t")?.as_str()? {
        "undefined" => Ok(ConstValue::Undefined),
        "null" => Ok(ConstValue::Null),
        "bool" => match Json::get(map, "v")? {
            Json::Bool(flag) => Ok(ConstValue::Bool(*flag)),
            _ => Err(IrError::InvalidEncoding("bool const".to_string())),
        },
        "number" => match Json::get(map, "v")? {
            Json::Number(number) => Ok(ConstValue::Number(*number)),
            _ => Err(IrError::InvalidEncoding("number const".to_string())),
        },
        "string" => Ok(ConstValue::String(
            Json::get(map, "v")?.as_str()?.to_string(),
        )),
        other => Err(IrError::InvalidEncoding(format!(
            "unknown const tag `{other}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::Artifact;

    #[test]
    fn artifact_json_round_trip() {
        let artifact = Artifact::minimal_return("hash-json");
        let text = encode_artifact(&artifact).unwrap();
        assert_eq!(decode_artifact(&text).unwrap(), artifact);
    }
}
