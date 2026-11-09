//! Union construction chooses one field; copying preserves all stored bytes.

use std::marker::PhantomData;
use std::mem::MaybeUninit;

#[derive(Clone, Copy)]
#[repr(C)]
union Overlay {
    invalid_if_read_as_bool: bool,
    bytes: [u8; 16],
    words: [u64; 2],
}

#[inline(never)]
pub fn aggregate_fields(seed: u64) -> u64 {
    let mut source = Overlay {
        words: [seed | 0x80, seed.wrapping_add(0x1234_5678_9abc_def0)],
    };
    let snapshot = source;
    source.words = [3, 5];
    let mut total = 0_u64;
    let mut index = 0;
    while index < 16 {
        total = total
            .wrapping_mul(3)
            .wrapping_add(unsafe { snapshot.bytes[index] } as u64);
        index += 1;
    }
    let bytes = Overlay {
        bytes: [0x80, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
    };
    total
        .wrapping_add(unsafe { source.words[0] })
        .wrapping_add(unsafe { source.words[1] })
        .wrapping_add(unsafe { bytes.words[0] })
        .wrapping_add(unsafe { bytes.words[1] })
}

union ZeroSized {
    unit: (),
    marker: PhantomData<u64>,
}

#[inline(never)]
pub fn maybe_uninit_and_zero_sized(seed: i64) -> i64 {
    let empty = ZeroSized {
        marker: PhantomData,
    };
    let second = ZeroSized {
        unit: unsafe { empty.unit },
    };
    let _ = unsafe { second.marker };
    let mut value = MaybeUninit::new((seed, seed.wrapping_add(7)));
    let snapshot = value;
    value.write((seed.wrapping_mul(3), seed.wrapping_mul(5)));
    let previous = unsafe { snapshot.assume_init() };
    let current = unsafe { value.assume_init() };
    previous
        .0
        .wrapping_add(previous.1)
        .wrapping_add(current.0)
        .wrapping_add(current.1)
}
