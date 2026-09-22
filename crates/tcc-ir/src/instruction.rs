use crate::artifact::{FuncId, LocalId, Pc};

/// Compile-time constants. Distinct from runtime `tcc-state` values so language
/// frontends can extend constants without collapsing language semantics.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
}

/// TypeScript-subset instruction surface.
/// Later frontends add instructions behind new required engine features.
#[derive(Debug, Clone, PartialEq)]
pub enum Instruction {
    Nop,
    Jump {
        target: Pc,
    },
    JumpIfTrue {
        target: Pc,
    },
    JumpIfFalse {
        target: Pc,
    },
    LoadLocal {
        local: LocalId,
    },
    StoreLocal {
        local: LocalId,
    },
    LoadConst {
        value: ConstValue,
    },
    Pop,
    NewObject,
    SetProp {
        key: String,
    },
    GetProp {
        key: String,
    },
    NewArray,
    ArrayPush,
    StrictEq,
    StrictNeq,
    Lt,
    Le,
    Gt,
    Ge,
    Not,
    Return,
    Call {
        func: FuncId,
        argc: u32,
    },
    Effect,
    Sleep,
    WaitForEvent,
    /// Pops `arg_count` values under the program name. Zero is omitted when encoded.
    Invoke {
        arg_count: u32,
    },
    /// Open a concurrent group. `join_pc` is the `JoinAll` or `JoinAny` that settles it.
    Fork {
        count: u32,
        join_pc: Pc,
    },
    /// Push the join aggregate in input order. Requires `durable.concurrent_group`.
    JoinAll,
    /// Push the winning branch value. Requires `durable.concurrent_group`.
    JoinAny,
    /// Copy `array[index]` onto the stack. The array stays underneath.
    ArrayIndex {
        index: u32,
    },
    Throw,
    PushTry {
        catch: Pc,
        finally: Option<Pc>,
    },
    PopTry,
}

impl Instruction {
    pub fn jump_targets(&self) -> impl Iterator<Item = Pc> {
        let mut targets = Vec::new();
        match self {
            Instruction::Jump { target }
            | Instruction::JumpIfTrue { target }
            | Instruction::JumpIfFalse { target } => targets.push(*target),
            Instruction::PushTry { catch, finally } => {
                targets.push(*catch);
                if let Some(pc) = finally {
                    targets.push(*pc);
                }
            }
            Instruction::Fork { join_pc, .. } => targets.push(*join_pc),
            _ => {}
        }
        targets.into_iter()
    }

    /// Locals read by this instruction. `LoadLocal` is the only use.
    pub fn uses_local(&self) -> Option<LocalId> {
        match self {
            Instruction::LoadLocal { local } => Some(*local),
            _ => None,
        }
    }

    /// Locals defined (killed) by this instruction. `StoreLocal` overwrites the slot.
    pub fn defs_local(&self) -> Option<LocalId> {
        match self {
            Instruction::StoreLocal { local } => Some(*local),
            _ => None,
        }
    }

    pub fn falls_through(&self) -> bool {
        !matches!(
            self,
            Instruction::Jump { .. } | Instruction::Return | Instruction::Throw
        )
    }
}
