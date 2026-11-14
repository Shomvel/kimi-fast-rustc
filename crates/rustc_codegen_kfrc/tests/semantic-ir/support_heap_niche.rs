//! humantime 2.4.0 `duration::Error`: the dataful variant `UnknownUnit` owns
//! a `String`, whose capacity niche encodes the other four variants. rustc's
//! niche range 0..=4 contains the untagged variant 2, so a variant test for
//! `UnknownUnit` must accept every capacity rather than one niche slot.
use std::fmt;

#[derive(Debug, PartialEq, Clone)]
pub enum UnitError {
    InvalidCharacter(usize),
    NumberExpected(usize),
    UnknownUnit {
        start: usize,
        end: usize,
        unit: String,
        value: u64,
    },
    NumberOverflow,
    Empty,
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitError::InvalidCharacter(offset) => write!(f, "invalid character at {offset}"),
            UnitError::NumberExpected(offset) => write!(f, "expected number at {offset}"),
            UnitError::UnknownUnit { unit, value, .. } if unit.is_empty() => {
                write!(f, "time unit needed, for example {value}sec")
            }
            UnitError::UnknownUnit { unit, .. } => write!(f, "unknown time unit {unit:?}"),
            UnitError::NumberOverflow => write!(f, "number is too large"),
            UnitError::Empty => write!(f, "value was empty"),
        }
    }
}

#[inline(never)]
pub fn make_unit_error(selector: u8, unit: &str) -> UnitError {
    match selector % 5 {
        0 => UnitError::InvalidCharacter(selector as usize),
        1 => UnitError::NumberExpected(selector as usize + 1),
        2 => UnitError::UnknownUnit {
            start: 1,
            end: 1 + unit.len(),
            unit: unit.to_string(),
            value: u64::from(selector),
        },
        3 => UnitError::NumberOverflow,
        _ => UnitError::Empty,
    }
}

#[inline(never)]
pub fn unit_error_variant(error: &UnitError) -> u32 {
    match error {
        UnitError::InvalidCharacter(_) => 0,
        UnitError::NumberExpected(_) => 1,
        UnitError::UnknownUnit { .. } => 2,
        UnitError::NumberOverflow => 3,
        UnitError::Empty => 4,
    }
}

#[inline(never)]
pub fn unit_error_guard(error: &UnitError) -> u32 {
    match error {
        UnitError::UnknownUnit { unit, .. } if unit.is_empty() => 10,
        UnitError::UnknownUnit { value, .. } => 20 + *value as u32,
        _ => 30,
    }
}

#[inline(never)]
pub fn unit_error_message(error: &UnitError) -> String {
    error.to_string()
}

#[inline(never)]
pub fn parse_unit(text: &str) -> Result<u64, UnitError> {
    match text {
        "" => Err(UnitError::Empty),
        "s" => Ok(1),
        "min" => Ok(60),
        _ if text.starts_with(|c: char| c.is_ascii_digit()) => Err(UnitError::NumberExpected(0)),
        _ => Err(UnitError::UnknownUnit {
            start: 0,
            end: text.len(),
            unit: text.to_string(),
            value: 1,
        }),
    }
}

#[inline(never)]
pub fn describe_parse(text: &str) -> String {
    match parse_unit(text) {
        Ok(seconds) => format!("ok {seconds}"),
        Err(error) => format!("err {error} {error:?}"),
    }
}
