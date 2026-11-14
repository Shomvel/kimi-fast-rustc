//! Shared byte-offset enum lanes survive machine lowering and match C.
#[path = "../../kfrc-son/tests/support/variant_lanes.rs"]
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
mod variant_lanes;

use kfrc_son::{FunctionGraph, build_graph, lower_layout, verify_graph};
use kfrc_x86::lower_function;
use variant_lanes::lanes_body;

fn layout(overlap: bool) -> FunctionGraph {
    let mut graph = build_graph(&lanes_body(overlap)).unwrap();
    verify_graph(&mut graph).unwrap();
    lower_layout(&mut graph).unwrap();
    verify_graph(&mut graph).unwrap_or_else(|e| panic!("{e}\n{}", graph.dump()));
    graph
}

// The machine evaluator has no memory model; Linux runs the C differential.
#[test]
fn shared_and_overlapping_lanes_lower_to_machine_code() {
    for overlap in [false, true] {
        lower_function(&layout(overlap)).unwrap();
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn shared_lane_machine_code_matches_reference() {
    use kfrc_x86::{FunctionCode, SymbolBinding, write_object};
    use std::fmt::Write;
    use std::process::Command;
    let functions: Vec<_> = [false, true]
        .map(|overlap| FunctionCode {
            function: lower_function(&layout(overlap)).unwrap(),
            binding: SymbolBinding::Global { hidden: false },
        })
        .into();
    let inputs = [
        (0u64, 0u64),
        (0x1ff, 0x1_2345_6789),
        (77, 0xfffe_dcba),
        (u64::MAX, u64::MAX),
    ];
    let mut cases = String::new();
    for overlap in [false, true] {
        for selector in 0..9u64 {
            for (a, bits) in inputs {
                let want = variant_lanes::expected(selector, a, bits, overlap);
                writeln!(
                    cases,
                    "    {{{}, {selector}ULL, {a}ULL, {bits}ULL, {want}ULL}},",
                    u8::from(overlap)
                )
                .unwrap();
            }
        }
    }
    let dir = std::env::temp_dir().join(format!("kfrc-variant-lanes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let object = dir.join("lanes.o");
    let source = dir.join("main.c");
    let exe = dir.join("check");
    std::fs::write(&object, write_object(&functions).unwrap()).unwrap();
    std::fs::write(
        &source,
        format!(
            r#"
#include <stdint.h>
#include <stdio.h>
extern uint64_t variant_lanes_0(uint64_t, uint64_t, uint64_t);
extern uint64_t variant_lanes_1(uint64_t, uint64_t, uint64_t);
struct case_ {{ int overlap; uint64_t selector, a, bits, want; }};
static const struct case_ cases[] = {{
{cases}}};
int main(void) {{
    for (unsigned i = 0; i < sizeof cases / sizeof cases[0]; i++) {{
        const struct case_ *c = &cases[i];
        uint64_t got = c->overlap ? variant_lanes_1(c->selector, c->a, c->bits)
                                  : variant_lanes_0(c->selector, c->a, c->bits);
        if (got != c->want) {{
            printf("case %u: got %llu want %llu\n", i, (unsigned long long)got,
                   (unsigned long long)c->want);
            return 1;
        }}
    }}
    return 0;
}}
"#
        ),
    )
    .unwrap();
    let linked = Command::new("cc")
        .args(["-O2", "-o"])
        .arg(&exe)
        .arg(&source)
        .arg(&object)
        .output()
        .unwrap();
    assert!(
        linked.status.success(),
        "{}",
        String::from_utf8_lossy(&linked.stderr)
    );
    let ran = Command::new(&exe).output().unwrap();
    assert!(
        ran.status.success(),
        "machine difference: {}",
        String::from_utf8_lossy(&ran.stdout)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
