//! Flatten rustc's per-argument CastTarget into its actual integer ABI lanes.
//!
//! This information belongs to the call ABI, rather than to a Rust type: the
//! same aggregate may have different representations in Rust and C functions.
//! Offsets follow the LLVM-compatible target layout, including padding between
//! prefix registers and an explicitly relocated second register.

use kfrc_body::{CastAbi, CastExtension, CastLane};
use rustc_abi::{Reg, RegKind, Size};
use rustc_middle::ty::TyCtxt;
use rustc_target::callconv::{ArgAttribute, ArgExtension, CastTarget};

pub(super) fn record_cast(
    tcx: TyCtxt<'_>,
    size: u64,
    pad_i32: bool,
    cast: &CastTarget,
) -> Result<CastAbi, String> {
    if cast.attrs.contains(ArgAttribute::InReg) {
        return Err("Cast ABI with an `inreg` attribute is not implemented".to_owned());
    }
    let extension = match cast.attrs.arg_ext {
        ArgExtension::None => CastExtension::None,
        ArgExtension::Zext => CastExtension::Zero,
        ArgExtension::Sext => CastExtension::Sign,
    };
    let mut lanes = [None; 2];
    let mut count = 0;
    let mut push = |offset: Size, register: Reg| -> Result<(), String> {
        if register.kind != RegKind::Integer {
            return Err(format!(
                "Cast ABI {:?} register is not supported (integer GPR lanes only)",
                register.kind
            ));
        }
        let bytes = register.size.bytes();
        if !(1..=8).contains(&bytes) {
            return Err(format!(
                "Cast ABI integer lane has {bytes} bytes (supported: 1..=8)"
            ));
        }
        let Some(lane) = lanes.get_mut(count) else {
            return Err("Cast ABI has more than two integer lanes".to_owned());
        };
        *lane = Some(CastLane {
            offset: offset.bytes(),
            bytes: bytes as u8,
        });
        count += 1;
        Ok(())
    };
    if let Some(offset) = cast.rest_offset {
        if cast.prefix[1..].iter().any(Option::is_some) || cast.rest.unit.size != cast.rest.total {
            return Err(
                "Cast ABI rest_offset requires exactly one prefix and one rest register".to_owned(),
            );
        }
        let first = cast.prefix[0].ok_or("Cast ABI rest_offset has no prefix register")?;
        push(Size::ZERO, first)?;
        push(offset, cast.rest.unit)?;
    } else {
        let mut offset = Size::ZERO;
        for register in cast.prefix.iter().flatten().copied() {
            integer_register(register)?;
            // LLVM integer types such as i24 have a three-byte store size but
            // a four-byte allocation stride. Respect that padding even if the
            // next prefix register has weaker alignment.
            let align = register.align(&tcx);
            offset = offset.align_to(align);
            push(offset, register)?;
            offset += register.size.align_to(align);
        }
        if cast.rest.total != Size::ZERO {
            if cast.rest.unit.size == Size::ZERO {
                return Err("Cast ABI has nonempty rest with a zero-sized unit".to_owned());
            }
            if cast.rest.unit.kind != RegKind::Integer {
                return Err(format!(
                    "Cast ABI {:?} rest is not supported",
                    cast.rest.unit.kind
                ));
            }
            integer_register(cast.rest.unit)?;
            let align = cast.rest.unit.align(&tcx);
            offset = offset.align_to(align);
            let unit_size = cast.rest.unit.size.bytes();
            let rest_count = cast.rest.total.bytes().div_ceil(unit_size);
            if rest_count > 2 {
                return Err("Cast ABI has more than two integer rest lanes".to_owned());
            }
            let stride = cast.rest.unit.size.align_to(align);
            for index in 0..rest_count {
                push(offset + stride * index, cast.rest.unit)?;
            }
        }
    }
    if count == 0 {
        return Err("nonignored Cast ABI has no integer lanes".to_owned());
    }
    if extension != CastExtension::None && count != 1 {
        return Err("Cast ABI extension on multiple integer lanes is not implemented".to_owned());
    }
    Ok(CastAbi {
        size,
        lanes,
        consecutive: cast.rest.is_consecutive,
        pad_i32,
        extension,
    })
}

fn integer_register(register: Reg) -> Result<(), String> {
    if register.kind != RegKind::Integer || !(1..=8).contains(&register.size.bytes()) {
        return Err(format!(
            "Cast ABI register `{register:?}` requires an integer of at most eight bytes"
        ));
    }
    Ok(())
}
