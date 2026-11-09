//! Definition MIR → polymorphic R. No Instance normalization, layout query or
//! trait resolution is permitted in this producer. Unsupported input is an
//! explicit eligibility result; a bug in a built R graph is a hard error.
mod emit;
mod oracle;
mod types;

use kfrc_body::fx_hash::{HashMap, HashSet};
use kfrc_body::{Mutability, RecordedTypes, TyId, TypeShape};
use kfrc_son::specialize::{RustDefinition, Specialization};
use kfrc_son::{FunctionGraph, GraphState, NodeId, Opcode, RustSignature, ValueType};
use rustc_middle::{
    mir,
    ty::{self, Instance, InstanceKind, Ty, TyCtxt},
};
use rustc_span::def_id::DefId;
use std::collections::VecDeque;

pub enum Eligibility<T> {
    Eligible(T),
    NotEligible(String),
}

pub struct Definition<'tcx> {
    pub prepared: RustDefinition,
    native_types: HashMap<TyId, Ty<'tcx>>,
    resolutions: HashMap<String, Resolution<'tcx>>,
}
#[derive(Clone, Copy)]
struct Resolution<'tcx> {
    item: DefId,
    args: ty::GenericArgsRef<'tcx>,
}

pub fn convert_definition<'tcx>(
    tcx: TyCtxt<'tcx>,
    def_id: DefId,
) -> Result<Eligibility<Definition<'tcx>>, String> {
    if tcx.generics_of(def_id).count() == 0 {
        return Ok(Eligibility::NotEligible(
            "definition has no generic parameters".into(),
        ));
    }
    let item = InstanceKind::Item(def_id);
    let typing_env = ty::TypingEnv::post_analysis(tcx, def_id);
    if item.requires_caller_location(tcx) {
        return Ok(Eligibility::NotEligible(
            "track_caller requires a source ABI adapter".into(),
        ));
    }
    // The backend's mir-opt-level=0 input contract is shared with mono conversion.
    // instance_mir(Item(def_id)) returns the definition, without substituting args.
    let body = tcx.instance_mir(item);
    if body.spread_arg.is_some() || body.basic_blocks.iter().any(|block| block.is_cleanup) {
        return Ok(Eligibility::NotEligible(
            "spread arguments or cleanup blocks".into(),
        ));
    }
    let mut cx = DefinitionCx {
        tcx,
        body,
        typing_env,
        graph: FunctionGraph::new_rust(
            tcx.def_path_str(def_id),
            RustSignature {
                args: Vec::new(),
                ret: TyId(0),
            },
            RecordedTypes::new(),
        ),
        types: HashMap::default(),
        native_types: HashMap::default(),
        resolutions: HashMap::default(),
        slots: Vec::new(),
        blocks: HashMap::default(),
        effects: HashMap::default(),
        incoming: HashMap::default(),
        control: NodeId::NONE,
        effect: NodeId::NONE,
    };
    if let Err(reason) = cx.emit() {
        return Ok(Eligibility::NotEligible(reason));
    }
    let optimize = std::env::var("KFRC_OPTIMIZE").ok().as_deref() != Some("0");
    let prepared = RustDefinition::prepare_with_optimization(cx.graph, optimize)?;
    Ok(Eligibility::Eligible(Definition {
        prepared,
        native_types: cx.native_types,
        resolutions: cx.resolutions,
    }))
}

/// Instance-level eligibility, decided before specialization starts. Every
/// call site's concrete callee must be a direct call taking exactly the source
/// arguments; any other callee sends the whole instance to the concrete path.
/// A call site whose callee does not resolve here is left to specialization,
/// which answers only reachable sites and reports their errors.
pub fn instance_eligibility<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: &Definition<'tcx>,
    instance: Instance<'tcx>,
) -> Eligibility<()> {
    let graph = definition.prepared.graph();
    let mut keys: Vec<_> = graph
        .ids()
        .filter(|&node| graph.op(node) == Opcode::TraitCall)
        .filter_map(|node| {
            let call = &graph.trait_calls[graph.payload(node) as usize];
            match &graph.holes[call.resolve as usize] {
                kfrc_son::Hole::Resolve { item, .. } => Some(item),
                _ => None,
            }
        })
        .collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let Ok(Some(callee)) = oracle::resolve(tcx, instance, definition.resolutions[key]) else {
            continue;
        };
        if let Some(reason) = oracle::call_adapter_requirement(tcx, callee) {
            return Eligibility::NotEligible(format!("resolved callee: {reason}"));
        }
    }
    Eligibility::Eligible(())
}

pub fn specialize<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: &Definition<'tcx>,
    instance: Instance<'tcx>,
) -> Result<Specialization, String> {
    let mut oracle = oracle::Oracle::new(tcx, definition, instance)?;
    definition
        .prepared
        .specialize(tcx.symbol_name(instance).name.to_string(), &mut oracle)
}

struct DefinitionCx<'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'tcx mir::Body<'tcx>,
    typing_env: ty::TypingEnv<'tcx>,
    graph: FunctionGraph,
    types: HashMap<Ty<'tcx>, TyId>,
    native_types: HashMap<TyId, Ty<'tcx>>,
    resolutions: HashMap<String, Resolution<'tcx>>,
    slots: Vec<NodeId>,
    blocks: HashMap<mir::BasicBlock, NodeId>,
    effects: HashMap<mir::BasicBlock, NodeId>,
    incoming: HashMap<mir::BasicBlock, Vec<(NodeId, NodeId)>>,
    control: NodeId,
    effect: NodeId,
}

impl<'tcx> DefinitionCx<'tcx> {
    fn emit(&mut self) -> Result<(), String> {
        let ret = self.ty(self.body.local_decls[mir::RETURN_PLACE].ty)?;
        let args = self
            .body
            .args_iter()
            .map(|local| self.ty(self.body.local_decls[local].ty))
            .collect::<Result<Vec<_>, _>>()?;
        self.graph.rust_signature = Some(RustSignature {
            args: args.clone(),
            ret,
        });
        let mut result = vec![ValueType::Control, ValueType::Effect];
        result.extend(args.iter().copied().map(ValueType::Rust));
        let start = self.graph.add_node(
            Opcode::Start,
            NodeId::NONE,
            &[],
            0,
            ValueType::Tuple(result),
        );
        self.graph.start = start;
        self.control = self.projection(start, 0, ValueType::Control);
        self.effect = self.projection(start, 1, ValueType::Effect);
        for (local, decl) in self.body.local_decls.iter_enumerated() {
            let pointee = self.ty(decl.ty)?;
            let pointer = self.graph.oracle.intern(
                TypeShape::RawPointer {
                    mutability: Mutability::Mut,
                    pointee,
                },
                None,
            );
            // Remember synthetic types as well: specialization uses rustc's
            // canonical concrete recorder, not a guessed pointer layout.
            self.native_types
                .insert(pointer, Ty::new_mut_ptr(self.tcx, decl.ty));
            self.slots.push(self.graph.add_node(
                Opcode::LocalSlot,
                NodeId::NONE,
                &[],
                local.as_u32(),
                ValueType::Rust(pointer),
            ));
        }
        for (index, &ty) in args.iter().enumerate() {
            let value = self.projection(start, index as u32 + 2, ValueType::Rust(ty));
            self.store(self.slots[index + 1], value);
        }
        // Reachability is definition-level: no mono_successors or type queries.
        let mut queue = VecDeque::from([mir::START_BLOCK]);
        let mut reachable = HashSet::default();
        while let Some(block) = queue.pop_front() {
            if !reachable.insert(block) {
                continue;
            }
            queue.extend(self.body.basic_blocks[block].terminator().successors());
        }
        let mut order: Vec<_> = reachable.into_iter().collect();
        order.sort_by_key(|block| block.as_u32());
        for &block in &order {
            let region =
                self.graph
                    .add_node(Opcode::Region, NodeId::NONE, &[], 0, ValueType::Control);
            let effect = self
                .graph
                .add_node(Opcode::Phi, region, &[region], 0, ValueType::Effect);
            self.blocks.insert(block, region);
            self.effects.insert(block, effect);
        }
        self.edge(mir::START_BLOCK);
        for block in order {
            self.control = self.blocks[&block];
            self.effect = self.effects[&block];
            for statement in &self.body.basic_blocks[block].statements {
                self.statement(statement)?;
            }
            self.terminator(self.body.basic_blocks[block].terminator())?;
        }
        for (&block, &region) in &self.blocks {
            let incoming = self
                .incoming
                .get(&block)
                .ok_or("definition block has no predecessor")?;
            self.graph.set_inputs(
                region,
                &incoming.iter().map(|edge| edge.0).collect::<Vec<_>>(),
            );
            let mut values = vec![region];
            values.extend(incoming.iter().map(|edge| edge.1));
            self.graph.set_inputs(self.effects[&block], &values);
        }
        self.graph.state = GraphState::Sealed;
        Ok(())
    }
    fn projection(&mut self, value: NodeId, index: u32, ty: ValueType) -> NodeId {
        self.graph
            .add_node(Opcode::Projection, NodeId::NONE, &[value], index, ty)
    }
    fn edge(&mut self, target: mir::BasicBlock) {
        let jump = self
            .graph
            .add_node(Opcode::Jump, self.control, &[], 0, ValueType::Control);
        self.incoming
            .entry(target)
            .or_default()
            .push((jump, self.effect));
    }
    fn load(&mut self, pointer: NodeId, ty: TyId) -> NodeId {
        let load = self.graph.add_node(
            Opcode::LoadValue,
            self.control,
            &[self.effect, pointer],
            0,
            ValueType::Tuple(vec![
                ValueType::Control,
                ValueType::Effect,
                ValueType::Rust(ty),
            ]),
        );
        self.control = self.projection(load, 0, ValueType::Control);
        self.effect = self.projection(load, 1, ValueType::Effect);
        self.projection(load, 2, ValueType::Rust(ty))
    }
    fn store(&mut self, pointer: NodeId, value: NodeId) {
        let store = self.graph.add_node(
            Opcode::StoreValue,
            self.control,
            &[self.effect, pointer, value],
            0,
            ValueType::Tuple(vec![ValueType::Control, ValueType::Effect]),
        );
        self.control = self.projection(store, 0, ValueType::Control);
        self.effect = self.projection(store, 1, ValueType::Effect);
    }
}
