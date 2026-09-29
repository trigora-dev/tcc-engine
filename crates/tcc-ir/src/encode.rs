// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

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
    if function.param_defaults.iter().any(|item| item.is_some()) {
        map.insert(
            "param_defaults".to_string(),
            Json::Array(
                function
                    .param_defaults
                    .iter()
                    .map(|item| match item {
                        Some(value) => const_to_json(value),
                        None => Json::Null,
                    })
                    .collect(),
            ),
        );
    }
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

fn instruction_name(instruction: &Instruction) -> &'static str {
    match instruction {
        Instruction::Add => "Add",
        Instruction::Sub => "Sub",
        Instruction::Mul => "Mul",
        Instruction::Div => "Div",
        Instruction::Rem => "Rem",
        Instruction::Neg => "Neg",
        Instruction::Pow => "Pow",
        Instruction::FloorDiv => "FloorDiv",
        Instruction::GetIndex => "GetIndex",
        Instruction::SetIndex => "SetIndex",
        Instruction::Length => "Length",
        Instruction::WatchIter => "WatchIter",
        Instruction::UnwatchIter => "UnwatchIter",
        Instruction::Same => "Same",
        Instruction::NewCell => "NewCell",
        Instruction::NewEnv { .. } => "NewEnv",
        Instruction::NewClosure { .. } => "NewClosure",
        Instruction::EnvGet { .. } => "EnvGet",
        Instruction::EnvSet { .. } => "EnvSet",
        Instruction::EnvSlot { .. } => "EnvSlot",
        Instruction::CallClosure { .. } => "CallClosure",
        Instruction::LoadFunc { .. } => "LoadFunc",
        _ => "Nop",
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
        Instruction::GetProp { key } => {
            map.insert("op".to_string(), Json::String("GetProp".to_string()));
            map.insert("key".to_string(), Json::String(key.clone()));
        }
        Instruction::NewArray => {
            map.insert("op".to_string(), Json::String("NewArray".to_string()));
        }
        Instruction::ArrayPush => {
            map.insert("op".to_string(), Json::String("ArrayPush".to_string()));
        }
        Instruction::StrictEq => {
            map.insert("op".to_string(), Json::String("StrictEq".to_string()));
        }
        Instruction::StrictNeq => {
            map.insert("op".to_string(), Json::String("StrictNeq".to_string()));
        }
        Instruction::Lt => {
            map.insert("op".to_string(), Json::String("Lt".to_string()));
        }
        Instruction::Le => {
            map.insert("op".to_string(), Json::String("Le".to_string()));
        }
        Instruction::Gt => {
            map.insert("op".to_string(), Json::String("Gt".to_string()));
        }
        Instruction::Ge => {
            map.insert("op".to_string(), Json::String("Ge".to_string()));
        }
        Instruction::Not => {
            map.insert("op".to_string(), Json::String("Not".to_string()));
        }
        Instruction::Return => {
            map.insert("op".to_string(), Json::String("Return".to_string()));
        }
        Instruction::Call { func, argc } => {
            map.insert("op".to_string(), Json::String("Call".to_string()));
            map.insert("func".to_string(), Json::Number(func.0 as f64));
            map.insert("argc".to_string(), Json::Number(*argc as f64));
        }
        Instruction::Effect { has_input } => {
            map.insert("op".to_string(), Json::String("Effect".to_string()));
            if *has_input {
                map.insert("has_input".to_string(), Json::Bool(true));
            }
        }
        Instruction::Sleep => {
            map.insert("op".to_string(), Json::String("Sleep".to_string()));
        }
        Instruction::WaitForEvent => {
            map.insert("op".to_string(), Json::String("WaitForEvent".to_string()));
        }
        Instruction::Invoke { arg_count } => {
            map.insert("op".to_string(), Json::String("Invoke".to_string()));
            if *arg_count != 0 {
                map.insert("arg_count".to_string(), Json::Number(*arg_count as f64));
            }
        }
        Instruction::Fork { count, join_pc } => {
            map.insert("op".to_string(), Json::String("Fork".to_string()));
            map.insert("count".to_string(), Json::Number(*count as f64));
            map.insert("join_pc".to_string(), Json::Number(join_pc.0 as f64));
        }
        Instruction::JoinAll => {
            map.insert("op".to_string(), Json::String("JoinAll".to_string()));
        }
        Instruction::JoinAny => {
            map.insert("op".to_string(), Json::String("JoinAny".to_string()));
        }
        Instruction::ArrayIndex { index } => {
            map.insert("op".to_string(), Json::String("ArrayIndex".to_string()));
            map.insert("index".to_string(), Json::Number(*index as f64));
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
        Instruction::NewEnv { count } => {
            map.insert("op".to_string(), Json::String("NewEnv".to_string()));
            map.insert("count".to_string(), Json::Number(*count as f64));
        }
        Instruction::NewClosure { func } => {
            map.insert("op".to_string(), Json::String("NewClosure".to_string()));
            map.insert("func".to_string(), Json::Number(func.0 as f64));
        }
        Instruction::EnvGet { index } => {
            map.insert("op".to_string(), Json::String("EnvGet".to_string()));
            map.insert("index".to_string(), Json::Number(*index as f64));
        }
        Instruction::EnvSet { index } => {
            map.insert("op".to_string(), Json::String("EnvSet".to_string()));
            map.insert("index".to_string(), Json::Number(*index as f64));
        }
        Instruction::EnvSlot { index } => {
            map.insert("op".to_string(), Json::String("EnvSlot".to_string()));
            map.insert("index".to_string(), Json::Number(*index as f64));
        }
        Instruction::CallClosure { argc } => {
            map.insert("op".to_string(), Json::String("CallClosure".to_string()));
            map.insert("argc".to_string(), Json::Number(*argc as f64));
        }
        Instruction::LoadFunc { func } => {
            map.insert("op".to_string(), Json::String("LoadFunc".to_string()));
            map.insert("func".to_string(), Json::Number(func.0 as f64));
        }
        Instruction::Add
        | Instruction::Sub
        | Instruction::Mul
        | Instruction::Div
        | Instruction::Rem
        | Instruction::Neg
        | Instruction::Pow
        | Instruction::FloorDiv
        | Instruction::GetIndex
        | Instruction::SetIndex
        | Instruction::Length
        | Instruction::WatchIter
        | Instruction::UnwatchIter
        | Instruction::Same
        | Instruction::NewCell => {
            map.insert(
                "op".to_string(),
                Json::String(instruction_name(instruction).into()),
            );
        }
    }
    Json::Object(map)
}

fn number_payload(number: f64) -> Json {
    if number.is_nan() {
        Json::String("NaN".to_string())
    } else if number.is_infinite() && number.is_sign_positive() {
        Json::String("Infinity".to_string())
    } else if number.is_infinite() {
        Json::String("-Infinity".to_string())
    } else {
        Json::Number(number)
    }
}

fn json_to_number(json: &Json) -> Result<f64, IrError> {
    match json {
        Json::Number(number) if number.is_finite() => Ok(*number),
        Json::String(text) => match text.as_str() {
            "NaN" => Ok(f64::NAN),
            "Infinity" => Ok(f64::INFINITY),
            "-Infinity" => Ok(f64::NEG_INFINITY),
            _ => Err(IrError::InvalidEncoding(
                "number const must be finite or NaN/Infinity/-Infinity".to_string(),
            )),
        },
        _ => Err(IrError::InvalidEncoding("number const".to_string())),
    }
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
            map.insert("v".to_string(), number_payload(*number));
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
        param_defaults: match map.get("param_defaults") {
            None | Some(Json::Null) => Vec::new(),
            Some(value) => value
                .as_array()?
                .iter()
                .map(|item| {
                    if matches!(item, Json::Null) {
                        Ok(None)
                    } else {
                        json_to_const(item).map(Some)
                    }
                })
                .collect::<Result<_, _>>()?,
        },
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
        "GetProp" => Ok(Instruction::GetProp {
            key: Json::get(map, "key")?.as_str()?.to_string(),
        }),
        "NewArray" => Ok(Instruction::NewArray),
        "ArrayPush" => Ok(Instruction::ArrayPush),
        "StrictEq" => Ok(Instruction::StrictEq),
        "StrictNeq" => Ok(Instruction::StrictNeq),
        "Lt" => Ok(Instruction::Lt),
        "Le" => Ok(Instruction::Le),
        "Gt" => Ok(Instruction::Gt),
        "Ge" => Ok(Instruction::Ge),
        "Not" => Ok(Instruction::Not),
        "Return" => Ok(Instruction::Return),
        "Call" => Ok(Instruction::Call {
            func: FuncId(Json::get(map, "func")?.as_u32()?),
            argc: Json::get(map, "argc")?.as_u32()?,
        }),
        "Effect" => {
            let has_input = match map.get("has_input") {
                None => false,
                Some(Json::Bool(flag)) => *flag,
                Some(_) => {
                    return Err(IrError::InvalidEncoding(
                        "Effect has_input must be a boolean".to_string(),
                    ))
                }
            };
            Ok(Instruction::Effect { has_input })
        }
        "Sleep" => Ok(Instruction::Sleep),
        "WaitForEvent" => Ok(Instruction::WaitForEvent),
        "Invoke" => Ok(Instruction::Invoke {
            arg_count: match map.get("arg_count") {
                None => 0,
                Some(value) => value.as_u32().map_err(|_| {
                    IrError::InvalidEncoding("Invoke arg_count must be an integer".to_string())
                })?,
            },
        }),
        "Fork" => Ok(Instruction::Fork {
            count: Json::get(map, "count")?.as_u32()?,
            join_pc: Pc(Json::get(map, "join_pc")?.as_u32()?),
        }),
        "JoinAll" => Ok(Instruction::JoinAll),
        "JoinAny" => Ok(Instruction::JoinAny),
        "ArrayIndex" => Ok(Instruction::ArrayIndex {
            index: Json::get(map, "index")?.as_u32()?,
        }),
        "Throw" => Ok(Instruction::Throw),
        "PushTry" => Ok(Instruction::PushTry {
            catch: Pc(Json::get(map, "catch")?.as_u32()?),
            finally: match Json::get(map, "finally")? {
                Json::Null => None,
                other => Some(Pc(other.as_u32()?)),
            },
        }),
        "PopTry" => Ok(Instruction::PopTry),
        "Add" => Ok(Instruction::Add),
        "Sub" => Ok(Instruction::Sub),
        "Mul" => Ok(Instruction::Mul),
        "Div" => Ok(Instruction::Div),
        "Rem" => Ok(Instruction::Rem),
        "Neg" => Ok(Instruction::Neg),
        "Pow" => Ok(Instruction::Pow),
        "FloorDiv" => Ok(Instruction::FloorDiv),
        "GetIndex" => Ok(Instruction::GetIndex),
        "SetIndex" => Ok(Instruction::SetIndex),
        "Length" => Ok(Instruction::Length),
        "WatchIter" => Ok(Instruction::WatchIter),
        "UnwatchIter" => Ok(Instruction::UnwatchIter),
        "Same" => Ok(Instruction::Same),
        "NewCell" => Ok(Instruction::NewCell),
        "NewEnv" => Ok(Instruction::NewEnv {
            count: Json::get(map, "count")?.as_u32()?,
        }),
        "NewClosure" => Ok(Instruction::NewClosure {
            func: FuncId(Json::get(map, "func")?.as_u32()?),
        }),
        "EnvGet" => Ok(Instruction::EnvGet {
            index: Json::get(map, "index")?.as_u32()?,
        }),
        "EnvSet" => Ok(Instruction::EnvSet {
            index: Json::get(map, "index")?.as_u32()?,
        }),
        "EnvSlot" => Ok(Instruction::EnvSlot {
            index: Json::get(map, "index")?.as_u32()?,
        }),
        "CallClosure" => Ok(Instruction::CallClosure {
            argc: Json::get(map, "argc")?.as_u32()?,
        }),
        "LoadFunc" => Ok(Instruction::LoadFunc {
            func: FuncId(Json::get(map, "func")?.as_u32()?),
        }),
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
        "number" => Ok(ConstValue::Number(json_to_number(Json::get(map, "v")?)?)),
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

    #[test]
    fn unknown_opcode_is_invalid_encoding_not_panic() {
        let mut artifact = Artifact::minimal_return("hash-json");
        artifact.program.functions[0].instructions = vec![Instruction::Nop];
        artifact.program.functions[0].spans = vec![None];
        let text = encode_artifact(&artifact)
            .unwrap()
            .replace("\"op\":\"Nop\"", "\"op\":\"Nope\"");
        let result = std::panic::catch_unwind(|| decode_artifact(&text));
        assert!(result.is_ok(), "decode panicked");
        assert!(matches!(
            result.unwrap(),
            Err(IrError::InvalidEncoding(message)) if message.contains("Nope")
        ));
    }

    #[test]
    fn effect_omits_has_input_unless_a_value_is_popped() {
        let plain = instruction_to_json(&Instruction::Effect { has_input: false });
        let text = plain.stringify();
        assert!(text.contains("\"op\":\"Effect\""));
        assert!(!text.contains("has_input"));
        assert_eq!(
            json_to_instruction(&Json::parse(r#"{"op":"Effect"}"#).unwrap()).unwrap(),
            Instruction::Effect { has_input: false }
        );
        let with_input = instruction_to_json(&Instruction::Effect { has_input: true });
        assert!(with_input.stringify().contains("\"has_input\":true"));
        assert_eq!(
            json_to_instruction(&with_input).unwrap(),
            Instruction::Effect { has_input: true }
        );
    }

    #[test]
    fn zero_argument_invoke_omits_arg_count() {
        let mut artifact = Artifact::minimal_return("invoke-plain");
        artifact.program.functions[0].instructions = vec![Instruction::Invoke { arg_count: 0 }];
        artifact.program.functions[0].spans = vec![None];
        let text = encode_artifact(&artifact).unwrap();
        assert!(text.contains("\"op\":\"Invoke\""));
        assert!(!text.contains("arg_count"));
        let decoded = decode_artifact(&text).unwrap();
        assert_eq!(
            decoded.program.functions[0].instructions[0],
            Instruction::Invoke { arg_count: 0 }
        );
        assert_eq!(encode_artifact(&decoded).unwrap(), text);
    }

    #[test]
    fn invoke_arg_count_round_trips() {
        let mut artifact = Artifact::minimal_return("invoke-args");
        artifact.program.functions[0].instructions = vec![Instruction::Invoke { arg_count: 2 }];
        artifact.program.functions[0].spans = vec![None];
        let text = encode_artifact(&artifact).unwrap();
        assert!(text.contains("\"arg_count\":2"));
        assert_eq!(decode_artifact(&text).unwrap(), artifact);
    }

    #[test]
    fn malformed_json_is_invalid_encoding_not_panic() {
        let result = std::panic::catch_unwind(|| decode_artifact("{not json"));
        assert!(result.is_ok(), "decode panicked");
        assert!(result.unwrap().is_err());
    }
}
