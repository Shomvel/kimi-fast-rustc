//! Memory and aggregate operations exercised through the real rustc MIR path.

#![feature(
    coerce_unsized,
    const_eval_select,
    core_intrinsics,
    fn_traits,
    funnel_shifts,
    portable_simd,
    ptr_mask,
    ptr_metadata,
    simd_ffi,
    thread_local,
    unsize
)]
#![allow(internal_features)]

pub mod bitops;
pub mod const_hints;
pub mod intrinsic_values;
pub mod pointer_comparisons;
pub mod pointer_metadata;
pub mod rustcall;
pub mod saturation;
pub mod select;
pub mod simd;
pub mod stack_alignment;
pub mod static_data;
pub mod thread_local;
pub mod unions;
pub mod zero_sized;

#[inline(never)]
pub fn guard_loop(v: i64) -> i64 {
    let mut x = 1;
    let y = match v {
        n if {
            loop {
                x = 4;
                break n > 5;
            }
        } =>
        {
            10
        }
        _ => 20,
    };
    x + y
}

#[inline(never)]
pub fn mutate(value: &mut i64, increment: i64) {
    *value += increment;
}

#[inline(never)]
pub fn references(a: i64, b: i64) -> i64 {
    let mut value = a;
    mutate(&mut value, b);
    let view = &value;
    *view * 3
}

#[inline(never)]
pub fn pair(a: i64) -> (i64, bool) {
    (a + 7, a < 0)
}

#[inline(never)]
pub fn pair_use(a: i64) -> i64 {
    let (value, negative) = pair(a);
    if negative {
        -value
    } else {
        value
    }
}

#[inline(never)]
pub fn pair_argument(values: (i64, bool)) -> i64 {
    if values.1 {
        values.0 + 11
    } else {
        values.0 - 13
    }
}

#[inline(never)]
pub fn pair_argument_use(a: i64) -> i64 {
    pair_argument(pair(a))
}

#[inline(never)]
pub fn tuple_fields(a: i64, b: i64) -> i64 {
    let mut values = (a, b, a + b);
    values.0 += values.2;
    values.0 - values.1
}

#[inline(never)]
pub fn array_index(a: i64, index: usize) -> i64 {
    let mut values = [a, a + 1, a + 2, a + 3];
    values[index] += 9;
    values[0] + values[1] * 3 + values[2] * 5 + values[3] * 7
}

#[inline(never)]
pub fn checked(a: i32, b: i32) -> i32 {
    (a + b) * 3 - a
}

#[inline(never)]
pub fn division(a: i64, b: i64) -> i64 {
    a / b + a % b
}

#[inline(never)]
pub fn const_bytes(index: usize) -> u8 {
    b"kfrc-poc"[index]
}

#[inline(never)]
pub fn string_length() -> usize {
    "readonly relocation".len()
}

#[inline(never)]
pub fn string_byte() -> u8 {
    unsafe { *"readonly relocation".as_ptr().add(4) }
}

#[repr(i8)]
enum Sign {
    Negative = -3,
    Zero = 0,
    Positive = 5,
}

#[inline(never)]
pub fn explicit_enum(a: i64) -> i64 {
    let sign = if a < 0 {
        Sign::Negative
    } else if a == 0 {
        Sign::Zero
    } else {
        Sign::Positive
    };
    sign as i8 as i64
}

#[inline(never)]
pub fn niche_reference(a: i64) -> i64 {
    let value = if a < 0 { None } else { Some(&a) };
    match value {
        Some(value) => *value + 17,
        None => -19,
    }
}

#[inline(never)]
pub fn tagged_option(a: i64) -> i64 {
    let value = if a < 0 { None } else { Some(a as u8) };
    match value {
        Some(value) => value as i64,
        None => -23,
    }
}

#[inline(never)]
fn slice_sum(values: &[i64]) -> i64 {
    let mut sum = 0;
    let mut index = 0;
    while index < values.len() {
        sum += values[index];
        index += 1;
    }
    sum
}

#[inline(never)]
pub fn slice_use(a: i64) -> i64 {
    let values = [a, a + 2, a + 4];
    slice_sum(&values)
}

#[inline(never)]
pub fn raw_slice_use(a: i64) -> i64 {
    let values = [a, a + 3, a + 6];
    let view = unsafe { std::slice::from_raw_parts(values.as_ptr(), 3) };
    slice_sum(view)
}

#[inline(never)]
pub fn black_box_value(a: i64) -> i64 {
    std::hint::black_box(a) + 3
}

#[inline(never)]
pub fn ordering(a: i64, b: i64) -> i64 {
    a.cmp(&b) as i8 as i64
}

#[inline(never)]
fn slice_pattern(bytes: &[u8]) -> i64 {
    match bytes {
        [_, middle @ .., _] => {
            let last = middle.len() - 1;
            middle.len() as i64 + middle[last] as i64
        }
        _ => -1,
    }
}

#[inline(never)]
pub fn subslice(a: i64) -> i64 {
    let bytes = [a as u8, 17, 23, 41, 59];
    slice_pattern(&bytes)
}

unsafe extern "C" {
    // Deliberately undefined: mono collection and codegen must agree that the
    // false const branch cannot contribute a target relocation.
    fn kfrc_disabled_const_branch_symbol() -> i64;
}

#[inline(never)]
fn const_check<const ENABLED: bool>(a: i64) -> i64 {
    if ENABLED {
        unsafe { kfrc_disabled_const_branch_symbol() }
    } else {
        a + 5
    }
}

#[inline(never)]
pub fn disabled_check_paths(a: i64) -> i64 {
    const_check::<false>(a)
}

#[inline(never)]
pub fn trivial_drop_paths(a: u64) -> u64 {
    let mut values = [a, a.wrapping_add(1)];
    let slice = &mut values[..];
    unsafe { std::ptr::drop_in_place(slice as *mut [u64]) };
    // Unlike the direct call above, taking the address must collect the
    // actual empty drop glue so the function pointer remains callable.
    let dropper: unsafe fn(*mut u64) = std::ptr::drop_in_place::<u64>;
    unsafe { dropper(&mut values[0]) };
    values[0].wrapping_add(values[1])
}

#[track_caller]
#[inline(never)]
fn tracked_location() -> (u32, u32) {
    let caller = std::panic::Location::caller();
    (caller.line(), caller.column())
}

#[inline(never)]
pub fn reified_tracked_location() -> (u32, u32) {
    let function: fn() -> (u32, u32) = tracked_location;
    function()
}

#[inline(never)]
pub fn reified_closure(a: i64) -> i64 {
    let function: fn(i64) -> i64 = |value| value.wrapping_mul(3).wrapping_sub(7);
    function(a)
}

trait Measure {
    fn measure(&self, increment: i64) -> i64;
}

struct Measured(i64);

impl Measure for Measured {
    #[inline(never)]
    fn measure(&self, increment: i64) -> i64 {
        self.0.wrapping_mul(5).wrapping_add(increment)
    }
}

#[inline(never)]
fn measure_object(value: &dyn Measure, increment: i64) -> i64 {
    value
        .measure(increment)
        .wrapping_add(std::mem::size_of_val(value) as i64)
        .wrapping_add(std::mem::align_of_val(value) as i64)
}

#[inline(never)]
pub fn trait_object_use(a: i64) -> i64 {
    let value = Measured(a);
    let with_auto: &(dyn Measure + Sync) = &value;
    let without_auto: &dyn Measure = with_auto;
    measure_object(without_auto, 7)
}

struct MarkDrop {
    marker: *mut i64,
    value: i64,
}

impl Measure for MarkDrop {
    fn measure(&self, increment: i64) -> i64 {
        self.value.wrapping_add(increment)
    }
}

impl Drop for MarkDrop {
    fn drop(&mut self) {
        unsafe { *self.marker = self.value.wrapping_add(41) };
    }
}

#[inline(never)]
pub fn trait_object_drop(a: i64) -> i64 {
    let mut marker = 0;
    let value = MarkDrop {
        marker: &mut marker,
        value: a,
    };
    let mut value = std::mem::ManuallyDrop::new(value);
    let object: &mut dyn Measure = &mut *value;
    unsafe { std::ptr::drop_in_place(object as *mut dyn Measure) };
    marker
}

#[inline(never)]
pub fn trait_object_trivial_drop(a: i64) -> i64 {
    let mut value = Measured(a);
    let object: &mut dyn Measure = &mut value;
    unsafe { std::ptr::drop_in_place(object as *mut dyn Measure) };
    value.0
}

#[inline(never)]
pub fn boxed_slice(a: i64) -> i64 {
    let source = Box::new([a, a.wrapping_add(3), a.wrapping_add(9)]);
    let values: Box<[i64]> = source;
    let result = values[0]
        .wrapping_add(values[1].wrapping_mul(3))
        .wrapping_add(values[2].wrapping_mul(5));
    result.wrapping_add(values.len() as i64)
}

#[inline(never)]
pub fn boxed_trait_object(a: i64) -> i64 {
    let mut marker = 0;
    let source = Box::new(MarkDrop {
        marker: &mut marker,
        value: a,
    });
    let object: Box<dyn Measure> = source;
    let result = measure_object(&*object, 13);
    drop(object);
    result.wrapping_add(marker)
}

// The non-ZST fields change physical offsets when the Box pointer widens.
// PhantomData changes its type without contributing bytes. Coercing this
// wrapper must preserve both scalar fields and rebuild the complete nested
// Box -> Unique -> NonNull pointer representation.
struct Tagged<T: ?Sized> {
    before: i64,
    pointer: Box<T>,
    after: i64,
    marker: std::marker::PhantomData<T>,
}

impl<T: ?Sized + std::marker::Unsize<U>, U: ?Sized> std::ops::CoerceUnsized<Tagged<U>>
    for Tagged<T>
{
}

#[inline(never)]
pub fn tagged_boxed_slice(a: i64) -> i64 {
    let source = Tagged {
        before: a.wrapping_add(17),
        pointer: Box::new([a, a.wrapping_add(2)]),
        after: a.wrapping_sub(23),
        marker: std::marker::PhantomData,
    };
    let target: Tagged<[i64]> = source;
    target
        .before
        .wrapping_mul(3)
        .wrapping_add(target.after.wrapping_mul(5))
        .wrapping_add(target.pointer[0])
        .wrapping_add(target.pointer[1])
        .wrapping_add(target.pointer.len() as i64)
}

struct SliceTail<T: ?Sized> {
    tag: i64,
    values: T,
}

#[inline(never)]
pub fn struct_tail_slice(a: i64) -> i64 {
    let source = SliceTail {
        tag: a.wrapping_add(19),
        values: [a, a.wrapping_add(7)],
    };
    let target: &SliceTail<[i64]> = &source;
    target
        .tag
        .wrapping_mul(3)
        .wrapping_add(target.values[0])
        .wrapping_add(target.values[1])
        .wrapping_add(target.values.len() as i64)
}

#[inline(never)]
fn callback(a: i64) -> i64 {
    a.wrapping_mul(3).wrapping_add(11)
}

#[inline(never)]
fn other_callback(a: i64) -> i64 {
    a.wrapping_mul(5).wrapping_sub(7)
}

#[inline(never)]
fn invoke_once<F: FnOnce(i64) -> i64>(function: F, a: i64) -> i64 {
    function(a)
}

#[inline(never)]
fn invoke_shared<F: Fn(i64) -> i64>(function: F, a: i64) -> i64 {
    function(a)
}

#[inline(never)]
fn invoke_mut<F: FnMut(i64) -> i64>(mut function: F, a: i64) -> i64 {
    let first = function(a);
    first.wrapping_add(function(a.wrapping_add(1)))
}

struct CallbackField<F> {
    marker: i64,
    function: F,
}

#[inline(never)]
fn invoke_field<F: FnOnce(i64) -> i64>(value: CallbackField<F>, a: i64) -> i64 {
    (value.function)(a).wrapping_add(value.marker)
}

#[inline(never)]
pub fn function_item_callbacks(a: i64) -> i64 {
    invoke_once(callback, a)
        .wrapping_add(invoke_shared(callback, a))
        .wrapping_add(invoke_mut(callback, a))
        .wrapping_add(invoke_field(
            CallbackField {
                marker: 17,
                function: callback,
            },
            a,
        ))
}

#[inline(never)]
pub fn function_pointer_callbacks(a: i64) -> i64 {
    let function: fn(i64) -> i64 = if a < 0 { callback } else { other_callback };
    invoke_once(function, a)
        .wrapping_add(invoke_shared(function, a))
        .wrapping_add(invoke_mut(function, a))
}

#[inline(never)]
pub fn captured_callbacks(a: i64) -> i64 {
    let mut count = a;
    let mutating = |value: i64| {
        count = count.wrapping_add(1);
        value.wrapping_add(count)
    };
    invoke_mut(mutating, a)
        .wrapping_add(invoke_once(|value| value.wrapping_add(a), 13))
        .wrapping_add(invoke_shared(|value| value.wrapping_sub(a), 19))
}

#[track_caller]
#[inline(never)]
fn tracked_callback(a: i64) -> i64 {
    let location = std::panic::Location::caller();
    a.wrapping_add((location.line() as i64).wrapping_mul(1000))
        .wrapping_add(location.column() as i64)
}

#[inline(never)]
pub fn tracked_function_callbacks(a: i64) -> i64 {
    let pointer: fn(i64) -> i64 = tracked_callback;
    invoke_once(tracked_callback, a)
        .wrapping_add(invoke_shared(tracked_callback, a))
        .wrapping_add(invoke_mut(tracked_callback, a))
        .wrapping_add(invoke_once(pointer, a))
}

#[inline(never)]
fn invoke_method<F: FnOnce(&(dyn Measure + 'static), i64) -> i64>(
    method: F,
    value: &(dyn Measure + 'static),
    increment: i64,
) -> i64 {
    method(value, increment)
}

#[inline(never)]
pub fn virtual_method_item(a: i64) -> i64 {
    let value = Measured(a);
    let method = <dyn Measure as Measure>::measure;
    method(&value, 3).wrapping_add(invoke_method(method, &value, 7))
}
