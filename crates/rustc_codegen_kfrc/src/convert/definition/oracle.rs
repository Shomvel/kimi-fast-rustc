use super::*;
use kfrc_body::{BodyBuilder, FunctionAbi, SizeAlign};
use kfrc_son::specialize::{HoleAnswer, SpecializationOracle};
use kfrc_son::{CallTarget, Hole};
use rustc_middle::ty::TypingEnv;

pub(super) struct Oracle<'a, 'tcx> {
    definition: &'a Definition<'tcx>,
    converter: super::super::Converter<'tcx>,
    abi: FunctionAbi,
}
impl<'a, 'tcx> Oracle<'a, 'tcx> {
    pub(super) fn new(
        tcx: TyCtxt<'tcx>,
        definition: &'a Definition<'tcx>,
        instance: Instance<'tcx>,
    ) -> Result<Self, String> {
        let abi = crate::abi::instance_abi(tcx, instance)?;
        Ok(Self {
            definition,
            abi: abi.clone(),
            converter: super::super::Converter {
                tcx,
                instance,
                types: HashMap::default(),
                constants: crate::constants::ConstantCx::new(
                    tcx,
                    tcx.symbol_name(instance).name.to_string(),
                ),
                caller_location: None,
                local_map: HashMap::default(),
                pending_blocks: Vec::new(),
                original_blocks: 0,
                builder: BodyBuilder::new(
                    tcx.symbol_name(instance).name.to_string(),
                    abi,
                    RecordedTypes::new(),
                ),
            },
        })
    }
    fn native(&self, id: TyId) -> Result<Ty<'tcx>, String> {
        let native = self
            .definition
            .native_types
            .get(&id)
            .ok_or("definition type has no rustc identity")?;
        Ok(self.converter.monomorphize(*native))
    }
}
impl SpecializationOracle for Oracle<'_, '_> {
    fn instantiate_type(&mut self, ty: TyId, into: &mut RecordedTypes) -> Result<TyId, String> {
        let native = self.native(ty)?;
        // Reuse the ABI-correct concrete recorder, but only on demand. The R
        // type table itself has no physical layouts and is never overwritten.
        std::mem::swap(self.converter.builder.types(), into);
        let id = self.converter.ty(native);
        std::mem::swap(self.converter.builder.types(), into);
        Ok(id)
    }
    fn function_abi(&mut self) -> Result<FunctionAbi, String> {
        Ok(self.abi.clone())
    }
    fn answer(&mut self, hole: &Hole) -> Result<HoleAnswer, String> {
        let tcx = self.converter.tcx;
        match hole {
            Hole::SizeAlign(ty) => {
                let ty = self.native(*ty)?;
                let layout = tcx
                    .layout_of(TypingEnv::fully_monomorphized().as_query_input(ty))
                    .map_err(|error| format!("SizeAlign({ty}): {error:?}"))?;
                Ok(HoleAnswer::SizeAlign(SizeAlign {
                    size: layout.size.bytes(),
                    align: layout.align.abi.bytes(),
                }))
            }
            Hole::NeedsDrop(ty) => Ok(HoleAnswer::NeedsDrop(
                self.native(*ty)?
                    .needs_drop(tcx, TypingEnv::fully_monomorphized()),
            )),
            Hole::Resolve { item, .. } => {
                let resolution = self
                    .definition
                    .resolutions
                    .get(item)
                    .ok_or("unknown R resolution identity")?;
                let instance = resolve(tcx, self.converter.instance, *resolution)
                    .map_err(|_| format!("failed to resolve {item}"))?
                    .ok_or_else(|| format!("unresolved static call {item}"))?;
                // definition::instance_eligibility sends these instances to
                // the concrete path before specialization starts.
                if let Some(reason) = call_adapter_requirement(tcx, instance) {
                    return Err(format!("R resolved call to {instance:?}: {reason}"));
                }
                let symbol = tcx.symbol_name(instance).name.to_string();
                let abi = crate::abi::instance_abi(tcx, instance)?;
                // A nominal target is conservative: distinct implementations
                // never collide. Recursive semantic-class reuse is not claimed.
                Ok(HoleAnswer::Resolved {
                    target: CallTarget {
                        symbol: symbol.clone(),
                        abi,
                    },
                    identity: symbol,
                })
            }
            Hole::VariantEncoding(_) => {
                Err("R producer leaves variant encoding to layout lowering".into())
            }
        }
    }
}

/// The concrete callee of an R call site in `instance`.
pub(super) fn resolve<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: Instance<'tcx>,
    resolution: super::Resolution<'tcx>,
) -> Result<Option<Instance<'tcx>>, rustc_span::ErrorGuaranteed> {
    let args = instance.instantiate_mir_and_normalize_erasing_regions(
        tcx,
        TypingEnv::fully_monomorphized(),
        rustc_middle::ty::EarlyBinder::bind(resolution.args),
    );
    Instance::try_resolve(tcx, TypingEnv::fully_monomorphized(), resolution.item, args)
}

/// Why a resolved callee needs more than R's direct call with the source
/// arguments: dynamic dispatch, an intrinsic body, or rustc's implicit
/// `#[track_caller]` location argument. The concrete MIR path supplies each.
pub(super) fn call_adapter_requirement<'tcx>(
    tcx: TyCtxt<'tcx>,
    callee: Instance<'tcx>,
) -> Option<&'static str> {
    match callee.def {
        InstanceKind::Virtual(..) => Some("dynamic dispatch needs a vtable call"),
        InstanceKind::Intrinsic(_) => Some("intrinsic needs its concrete lowering"),
        _ if callee.def.requires_caller_location(tcx) => {
            Some("track_caller callee needs a caller location argument")
        }
        _ => None,
    }
}
