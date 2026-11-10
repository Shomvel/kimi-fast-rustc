//! Follow actual relocations after semantic inlining, before lowering another
//! internal function. The source symbol table can contain erased calls, so it
//! is deliberately not a reachability root. Data remains conservative: every
//! emitted static, allocation and vtable relocation keeps its target alive.

use std::collections::{BTreeMap, BTreeSet};

use crate::codegen::{self, CguGraphs, CompileError, CrateGraphs};
use kfrc_body::DataObject;
use kfrc_x86::{FunctionCode, MachineFunction, MachineInstr, SymbolBinding};

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

/// All rustc-dependent preparation is complete before a job enters the queue.
pub(crate) struct CguJob {
    pub available: BTreeSet<String>,
    pub pending_functions: BTreeMap<String, SymbolBinding>,
    pub needed: BTreeSet<String>,
    pub data_objects: Vec<DataObject>,
}

pub(crate) struct CompiledCgu {
    pub functions: Vec<FunctionCode>,
    pub data_objects: Vec<DataObject>,
    pub internal_omitted: usize,
}

type CompiledFunction = Result<(MachineFunction, Vec<DataObject>), CompileError>;

pub(crate) fn compile(
    mut jobs: Vec<CguJob>,
    threads: usize,
    graphs: &CrateGraphs,
    adapters: &BTreeMap<String, (MachineFunction, Vec<DataObject>)>,
) -> Vec<Result<CompiledCgu, CompileError>> {
    for job in &mut jobs {
        data_references(&job.data_objects, &mut job.needed);
    }
    let mut pending: Vec<_> = jobs.iter().map(|job| job.needed.clone()).collect();
    let mut compiled: Vec<BTreeMap<String, CompiledFunction>> =
        (0..jobs.len()).map(|_| BTreeMap::new()).collect();
    loop {
        // Discover work only from roots or actual machine/data relocations of
        // an already compiled body. This also exposes independent internal
        // functions in the same CGU, without compiling erased source callees.
        let mut ready = Vec::new();
        for (cgu, pending) in pending.iter_mut().enumerate() {
            while let Some(symbol) = pending.pop_first() {
                if jobs[cgu].pending_functions.contains_key(&symbol)
                    && !compiled[cgu].contains_key(&symbol)
                {
                    ready.push((cgu, symbol));
                }
            }
        }
        if ready.is_empty() {
            break;
        }
        let batch = crate::parallel::map(ready, threads, |(cgu, symbol)| {
            let provider = CguGraphs {
                all: graphs,
                available: &jobs[cgu].available,
            };
            let result = if let Some(adapter) = adapters.get(&symbol) {
                Ok(adapter.clone())
            } else {
                codegen::compile_function(graphs.bodies[&symbol].clone(), &provider)
            };
            (cgu, symbol, result)
        });
        for (cgu, symbol, result) in batch {
            if let Ok((function, objects)) = &result {
                function_references(function, &mut pending[cgu]);
                data_references(objects, &mut pending[cgu]);
            }
            compiled[cgu].insert(symbol, result);
        }
    }
    // Replay the original traversal to preserve function/data order, conflicts
    // and the first diagnostic. Errors in compiled functions are reported only
    // when their turn in that traversal is reached.
    jobs.into_iter()
        .zip(compiled)
        .map(|(job, functions)| assemble_cgu(job, functions))
        .collect()
}

fn assemble_cgu(
    mut job: CguJob,
    mut compiled: BTreeMap<String, CompiledFunction>,
) -> Result<CompiledCgu, CompileError> {
    let mut functions = Vec::new();
    // Keep the original ordered reachability walk. Compiling every source
    // function eagerly would restore internal bodies erased by optimization.
    while let Some(symbol) = job.needed.pop_first() {
        let Some(binding) = job.pending_functions.remove(&symbol) else {
            continue;
        };
        let (function, objects) = compiled
            .remove(&symbol)
            .expect("reachable function was compiled")?;
        function_references(&function, &mut job.needed);
        data_references(&objects, &mut job.needed);
        for object in objects {
            if let Some(existing) = job
                .data_objects
                .iter()
                .find(|existing| existing.symbol == object.symbol)
            {
                if existing != &object {
                    return Err(CompileError {
                        symbol,
                        message: format!("conflicting definitions of data `{}`", object.symbol),
                    });
                }
            } else {
                job.data_objects.push(object);
            }
        }
        functions.push(FunctionCode { function, binding });
    }
    Ok(CompiledCgu {
        functions,
        data_objects: job.data_objects,
        internal_omitted: job.pending_functions.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(names: &[&str], roots: &[&str]) -> CguJob {
        CguJob {
            available: names.iter().map(|s| s.to_string()).collect(),
            pending_functions: names
                .iter()
                .map(|s| (s.to_string(), SymbolBinding::Local))
                .collect(),
            needed: roots.iter().map(|s| s.to_string()).collect(),
            data_objects: Vec::new(),
        }
    }

    #[test]
    fn relocation_closure_preserves_walk_order_cycles_and_omissions() {
        let mut a = MachineFunction::new("a");
        a.symbols.push("b".into());
        a.instructions.push(MachineInstr::Call { target: 0 });
        let mut b = MachineFunction::new("b");
        b.symbols.push("a".into());
        b.instructions.push(MachineInstr::Call { target: 0 });
        let table = DataObject {
            symbol: "table".into(),
            bytes: vec![0; 8],
            align: 8,
            relocations: vec![kfrc_body::DataRelocation {
                offset: 0,
                target: "c".into(),
                addend: 0,
            }],
            ..Default::default()
        };
        let adapters = BTreeMap::from([
            ("a".into(), (a, vec![table.clone()])),
            ("b".into(), (b, Vec::new())),
            ("c".into(), (MachineFunction::new("c"), Vec::new())),
            ("z".into(), (MachineFunction::new("z"), Vec::new())),
        ]);
        // The erased function has no compilable body. Eagerly compiling all
        // source functions, or confusing source symbols with roots, fails here.
        let result = compile(
            vec![job(&["a", "b", "c", "dead", "z"], &["a", "z"])],
            1,
            &CrateGraphs::default(),
            &adapters,
        )
        .pop()
        .unwrap()
        .unwrap_or_else(|e| panic!("{}: {}", e.symbol, e.message));
        assert_eq!(
            result
                .functions
                .iter()
                .map(|f| f.function.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c", "z"]
        );
        assert_eq!(result.data_objects, vec![table]);
        assert_eq!(result.internal_omitted, 1);
    }

    #[test]
    fn diagnostic_follows_reachability_order_not_completion_order() {
        let mut root = MachineFunction::new("a");
        root.symbols.push("b".into());
        root.instructions.push(MachineInstr::Call { target: 0 });
        let compiled = BTreeMap::from([
            (
                "z".into(),
                Err(CompileError {
                    symbol: "z".into(),
                    message: "later root".into(),
                }),
            ),
            (
                "b".into(),
                Err(CompileError {
                    symbol: "b".into(),
                    message: "first dependency".into(),
                }),
            ),
            ("a".into(), Ok((root, Vec::new()))),
        ]);
        let result = assemble_cgu(job(&["a", "b", "z"], &["a", "z"]), compiled);
        let Err(error) = result else {
            panic!("expected dependency error")
        };
        assert_eq!(error.symbol, "b");
        assert_eq!(error.message, "first dependency");
    }
}
