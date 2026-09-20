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
    Jump { target: Pc },
    JumpIfTrue { target: Pc },
    JumpIfFalse { target: Pc },
    LoadLocal { local: LocalId },
    StoreLocal { local: LocalId },
    LoadConst { value: ConstValue },
    Pop,
    NewObject,
    SetProp { key: String },
    GetProp { key: String },
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
    Call { func: FuncId, argc: u32 },
    Effect,
    Sleep,
    WaitForEvent,
    Invoke,
    Throw,
    PushTry { catch: Pc, finally: Option<Pc> },
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
