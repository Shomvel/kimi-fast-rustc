//! Nested niche and direct-tag values must keep by-value copies independent.
#[derive(Clone, Copy)]
pub enum Error {
    Empty,
    Byte(u8),
    Words([u64; 4]),
}

#[inline(never)]
fn overwrite(value: &mut Error) {
    *value = Error::Empty;
}

#[inline(never)]
fn consume(mut value: Error) -> u64 {
    let sum = checksum(value);
    overwrite(&mut value);
    std::hint::black_box(value);
    sum
}

#[inline(never)]
pub fn checksum(value: Error) -> u64 {
    match value {
        Error::Empty => 17,
        Error::Byte(value) => value as u64,
        Error::Words(words) => words[0] ^ words[1] ^ words[2] ^ words[3],
    }
}

#[inline(never)]
fn source(selector: u8, value: u64) -> Result<u64, Error> {
    match selector {
        0 => Ok(value),
        1 => Err(Error::Empty),
        2 => Err(Error::Byte(value as u8)),
        _ => Err(Error::Words([value, 3, 19, 71])),
    }
}

#[inline(never)]
pub fn parameter(value: Result<u64, Error>) -> (u64, Result<u64, Error>) {
    match value {
        Ok(n) => (n, Ok(n)),
        Err(e) => (consume(e), Err(e)),
    }
}

#[inline(never)]
pub fn call_result(selector: u8, value: u64) -> (u64, Result<u64, Error>) {
    parameter(source(selector, value))
}
