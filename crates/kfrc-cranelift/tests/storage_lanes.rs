//! Frame storage that only holds scalars at fixed offsets leaves no memory
//! traffic: the exported Cranelift IR carries its lanes as values.
use cranelift_codegen::ir;
use kfrc_body::{
    FieldLayout, FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape,
};
use kfrc_cranelift::{Machine, MachineOptions, export_function, lower_function};
use kfrc_son::lower_layout::LayoutOptions;
use kfrc_son::{
    CallTarget, ComparePredicate, FunctionGraph, GraphLevel, NodeId, Opcode, ValueType,
    verify_graph,
};

fn machine(promote_storage: bool) -> Machine {
    Machine::new(MachineOptions {
        verify: true,
        promote_storage,
        ..MachineOptions::default()
    })
    .unwrap()
}

fn clif(graph: &FunctionGraph, promote_storage: bool) -> String {
    let mut function = ir::Function::new();
    export_function(
        graph,
        &machine(promote_storage),
        LayoutOptions::default(),
        &mut function,
    )
    .unwrap();
    // Both configurations must also survive verification and compilation.
    lower_function(graph, &machine(promote_storage))
        .unwrap_or_else(|error| panic!("{error}\n{}", function.display()));
    function.display().to_string()
}

fn count(text: &str, pattern: &str) -> usize {
    text.matches(pattern).count()
}

/// Instructions of the given opcode, with or without a type suffix.
fn instructions(text: &str, opcode: &str) -> usize {
    text.lines()
        .filter(|line| {
            // Drop the trailing value comment, then the result list.
            let line = line.split(';').next().unwrap_or(line).trim_start();
            let line = line.split_once(" = ").map_or(line, |(_, rest)| rest);
            line.strip_prefix(opcode)
                .is_some_and(|rest| rest.starts_with(' ') || rest.starts_with('.'))
        })
        .count()
}

struct Types {
    table: RecordedTypes,
    word: TyId,
    boolean: TyId,
    pair: TyId,
    triple: TyId,
    word_place: TyId,
    pair_place: TyId,
    triple_place: TyId,
}

fn types() -> Types {
    let mut table = RecordedTypes::new();
    let word = table.intern(
        TypeShape::Int {
            width: IntegerWidth::W64,
            signed: false,
        },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let boolean = table.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
    let pair = table.intern(
        TypeShape::Tuple(vec![word, word]),
        Some(SizeAlign { size: 16, align: 8 }),
    );
    table.record_abi_pair(
        pair,
        [
            FieldLayout {
                ty: word,
                offset: 0,
            },
            FieldLayout {
                ty: word,
                offset: 8,
            },
        ],
    );
    let triple = table.intern(
        TypeShape::Tuple(vec![word, word, word]),
        Some(SizeAlign { size: 24, align: 8 }),
    );
    let word_place = table.intern(TypeShape::PlaceAddress { pointee: word }, None);
    let pair_place = table.intern(TypeShape::PlaceAddress { pointee: pair }, None);
    let triple_place = table.intern(TypeShape::PlaceAddress { pointee: triple }, None);
    Types {
        table,
        word,
        boolean,
        pair,
        triple,
        word_place,
        pair_place,
        triple_place,
    }
}

struct Builder {
    graph: FunctionGraph,
    control: NodeId,
    effect: NodeId,
    word: TyId,
    word_place: TyId,
}

impl Builder {
    fn new(symbol: &str, types: &Types, abi: FunctionAbi, parameters: Vec<ValueType>) -> Self {
        let mut graph = FunctionGraph::new(symbol, abi, types.table.clone());
        graph.level = GraphLevel::Layout;
        let mut tuple = vec![ValueType::Control, ValueType::Effect];
        tuple.extend(parameters);
        let start = graph.add_node(Opcode::Start, NodeId::NONE, &[], 0, ValueType::Tuple(tuple));
        graph.start = start;
        let control = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            0,
            ValueType::Control,
        );
        let effect = graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[start],
            1,
            ValueType::Effect,
        );
        Self {
            graph,
            control,
            effect,
            word: types.word,
            word_place: types.word_place,
        }
    }

    fn parameter(&mut self, index: u32, ty: ValueType) -> NodeId {
        let start = self.graph.start;
        self.graph
            .add_node(Opcode::Projection, NodeId::NONE, &[start], 2 + index, ty)
    }

    fn slot(&mut self, layout: SizeAlign, place: TyId) -> NodeId {
        let index = self.graph.stack_slots.len() as u32;
        self.graph.stack_slots.push(layout);
        self.graph.add_node(
            Opcode::StackSlot,
            NodeId::NONE,
            &[],
            index,
            ValueType::Rust(place),
        )
    }

    fn at(&mut self, base: NodeId, offset: u64) -> NodeId {
        let offset = self.graph.add_constant(u128::from(offset), self.word);
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[base, offset],
            0,
            ValueType::Rust(self.word_place),
        )
    }

    fn load(&mut self, base: NodeId, offset: u64) -> NodeId {
        self.load_as(base, offset, self.word)
    }

    fn load_as(&mut self, base: NodeId, offset: u64, ty: TyId) -> NodeId {
        let address = self.at(base, offset);
        let load = self.graph.add_node(
            Opcode::Load,
            self.control,
            &[self.effect, address],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        self.advance(load);
        self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[load],
            2,
            ValueType::Rust(ty),
        )
    }

    fn store(&mut self, base: NodeId, offset: u64, value: NodeId) {
        let address = self.at(base, offset);
        let store = self.graph.add_node(
            Opcode::Store,
            self.control,
            &[self.effect, address, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.advance(store);
    }

    fn advance(&mut self, fixed: NodeId) {
        self.control = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[fixed],
            0,
            ValueType::Control,
        );
        self.effect = self.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[fixed],
            1,
            ValueType::Effect,
        );
    }

    fn value_of(&mut self, base: NodeId, ty: TyId) -> NodeId {
        let zero = self.graph.add_constant(0, self.word);
        self.graph.add_node(
            Opcode::PtrOffset,
            NodeId::NONE,
            &[base, zero],
            0,
            ValueType::LayoutValue(ty),
        )
    }

    fn ret(&mut self, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            self.control,
            &[self.effect, value],
            0,
            ValueType::Control,
        );
    }

    fn finish(mut self) -> FunctionGraph {
        verify_graph(&mut self.graph)
            .unwrap_or_else(|error| panic!("{error}\n{}", self.graph.dump()));
        self.graph
    }
}

/// Mixed widths are accesses to the same object bytes, independent of the
/// ABI's two word lanes. The whole word store followed by a narrow write
/// must preserve every untouched byte without allocating frame storage.
#[test]
fn overlapping_widths_use_one_byte_representation() {
    for (width, size) in [
        (IntegerWidth::W8, 1),
        (IntegerWidth::W16, 2),
        (IntegerWidth::W32, 4),
    ] {
        for offset in 0..=16 - size {
            let mut types = types();
            let narrow = types.table.intern(
                TypeShape::Int {
                    width,
                    signed: false,
                },
                Some(SizeAlign { size, align: 1 }),
            );
            let mut builder = Builder::new(
                "mixed_widths",
                &types,
                FunctionAbi {
                    args: vec![PassMode::Pair, PassMode::Direct],
                    ret: PassMode::Pair,
                    c_variadic: false,
                },
                vec![ValueType::LayoutValue(types.pair), ValueType::Rust(narrow)],
            );
            let pair = builder.parameter(0, ValueType::LayoutValue(types.pair));
            let replacement = builder.parameter(1, ValueType::Rust(narrow));
            builder.store(pair, offset, replacement);
            // Read the narrow bytes as well as both complete ABI words.
            let observed = builder.load_as(pair, offset, narrow);
            builder.store(pair, offset, observed);
            builder.ret(pair);
            let graph = builder.finish();
            let text = clif(&graph, true);
            assert_eq!(instructions(&text, "load"), 0, "{offset} {size}\n{text}");
            assert_eq!(instructions(&text, "store"), 0, "{offset} {size}\n{text}");
            assert!(!text.contains("stack_addr"), "{offset} {size}\n{text}");
            clif(&graph, false);
        }
    }
}

/// An eleven-byte move between overlapping portions of a single received
/// pair snapshots source bytes first; it does not expose the pair's address.
#[test]
fn copies_can_overlap_and_cross_lane_boundaries() {
    let types = types();
    for reverse in [false, true] {
        let mut builder = Builder::new(
            "overlap",
            &types,
            pair_abi(PassMode::Pair),
            vec![ValueType::LayoutValue(types.pair)],
        );
        let pair = builder.parameter(0, ValueType::LayoutValue(types.pair));
        let first = builder.at(pair, 0);
        let shifted = builder.at(pair, 3);
        let (source, destination) = if reverse {
            (shifted, first)
        } else {
            (first, shifted)
        };
        let size = builder.graph.add_constant(11, types.word);
        let callee = builder.graph.add_callee(CallTarget {
            symbol: "memmove".into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct; 3],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let call = builder.graph.add_node(
            Opcode::Call,
            builder.control,
            &[builder.effect, destination, source, size],
            callee,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(types.word_place),
            ]),
        );
        builder.advance(call);
        builder.ret(pair);
        let text = clif(&builder.finish(), true);
        for opcode in ["load", "store", "call"] {
            assert_eq!(instructions(&text, opcode), 0, "{text}");
        }
        assert!(!text.contains("stack_addr"), "{text}");
    }
}

/// A whole indirect call result immediately returned has one destination:
/// the caller's result storage. No temporary stack object or copy is needed.
#[test]
fn an_indirect_result_is_built_in_its_final_destination() {
    let types = types();
    let mut builder = Builder::new(
        "forward_sret",
        &types,
        FunctionAbi {
            args: vec![],
            ret: PassMode::Indirect,
            c_variadic: false,
        },
        vec![],
    );
    let callee = builder.graph.add_callee(CallTarget {
        symbol: "produce".into(),
        abi: FunctionAbi {
            args: vec![],
            ret: PassMode::Indirect,
            c_variadic: false,
        },
    });
    let call = builder.graph.add_node(
        Opcode::Call,
        builder.control,
        &[builder.effect],
        callee,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::LayoutValue(types.triple),
        ]),
    );
    builder.advance(call);
    let result = builder.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        2,
        ValueType::LayoutValue(types.triple),
    );
    let returned = builder.value_of(result, types.triple);
    builder.ret(returned);
    let graph = builder.finish();
    for promote in [false, true] {
        let text = clif(&graph, promote);
        assert_eq!(instructions(&text, "call"), 1, "{text}");
        assert_eq!(instructions(&text, "load"), 0, "{text}");
        assert_eq!(instructions(&text, "store"), 0, "{text}");
        assert!(!text.contains("stack_addr"), "{text}");
    }
}

fn pair_abi(ret: PassMode) -> FunctionAbi {
    FunctionAbi {
        args: vec![PassMode::Pair],
        ret,
        c_variadic: false,
    }
}

/// `fn(p: (u64, u64)) -> (u64, u64)` returning the pair with swapped fields
/// through a frame slot.
fn swap() -> FunctionGraph {
    let types = types();
    let mut builder = Builder::new(
        "swap",
        &types,
        pair_abi(PassMode::Pair),
        vec![ValueType::LayoutValue(types.pair)],
    );
    let pair = builder.parameter(0, ValueType::LayoutValue(types.pair));
    let first = builder.load(pair, 0);
    let second = builder.load(pair, 8);
    let slot = builder.slot(SizeAlign { size: 16, align: 8 }, types.pair_place);
    builder.store(slot, 0, second);
    builder.store(slot, 8, first);
    let value = builder.value_of(slot, types.pair);
    builder.ret(value);
    builder.finish()
}

#[test]
fn a_pair_travels_from_registers_to_registers_without_memory() {
    let graph = swap();
    let promoted = clif(&graph, true);
    assert_eq!(count(&promoted, "stack_addr"), 0, "{promoted}");
    assert_eq!(instructions(&promoted, "load"), 0, "{promoted}");
    assert_eq!(instructions(&promoted, "store"), 0, "{promoted}");
    assert!(promoted.contains("return v1, v0"), "{promoted}");

    let memory = clif(&graph, false);
    assert_eq!(instructions(&memory, "load"), 4, "{memory}");
    assert_eq!(instructions(&memory, "store"), 4, "{memory}");
}

/// `fn(p: (u64, u64)) -> u64` passing the pair on to `callee(p)`.
#[test]
fn a_received_pair_is_passed_on_by_lanes() {
    let types = types();
    let mut builder = Builder::new(
        "forward",
        &types,
        pair_abi(PassMode::Direct),
        vec![ValueType::LayoutValue(types.pair)],
    );
    let pair = builder.parameter(0, ValueType::LayoutValue(types.pair));
    let callee = builder.graph.add_callee(CallTarget {
        symbol: "callee".into(),
        abi: pair_abi(PassMode::Direct),
    });
    let call = builder.graph.add_node(
        Opcode::Call,
        builder.control,
        &[builder.effect, pair],
        callee,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(types.word),
        ]),
    );
    builder.advance(call);
    let result = builder.graph.add_node(
        Opcode::Projection,
        NodeId::NONE,
        &[call],
        2,
        ValueType::Rust(types.word),
    );
    builder.ret(result);
    let graph = builder.finish();
    let promoted = clif(&graph, true);
    assert_eq!(count(&promoted, "stack_addr"), 0, "{promoted}");
    assert_eq!(instructions(&promoted, "load"), 0, "{promoted}");
    assert!(promoted.contains("(v0, v1)"), "{promoted}");
}

/// `fn(c: bool, x: u64) -> T`: each arm fills its own slot and a phi merges
/// the two addresses. `T` is three words returned through the caller's
/// storage, or two words returned in registers.
fn merged_arms(ret: PassMode) -> FunctionGraph {
    let types = types();
    let (ty, place, size, last) = if ret == PassMode::Indirect {
        (types.triple, types.triple_place, 24, 16)
    } else {
        (types.pair, types.pair_place, 16, 8)
    };
    let mut builder = Builder::new(
        "merged_arms",
        &types,
        FunctionAbi {
            args: vec![PassMode::Direct, PassMode::Direct],
            ret,
            c_variadic: false,
        },
        vec![ValueType::Rust(types.boolean), ValueType::Rust(types.word)],
    );
    let condition = builder.parameter(0, ValueType::Rust(types.boolean));
    let x = builder.parameter(1, ValueType::Rust(types.word));
    let one = builder.graph.add_constant(1, types.word);
    let two = builder.graph.add_constant(2, types.word);
    let branch = builder.graph.add_node(
        Opcode::If,
        builder.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let (entry_effect, layout) = (builder.effect, SizeAlign { size, align: 8 });
    let mut arms = Vec::new();
    for (index, parts) in [[(0, x), (8, one)], [(0, two), (last, x)]]
        .into_iter()
        .enumerate()
    {
        builder.control = builder.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            index as u32,
            ValueType::Control,
        );
        builder.effect = entry_effect;
        let slot = builder.slot(layout, place);
        for (offset, value) in parts {
            builder.store(slot, offset, value);
        }
        let jump =
            builder
                .graph
                .add_node(Opcode::Jump, builder.control, &[], 0, ValueType::Control);
        arms.push((jump, builder.effect, slot));
    }
    let region = builder.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[arms[0].0, arms[1].0],
        0,
        ValueType::Control,
    );
    builder.control = region;
    builder.effect = builder.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, arms[0].1, arms[1].1],
        0,
        ValueType::Effect,
    );
    let merged = builder.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, arms[0].2, arms[1].2],
        0,
        ValueType::Rust(place),
    );
    let value = builder.value_of(merged, ty);
    builder.ret(value);
    builder.finish()
}

#[test]
fn each_arm_writes_its_own_lanes_to_the_callers_storage() {
    let graph = merged_arms(PassMode::Indirect);
    let promoted = clif(&graph, true);
    assert_eq!(count(&promoted, "stack_addr"), 0, "{promoted}");
    assert_eq!(instructions(&promoted, "load"), 0, "{promoted}");
    // Two lanes per arm, written before the arms meet; the merge block only
    // returns the caller's pointer.
    assert_eq!(instructions(&promoted, "store"), 4, "{promoted}");
    assert!(promoted.contains("block3:\n    return v0"), "{promoted}");

    let memory = clif(&graph, false);
    assert_eq!(count(&memory, "stack_addr"), 2, "{memory}");
}

#[test]
fn merged_slots_returned_in_registers_become_block_parameters() {
    let graph = merged_arms(PassMode::Pair);
    let promoted = clif(&graph, true);
    assert_eq!(count(&promoted, "stack_addr"), 0, "{promoted}");
    assert_eq!(instructions(&promoted, "load"), 0, "{promoted}");
    assert_eq!(instructions(&promoted, "store"), 0, "{promoted}");
    assert!(promoted.contains("block3(v"), "{promoted}");
}

/// `fn(c: bool, x: u64) -> u64`: two slots merge, the merge is written
/// through, and one input is read again in the merge block. The read must
/// observe the write, so the storage stays in memory.
#[test]
fn an_input_read_behind_its_merge_keeps_the_storage_in_memory() {
    let types = types();
    let mut builder = Builder::new(
        "aliased_merge",
        &types,
        FunctionAbi {
            args: vec![PassMode::Direct, PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        vec![ValueType::Rust(types.boolean), ValueType::Rust(types.word)],
    );
    let condition = builder.parameter(0, ValueType::Rust(types.boolean));
    let x = builder.parameter(1, ValueType::Rust(types.word));
    let seven = builder.graph.add_constant(7, types.word);
    let branch = builder.graph.add_node(
        Opcode::If,
        builder.control,
        &[condition],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Control]),
    );
    let entry_effect = builder.effect;
    let mut arms = Vec::new();
    for index in 0..2 {
        builder.control = builder.graph.add_node(
            Opcode::Projection,
            NodeId::NONE,
            &[branch],
            index,
            ValueType::Control,
        );
        builder.effect = entry_effect;
        let slot = builder.slot(SizeAlign { size: 8, align: 8 }, types.word_place);
        builder.store(slot, 0, x);
        let jump =
            builder
                .graph
                .add_node(Opcode::Jump, builder.control, &[], 0, ValueType::Control);
        arms.push((jump, builder.effect, slot));
    }
    let region = builder.graph.add_node(
        Opcode::Region,
        NodeId::NONE,
        &[arms[0].0, arms[1].0],
        0,
        ValueType::Control,
    );
    builder.control = region;
    builder.effect = builder.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, arms[0].1, arms[1].1],
        0,
        ValueType::Effect,
    );
    let merged = builder.graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, arms[0].2, arms[1].2],
        0,
        ValueType::Rust(types.word_place),
    );
    builder.store(merged, 0, seven);
    let first = builder.load(arms[0].2, 0);
    builder.ret(first);
    let graph = builder.finish();
    let text = clif(&graph, true);
    // Both slots keep their memory and the merge is a pointer parameter.
    assert_eq!(count(&text, " = stack_addr"), 2, "{text}");
    assert_eq!(instructions(&text, "store"), 3, "{text}");
    assert_eq!(instructions(&text, "load"), 1, "{text}");
    assert!(text.contains("block3(v2: i64)"), "{text}");
}

/// A slot whose address is compared keeps its memory.
#[test]
fn storage_with_an_observed_address_stays_in_memory() {
    let types = types();
    let mut builder = Builder::new(
        "observed",
        &types,
        FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        },
        vec![ValueType::Rust(types.word)],
    );
    let x = builder.parameter(0, ValueType::Rust(types.word));
    let slot = builder.slot(SizeAlign { size: 8, align: 8 }, types.word_place);
    builder.store(slot, 0, x);
    let other = builder.slot(SizeAlign { size: 8, align: 8 }, types.word_place);
    let same = builder.graph.add_node(
        Opcode::Compare,
        NodeId::NONE,
        &[slot, other],
        ComparePredicate::Equal as u32,
        ValueType::Rust(types.boolean),
    );
    let widened = builder.graph.add_node(
        Opcode::IntCast,
        NodeId::NONE,
        &[same],
        0,
        ValueType::Rust(types.word),
    );
    let loaded = builder.load(slot, 0);
    let sum = builder.graph.add_node(
        Opcode::Add,
        NodeId::NONE,
        &[loaded, widened],
        0,
        ValueType::Rust(types.word),
    );
    builder.ret(sum);
    let graph = builder.finish();
    let text = clif(&graph, true);
    assert_eq!(instructions(&text, "load"), 1, "{text}");
    assert_eq!(instructions(&text, "store"), 1, "{text}");
}
