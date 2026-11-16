static BYTES: [u8; 8] = [1, 0, 0x7f, 0x80, b'=', 0xff, 37, 9];
static SIGNED: [i16; 4] = [i16::MIN, -1, 37, i16::MAX];
static TARGET: u8 = 91;
static INDIRECT: &u8 = &TARGET;
static OFFSET: &u8 = &BYTES[4];
static CHAIN: &&u8 = &OFFSET;

#[inline(never)]
pub fn constant_byte() -> u8 {
    BYTES[4]
}

#[inline(never)]
pub fn constant_signed() -> i16 {
    SIGNED[0]
}

#[inline(never)]
pub fn indexed(index: usize) -> u8 {
    BYTES[index & 7]
}

#[inline(never)]
pub fn indirect() -> u8 {
    **&INDIRECT
}

#[inline(never)]
pub fn equals(value: Option<&u8>) -> bool {
    value == Some(&b'=')
}

#[inline(never)]
pub fn relocated_address() -> usize {
    *&OFFSET as *const u8 as usize
}

#[inline(never)]
pub fn relocated_chain() -> u8 {
    ***&CHAIN
}

#[inline(never)]
pub fn offset_address_matches() -> bool {
    std::ptr::eq(*&OFFSET, &BYTES[4])
}

static mut WRITABLE: u64 = 17;

#[inline(never)]
pub unsafe fn writable(value: u64) -> u64 {
    let previous = WRITABLE;
    WRITABLE = value;
    previous
}
