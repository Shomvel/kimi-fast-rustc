//! SysV integer and SSE register banks advance independently. Only exhausted
//! arguments share the outgoing stack area; a spilled 128-bit vector is aligned
//! to sixteen bytes. Internal graph vectors are addresses, not ABI pointers.
use super::*;
use kfrc_body::FunctionAbi;

#[derive(Clone, Copy, Debug)]
pub(super) enum VectorLocation {
    Register(Xmm),
    /// Index in the existing eight-byte stack-argument space (6 means Rsp+0).
    Stack(usize),
}

pub(super) struct ArgumentPlan {
    pub integers: Vec<Vec<usize>>,
    pub vectors: Vec<Option<VectorLocation>>,
}

pub(super) fn argument_plan(abi: &FunctionAbi) -> Result<ArgumentPlan, LowerError> {
    for mode in abi.args.iter().chain(std::iter::once(&abi.ret)) {
        if let PassMode::Vector { bytes } = mode {
            if *bytes != 16 {
                return Err(LowerError::Unsupported(format!("{bytes}-byte vector ABI")));
            }
        }
    }
    let mut registers = usize::from(abi.ret == PassMode::Indirect);
    let mut vector_registers = 0;
    let mut stack = 6usize;
    let mut integers = Vec::new();
    let mut vectors = Vec::new();
    for mode in &abi.args {
        if matches!(mode, PassMode::Vector { .. }) {
            let location = if vector_registers < 8 {
                let reg = Xmm(vector_registers);
                vector_registers += 1;
                VectorLocation::Register(reg)
            } else {
                stack += (stack - 6) % 2;
                let location = VectorLocation::Stack(stack);
                stack += 2;
                location
            };
            integers.push(Vec::new());
            vectors.push(Some(location));
            continue;
        }
        let mut allocate = |count: usize, consecutive: bool| {
            if consecutive && registers < 6 && registers + count > 6 {
                let locations = (stack..stack + count).collect();
                stack += count;
                return locations;
            }
            (0..count)
                .map(|_| {
                    if registers < 6 {
                        let location = registers;
                        registers += 1;
                        location
                    } else {
                        let location = stack;
                        stack += 1;
                        location
                    }
                })
                .collect::<Vec<_>>()
        };
        integers.push(match mode {
            PassMode::Ignore => Vec::new(),
            PassMode::Pair => allocate(2, false),
            PassMode::Cast(cast) => {
                let mut locations = if cast.pad_i32 {
                    allocate(1, false)
                } else {
                    Vec::new()
                };
                locations.extend(allocate(
                    cast.lanes.iter().flatten().count(),
                    cast.consecutive,
                ));
                locations
            }
            _ => allocate(1, false),
        });
        vectors.push(None);
    }
    Ok(ArgumentPlan { integers, vectors })
}

impl Lowering<'_> {
    pub(super) fn receive_vector_argument(
        &mut self,
        parameter: NodeId,
        ty: TyId,
        location: VectorLocation,
    ) -> Result<(), LowerError> {
        self.allocate_slot(register(parameter), ty)?;
        let address = VirtualValue::Reg(register(parameter));
        match location {
            VectorLocation::Register(src) => self
                .instructions
                .push(LowerInstr::StoreVector { address, src }),
            VectorLocation::Stack(index) => {
                for lane in 0..2 {
                    let raw = self.temporary();
                    self.push(VirtualInstr::LoadArg {
                        dst: raw,
                        index: index + lane,
                    });
                    self.store_raw_bytes(address, lane as u64 * 8, 8, VirtualValue::Reg(raw));
                }
            }
        }
        Ok(())
    }
}
