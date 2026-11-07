use kimi_fast_rustc::native_cache::{SUPPORTED_ABI, SUPPORTED_TARGET};
use kimi_fast_rustc::repro::{ManifestDiff, NativeBuildManifest, ReproChange};
use kimi_fast_rustc::son::{BinaryOp, Graph, NodeId, NodeKind};

const SOURCE: &[u8] = b"fn answer(x: i64) -> i64 { x * 6 + 7 }";
const BACKEND_V1: &str = "native-x86_64-elf-integration-v1";
const BACKEND_V2: &str = "native-x86_64-elf-integration-v2";
const OTHER_TARGET: &str = "aarch64-unknown-linux-gnu";

fn graph() -> Graph {
    let mut graph = Graph::new();
    let start = graph.start();
    let argument = graph.parameter(0);
    let six = graph.constant(6);
    let seven = graph.constant(7);
    let product = graph.binary(BinaryOp::Mul, argument, six);
    let result = graph.binary(BinaryOp::Add, product, seven);
    graph.return_(start, Some(result));
    graph
}

fn compile(graph: &Graph, target: &str, backend_version: &str) -> (NativeBuildManifest, Vec<u8>) {
    NativeBuildManifest::compile(
        SOURCE,
        graph,
        "answer",
        target,
        SUPPORTED_ABI,
        backend_version,
        Some(1_700_000_000),
    )
    .unwrap()
}

fn assert_only(diff: &ManifestDiff, expected_changed: &[bool]) {
    let actual = [
        diff.source_changed,
        diff.graph_changed,
        diff.target_changed,
        diff.abi_changed,
        diff.backend_version_changed,
        diff.elf_changed,
        diff.functions_changed,
    ];
    assert_eq!(actual, expected_changed);
}

#[test]
fn repeated_public_compilation_has_identical_manifest_elf_and_json() {
    let graph = graph();
    let (first, first_elf) = compile(&graph, SUPPORTED_TARGET, BACKEND_V1);
    let (second, second_elf) = compile(&graph, SUPPORTED_TARGET, BACKEND_V1);

    assert_eq!(first, second, "the complete manifests must match");
    assert_eq!(first_elf, second_elf, "the complete ELF objects must match");
    assert_eq!(first.to_json(), second.to_json());
    assert_eq!(&first_elf[..4], b"\x7fELF");

    let diff = first.compare(&second);
    assert_only(&diff, &[false; 7]);
    assert!(!diff.input_changed());
    assert!(!diff.backend_changed());
    assert!(!diff.output_changed());
    assert_eq!(diff.classification(), ReproChange::Identical);
}

#[test]
fn changed_graph_is_an_input_change() {
    let (first, first_elf) = compile(&graph(), SUPPORTED_TARGET, BACKEND_V1);
    let mut changed = graph();
    changed.node_mut(NodeId(2)).unwrap().kind = NodeKind::Const(8);
    let (second, second_elf) = compile(&changed, SUPPORTED_TARGET, BACKEND_V1);

    assert_eq!(first.source_sha256, second.source_sha256);
    let diff = first.compare(&second);
    assert_only(&diff, &[false, true, false, false, false, true, false]);
    assert!(diff.input_changed());
    assert!(!diff.backend_changed());
    assert_eq!(diff.classification(), ReproChange::InputChanged);
    assert_ne!(first_elf, second_elf);
}

#[test]
fn changed_target_is_a_backend_change_even_though_graph_key_changes() {
    let (first, first_elf) = compile(&graph(), SUPPORTED_TARGET, BACKEND_V1);
    let (second, second_elf) = compile(&graph(), OTHER_TARGET, BACKEND_V1);

    let diff = first.compare(&second);
    assert_only(&diff, &[false, true, true, false, false, false, false]);
    assert!(diff.backend_changed());
    assert_eq!(diff.classification(), ReproChange::BackendChanged);
    assert_eq!(first_elf, second_elf);
}

#[test]
fn changed_backend_version_is_a_backend_change() {
    let (first, first_elf) = compile(&graph(), SUPPORTED_TARGET, BACKEND_V1);
    let (second, second_elf) = compile(&graph(), SUPPORTED_TARGET, BACKEND_V2);

    let diff = first.compare(&second);
    assert_only(&diff, &[false, true, false, false, true, false, false]);
    assert!(diff.backend_changed());
    assert_eq!(diff.classification(), ReproChange::BackendChanged);
    assert_eq!(first_elf, second_elf);
}
