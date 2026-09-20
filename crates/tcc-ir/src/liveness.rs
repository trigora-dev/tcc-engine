//! Backward liveness of function-level local slots.
//!
//! Used by `tcc-core` at persist time so dead slots become `undefined` in place.
//! Join is union (conservative). Frontends still allocate function-level slots.

use std::collections::HashMap;

use crate::artifact::{FuncId, Function, Program};
use crate::instruction::Instruction;

/// Packed live-slot bitset for one program point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSet {
    bits: Vec<u64>,
    nbits: u32,
}

impl LiveSet {
    pub fn new(local_count: u32) -> Self {
        let words = (local_count as usize).div_ceil(64);
        Self {
            bits: vec![0; words],
            nbits: local_count,
        }
    }

    pub fn local_count(&self) -> u32 {
        self.nbits
    }

    pub fn contains(&self, slot: u32) -> bool {
        if slot >= self.nbits {
            return false;
        }
        let (word, mask) = Self::word_mask(slot);
        self.bits.get(word).is_some_and(|bits| bits & mask != 0)
    }

    pub fn insert(&mut self, slot: u32) -> bool {
        if slot >= self.nbits {
            return false;
        }
        let (word, mask) = Self::word_mask(slot);
        let bits = &mut self.bits[word];
        let changed = *bits & mask == 0;
        *bits |= mask;
        changed
    }

    pub fn remove(&mut self, slot: u32) {
        if slot >= self.nbits {
            return;
        }
        let (word, mask) = Self::word_mask(slot);
        self.bits[word] &= !mask;
    }

    pub fn union(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (left, right) in self.bits.iter_mut().zip(other.bits.iter()) {
            let next = *left | *right;
            if next != *left {
                changed = true;
                *left = next;
            }
        }
        changed
    }

    pub fn slots(&self) -> Vec<u32> {
        (0..self.nbits)
            .filter(|&slot| self.contains(slot))
            .collect()
    }

    fn word_mask(slot: u32) -> (usize, u64) {
        ((slot as usize) / 64, 1u64 << (slot % 64))
    }
}

/// Live-in sets for every instruction of one function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionLiveness {
    live_in: Vec<LiveSet>,
    empty: LiveSet,
}

impl FunctionLiveness {
    pub fn live_in_at(&self, pc: u32) -> &LiveSet {
        self.live_in.get(pc as usize).unwrap_or(&self.empty)
    }

    pub fn is_live_at(&self, pc: u32, slot: u32) -> bool {
        self.live_in_at(pc).contains(slot)
    }
}

pub fn analyze_program(program: &Program) -> HashMap<FuncId, FunctionLiveness> {
    program
        .functions
        .iter()
        .map(|function| (function.id, analyze_function(function)))
        .collect()
}

pub fn analyze_function(function: &Function) -> FunctionLiveness {
    let local_count = function.local_count;
    let empty = LiveSet::new(local_count);
    let instructions = &function.instructions;
    let len = instructions.len();
    if len == 0 {
        return FunctionLiveness {
            live_in: Vec::new(),
            empty,
        };
    }

    let successors = successors(instructions);
    let mut live_in = vec![LiveSet::new(local_count); len];
    let mut changed = true;
    while changed {
        changed = false;
        for pc in (0..len).rev() {
            let mut live_out = LiveSet::new(local_count);
            for &succ in &successors[pc] {
                live_out.union(&live_in[succ]);
            }
            if let Some(local) = instructions[pc].defs_local() {
                live_out.remove(local.0);
            }
            if let Some(local) = instructions[pc].uses_local() {
                live_out.insert(local.0);
            }
            if live_out != live_in[pc] {
                live_in[pc] = live_out;
                changed = true;
            }
        }
    }

    FunctionLiveness { live_in, empty }
}

fn successors(instructions: &[Instruction]) -> Vec<Vec<usize>> {
    let len = instructions.len();
    let regions = try_regions(instructions);
    let mut succs = Vec::with_capacity(len);
    for (pc, instruction) in instructions.iter().enumerate() {
        let mut next = normal_successors(instruction, pc, len);
        for region in &regions {
            if pc >= region.start && pc <= region.end {
                push_unique(&mut next, region.catch as usize);
                if let Some(finally) = region.finally {
                    push_unique(&mut next, finally as usize);
                }
            }
        }
        succs.push(next);
    }
    succs
}

fn normal_successors(instruction: &Instruction, pc: usize, len: usize) -> Vec<usize> {
    match instruction {
        Instruction::Jump { target } => vec![target.0 as usize],
        Instruction::JumpIfTrue { target } | Instruction::JumpIfFalse { target } => {
            let mut next = vec![target.0 as usize];
            if pc + 1 < len {
                next.push(pc + 1);
            }
            next
        }
        _ if !instruction.falls_through() => Vec::new(),
        _ if pc + 1 < len => vec![pc + 1],
        _ => Vec::new(),
    }
}

struct TryRegion {
    start: usize,
    end: usize,
    catch: u32,
    finally: Option<u32>,
}

fn try_regions(instructions: &[Instruction]) -> Vec<TryRegion> {
    let mut open = Vec::new();
    let mut regions = Vec::new();
    for (pc, instruction) in instructions.iter().enumerate() {
        match instruction {
            Instruction::PushTry { catch, finally } => {
                open.push((pc, catch.0, finally.map(|pc| pc.0)));
            }
            Instruction::PopTry => {
                if let Some((start, catch, finally)) = open.pop() {
                    regions.push(TryRegion {
                        start,
                        end: pc,
                        catch,
                        finally,
                    });
                }
            }
            _ => {}
        }
    }
    for (start, catch, finally) in open {
        regions.push(TryRegion {
            start,
            end: instructions.len().saturating_sub(1),
            catch,
            finally,
        });
    }
    regions
}

fn push_unique(targets: &mut Vec<usize>, target: usize) {
    if !targets.contains(&target) {
        targets.push(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{FuncId, LocalId, Pc};
    use crate::instruction::ConstValue;

    fn func(local_count: u32, instructions: Vec<Instruction>) -> Function {
        let spans = vec![None; instructions.len()];
        Function {
            id: FuncId(0),
            name: "main".into(),
            param_count: 0,
            local_count,
            instructions,
            spans,
        }
    }

    fn live(function: &Function, pc: u32) -> Vec<u32> {
        analyze_function(function).live_in_at(pc).slots()
    }

    fn str_const(text: &str) -> Instruction {
        Instruction::LoadConst {
            value: ConstValue::String(text.into()),
        }
    }

    #[test]
    fn last_use_kills_slot_before_later_wait() {
        let function = func(
            1,
            vec![
                str_const("v"),
                Instruction::StoreLocal { local: LocalId(0) },
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::Pop,
                str_const("go"),
                Instruction::WaitForEvent,
                Instruction::Nop,
                Instruction::Return,
            ],
        );
        assert_eq!(live(&function, 2), vec![0]);
        assert!(live(&function, 6).is_empty());
    }

    #[test]
    fn store_without_load_is_dead_at_next_pc() {
        let function = func(
            1,
            vec![
                str_const("v"),
                Instruction::StoreLocal { local: LocalId(0) },
                Instruction::Nop,
                Instruction::Return,
            ],
        );
        assert!(live(&function, 2).is_empty());
    }

    #[test]
    fn live_in_at_wait_resume_pc_keeps_later_use() {
        let function = func(
            2,
            vec![
                str_const("keep"),
                Instruction::StoreLocal { local: LocalId(0) },
                str_const("go"),
                Instruction::WaitForEvent,
                Instruction::StoreLocal { local: LocalId(1) },
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::Return,
            ],
        );
        assert_eq!(live(&function, 4), vec![0]);
        assert!(!analyze_function(&function).is_live_at(4, 1));
    }

    #[test]
    fn branch_keeps_arm_local_only_on_that_arm() {
        let function = func(
            3,
            vec![
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::JumpIfFalse { target: Pc(6) },
                Instruction::LoadLocal { local: LocalId(1) },
                Instruction::Pop,
                Instruction::Jump { target: Pc(8) },
                Instruction::Nop,
                Instruction::LoadLocal { local: LocalId(2) },
                Instruction::Pop,
                Instruction::Return,
            ],
        );
        assert_eq!(live(&function, 2), vec![1]);
        assert_eq!(live(&function, 6), vec![2]);
        let at_flag = live(&function, 0);
        assert!(at_flag.contains(&0));
        assert!(at_flag.contains(&1));
        assert!(at_flag.contains(&2));
    }

    #[test]
    fn loop_carried_slot_stays_live_at_header() {
        let function = func(
            1,
            vec![
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::JumpIfFalse { target: Pc(5) },
                Instruction::Nop,
                Instruction::Jump { target: Pc(0) },
                Instruction::Nop,
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::Return,
            ],
        );
        assert_eq!(live(&function, 0), vec![0]);
        assert_eq!(live(&function, 2), vec![0]);
        assert_eq!(live(&function, 5), vec![0]);
    }

    #[test]
    fn try_catch_keeps_slot_used_in_handler() {
        let function = func(
            1,
            vec![
                Instruction::PushTry {
                    catch: Pc(5),
                    finally: None,
                },
                Instruction::Nop,
                Instruction::WaitForEvent,
                Instruction::Throw,
                Instruction::PopTry,
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::Return,
            ],
        );
        assert_eq!(live(&function, 2), vec![0]);
        assert_eq!(live(&function, 1), vec![0]);
    }

    #[test]
    fn slot_dead_after_finally_is_not_live_at_handler() {
        let function = func(
            2,
            vec![
                Instruction::PushTry {
                    catch: Pc(5),
                    finally: Some(Pc(7)),
                },
                Instruction::StoreLocal { local: LocalId(0) },
                Instruction::Nop,
                Instruction::PopTry,
                Instruction::Jump { target: Pc(7) },
                Instruction::StoreLocal { local: LocalId(1) },
                Instruction::Jump { target: Pc(7) },
                Instruction::WaitForEvent,
                Instruction::Return,
            ],
        );
        assert!(live(&function, 7).is_empty());
        assert!(!analyze_function(&function).is_live_at(2, 0));
    }

    #[test]
    fn join_is_union_across_if_arms() {
        let function = func(
            2,
            vec![
                Instruction::JumpIfFalse { target: Pc(3) },
                Instruction::LoadLocal { local: LocalId(0) },
                Instruction::Jump { target: Pc(4) },
                Instruction::LoadLocal { local: LocalId(1) },
                Instruction::Return,
            ],
        );
        let header = live(&function, 0);
        assert!(header.contains(&0));
        assert!(header.contains(&1));
    }
}
