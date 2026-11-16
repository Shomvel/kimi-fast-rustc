use super::*;

pub(super) struct Fixture {
    pub graph: FunctionGraph,
    pub word: TyId,
    pub entry: NodeId,
    pub events: Vec<NodeId>,
    pub guards: Vec<NodeId>,
    pub failures: Vec<NodeId>,
    pub success_returns: Vec<NodeId>,
    pub success_regions: Vec<NodeId>,
}

fn projection(graph: &mut FunctionGraph, source: NodeId, slot: u32, ty: ValueType) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

/// Every compared buffer starts with these bytes; a literal matches the input
/// exactly when it keeps them.
const MATCH: u8 = b'a';
const MISMATCH: u8 = b'b';
const INPUT_SYMBOL: &str = "input";

fn literal_symbol(index: usize) -> String {
    format!("literal_{index}")
}

/// Defines a read-only data object and returns a node holding its address.
fn data_address(graph: &mut FunctionGraph, symbol: String, bytes: Vec<u8>, pointer: TyId) -> NodeId {
    graph.symbols.push((symbol.clone(), 0));
    graph.data_objects.push(DataObject {
        symbol,
        bytes,
        align: 1,
        ..Default::default()
    });
    graph.add_node(
        Opcode::SymbolAddress,
        NodeId::NONE,
        &[],
        graph.symbols.len() as u32 - 1,
        ValueType::Rust(pointer),
    )
}

/// Records the (literal address, length) pair of every executed comparison.
#[derive(Default)]
struct ComparisonTrace(Vec<(u64, u64)>);

impl CallHandler for ComparisonTrace {
    fn call(&mut self, symbol: &str, _arguments: &[u64]) -> Result<u64, InterpretError> {
        Err(InterpretError::UnknownCallee(symbol.to_owned()))
    }

    fn observe(&mut self, symbol: &str, arguments: &[u64]) {
        assert_eq!(symbol, RuntimeComparison::Bcmp.symbol());
        self.0.push((arguments[1], arguments[2]));
    }
}

fn wrap(graph: &mut FunctionGraph, control: NodeId, value: NodeId) -> (NodeId, NodeId) {
    let jump = graph.add_node(Opcode::Jump, control, &[], 0, ValueType::Control);
    let region = graph.add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
    let value = graph.add_node(
        Opcode::Phi,
        NodeId::NONE,
        &[region, value],
        0,
        graph.value_type(value).clone(),
    );
    (region, value)
}

impl Fixture {
    pub fn new(lengths: &[u64], byte_loads: bool, wrapped: bool) -> Self {
        let mut types = RecordedTypes::new();
        let byte = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W8,
                signed: false,
            },
            Some(SizeAlign { size: 1, align: 1 }),
        );
        let word = types.intern(
            TypeShape::Int {
                width: IntegerWidth::Pointer,
                signed: false,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let integer = types.intern(
            TypeShape::Int {
                width: IntegerWidth::W32,
                signed: true,
            },
            Some(SizeAlign { size: 4, align: 4 }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let pointer = types.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: byte,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let mut graph = FunctionGraph::new(
            "byte_ladder_fixture",
            FunctionAbi {
                args: vec![PassMode::Direct],
                ret: PassMode::Direct,
                c_variadic: false,
            },
            types,
        );
        graph.level = GraphLevel::Layout;
        graph.start = graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(word),
            ]),
        );
        let start = graph.start;
        let entry = projection(&mut graph, start, 0, ValueType::Control);
        let mut effect = projection(&mut graph, start, 1, ValueType::Effect);
        let length = projection(&mut graph, start, 2, ValueType::Rust(word));
        let mut control = entry;
        let mut events = Vec::new();
        let mut guards = Vec::new();
        let mut failures = Vec::new();
        let mut success_jumps = Vec::new();
        let mut success_effects = Vec::new();
        let left = data_address(&mut graph, INPUT_SYMBOL.to_owned(), vec![MATCH; 32], pointer);
        let target = graph.add_runtime_comparison(RuntimeComparison::Bcmp);
        let mut success_regions = Vec::new();
        let mut success_returns = Vec::new();
        for (index, &count) in lengths.iter().enumerate() {
            let count_node = graph.add_constant(u128::from(count), word);
            let comparison = graph.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &[length, count_node],
                ComparePredicate::Equal as u32,
                ValueType::Rust(boolean),
            );
            let guard = graph.add_node(
                Opcode::If,
                control,
                &[comparison],
                0,
                ValueType::Tuple(vec![ValueType::Control; 2]),
            );
            guards.push(guard);
            let selected = projection(&mut graph, guard, 0, ValueType::Control);
            let bypass = projection(&mut graph, guard, 1, ValueType::Control);
            let bypass = graph.add_node(Opcode::Jump, bypass, &[], 0, ValueType::Control);
            let is_byte = byte_loads && count == 1;
            let event = if is_byte {
                let address = data_address(&mut graph, format!("byte_{index}"), vec![b'x'], pointer);
                graph.add_node(
                    Opcode::Load,
                    selected,
                    &[effect, address],
                    0,
                    ValueType::Tuple(vec![
                        ValueType::Control,
                        ValueType::Effect,
                        ValueType::Rust(byte),
                    ]),
                )
            } else {
                let right = data_address(
                    &mut graph,
                    literal_symbol(index),
                    vec![MATCH; count as usize],
                    pointer,
                );
                graph.add_node(
                    Opcode::Call,
                    selected,
                    &[effect, left, right, count_node],
                    target,
                    ValueType::Tuple(vec![
                        ValueType::Control,
                        ValueType::Effect,
                        ValueType::Rust(integer),
                    ]),
                )
            };
            events.push(event);
            let mut after = projection(&mut graph, event, 0, ValueType::Control);
            let mut outgoing = projection(&mut graph, event, 1, ValueType::Effect);
            let mut result = projection(
                &mut graph,
                event,
                2,
                ValueType::Rust(if is_byte { byte } else { integer }),
            );
            if is_byte {
                result = graph.add_node(
                    Opcode::IntCast,
                    NodeId::NONE,
                    &[result],
                    0,
                    ValueType::Rust(integer),
                );
                // An alternating matching literal proves the retained read
                // continuation has both successful and failing byte paths.
                let literal = graph.add_constant(
                    if index % 2 == 0 {
                        u128::from(b'x')
                    } else {
                        u128::from(b'y')
                    },
                    integer,
                );
                result = graph.add_node(
                    Opcode::Sub,
                    NodeId::NONE,
                    &[result, literal],
                    0,
                    ValueType::Rust(integer),
                );
            }
            let zero = graph.add_constant(0, integer);
            let mut equality = graph.add_node(
                Opcode::Compare,
                NodeId::NONE,
                &[result, zero],
                ComparePredicate::Equal as u32,
                ValueType::Rust(boolean),
            );
            if wrapped {
                (after, outgoing) = wrap(&mut graph, after, outgoing);
                equality = graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[after, equality],
                    0,
                    ValueType::Rust(boolean),
                );
            }
            let test = graph.add_node(
                Opcode::If,
                after,
                &[equality],
                0,
                ValueType::Tuple(vec![ValueType::Control; 2]),
            );
            let yes = projection(&mut graph, test, 0, ValueType::Control);
            let no = projection(&mut graph, test, 1, ValueType::Control);
            success_jumps.push(graph.add_node(Opcode::Jump, yes, &[], 0, ValueType::Control));
            success_effects.push(outgoing);
            let mismatch = graph.add_node(Opcode::Jump, no, &[], 0, ValueType::Control);
            control = graph.add_node(
                Opcode::Region,
                NodeId::NONE,
                &[bypass, mismatch],
                0,
                ValueType::Control,
            );
            effect = graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &[control, effect, outgoing],
                0,
                ValueType::Effect,
            );
            failures.push(control);
        }
        // Each original success predecessor is stable even when several
        // lengths share this Region. A second effect-only target is used in
        // the unwrapped fixture for targeted negative boundary mutations.
        if wrapped {
            let region = graph.add_node(
                Opcode::Region,
                NodeId::NONE,
                &success_jumps,
                0,
                ValueType::Control,
            );
            let mut effect_inputs = vec![region];
            effect_inputs.extend(success_effects);
            let effect = graph.add_node(
                Opcode::Phi,
                NodeId::NONE,
                &effect_inputs,
                0,
                ValueType::Effect,
            );
            // Value Phi success targets are deliberately outside v1. Keep
            // effect-only joins by returning one fixed value for shared arms.
            let answer = graph.add_constant(1, word);
            let ret = graph.add_node(
                Opcode::Return,
                region,
                &[effect, answer],
                0,
                ValueType::Control,
            );
            success_regions.resize(lengths.len(), region);
            success_returns.resize(lengths.len(), ret);
        } else {
            for (index, (jump, effect)) in
                success_jumps.into_iter().zip(success_effects).enumerate()
            {
                let region =
                    graph.add_node(Opcode::Region, NodeId::NONE, &[jump], 0, ValueType::Control);
                let effect = graph.add_node(
                    Opcode::Phi,
                    NodeId::NONE,
                    &[region, effect],
                    0,
                    ValueType::Effect,
                );
                let answer = graph.add_constant(index as u128 + 1, word);
                let ret = graph.add_node(
                    Opcode::Return,
                    region,
                    &[effect, answer],
                    0,
                    ValueType::Control,
                );
                success_regions.push(region);
                success_returns.push(ret);
            }
        }
        let reject = graph.add_constant(0, word);
        graph.add_node(
            Opcode::Return,
            control,
            &[effect, reject],
            0,
            ValueType::Control,
        );
        graph.state = GraphState::Sealed;
        Self {
            graph,
            word,
            entry,
            events,
            guards,
            failures,
            success_returns,
            success_regions,
        }
    }

    /// Runs `graph` on `length`; bit `i` of `outcomes` makes cell `i`'s
    /// literal equal the input. Returns the outcome and the comparisons made.
    pub fn execute(
        graph: &FunctionGraph,
        length: u64,
        outcomes: u64,
    ) -> (Outcome, Vec<(u64, u64)>) {
        let mut graph = graph.clone();
        for object in &mut graph.data_objects {
            let Some(cell) = (0..64).find(|cell| object.symbol == literal_symbol(*cell)) else {
                continue;
            };
            let byte = if outcomes & (1 << cell) != 0 { MATCH } else { MISMATCH };
            object.bytes.fill(byte);
        }
        let mut trace = ComparisonTrace::default();
        let answer = Interpreter::new(&graph).run(&[length], &mut trace).unwrap();
        (answer, trace.0)
    }

    pub fn decline(&mut self) {
        let before = self.graph.dump();
        let callees = self.graph.callees.clone();
        let stats = transform(&mut self.graph, MAX_WORK);
        assert_eq!(stats.ladders_grouped, 0);
        assert_eq!(self.graph.dump(), before);
        assert_eq!(self.graph.callees, callees);
        assert!(self.graph.integer_switches.is_empty());
    }
}
