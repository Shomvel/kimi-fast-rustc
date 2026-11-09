//! Rust-level demand and snapshot cases. The LLVM-built driver checks each arm.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Record {
    pub left: (u64, u64),
    pub right: (u64, u64),
}

#[derive(Clone, Copy)]
pub enum Choice {
    Empty,
    Number(Record),
    Pair(Record, Record),
}

#[inline(never)]
pub fn selected_field(a: Record, b: Record, choose: bool) -> u64 {
    let value = if choose { a } else { b };
    value.right.0
}

#[inline(never)]
pub fn selected_payload(a: Choice, b: Choice, choose: bool) -> u64 {
    match if choose { a } else { b } {
        Choice::Empty => 71,
        Choice::Number(record) => record.left.1,
        Choice::Pair(_, record) => record.right.0,
    }
}

#[inline(never)]
pub fn sparse_loop(mut a: Record, mut b: Record, count: u64) -> u64 {
    let mut i = 0;
    while i < count {
        let old = a;
        a = b;
        b = old;
        a.right.0 = a.right.0.wrapping_add(i);
        // Unobserved fields are deliberately updated, without affecting the
        // demanded right.0 lane carried around the loop.
        b.left.0 = b.left.0.wrapping_mul(3);
        i += 1;
    }
    a.right.0
}

#[inline(never)]
pub unsafe fn snapshot_then_overwrite(source: *mut Record, replacement: Record) -> u64 {
    let snapshot = *source;
    *source = replacement;
    snapshot.left.1 ^ (*source).right.0
}

#[inline(never)]
pub unsafe fn enum_snapshot_then_overwrite(source: *mut Choice, replacement: Choice) -> u64 {
    let snapshot = *source;
    *source = replacement;
    match snapshot {
        Choice::Empty => 71,
        Choice::Number(record) => record.left.1,
        Choice::Pair(_, record) => record.right.0,
    }
}

#[inline(never)]
pub fn whole_value(a: Record, b: Record, choose: bool) -> Record {
    let mut value = if choose { a } else { b };
    value.left.0 ^= 99;
    value
}

#[inline(never)]
pub fn raw_snapshot(a: Record, b: Record) -> u64 {
    let mut storage = std::mem::MaybeUninit::new(a);
    let snapshot = storage;
    storage.write(b);
    unsafe { snapshot.assume_init().right.0 ^ storage.assume_init().left.1 }
}

#[inline(always)]
fn generic_field<const SECOND: bool>(value: Record) -> u64 {
    if SECOND { value.right.0 } else { value.left.1 }
}

#[inline(never)]
pub fn concrete_fields(value: Record) -> u64 {
    generic_field::<false>(value).wrapping_add(generic_field::<true>(value))
}

#[inline(never)]
pub unsafe fn repeated_snapshot(source: *mut Record, replacement: Record) -> u64 {
    let first = *source;
    let second = *source;
    *source = replacement;
    first.left.1.wrapping_add(second.left.1) ^ (*source).right.0
}

#[inline(never)]
pub fn overwritten_field(mut value: Record, replacement: u64) -> u64 {
    value.right.0 = replacement;
    value.right.0
}

#[inline(never)]
pub fn transport_choice(a: Choice, b: Choice, choose: bool) -> Choice {
    if choose { a } else { b }
}

#[inline(never)]
pub fn transport_chain(seed: u64) -> Choice {
    let record = Record {
        left: (seed, seed.wrapping_add(1)),
        right: (seed ^ 77, seed.wrapping_mul(3)),
    };
    if seed & 1 == 0 {
        Choice::Empty
    } else if seed & 2 == 0 {
        Choice::Number(record)
    } else {
        let mut second = record;
        second.left.0 ^= 37;
        if seed & 4 == 0 {
            Choice::Pair(record, second)
        } else {
            Choice::Pair(second, record)
        }
    }
}

#[inline(never)]
pub unsafe fn branch_stored_snapshot(source: *mut u64, value: u64, a: bool, b: bool) -> u64 {
    *source = value;
    if a {
        if b {
            (*source).wrapping_add(17)
        } else {
            *source ^ 99
        }
    } else {
        71
    }
}

#[inline(never)]
pub unsafe fn branch_loaded_snapshot(source: *mut u64, a: bool, b: bool) -> u64 {
    let first = *source;
    if a {
        if b {
            first.wrapping_add(*source)
        } else {
            first ^ *source
        }
    } else {
        first.wrapping_sub(3)
    }
}

#[inline(never)]
pub unsafe fn branch_alias_write(
    source: *mut u64,
    alias: *mut u64,
    value: u64,
    replacement: u64,
    choose: bool,
) -> u64 {
    *source = value;
    if choose {
        *alias = replacement;
        *source
    } else {
        *source ^ 99
    }
}

#[inline(never)]
pub unsafe fn transport_multiple_sinks(
    seed: u64,
    choose: bool,
    first_sink: *mut Choice,
    second_sink: *mut Choice,
    chosen_sink: *mut Choice,
) {
    let record = Record {
        left: (seed, seed ^ 17),
        right: (seed.wrapping_add(5), seed.wrapping_mul(3)),
    };
    let first = if seed & 1 == 0 {
        Choice::Number(record)
    } else {
        Choice::Pair(record, record)
    };
    let mut changed = record;
    changed.right.0 ^= 203;
    let second = if seed & 2 == 0 {
        Choice::Pair(changed, record)
    } else {
        Choice::Number(changed)
    };
    *first_sink = first;
    *second_sink = second;
    *chosen_sink = if choose { first } else { second };
}

#[inline(never)]
pub unsafe fn transport_inside_loop(seed: u64, count: u64, output: *mut Choice) {
    let mut i = 0;
    while i < count {
        let round = seed.wrapping_add(i);
        let record = Record {
            left: (round, round ^ 17),
            right: (round.wrapping_add(5), round.wrapping_mul(3)),
        };
        let value = if round & 1 == 0 {
            Choice::Number(record)
        } else {
            Choice::Pair(record, record)
        };
        *output = value;
        i += 1;
    }
}

#[derive(Clone, Copy)]
pub enum Problem {
    Missing,
    Character(char),
    Pair(u64, u64),
}

#[inline(never)]
pub unsafe fn nested_result_source(source: *const Record, selector: u64) -> Result<u64, Problem> {
    let value = *source;
    match selector & 3 {
        0 => Ok(value.right.0),
        1 => Err(Problem::Missing),
        2 => Err(Problem::Character('界')),
        _ => Err(Problem::Pair(value.left.0, value.right.1)),
    }
}

#[inline(never)]
pub unsafe fn nested_result_transport(
    source: *mut Record,
    selector: u64,
    replacement: Record,
) -> Result<Record, Problem> {
    let snapshot = nested_result_source(source, selector);
    *source = replacement;
    match snapshot {
        Ok(value) => Ok(Record {
            left: (value, 17),
            right: (99, value ^ 203),
        }),
        Err(problem) => Err(problem),
    }
}

#[inline(never)]
pub unsafe fn nested_result_observed(source: *const Record, selector: u64) -> (u64, Problem) {
    match nested_result_source(source, selector) {
        Ok(value) => (value, Problem::Missing),
        Err(problem) => {
            let observed = match problem {
                Problem::Missing => 71,
                Problem::Character(c) => c as u64,
                Problem::Pair(a, b) => a ^ b,
            };
            (observed, problem)
        }
    }
}
