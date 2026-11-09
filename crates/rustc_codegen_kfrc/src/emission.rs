//! Follow actual relocations after semantic inlining, before lowering another
//! internal function. The source symbol table can contain erased calls, so it
//! is deliberately not a reachability root. Data remains conservative: every
//! emitted static, allocation and vtable relocation keeps its target alive.

use std::collections::BTreeSet;

use kfrc_body::DataObject;
use kfrc_x86::{MachineFunction, MachineInstr};

pub(crate) fn function_references(function: &MachineFunction, needed: &mut BTreeSet<String>) {
    for instruction in &function.instructions {
        let symbol = match instruction {
            MachineInstr::Call { target } => *target,
            MachineInstr::SymbolAddress { symbol, .. }
            | MachineInstr::ThreadLocalAddress { symbol, .. } => *symbol,
            _ => continue,
        };
        needed.insert(function.symbols[symbol as usize].clone());
    }
}

pub(crate) fn data_references(objects: &[DataObject], needed: &mut BTreeSet<String>) {
    needed.extend(objects.iter().flat_map(|object| {
        object
            .relocations
            .iter()
            .map(|relocation| relocation.target.clone())
    }));
}
