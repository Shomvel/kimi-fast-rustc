//! Transfer rustc's evaluated constants to target data and symbolic addresses.
//!
//! A CTFE allocation is already laid out by rustc. Its bytes are copied as-is;
//! its provenance entries become target relocations. In particular, a panic
//! `Location` remains an opaque data object, including its pointer to the file
//! name. No Rust layout is reconstructed here.

use std::collections::HashSet;

use kfrc_body::{BodyBuilder, ConstantKind, DataBinding, DataObject, DataRelocation};
use rustc_middle::mir::ConstValue;
use rustc_middle::mir::interpret::{
    AllocId, ConstAllocation, GlobalAlloc, Scalar, read_target_uint,
};
use rustc_middle::ty::{Instance, TyCtxt};
use rustc_span::def_id::DefId;

/// The converter supplies the Rust type and materializes pair/aggregate values
/// in places. This module only translates evaluated storage and provenance.
pub enum LoweredConstant {
    Immediate(ConstantKind),
    Pair {
        first: ConstantKind,
        second: ConstantKind,
    },
    ByReference {
        symbol: String,
        offset: u64,
    },
}

/// Per-body state. Names include the body's linker symbol, so independent
/// codegen units can emit their own local copies of anonymous allocations.
/// References to statics and functions always keep their real linker symbols.
pub struct ConstantCx<'tcx> {
    tcx: TyCtxt<'tcx>,
    namespace: String,
    emitted_allocs: HashSet<AllocId>,
    emitted_statics: HashSet<DefId>,
}

impl<'tcx> ConstantCx<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>, namespace: impl Into<String>) -> Self {
        Self {
            tcx,
            namespace: namespace.into(),
            emitted_allocs: HashSet::new(),
            emitted_statics: HashSet::new(),
        }
    }

    pub fn lower(
        &mut self,
        value: ConstValue,
        builder: &mut BodyBuilder,
    ) -> Result<LoweredConstant, String> {
        Ok(match value {
            ConstValue::ZeroSized => LoweredConstant::Immediate(ConstantKind::ZeroSized),
            ConstValue::Scalar(Scalar::Int(value)) => {
                LoweredConstant::Immediate(ConstantKind::Scalar(value.to_bits(value.size())))
            }
            ConstValue::Scalar(Scalar::Ptr(pointer, _)) => {
                let (provenance, offset) = pointer.prov_and_relative_offset();
                LoweredConstant::Immediate(self.address(
                    provenance.alloc_id(),
                    offset.bytes(),
                    builder,
                )?)
            }
            ConstValue::Slice { alloc_id, meta } => LoweredConstant::Pair {
                first: self.address(alloc_id, 0, builder)?,
                second: ConstantKind::Scalar(u128::from(meta)),
            },
            ConstValue::Indirect { alloc_id, offset } => LoweredConstant::ByReference {
                symbol: self.memory_symbol(alloc_id, builder)?,
                offset: offset.bytes(),
            },
        })
    }

    /// Resolve an allocation provenance to a target address. Empty immutable
    /// allocations use their alignment as a non-null dangling address, matching
    /// rustc's other codegen backends.
    pub fn address(
        &mut self,
        alloc_id: AllocId,
        offset: u64,
        builder: &mut BodyBuilder,
    ) -> Result<ConstantKind, String> {
        let symbol = match self.tcx.global_alloc(alloc_id) {
            GlobalAlloc::Memory(allocation) => {
                if allocation.inner().len() == 0 {
                    return Ok(ConstantKind::Scalar(u128::from(
                        allocation.inner().align.bytes().wrapping_add(offset),
                    )));
                }
                self.memory_symbol(alloc_id, builder)?
            }
            GlobalAlloc::Function { instance } => self.tcx.symbol_name(instance).name.to_owned(),
            GlobalAlloc::Static(def_id) => {
                // A TLS address is resolved in the executing thread, never
                // through a process-wide constant/data relocation. MIR uses
                // ThreadLocalRef for that separate operation.
                if self.tcx.is_thread_local_static(def_id) {
                    return Err(format!(
                        "thread-local static `{}` cannot have a constant address",
                        self.tcx.def_path_str(def_id)
                    ));
                }
                self.tcx
                    .symbol_name(Instance::mono(self.tcx, def_id))
                    .name
                    .to_owned()
            }
            GlobalAlloc::VTable(ty, dyn_ty) => {
                let trait_ref = dyn_ty
                    .principal()
                    .map(|principal| self.tcx.instantiate_bound_regions_with_erased(principal));
                let allocation = self.tcx.vtable_allocation((ty, trait_ref));
                self.memory_symbol(allocation, builder)?
            }
            // TypeId provenance represents integer bits already contained in
            // the pointer offset, rather than an address to be relocated.
            GlobalAlloc::TypeId { .. } => {
                return Ok(ConstantKind::Scalar(u128::from(offset)));
            }
        };
        Ok(ConstantKind::Address {
            symbol,
            addend: offset as i64,
        })
    }

    /// Emit a static selected by rustc's mono-item collector. Static references
    /// encountered while walking anonymous allocations are only declarations;
    /// emitting them here avoids duplicate definitions of cross-crate statics.
    pub fn static_data(
        &mut self,
        def_id: DefId,
        binding: DataBinding,
        builder: &mut BodyBuilder,
    ) -> Result<String, String> {
        let attrs = self.tcx.codegen_fn_attrs(def_id);
        if attrs.link_section.is_some() || attrs.import_linkage.is_some() {
            return Err(format!(
                "static `{}` has an unsupported link section or import linkage",
                self.tcx.def_path_str(def_id)
            ));
        }
        let symbol = self
            .tcx
            .symbol_name(Instance::mono(self.tcx, def_id))
            .name
            .to_owned();
        if self.emitted_statics.insert(def_id) {
            let allocation = self.tcx.eval_static_initializer(def_id).map_err(|error| {
                format!(
                    "cannot evaluate static `{}`: {error:?}",
                    self.tcx.def_path_str(def_id)
                )
            })?;
            self.emit_allocation(
                symbol.clone(),
                allocation,
                binding,
                self.tcx.is_mutable_static(def_id),
                self.tcx.is_thread_local_static(def_id),
                false,
                builder,
            )?;
        }
        Ok(symbol)
    }

    fn memory_symbol(
        &mut self,
        alloc_id: AllocId,
        builder: &mut BodyBuilder,
    ) -> Result<String, String> {
        let symbol = format!(".Lkfrc_{}_alloc{}", self.namespace, alloc_id.0);
        if self.emitted_allocs.insert(alloc_id) {
            let GlobalAlloc::Memory(allocation) = self.tcx.global_alloc(alloc_id) else {
                return Err(format!("expected memory for constant {alloc_id:?}"));
            };
            // Marking the allocation before following its provenance breaks
            // cycles in constant graphs without recursive cloning.
            self.emit_allocation(
                symbol.clone(),
                allocation,
                DataBinding::Local,
                false,
                false,
                true,
                builder,
            )?;
        }
        Ok(symbol)
    }

    fn emit_allocation(
        &mut self,
        symbol: String,
        allocation: ConstAllocation<'tcx>,
        binding: DataBinding,
        mutable_static: bool,
        thread_local: bool,
        unnamed_address: bool,
        builder: &mut BodyBuilder,
    ) -> Result<(), String> {
        let allocation = allocation.inner();
        // CTFE also marks immutable statics containing UnsafeCell as mutable.
        // Keep the source static's explicit mutability even for zero-sized or
        // specially interned initializers, rather than guessing from a symbol.
        let writable = mutable_static || allocation.mutability.is_mut();
        let pointer_size = self.tcx.data_layout.pointer_size().bytes() as usize;
        if pointer_size != 8 {
            return Err("constant relocation requires the x86-64 target".to_owned());
        }
        let mut bytes = allocation
            .inspect_with_uninit_and_ptr_outside_interpreter(0..allocation.len())
            .to_vec();
        let mut relocations = Vec::new();
        for &(offset, provenance) in allocation.provenance().ptrs().iter() {
            let start = usize::try_from(offset.bytes())
                .map_err(|_| "constant pointer offset exceeds the host address space")?;
            let end = start
                .checked_add(pointer_size)
                .ok_or("constant pointer range overflow")?;
            let pointer_bytes = bytes
                .get(start..end)
                .ok_or("constant relocation outside its allocation")?;
            let addend = read_target_uint(self.tcx.data_layout.endian, pointer_bytes)
                .map_err(|error| format!("invalid constant pointer: {error:?}"))?
                as u64;
            match self.address(provenance.alloc_id(), addend, builder)? {
                ConstantKind::Address {
                    symbol: target,
                    addend,
                } => {
                    // ELF RELA carries the addend in the relocation itself.
                    // Clearing the placeholder also prevents provenance bytes
                    // being mistaken for a second, implicit addend.
                    bytes[start..end].fill(0);
                    relocations.push(DataRelocation {
                        offset: offset.bytes(),
                        target,
                        addend,
                    });
                }
                ConstantKind::Scalar(value) => {
                    // Empty allocations and TypeId provenance have no target
                    // symbol. Their concrete pointer-sized bits are emitted.
                    bytes[start..end].copy_from_slice(&(value as u64).to_le_bytes());
                }
                _ => return Err("allocation provenance did not produce a pointer".to_owned()),
            }
        }
        builder.add_data_object(DataObject {
            symbol,
            binding,
            bytes,
            align: allocation.align.bytes(),
            writable,
            thread_local,
            unnamed_address,
            relocations,
        });
        Ok(())
    }
}
