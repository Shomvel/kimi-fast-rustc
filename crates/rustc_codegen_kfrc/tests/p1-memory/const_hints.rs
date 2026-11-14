//! Runtime hint operands and rustc's separate CTFE/runtime branch remapping.

#[inline(never)]
fn observed_argument(effects: &mut i64, value: i64) -> i64 {
    *effects += 1;
    std::hint::black_box(value)
}

#[inline(never)]
pub fn runtime_hints(value: i64) -> (bool, bool, bool, bool, i64) {
    let mut effects = 0;
    let scalar = observed_argument(&mut effects, value);
    let boolean = std::hint::black_box(value < 0);
    let character = std::hint::black_box(if value < 0 { 'x' } else { 'y' });
    let pointer = std::hint::black_box(&scalar as *const i64);
    (
        std::intrinsics::is_val_statically_known(scalar),
        std::intrinsics::is_val_statically_known(boolean),
        std::intrinsics::is_val_statically_known(character),
        std::intrinsics::is_val_statically_known(pointer),
        effects,
    )
}

#[inline(never)]
pub fn discarded_hint(value: i64) -> i64 {
    let mut effects = 0;
    let _ = std::intrinsics::is_val_statically_known(observed_argument(&mut effects, value));
    effects
}

const fn selected(value: i64) -> i64 {
    const fn in_const(value: i64) -> i64 {
        value.wrapping_mul(3).wrapping_add(5)
    }
    fn at_runtime(value: i64) -> i64 {
        std::hint::black_box(value).wrapping_mul(3).wrapping_add(5)
    }
    std::intrinsics::const_eval_select((value,), in_const, at_runtime)
}

const FROM_CTFE: i64 = selected(17);

#[inline(never)]
pub fn const_and_runtime(value: i64) -> (i64, i64) {
    (FROM_CTFE, selected(std::hint::black_box(value)))
}

#[inline(never)]
pub fn literal_hint(value: i64) -> i64 {
    // Both choices have the same semantic value: a literal may be recognized
    // by LLVM, while a backend is also allowed to answer false for it.
    let adjust = if std::intrinsics::is_val_statically_known(5u32) {
        17i64.wrapping_mul(3)
    } else {
        51
    };
    value.wrapping_add(adjust)
}
