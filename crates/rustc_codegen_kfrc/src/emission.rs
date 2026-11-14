//! Reachability and CGU ownership are decisions over the whole crate. Every
//! function is compiled once; live machine/data relocations decide emission.

use std::collections::{BTreeMap, BTreeSet};

use crate::codegen::{self, CompileError, CrateGraphs, CrateProvider};
use crate::machine::CompiledFunction;
use crate::program_data::{self, ProgramData};
use kfrc_body::{DataBinding, DataObject, EncodedFunction};
use kfrc_object::{ObjectFunction, SymbolBinding};

pub(crate) fn function_references(function: &EncodedFunction, needed: &mut BTreeSet<String>) {
    needed.extend(function.references().map(str::to_owned));
}

pub(crate) fn data_references(objects: &[DataObject], needed: &mut BTreeSet<String>) {
    needed.extend(objects.iter().flat_map(|object| {
        object
            .relocations
            .iter()
            .map(|relocation| relocation.target.clone())
    }));
}

pub(crate) struct CguJob {
    pub pending_functions: BTreeMap<String, SymbolBinding>,
    pub needed: BTreeSet<String>,
    pub data_objects: Vec<DataObject>,
}

#[derive(Default)]
pub(crate) struct CompiledCgu {
    pub functions: Vec<ObjectFunction>,
    pub data_objects: Vec<DataObject>,
    pub internal_omitted: usize,
}

fn failed(
    count: usize,
    owner: usize,
    error: CompileError,
) -> Vec<Result<CompiledCgu, CompileError>> {
    let mut result: Vec<_> = (0..count).map(|_| Ok(CompiledCgu::default())).collect();
    result[owner] = Err(error);
    result
}

fn function_owners(jobs: &[CguJob]) -> BTreeMap<String, (usize, SymbolBinding)> {
    let mut owners = BTreeMap::new();
    for (cgu, job) in jobs.iter().enumerate() {
        for (symbol, &binding) in &job.pending_functions {
            let entry = owners.entry(symbol.clone()).or_insert((cgu, binding));
            // The first CGU owns the sole definition. Preserve any public
            // declaration even if rustc also placed an internal copy elsewhere.
            match (entry.1, binding) {
                (_, SymbolBinding::Global { hidden: false }) => entry.1 = binding,
                (SymbolBinding::Local, SymbolBinding::Global { .. }) => entry.1 = binding,
                _ => {}
            }
        }
    }
    owners
}

pub(crate) fn compile(
    mut jobs: Vec<CguJob>,
    threads: usize,
    graphs: &CrateGraphs,
    adapters: &BTreeMap<String, CompiledFunction>,
) -> Vec<Result<CompiledCgu, CompileError>> {
    let owners = function_owners(&jobs);
    let mut data = graphs.data.clone();
    let mut pending = BTreeSet::new();
    for (cgu, job) in jobs.iter_mut().enumerate() {
        for object in std::mem::take(&mut job.data_objects) {
            if !object.unnamed_address || object.binding != DataBinding::Local {
                job.needed.insert(object.symbol.clone());
            }
            if let Err(error) = data.insert(object, cgu) {
                return failed(jobs.len(), cgu, error);
            }
        }
        pending.extend(job.needed.iter().cloned());
    }
    let mut compiled = BTreeMap::<String, EncodedFunction>::new();
    let mut live_data = BTreeSet::new();
    loop {
        let mut ready = BTreeSet::new();
        while let Some(symbol) = pending.pop_first() {
            if let Some(object) = data.objects.get(&symbol) {
                if live_data.insert(symbol) {
                    pending.extend(
                        object
                            .relocations
                            .iter()
                            .map(|relocation| relocation.target.clone()),
                    );
                }
            } else if owners.contains_key(&symbol) && !compiled.contains_key(&symbol) {
                ready.insert(symbol);
            }
        }
        if ready.is_empty() {
            break;
        }
        let batch = crate::parallel::map(ready.into_iter().collect(), threads, |symbol| {
            let provider = CrateProvider { all: graphs };
            let result = if let Some(adapter) = adapters.get(&symbol) {
                Ok(adapter.clone())
            } else {
                let start = provider.emission_graph(&symbol);
                codegen::compile_function(start.graph.clone(), &provider, start.inlining_complete)
            };
            (symbol, result)
        });
        for (symbol, result) in batch {
            let owner = owners[&symbol].0;
            let (function, objects) = match result {
                Ok(result) => result,
                Err(error) => return failed(jobs.len(), owner, error),
            };
            for object in objects {
                if object.binding != DataBinding::Local {
                    jobs[owner].needed.insert(object.symbol.clone());
                    pending.insert(object.symbol.clone());
                }
                if let Err(error) = data.insert(object, owner) {
                    return failed(jobs.len(), owner, error);
                }
            }
            function_references(&function, &mut pending);
            compiled.insert(symbol, function);
        }
    }
    // Layout-generated constants join the same pool. Rewrite the final code's
    // symbolic relocations before writing objects; machine bytes do not change.
    let aliases = data.canonicalize();
    for function in compiled.values_mut() {
        program_data::rewrite_code(function, &aliases);
    }
    for job in &mut jobs {
        program_data::rewrite_names(&mut job.needed, &aliases);
    }
    assemble(jobs, owners, compiled, data)
        .into_iter()
        .map(Ok)
        .collect()
}

fn assemble(
    jobs: Vec<CguJob>,
    owners: BTreeMap<String, (usize, SymbolBinding)>,
    compiled: BTreeMap<String, EncodedFunction>,
    data: ProgramData,
) -> Vec<CompiledCgu> {
    let mut pending: Vec<_> = jobs
        .iter()
        .map(|job| {
            job.needed
                .iter()
                .filter(|name| data.objects.contains_key(*name))
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .collect();
    let mut functions_promoted = BTreeSet::new();
    let mut data_promoted = BTreeSet::new();
    for (symbol, function) in &compiled {
        function_references(function, &mut pending[owners[symbol].0]);
    }
    let mut placed: Vec<BTreeSet<String>> = vec![BTreeSet::new(); jobs.len()];
    loop {
        let mut changed = false;
        for cgu in 0..jobs.len() {
            while let Some(symbol) = pending[cgu].pop_first() {
                changed = true;
                if let Some(object) = data.objects.get(&symbol) {
                    let owner = if program_data::anonymous(object) {
                        cgu
                    } else {
                        data.owners[&symbol]
                    };
                    if owner != cgu {
                        data_promoted.insert(symbol.clone());
                        pending[owner].insert(symbol);
                    } else if placed[cgu].insert(symbol) {
                        pending[cgu].extend(
                            object
                                .relocations
                                .iter()
                                .map(|relocation| relocation.target.clone()),
                        );
                    }
                } else if let Some(&(owner, _)) = owners.get(&symbol) {
                    if owner != cgu {
                        functions_promoted.insert(symbol);
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut result: Vec<_> = jobs
        .iter()
        .map(|job| CompiledCgu {
            internal_omitted: job.pending_functions.len(),
            ..Default::default()
        })
        .collect();
    for (symbol, function) in compiled {
        let (owner, mut binding) = owners[&symbol];
        if binding == SymbolBinding::Local && functions_promoted.contains(&symbol) {
            binding = SymbolBinding::Global { hidden: true };
        }
        result[owner]
            .functions
            .push(ObjectFunction { function, binding });
        result[owner].internal_omitted -= 1;
    }
    for (cgu, symbols) in placed.into_iter().enumerate() {
        for symbol in symbols {
            let mut object = data.objects[&symbol].clone();
            if object.binding == DataBinding::Local && data_promoted.contains(&symbol) {
                object.binding = DataBinding::Global { hidden: true };
            }
            result[cgu].data_objects.push(object);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(names: &[&str], roots: &[&str]) -> CguJob {
        CguJob {
            pending_functions: names
                .iter()
                .map(|s| (s.to_string(), SymbolBinding::Local))
                .collect(),
            needed: roots.iter().map(|s| s.to_string()).collect(),
            data_objects: Vec::new(),
        }
    }

    /// A function whose code calls each of `callees`.
    fn function(symbol: &str, callees: &[&str]) -> EncodedFunction {
        EncodedFunction {
            symbol: symbol.into(),
            bytes: vec![0; 5 * callees.len()],
            align: 16,
            relocations: callees
                .iter()
                .enumerate()
                .map(|(index, callee)| kfrc_body::CodeRelocation {
                    offset: 5 * index as u64 + 1,
                    target: (*callee).into(),
                    addend: -4,
                    kind: kfrc_body::CodeRelocationKind::Call,
                })
                .collect(),
        }
    }

    #[test]
    fn relocation_closure_preserves_walk_order_cycles_and_omissions() {
        let a = function("a", &["b"]);
        let b = function("b", &["a"]);
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
            ("c".into(), (function("c", &[]), Vec::new())),
            ("z".into(), (function("z", &[]), Vec::new())),
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
                .map(|f| f.function.symbol.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "z"]
        );
        assert!(result.data_objects.is_empty());
        assert_eq!(result.internal_omitted, 2);
    }

    #[test]
    fn cross_cgu_dependencies_have_one_hidden_owner_and_dead_data_is_omitted() {
        let mut root = function("root", &["private", "table"]);
        root.relocations[1].kind = kfrc_body::CodeRelocationKind::PcRelative;
        let table = DataObject {
            symbol: "table".into(),
            bytes: vec![0; 8],
            align: 8,
            relocations: vec![kfrc_body::DataRelocation {
                offset: 0,
                target: "callback".into(),
                addend: 0,
            }],
            ..Default::default()
        };
        let adapters = BTreeMap::from([
            ("root".into(), (root, vec![table])),
            ("private".into(), (function("private", &[]), Vec::new())),
            ("callback".into(), (function("callback", &[]), Vec::new())),
        ]);
        let mut jobs = vec![
            job(&["private", "callback"], &[]),
            job(&["root", "private"], &["root"]),
        ];
        jobs[0].data_objects.push(DataObject {
            symbol: "table".into(),
            bytes: vec![0; 8],
            align: 8,
            relocations: vec![kfrc_body::DataRelocation {
                offset: 0,
                target: "callback".into(),
                addend: 0,
            }],
            ..Default::default()
        });
        let result: Vec<_> = compile(jobs, 1, &CrateGraphs::default(), &adapters)
            .into_iter()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            result[0]
                .functions
                .iter()
                .map(|f| f.function.symbol.as_str())
                .collect::<Vec<_>>(),
            ["callback", "private"]
        );
        assert_eq!(
            result[1]
                .functions
                .iter()
                .map(|f| f.function.symbol.as_str())
                .collect::<Vec<_>>(),
            ["root"]
        );
        let private = result[0]
            .functions
            .iter()
            .find(|f| f.function.symbol == "private")
            .unwrap();
        assert_eq!(private.binding, SymbolBinding::Global { hidden: true });
        assert_eq!(
            result[0].data_objects[0].binding,
            DataBinding::Global { hidden: true }
        );
        assert!(result[1].data_objects.is_empty());
    }
}
