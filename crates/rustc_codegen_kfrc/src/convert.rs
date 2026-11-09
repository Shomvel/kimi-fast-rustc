//! rustc MIR → kfrc Typed Body (seam 2 of `docs/plan/poc/architecture.md`).
//!
//! The MIR of a mono item is monomorphized and translated one-to-one into
//! the Body contract: blocks keep their indices, places keep their
//! projections, and call targets are resolved to concrete instances whose
//! linker symbols and `PassMode`s come from rustc (so mixed linking with
//! rustc-compiled std is ABI-correct). Types are recorded with rustc's
//! layouts into the Body's `RecordedTypes`; the SoN never sees `Ty<'tcx>`.
//!
//! This converter is throwaway PoC code: what it cannot express is a gap in
//! the Body contract, recorded rather than worked around.

use kfrc_body::fx_hash::HashMap;

use crate::constants::ConstantCx;
use kfrc_body::{
    AggregateKind, Body, BodyBuilder, LocalDecl, LocalId, Mutability, Operand, Place,
    RecordedTypes, Rvalue, StatementKind, Terminator, TyId,
};
use rustc_middle::mir;
use rustc_middle::ty::{self, Instance, Ty, TyCtxt, TypingEnv};

mod calls;
mod casts;
pub mod definition;
mod intrinsics;
mod places;
mod rvalues;
mod statements;
mod terminators;
mod trait_object;
mod types;
mod unsize;
mod unsize_adt;

pub fn convert_instance<'tcx>(tcx: TyCtxt<'tcx>, instance: Instance<'tcx>) -> Result<Body, String> {
    let mir = tcx.instance_mir(instance.def);
    let abi = crate::abi::instance_abi(tcx, instance)?;
    let definition = crate::abi::definition_args(tcx, instance, mir)?;
    let mut converter = Converter {
        tcx,
        instance,
        types: HashMap::default(),
        constants: ConstantCx::new(tcx, tcx.symbol_name(instance).name.to_string()),
        caller_location: None,
        local_map: HashMap::default(),
        pending_blocks: Vec::new(),
        original_blocks: mir.basic_blocks.len(),
        builder: BodyBuilder::new(
            tcx.symbol_name(instance).name.to_string(),
            abi,
            RecordedTypes::new(),
        ),
    };
    let ret_ty = converter.ty(converter.monomorphize(mir.local_decls[mir::RETURN_PLACE].ty));
    let ret = converter.builder.add_local(LocalDecl {
        ty: ret_ty,
        mutability: Mutability::Mut,
        name: None,
    });
    converter.local_map.insert(mir::RETURN_PLACE, ret);
    let mut spread_parts = Vec::new();
    for part in &definition.parts {
        let ty = converter.ty(part.ty);
        let local = converter.builder.add_local(LocalDecl {
            ty,
            mutability: Mutability::Mut,
            name: None,
        });
        match (part.local, part.field) {
            (Some(mir_local), None) => {
                converter.local_map.insert(mir_local, local);
            }
            (Some(_), Some(field)) => spread_parts.push((field, local)),
            (None, _) => converter.caller_location = Some(local),
        }
    }
    converter.builder.set_arg_count(definition.parts.len());
    for (local, declaration) in mir.local_decls.iter_enumerated() {
        if converter.local_map.contains_key(&local) {
            continue;
        }
        let ty = converter.ty(converter.monomorphize(declaration.ty));
        let mutability = convert_mutability(declaration.mutability);
        let name = mir.var_debug_info.iter()
            .find(|info| matches!(info.value, mir::VarDebugInfoContents::Place(place) if place.local == local && place.projection.is_empty()))
            .map(|info| info.name.to_string());
        let new_local = converter.builder.add_local(LocalDecl {
            ty,
            mutability,
            name,
        });
        converter.local_map.insert(local, new_local);
    }
    if let Some(local) = definition.spread_local {
        spread_parts.sort_by_key(|(field, _)| *field);
        let operands = spread_parts
            .into_iter()
            .map(|(_, part)| Operand::Copy(Place::local(part)))
            .collect();
        converter.builder.push_statement(StatementKind::Assign(
            Place::local(converter.local_id(local)),
            Rvalue::Aggregate(AggregateKind::Tuple, operands),
        ));
    }
    // Use exactly the collector's instance-aware reachability. Emitting a
    // relocation from a disabled UB/contract/const branch would reference a
    // mono item rustc deliberately did not collect, even if it never executes.
    let reachable = mir::traversal::mono_reachable_as_bitset(mir, tcx, instance);
    for (index, block) in mir.basic_blocks.iter_enumerated() {
        if !reachable.contains(index) {
            converter
                .builder
                .finish_block(Terminator::Unreachable, block.is_cleanup);
            continue;
        }
        for statement in &block.statements {
            if let Some(statement) = converter.statement(statement)? {
                converter.builder.push_statement(statement);
            }
        }
        let terminator = if matches!(
            block.terminator().kind,
            mir::TerminatorKind::SwitchInt { .. }
        ) {
            let mut successors = block.mono_successors(tcx, instance);
            match (successors.next(), successors.next()) {
                (Some(target), None) => Terminator::Goto(kfrc_body::BlockId(target.as_u32())),
                _ => converter.terminator(block.terminator())?,
            }
        } else {
            converter.terminator(block.terminator())?
        };
        converter.builder.finish_block(terminator, block.is_cleanup);
    }
    for (statements, terminator) in converter.pending_blocks {
        for statement in statements {
            converter.builder.push_statement(statement);
        }
        converter.builder.finish_block(terminator, false);
    }
    Ok(converter.builder.finish())
}

struct Converter<'tcx> {
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    types: HashMap<Ty<'tcx>, TyId>,
    constants: ConstantCx<'tcx>,
    caller_location: Option<LocalId>,
    pending_blocks: Vec<(Vec<StatementKind>, Terminator)>,
    original_blocks: usize,
    local_map: HashMap<mir::Local, LocalId>,
    builder: BodyBuilder,
}

impl<'tcx> Converter<'tcx> {
    fn local_id(&self, local: mir::Local) -> LocalId {
        self.local_map[&local]
    }

    fn monomorphize<T>(&self, value: T) -> T
    where
        T: ty::TypeFoldable<TyCtxt<'tcx>>,
    {
        self.instance.instantiate_mir_and_normalize_erasing_regions(
            self.tcx,
            TypingEnv::fully_monomorphized(),
            ty::EarlyBinder::bind(value),
        )
    }
}

fn convert_mutability(mutability: mir::Mutability) -> Mutability {
    match mutability {
        mir::Mutability::Mut => Mutability::Mut,
        mir::Mutability::Not => Mutability::Not,
    }
}
