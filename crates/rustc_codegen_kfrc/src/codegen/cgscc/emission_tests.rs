//! Exercise the crate-wide graph source, actual compiler and relocation walk.
use super::{leaf, optimize, wrapper};
use crate::codegen::{CrateGraphs, CrateProvider};
use crate::emission::{self, CguJob, CompiledCgu};
use kfrc_body::{DataBinding, DataObject, DataRelocation};
use kfrc_object::SymbolBinding;
use kfrc_son::GraphProvider;
use std::collections::BTreeMap;
use std::sync::Once;

fn job(names: &[&str], roots: &[&str]) -> CguJob {
    CguJob {
        pending_functions: names
            .iter()
            .map(|name| ((*name).to_owned(), SymbolBinding::Local))
            .collect(),
        needed: roots.iter().map(|name| (*name).to_owned()).collect(),
        data_objects: Vec::new(),
    }
}

fn jobs() -> Vec<CguJob> {
    vec![
        job(&["private", "nested"], &[]),
        job(&["root", "private"], &["root"]),
    ]
}

fn compile(graphs: &CrateGraphs, threads: usize, jobs: Vec<CguJob>) -> Vec<CompiledCgu> {
    static JOBSERVER: Once = Once::new();
    JOBSERVER.call_once(|| {
        rustc_data_structures::jobserver::initialize_checked(|warning| eprintln!("{warning}"));
    });
    emission::compile(jobs, threads, graphs, &BTreeMap::new())
        .into_iter()
        .map(|result| result.unwrap())
        .collect()
}

fn names(cgu: &CompiledCgu) -> Vec<&str> {
    cgu.functions
        .iter()
        .map(|f| f.function.symbol.as_str())
        .collect()
}

fn object(cgu: &CompiledCgu) -> Vec<u8> {
    kfrc_object::write_object_with_data(&cgu.functions, &cgu.data_objects).unwrap()
}

#[test]
fn all_callers_use_one_prepared_body_and_a_cross_cgu_callee_has_one_owner() {
    let private = leaf("private");
    let nested = wrapper("nested", &private);
    let root = wrapper("root", &nested);
    let mut graphs = CrateGraphs::default();
    for graph in [private, nested, root] {
        graphs.bodies.insert(graph.symbol.clone(), graph);
    }
    graphs.internal.insert("private".into());
    graphs.no_inline.insert("private".into());
    graphs.lift_data(&mut jobs(), &mut BTreeMap::new()).unwrap();
    let summary = optimize(&mut graphs, &["root"], super::super::MAX_WORK);
    assert_eq!(summary.private_reference, 0);
    assert_eq!(super::fixture::calls(&graphs.bodies["root"]), ["private"]);
    graphs.infer_function_attrs();
    graphs.apply_function_attrs().unwrap();
    let provider = CrateProvider { all: &graphs };
    assert!(std::ptr::eq(
        provider.graph("root").unwrap(),
        provider.emission_graph("root").graph
    ));
    let serial = compile(&graphs, 1, jobs());
    let parallel = compile(&graphs, 8, jobs());
    assert_eq!(names(&serial[0]), ["private"]);
    assert_eq!(names(&serial[1]), ["root"]);
    assert_eq!(
        serial[0].functions[0].binding,
        SymbolBinding::Global { hidden: true }
    );
    for (serial, parallel) in serial.iter().zip(&parallel) {
        assert_eq!(object(serial), object(parallel));
    }
}

#[test]
fn exported_data_relocations_survive_while_orphan_constants_do_not_keep_functions_alive() {
    let private = leaf("private");
    let mut nested = leaf("nested");
    let table = DataObject {
        symbol: "table".into(),
        bytes: vec![0; 8],
        align: 8,
        binding: DataBinding::Global { hidden: true },
        relocations: vec![DataRelocation {
            offset: 0,
            target: "private".into(),
            addend: 0,
        }],
        ..Default::default()
    };
    nested.data_objects.push(table.clone());
    nested.data_objects.push(DataObject {
        symbol: "orphan".into(),
        bytes: vec![0; 8],
        align: 8,
        unnamed_address: true,
        relocations: vec![DataRelocation {
            offset: 0,
            target: "unreachable".into(),
            addend: 0,
        }],
        ..Default::default()
    });
    let root = wrapper("root", &nested);
    let mut graphs = CrateGraphs::default();
    for graph in [private, nested, root, leaf("unreachable")] {
        graphs.bodies.insert(graph.symbol.clone(), graph);
    }
    let mut jobs = jobs();
    jobs[0]
        .pending_functions
        .insert("unreachable".into(), SymbolBinding::Local);
    graphs.lift_data(&mut jobs, &mut BTreeMap::new()).unwrap();
    assert!(graphs.bodies.values().all(|g| g.data_objects.is_empty()));
    optimize(&mut graphs, &["root", "table"], super::super::MAX_WORK);
    let result = compile(&graphs, 1, jobs);
    assert_eq!(names(&result[0]), ["private"]);
    assert_eq!(names(&result[1]), ["root"]);
    assert_eq!(result[0].data_objects, [table]);
    assert!(result[1].data_objects.is_empty());
}
