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
}
