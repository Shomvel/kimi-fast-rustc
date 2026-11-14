//! Preserve a real vector register chosen by rustc's FnAbi.

use kfrc_body::PassMode;
use rustc_abi::{RegKind, Size};
use rustc_target::callconv::{ArgAttribute, ArgExtension, CastTarget};

pub(super) fn direct(bytes: u64) -> Result<PassMode, String> {
    if bytes != 16 {
        return Err(format!("direct SIMD ABI has {bytes} bytes (supported: 16)"));
    }
    Ok(PassMode::Vector { bytes: 16 })
}

pub(super) fn cast(
    size: u64,
    pad_i32: bool,
    cast: &CastTarget,
) -> Result<Option<PassMode>, String> {
    let has_vector = cast
        .prefix
        .iter()
        .flatten()
        .any(|reg| reg.kind == RegKind::Vector)
        || (cast.rest.total != Size::ZERO && cast.rest.unit.kind == RegKind::Vector);
    if !has_vector {
        return Ok(None);
    }
    // SysV can classify a one-field wrapper as SSE/SSEUP and select one
    // vector register through Cast instead of Direct. Preserve that register;
    // two integer eightbytes would have a different calling convention.
    let mut registers = cast.prefix.iter().flatten().copied().collect::<Vec<_>>();
    if cast.rest.total != Size::ZERO {
        if cast.rest.total != cast.rest.unit.size {
            return Err("SIMD Cast ABI has more than one rest register".to_owned());
        }
        registers.push(cast.rest.unit);
    }
    if pad_i32
        || cast.rest_offset.is_some()
        || cast.attrs.contains(ArgAttribute::InReg)
        || cast.attrs.arg_ext != ArgExtension::None
        || size != 16
        || registers.len() != 1
        || registers[0].kind != RegKind::Vector
        || registers[0].size.bytes() != 16
    {
        return Err("SIMD Cast ABI requires one unextended 16-byte vector register".to_owned());
    }
    Ok(Some(PassMode::Vector { bytes: 16 }))
}
