use super::*;
use crate::{CallTarget, InterpretError, Interpreter, Outcome, verify_graph};
use kfrc_body::{FunctionAbi, IntegerWidth, PassMode, RecordedTypes, SizeAlign, TyId, TypeShape};

pub(super) struct Fixture {
    pub graph: FunctionGraph,
    pub word: TyId,
    pub boolean: TyId,
    pub control: NodeId,
    pub effect: NodeId,
    pub args: [NodeId; 3],
}

impl Fixture {
    pub fn new(width: IntegerWidth, signed: bool) -> Self {
        let mut types = RecordedTypes::new();
        let bytes = u64::from(width.bits(64) / 8);
        let word = types.intern(
            TypeShape::Int { width, signed },
            Some(SizeAlign {
                size: bytes,
                align: bytes.min(8),
            }),
        );
        let boolean = types.intern(TypeShape::Bool, Some(SizeAlign { size: 1, align: 1 }));
        let mut graph = FunctionGraph::new(
            "call_constants",
            FunctionAbi {
                args: vec![PassMode::Direct; 3],
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
                ValueType::Rust(word),
                ValueType::Rust(boolean),
            ]),
        );
        let start = graph.start;
        let control = projection(&mut graph, start, 0, ValueType::Control);
        let effect = projection(&mut graph, start, 1, ValueType::Effect);
        let args = std::array::from_fn(|slot| {
            let ty = if slot == 2 { boolean } else { word };
            projection(&mut graph, start, slot as u32 + 2, ValueType::Rust(ty))
        });
        Self {
            graph,
            word,
            boolean,
            control,
            effect,
            args,
        }
    }

    pub fn compare(&mut self, value: NodeId, constant: NodeId, predicate: P) -> NodeId {
        self.graph.add_node(
            Opcode::Compare,
            NodeId::NONE,
            &[value, constant],
            predicate as u32,
            ValueType::Rust(self.boolean),
        )
    }

    pub fn branch(&mut self, condition: NodeId) -> [NodeId; 2] {
        let branch = self.graph.add_node(
            Opcode::If,
            self.control,
            &[condition],
            0,
            ValueType::Tuple(vec![ValueType::Control; 2]),
        );
        std::array::from_fn(|slot| {
            projection(&mut self.graph, branch, slot as u32, ValueType::Control)
        })
    }

    pub fn fixed(
        &mut self,
        op: Opcode,
        operands: &[NodeId],
        payload: u32,
        result: Option<TyId>,
    ) -> (NodeId, Option<NodeId>) {
        let mut inputs = vec![self.effect];
        inputs.extend_from_slice(operands);
        let mut outputs = vec![ValueType::Control, ValueType::Effect];
        outputs.extend(result.map(ValueType::Rust));
        let node = self.graph.add_node(
            op,
            self.control,
            &inputs,
            payload,
            ValueType::Tuple(outputs),
        );
        self.control = projection(&mut self.graph, node, 0, ValueType::Control);
        self.effect = projection(&mut self.graph, node, 1, ValueType::Effect);
        let value = result.map(|ty| projection(&mut self.graph, node, 2, ValueType::Rust(ty)));
        (node, value)
    }

    pub fn call(&mut self, name: &str, inputs: &[NodeId]) -> (NodeId, NodeId) {
        let target = self.graph.add_callee(CallTarget {
            symbol: name.into(),
            abi: FunctionAbi {
                args: vec![PassMode::Direct; inputs.len()],
                ret: PassMode::Direct,
                c_variadic: false,
            },
        });
        let (call, result) = self.fixed(Opcode::Call, inputs, target, Some(self.word));
        (call, result.unwrap())
    }

    pub fn jump(&mut self, control: NodeId) -> NodeId {
        self.graph
            .add_node(Opcode::Jump, control, &[], 0, ValueType::Control)
    }

    pub fn ret(&mut self, control: NodeId, effect: NodeId, value: NodeId) {
        self.graph.add_node(
            Opcode::Return,
            control,
            &[effect, value],
            0,
            ValueType::Control,
        );
    }

    pub fn finish(&mut self, value: NodeId) {
        self.ret(self.control, self.effect, value);
        verify_graph(&mut self.graph).unwrap();
    }

    pub fn apply(&mut self, expected: usize) -> FunctionGraph {
        verify_graph(&mut self.graph).unwrap();
        let before = self.graph.clone();
        assert_eq!(
            refine(&mut self.graph, MAX_FACT_QUERIES)
                .unwrap()
                .arguments_refined,
            expected
        );
        verify_graph(&mut self.graph).unwrap();
        before
    }
}

pub(super) fn projection(
    graph: &mut FunctionGraph,
    source: NodeId,
    slot: u32,
    ty: ValueType,
) -> NodeId {
    graph.add_node(Opcode::Projection, NodeId::NONE, &[source], slot, ty)
}

pub(super) fn execute(
    graph: &FunctionGraph,
    args: [u64; 3],
) -> (Result<Outcome, InterpretError>, Vec<(String, Vec<u64>)>) {
    let mut calls = Vec::new();
    let outcome = Interpreter::new(graph).run(&args, &mut |name: &str, inputs: &[u64]| {
        calls.push((name.to_owned(), inputs.to_vec()));
        Ok(inputs
            .iter()
            .fold(11u64, |sum, input| sum.wrapping_add(*input)))
    });
    (outcome, calls)
}
