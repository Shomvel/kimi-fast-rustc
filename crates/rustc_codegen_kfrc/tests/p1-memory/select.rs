//! Selection copies values and preserves eager arguments and public drop glue.

#[derive(Clone, Copy)]
#[repr(C)]
struct Padded {
    tag: u8,
    value: i64,
}

#[derive(Clone, Copy)]
union Word {
    integer: u64,
    bytes: [u8; 8],
}

#[derive(Clone, Copy)]
enum Payload {
    First(i64),
    Second(u8, i64),
}

#[inline(never)]
pub fn scalars(condition: bool, left: u64, right: u64) -> (u64, bool, char, bool) {
    let condition = std::hint::black_box(condition);
    let value = std::intrinsics::select_unpredictable(condition, left, right);
    let boolean = std::intrinsics::select_unpredictable(condition, false, true);
    let character = std::intrinsics::select_unpredictable(condition, 'λ', '中');
    let pointer = std::intrinsics::select_unpredictable(condition, &left, &right);
    (value, boolean, character, *pointer == value)
}

#[inline(never)]
pub fn aggregates(condition: bool, left: i64, right: i64) -> (i64, bool, i64, u64, i64, bool) {
    let pair = std::intrinsics::select_unpredictable(condition, (left, true), (right, false));
    let padded = std::intrinsics::select_unpredictable(
        condition,
        Padded {
            tag: 3,
            value: left,
        },
        Padded {
            tag: 7,
            value: right,
        },
    );
    let word = std::intrinsics::select_unpredictable(
        condition,
        Word {
            integer: left as u64,
        },
        Word {
            integer: right as u64,
        },
    );
    let payload = std::intrinsics::select_unpredictable(
        condition,
        Payload::First(left),
        Payload::Second(11, right),
    );
    let payload = match payload {
        Payload::First(value) => value,
        Payload::Second(tag, value) => value.wrapping_add(tag as i64),
    };
    let niche = std::intrinsics::select_unpredictable(condition, Some(&left), None);
    (
        pair.0,
        pair.1,
        padded.value.wrapping_add(padded.tag as i64),
        unsafe { u64::from_ne_bytes(word.bytes) },
        payload,
        niche.is_some(),
    )
}

#[inline(never)]
pub fn copies(condition: bool, left: i64, right: i64) -> ([i64; 3], i64) {
    let mut a = [left, left.wrapping_add(1), left.wrapping_add(2)];
    let mut b = [right, right.wrapping_sub(1), right.wrapping_sub(2)];
    let selected = std::intrinsics::select_unpredictable(condition, a, b);
    a[0] = 777;
    b[1] = 888;
    (selected, a[0] + b[1])
}

struct Token {
    id: i64,
    drops: *mut i64,
}

impl Drop for Token {
    fn drop(&mut self) {
        unsafe { *self.drops += 1 };
    }
}

#[inline(never)]
pub fn owned(condition: bool, left: i64, right: i64) -> (i64, i64, i64) {
    let mut drops = 0;
    let selected = std::hint::select_unpredictable(
        condition,
        Token {
            id: left,
            drops: &mut drops,
        },
        Token {
            id: right,
            drops: &mut drops,
        },
    );
    let result = (selected.id, drops);
    drop(selected);
    (result.0, result.1, drops)
}

#[inline(never)]
pub fn intrinsic_owned(condition: bool, left: i64, right: i64) -> (i64, i64, i64) {
    let mut drops = 0;
    let selected = std::intrinsics::select_unpredictable(
        condition,
        Token {
            id: left,
            drops: &mut drops,
        },
        Token {
            id: right,
            drops: &mut drops,
        },
    );
    let result = (selected.id, drops);
    drop(selected);
    (result.0, result.1, drops)
}

#[inline(never)]
fn observed_argument(effects: &mut u32, value: u32) -> u32 {
    *effects = *effects * 10 + value;
    value
}

#[inline(never)]
pub fn eager_arguments(condition: bool) -> (u32, u32) {
    let mut effects = 0;
    let selected = std::intrinsics::select_unpredictable(
        condition,
        observed_argument(&mut effects, 1),
        observed_argument(&mut effects, 2),
    );
    (selected, effects)
}

#[inline(never)]
pub fn wide_pointer(condition: bool) -> (usize, u8) {
    let selected = std::intrinsics::select_unpredictable(condition, "alpha", "beta");
    (selected.len(), selected.as_bytes()[0])
}

#[inline(never)]
fn zst_argument(effects: &mut u32) {
    *effects = *effects * 3 + 1;
}

#[inline(never)]
pub fn zero_sized(condition: bool) -> u32 {
    let mut effects = 0;
    std::intrinsics::select_unpredictable(
        condition,
        zst_argument(&mut effects),
        zst_argument(&mut effects),
    );
    effects
}

static mut ZST_DROPS: u32 = 0;

struct ZeroToken;

impl Drop for ZeroToken {
    fn drop(&mut self) {
        unsafe { ZST_DROPS += 1 };
    }
}

#[inline(never)]
pub fn owned_zero_sized(condition: bool) -> (u32, u32) {
    unsafe { ZST_DROPS = 0 };
    let selected = std::hint::select_unpredictable(condition, ZeroToken, ZeroToken);
    let before = unsafe { ZST_DROPS };
    drop(selected);
    (before, unsafe { ZST_DROPS })
}
