//! Nested DST places retain logical owners through layout and native emission.
#[path = "../../kfrc-son/tests/support/dst.rs"]
mod dst;

use kfrc_body::Body;
use kfrc_son::{FunctionGraph, build_graph, lower_layout, verify_graph};
use kfrc_x86::lower_function;

fn lower(body: Body) -> FunctionGraph {
    let mut graph = build_graph(&body).unwrap();
    verify_graph(&mut graph).unwrap();
    lower_layout(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    verify_graph(&mut graph).unwrap_or_else(|error| panic!("{error}\n{}", graph.dump()));
    graph
}

#[test]
fn nested_slice_and_string_tail_addresses_reach_machine_lowering() {
    for body in [dst::nested_tail_body(true).0, dst::string_tail_body()] {
        let graph = lower(body);
        lower_function(&graph).unwrap_or_else(|error| panic!("{error:?}\n{}", graph.dump()));
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn native_nested_dst_places_keep_offsets_metadata_and_prefix_writes() {
    use kfrc_body::DataBinding;
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object_with_data};
    use std::process::Command;

    let mut functions = Vec::new();
    let mut data = Vec::new();
    for body in [dst::nested_tail_body(true).0, dst::string_tail_body()] {
        let mut graph = lower(body);
        // Expose the test allocation so C can vary contents and observe writes.
        for datum in &mut graph.data_objects {
            datum.binding = DataBinding::Global { hidden: true };
        }
        functions.push(FunctionCode {
            function: lower_function(&graph).unwrap(),
            binding: SymbolBinding::Global { hidden: false },
        });
        data.extend(graph.data_objects);
    }
    let directory = std::env::temp_dir().join(format!("kfrc-dst-places-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("main.c");
    let object = directory.join("dst.o");
    let executable = directory.join("check");
    std::fs::write(&object, write_object_with_data(&functions, &data).unwrap()).unwrap();
    std::fs::write(
        &source,
        r#"
#include <stdint.h>
extern uint64_t nested_tail(void), string_tail(void);
extern uint64_t nested_tail_data[6];
int main(void) {
    for (uint64_t seed = 0; seed < 256; ++seed) {
        for (unsigned i = 0; i < 6; ++i) nested_tail_data[i] = seed * 101 + i;
        if (nested_tail() != 35 + seed * 101 + 4) return 1;
        for (unsigned i = 0; i < 6; ++i)
            if (nested_tail_data[i] != (i == 1 ? 33 : seed * 101 + i)) return 2;
        if (string_tail() != 109) return 3;
    }
    return 0;
}
"#,
    )
    .unwrap();
    let linked = Command::new("cc")
        .arg("-O2")
        .arg(&source)
        .arg(&object)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let ran = Command::new(&executable).output().unwrap();
    assert!(
        ran.status.success(),
        "DST machine mismatch {:?}",
        ran.status
    );
    std::fs::remove_dir_all(directory).unwrap();
}
