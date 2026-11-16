#[inline(never)]
pub fn branch_fields(selector: u64, left: u64, right: u64) -> u64 {
    let mut fields = if selector & 1 == 0 {
        (0, left)
    } else {
        (1, right)
    };
    if selector & 2 != 0 {
        fields.1 = fields.1.wrapping_add(37);
    }
    if fields.0 == 0 {
        fields.1
    } else {
        fields.1.rotate_left(7)
    }
}

#[inline(never)]
pub fn loop_fields(count: u64, mut left: u64, mut right: u64) -> u64 {
    for index in 0..count {
        let old = (left, right);
        left = old.1.wrapping_add(index);
        right = old.0 ^ old.1.rotate_left(3);
    }
    left ^ right
}

#[inline(never)]
pub fn chunk_fields(input: &[u8], width: usize) -> u64 {
    let mut result = 0u64;
    for (index, chunk) in input.chunks_exact(width).enumerate() {
        result = result.wrapping_add((index as u64).wrapping_mul(chunk.len() as u64));
        for byte in chunk {
            result = result.rotate_left(1) ^ u64::from(*byte);
        }
    }
    result
}

#[inline(never)]
fn escape(fields: &mut (u64, u64), value: u64) {
    fields.0 = fields.0.wrapping_add(value);
    fields.1 ^= value;
}

#[inline(never)]
pub fn escaped_fields(value: u64) -> u64 {
    let mut fields = (37, 91);
    escape(&mut fields, value);
    fields.0 ^ fields.1
}
