use crate::fixture::{Function, projection};
use kfrc_body::{
    AggregateLayout, FieldLayout, FunctionAbi, IntegerWidth, Mutability, PassMode,
    SemanticAggregate, SizeAlign, TyId, TypeShape,
};
use kfrc_son::{
    ArgPermission, FunctionGraph, InterpretError, Interpreter, NodeId, Opcode, SemanticCallHandler,
    SemanticMemory, SemanticOutcome, Value, ValueType, promote_arguments, verify_graph,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Word,
    Pointer,
    Reference,
    Str,
    Slice,
}

impl Kind {
    pub fn wide(self) -> bool {
        matches!(self, Self::Str | Self::Slice)
    }
}

pub fn enabled() -> bool {
    std::env::var("KFRC_PROTECTED_ARGUMENT_PROMOTION").as_deref() != Ok("0")
}

pub fn load(f: &mut Function, address: NodeId, ty: TyId) -> NodeId {
    let load = f.graph.add_node(
        Opcode::LoadValue,
        f.control,
        &[f.effect, address],
        0,
        ValueType::Tuple(vec![
            ValueType::Control,
            ValueType::Effect,
            ValueType::Rust(ty),
        ]),
    );
    f.control = projection(&mut f.graph, load, 0, ValueType::Control);
    f.effect = projection(&mut f.graph, load, 1, ValueType::Effect);
    projection(&mut f.graph, load, 2, ValueType::Rust(ty))
}

pub fn store(f: &mut Function, address: NodeId, value: NodeId) {
    let store = f.graph.add_node(
        Opcode::StoreValue,
        f.control,
        &[f.effect, address, value],
        0,
        ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
    );
    f.control = projection(&mut f.graph, store, 0, ValueType::Control);
    f.effect = projection(&mut f.graph, store, 1, ValueType::Effect);
}

pub struct Fixture {
    pub f: Function,
    pub kind: Kind,
    pub container: TyId,
    pub root_ref: TyId,
    pub address: NodeId,
    pub first: NodeId,
    pub second: NodeId,
    pub touch: NodeId,
}

impl Fixture {
    pub fn new(kind: Kind, frozen: bool, partial: bool) -> Self {
        let mut f = Function::new("protected_reader", 2);
        let pointer = f.graph.oracle.intern(
            TypeShape::RawPointer {
                mutability: Mutability::Not,
                pointee: f.word,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let snapshot = match kind {
            Kind::Word => f.word,
            Kind::Pointer => pointer,
            Kind::Reference => f.graph.oracle.intern(
                TypeShape::Reference {
                    mutability: Mutability::Not,
                    pointee: f.word,
                },
                Some(SizeAlign { size: 8, align: 8 }),
            ),
            Kind::Str | Kind::Slice => {
                let pointee = match kind {
                    Kind::Str => f.graph.oracle.intern(TypeShape::Str, None),
                    _ => f
                        .graph
                        .oracle
                        .intern(TypeShape::Slice { element: f.word }, None),
                };
                let wide = f.graph.oracle.intern(
                    TypeShape::Reference {
                        mutability: Mutability::Not,
                        pointee,
                    },
                    Some(SizeAlign { size: 16, align: 8 }),
                );
                let metadata = f.graph.oracle.intern(
                    TypeShape::Int {
                        width: IntegerWidth::Pointer,
                        signed: false,
                    },
                    Some(SizeAlign { size: 8, align: 8 }),
                );
                f.graph.oracle.record_semantic_aggregate(
                    wide,
                    SemanticAggregate {
                        fields: vec![pointer, metadata],
                        variants: vec![],
                    },
                );
                f.graph.oracle.record_aggregate(
                    wide,
                    AggregateLayout {
                        fields: vec![
                            FieldLayout {
                                ty: pointer,
                                offset: 0,
                            },
                            FieldLayout {
                                ty: metadata,
                                offset: 8,
                            },
                        ],
                        variants: vec![],
                    },
                );
                wide
            }
        };
        let bytes = if kind.wide() { 24 } else { 16 };
        let container = f.graph.oracle.intern(
            TypeShape::Tuple(vec![f.word, snapshot]),
            Some(SizeAlign {
                size: bytes,
                align: 8,
            }),
        );
        f.graph.oracle.record_semantic_aggregate(
            container,
            SemanticAggregate {
                fields: vec![f.word, snapshot],
                variants: vec![],
            },
        );
        f.graph.oracle.record_aggregate(
            container,
            AggregateLayout {
                fields: vec![
                    FieldLayout {
                        ty: f.word,
                        offset: 0,
                    },
                    FieldLayout {
                        ty: snapshot,
                        offset: 8,
                    },
                ],
                variants: vec![],
            },
        );
        let root_ref = f.graph.oracle.intern(
            TypeShape::Reference {
                pointee: container,
                mutability: if frozen {
                    Mutability::Not
                } else {
                    Mutability::Mut
                },
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let out_ref = f.graph.oracle.intern(
            TypeShape::Reference {
                pointee: f.word,
                mutability: Mutability::Mut,
            },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let ValueType::Tuple(mut params) = f.graph.value_type(f.graph.start).clone() else {
            unreachable!()
        };
        for (index, ty) in [root_ref, out_ref].into_iter().enumerate() {
            params[index + 2] = ValueType::Rust(ty);
            f.graph.set_value_type(f.args[index], ValueType::Rust(ty));
        }
        f.graph
            .set_value_type(f.graph.start, ValueType::Tuple(params));
        f.graph.arg_permissions = vec![
            if frozen {
                ArgPermission::Frozen {
                    dereferenceable: Some(bytes),
                }
            } else {
                ArgPermission::Unique {
                    dereferenceable: Some(bytes),
                }
            },
            ArgPermission::Unique {
                dereferenceable: Some(8),
            },
        ];
        let place = f.graph.oracle.intern(
            TypeShape::PlaceAddress { pointee: container },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let root = f.node(Opcode::PointerCast, &[f.args[0]], 0, place);
        let leaf = f.graph.oracle.intern(
            TypeShape::PlaceAddress { pointee: snapshot },
            Some(SizeAlign { size: 8, align: 8 }),
        );
        let address = f.node(Opcode::FieldAddr, &[root], 1, leaf);
        let first = load(&mut f, address, snapshot);
        let observed = observe(&mut f, first, kind, partial);
        let nineteen = f.constant(19);
        let out = f.args[1];
        store(&mut f, out, nineteen);
        let abi = FunctionAbi {
            args: vec![PassMode::Direct],
            ret: PassMode::Direct,
            c_variadic: false,
        };
        let touched = f.call_named("touch_other", &abi, &[out]);
        let touch = f.graph.input(touched, 0);
        let second = load(&mut f, address, snapshot);
        let later = observe(&mut f, second, kind, partial);
        let sum = f.node(Opcode::Add, &[observed, later], 0, f.word);
        f.ret(sum);
        f.verify();
        Self {
            f,
            kind,
            container,
            root_ref,
            address,
            first,
            second,
            touch,
        }
    }

    pub fn graphs(&self) -> BTreeMap<String, FunctionGraph> {
        let source = self.f.graph.clone();
        BTreeMap::from([
            (source.symbol.clone(), source),
            ("caller".into(), caller(&self.f, "caller")),
        ])
    }
}

fn observe(f: &mut Function, mut value: NodeId, kind: Kind, partial: bool) -> NodeId {
    let abi = FunctionAbi {
        args: vec![if kind.wide() && !partial {
            PassMode::Pair
        } else {
            PassMode::Direct
        }],
        ret: PassMode::Direct,
        c_variadic: false,
    };
    if partial {
        let ty = f.graph.rust_type(value).unwrap();
        let meta = f.graph.oracle.semantic().field_type(ty, None, 1).unwrap();
        value = f.node(Opcode::Field, &[value], 1, meta);
        value = f.node(Opcode::IntCast, &[value], 0, f.word);
    }
    f.call_named("observe_snapshot", &abi, &[value])
}

pub fn caller(source: &Function, name: &str) -> FunctionGraph {
    let mut graph = source.graph.clone();
    graph.symbol = name.into();
    let ids: Vec<_> = graph.ids().collect();
    for node in ids {
        if node != graph.start
            && !(graph.op(node) == Opcode::Projection && graph.input(node, 0) == graph.start)
        {
            graph.remove_node(node);
        }
    }
    let mut f = Function {
        word: source.word,
        control: graph.find_projection(graph.start, 0).unwrap(),
        effect: graph.find_projection(graph.start, 1).unwrap(),
        args: (0..source.args.len())
            .map(|i| graph.find_projection(graph.start, i as u32 + 2).unwrap())
            .collect(),
        graph,
    };
    let args = f.args.clone();
    let result = f.call(&source.graph, &args);
    f.ret(result);
    f.verify();
    f.graph
}

pub fn promote(graphs: &mut BTreeMap<String, FunctionGraph>) -> kfrc_son::ArgumentPromotionStats {
    let stats = promote_arguments(graphs, &BTreeSet::from(["protected_reader".into()])).unwrap();
    for graph in graphs.values_mut() {
        verify_graph(graph).unwrap();
    }
    stats
}

/// Match the producer's Field -> thin address casts -> WidePtr round trip.
pub fn reconstruct_wide(f: &mut Function, value: NodeId, keep_data: bool) -> NodeId {
    let ty = f.graph.rust_type(value).unwrap();
    let TypeShape::Reference { pointee, .. } = *f.graph.oracle.semantic().shape(ty) else {
        unreachable!()
    };
    let data_ty = f.graph.oracle.semantic().field_type(ty, None, 0).unwrap();
    let meta_ty = f.graph.oracle.semantic().field_type(ty, None, 1).unwrap();
    let data = if keep_data {
        f.node(Opcode::Field, &[value], 0, data_ty)
    } else {
        f.graph.add_constant(0, data_ty)
    };
    let metadata = f.node(Opcode::Field, &[value], 1, meta_ty);
    let place = f.graph.oracle.intern(
        TypeShape::PlaceAddress { pointee },
        Some(SizeAlign { size: 8, align: 8 }),
    );
    let address = f.node(Opcode::PointerCast, &[data], 0, place);
    let data = f.node(Opcode::PointerCast, &[address], 0, data_ty);
    f.node(Opcode::WidePtr, &[data, metadata], 0, ty)
}

#[derive(Clone, Copy, Debug)]
pub enum Failure {
    None,
    Callback,
    Diverges,
}

struct Program<'a> {
    graphs: &'a BTreeMap<String, FunctionGraph>,
    failure: Failure,
    trace: Vec<(String, Vec<Value>)>,
}

impl SemanticCallHandler for Program<'_> {
    fn call(&mut self, _: &str, _: &[Value]) -> Result<Value, InterpretError> {
        unreachable!()
    }
    fn call_with_memory(
        &mut self,
        symbol: &str,
        args: &[Value],
        memory: &mut SemanticMemory,
    ) -> Result<Value, InterpretError> {
        if let Some(graph) = self.graphs.get(symbol) {
            return match Interpreter::new(graph).run_values_in_memory(args, self, memory)? {
                SemanticOutcome::Return(value) => Ok(value),
                SemanticOutcome::Unreachable => unreachable!(),
            };
        }
        self.trace.push((symbol.into(), args.to_vec()));
        match symbol {
            "observe_snapshot" => match &args[0] {
                Value::Scalar(v) => Ok(Value::Scalar(*v)),
                Value::WidePointer { metadata, .. } => Ok(*metadata.clone()),
                _ => unreachable!(),
            },
            "touch_other" => {
                match self.failure {
                    Failure::Callback => {
                        return Err(InterpretError::UndefinedBehaviour(
                            "callback failure".into(),
                        ));
                    }
                    Failure::Diverges => return Err(InterpretError::OutOfFuel),
                    Failure::None => {}
                }
                let Value::Scalar(address) = args[0] else {
                    unreachable!()
                };
                memory.write_bytes(address as u64, &91u64.to_le_bytes().map(Some))?;
                Ok(Value::Scalar(0))
            }
            _ => unreachable!("{symbol}"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct Run {
    pub outcome: Result<SemanticOutcome, InterpretError>,
    pub trace: Vec<(String, Vec<Value>)>,
    pub out: Vec<Option<u8>>,
}

pub fn execute(
    graphs: &BTreeMap<String, FunctionGraph>,
    kind: Kind,
    poison_part: Option<usize>,
    failure: Failure,
    alias: bool,
    poison_arg: bool,
) -> Run {
    let mut memory = SemanticMemory::new();
    let data = memory.allocate(64, 8, true).unwrap();
    let root = memory
        .allocate(if kind.wide() { 24 } else { 16 }, 8, true)
        .unwrap();
    let out = if alias {
        root + 8
    } else {
        memory.allocate(8, 8, true).unwrap()
    };
    let first = match kind {
        Kind::Word => 17,
        _ => data,
    };
    let mut bytes: Vec<_> = 45u64
        .to_le_bytes()
        .into_iter()
        .chain(first.to_le_bytes())
        .map(Some)
        .collect();
    if kind.wide() {
        bytes.extend(7u64.to_le_bytes().map(Some));
    }
    if let Some(part) = poison_part {
        bytes[8 + 8 * part..16 + 8 * part].fill(None);
    }
    memory.write_bytes(root, &bytes).unwrap();
    let mut program = Program {
        graphs,
        failure,
        trace: vec![],
    };
    let mut outcome = Interpreter::new(&graphs["caller"]).run_values_in_memory(
        &[
            Value::Scalar(root),
            if poison_arg {
                Value::Poison
            } else {
                Value::Scalar(out)
            },
        ],
        &mut program,
        &mut memory,
    );
    // ABI promotion moves a strict poison observation, not its observable
    // failure or preceding external effects. Node IDs are not program outputs.
    if matches!(outcome, Err(InterpretError::PoisonObserved(_))) {
        outcome = Err(InterpretError::PoisonObserved(NodeId::NONE));
    }
    Run {
        outcome,
        trace: program.trace,
        out: memory.read_bytes(out, 8).unwrap(),
    }
}
