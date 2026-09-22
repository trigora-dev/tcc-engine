//! Bounded valid-IR generator for differential tests. Not a public fuzzer.

use crate::artifact::{Artifact, Envelope, FuncId, Function, LocalId, Pc, Program};
use crate::features::{
    EngineFeature, HostCapability, ENGINE_FORMAT_VERSION, LANGUAGE_SEMANTICS_TS,
};
use crate::instruction::{ConstValue, Instruction};

const SEED_COUNT: u64 = 24;

pub fn seed_count() -> u64 {
    SEED_COUNT
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.0
    }

    fn pick(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n.max(1))) as u32
    }
}

pub fn generate(seed: u64) -> Artifact {
    let mut rng = Rng(seed | 1);
    match rng.pick(8) {
        0 => effect_then_wait(seed, 2),
        1 => branched_effects(seed),
        2 => loop_effect(seed),
        3 => try_finally_wait(seed),
        4 => last_use_then_wait(seed),
        5 => repeated_waits(seed),
        6 => sleep_then_effect(seed),
        _ => invoke_then_wait(seed),
    }
}

fn envelope(hash: String, instructions: &[Instruction]) -> Envelope {
    let mut engine = vec![EngineFeature(EngineFeature::TS_CONTROL_FLOW.into())];
    let mut host = vec![HostCapability(HostCapability::PERSIST_CHECKPOINT.into())];
    let mut add_engine = |id: &str| {
        if !engine.iter().any(|feature| feature.0 == id) {
            engine.push(EngineFeature(id.into()));
        }
    };
    let mut add_host = |id: &str| {
        if !host.iter().any(|cap| cap.0 == id) {
            host.push(HostCapability(id.into()));
        }
    };
    for instruction in instructions {
        match instruction {
            Instruction::Effect => {
                add_engine(EngineFeature::DURABLE_EFFECT);
                add_host(HostCapability::EFFECT);
            }
            Instruction::WaitForEvent => {
                add_engine(EngineFeature::DURABLE_WAIT_FOR_EVENT);
                add_host(HostCapability::EVENT);
            }
            Instruction::Sleep => {
                add_engine(EngineFeature::DURABLE_SLEEP);
                add_host(HostCapability::TIMER);
            }
            Instruction::Invoke { .. } => {
                add_engine(EngineFeature::DURABLE_INVOKE);
                add_host(HostCapability::CHILD);
            }
            Instruction::Fork { .. } | Instruction::JoinAll | Instruction::JoinAny => {
                add_engine(EngineFeature::DURABLE_CONCURRENT_GROUP);
            }
            Instruction::Throw | Instruction::PushTry { .. } | Instruction::PopTry => {
                add_engine(EngineFeature::EXCEPTIONS);
            }
            _ => {}
        }
    }
    Envelope {
        artifact_hash: hash,
        frontend_id: "generator".into(),
        frontend_version: "0.0.0".into(),
        language_semantics_version: LANGUAGE_SEMANTICS_TS.into(),
        engine_format_version: ENGINE_FORMAT_VERSION,
        required_engine_features: engine,
        required_host_capabilities: host,
        runtime_modules: Vec::new(),
    }
}

fn finish(seed: u64, local_count: u32, instructions: Vec<Instruction>) -> Artifact {
    let len = instructions.len();
    Artifact {
        envelope: envelope(format!("gen-{seed:016x}"), &instructions),
        program: Program {
            entry: FuncId(0),
            functions: vec![Function {
                id: FuncId(0),
                name: "run".into(),
                param_count: 0,
                local_count,
                instructions,
                spans: vec![None; len],
            }],
        },
    }
}

fn load_string(value: &str) -> Instruction {
    Instruction::LoadConst {
        value: ConstValue::String(value.into()),
    }
}

fn effect_then_wait(seed: u64, locals: u32) -> Artifact {
    finish(
        seed,
        locals,
        vec![
            load_string("generate"),
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(0) },
            load_string("approved"),
            Instruction::WaitForEvent,
            Instruction::StoreLocal { local: LocalId(1) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
        ],
    )
}

fn branched_effects(seed: u64) -> Artifact {
    // flag = effect("generate"); if (flag) { effect("taken") } else { effect("skipped") }; return
    finish(
        seed,
        2,
        vec![
            load_string("generate"),
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::JumpIfFalse { target: Pc(10) },
            load_string("taken"),
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(1) },
            Instruction::Jump { target: Pc(13) },
            Instruction::Nop,
            load_string("skipped"),
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(1) },
            Instruction::LoadLocal { local: LocalId(1) },
            Instruction::Return,
        ],
    )
}

fn loop_effect(seed: u64) -> Artifact {
    // go = 1; while (go) { x = effect("generate"); go = 0; return x }
    finish(
        seed,
        2,
        vec![
            Instruction::LoadConst {
                value: ConstValue::Number(1.0),
            },
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::JumpIfFalse { target: Pc(12) },
            load_string("generate"),
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(1) },
            Instruction::LoadConst {
                value: ConstValue::Number(0.0),
            },
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(1) },
            Instruction::Return,
            Instruction::Jump { target: Pc(2) },
            Instruction::LoadConst {
                value: ConstValue::Number(0.0),
            },
            Instruction::Return,
        ],
    )
}

fn try_finally_wait(seed: u64) -> Artifact {
    finish(
        seed,
        1,
        vec![
            Instruction::PushTry {
                catch: Pc(5),
                finally: None,
            },
            Instruction::LoadConst {
                value: ConstValue::String("boom".into()),
            },
            Instruction::Throw,
            Instruction::PopTry,
            Instruction::Jump { target: Pc(7) },
            Instruction::Pop,
            Instruction::PopTry,
            load_string("generate"),
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
        ],
    )
}

fn last_use_then_wait(seed: u64) -> Artifact {
    finish(
        seed,
        2,
        vec![
            load_string("payload"),
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Pop,
            load_string("approved"),
            Instruction::WaitForEvent,
            Instruction::StoreLocal { local: LocalId(1) },
            Instruction::LoadLocal { local: LocalId(1) },
            Instruction::Return,
        ],
    )
}

fn repeated_waits(seed: u64) -> Artifact {
    finish(
        seed,
        2,
        vec![
            load_string("approved"),
            Instruction::WaitForEvent,
            Instruction::StoreLocal { local: LocalId(0) },
            load_string("approved"),
            Instruction::WaitForEvent,
            Instruction::StoreLocal { local: LocalId(1) },
            Instruction::LoadLocal { local: LocalId(1) },
            Instruction::Return,
        ],
    )
}

fn sleep_then_effect(seed: u64) -> Artifact {
    finish(
        seed,
        1,
        vec![
            Instruction::LoadConst {
                value: ConstValue::Number(1.0),
            },
            Instruction::Sleep,
            Instruction::Pop,
            load_string("generate"),
            Instruction::Effect,
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
        ],
    )
}

fn invoke_then_wait(seed: u64) -> Artifact {
    finish(
        seed,
        1,
        vec![
            load_string("child"),
            Instruction::Invoke { arg_count: 0 },
            Instruction::StoreLocal { local: LocalId(0) },
            Instruction::LoadLocal { local: LocalId(0) },
            Instruction::Return,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::{validate, EngineCaps};

    #[test]
    fn generated_programs_validate() {
        for seed in 0..seed_count() {
            validate(&generate(seed), &EngineCaps::current()).unwrap();
        }
    }
}
