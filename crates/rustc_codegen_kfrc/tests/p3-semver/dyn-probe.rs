//! Small probes for the trait-object frontier exposed by semver and regex.
//! Compile this library through kfrc and its driver through LLVM.

use std::fmt::{self, Debug};

#[derive(Debug)]
struct Three {
    number: u64,
    optional: Option<u64>,
    text: String,
}

// rustc's derive uses the general names/values slice path above five fields.
#[derive(Debug)]
struct Six {
    a: u64,
    b: Option<u64>,
    c: String,
    d: u8,
    e: Vec<u64>,
    f: bool,
}

struct Manual(u64);

impl Debug for Manual {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Manual").field(&self.0).finish()
    }
}

pub fn debug_values(number: u64) -> String {
    let three = Three {
        number,
        optional: Some(number + 1),
        text: "three".into(),
    };
    let six = Six {
        a: number,
        b: None,
        c: "six".into(),
        d: 7,
        e: vec![3, 5],
        f: true,
    };
    format!("{three:?}|{six:?}|{:?}", Manual(number))
}

pub fn virtual_debug(value: &dyn Debug) -> String {
    format!("{value:?}")
}

pub fn captured_call(number: u64) -> u64 {
    let captured = 17;
    let callback = |value| number + captured + value;
    callback(3)
}
