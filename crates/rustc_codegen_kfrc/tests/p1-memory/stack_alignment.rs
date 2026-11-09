//! Stack-backed locals and closure captures retain rustc's requested alignment.

macro_rules! aligned {
    ($name:ident, $alignment:literal) => {
        #[repr(align($alignment))]
        struct $name([u64; 3]);
    };
}

aligned!(Align32, 32);
aligned!(Align64, 64);
aligned!(Align256, 256);
aligned!(Align4096, 4096);

#[inline(never)]
fn clobber(values: &mut [u64; 5]) {
    for index in 0..5 {
        values[index] = values[index].wrapping_mul(3).wrapping_add(index as u64);
    }
}

#[inline(never)]
pub fn locals(seed: u64) -> (usize, u64) {
    let mut before = [seed, 7, 11, 13, 17];
    let mut a = Align32([seed, 19, 23]);
    let mut b = Align64([29, seed, 31]);
    let mut c = Align256([37, 41, seed]);
    let mut d = Align4096([seed, 43, 47]);
    let mut after = [53, 59, 61, 67, seed];
    let misalignment = (std::hint::black_box(&mut a) as *mut Align32 as usize) % 32
        | (std::hint::black_box(&mut b) as *mut Align64 as usize) % 64
        | (std::hint::black_box(&mut c) as *mut Align256 as usize) % 256
        | (std::hint::black_box(&mut d) as *mut Align4096 as usize) % 4096;
    clobber(&mut before);
    clobber(&mut after);
    a.0[1] = a.0[1].wrapping_add(before[0]);
    b.0[2] = b.0[2].wrapping_add(after[4]);
    c.0[0] = c.0[0].wrapping_add(before[3]);
    d.0[2] = d.0[2].wrapping_add(after[1]);
    let digest = before[0]
        .wrapping_add(before[4])
        .wrapping_add(after[0])
        .wrapping_add(after[4])
        .wrapping_add(a.0[0])
        .wrapping_add(a.0[1])
        .wrapping_add(b.0[1])
        .wrapping_add(b.0[2])
        .wrapping_add(c.0[0])
        .wrapping_add(c.0[2])
        .wrapping_add(d.0[0])
        .wrapping_add(d.0[2]);
    (misalignment, digest)
}

impl Align256 {
    #[inline(never)]
    fn inspect(&self) -> (usize, u64) {
        let pointer = std::hint::black_box(self) as *const Self as usize;
        (
            pointer % 256,
            self.0[0].wrapping_add(self.0[1]).wrapping_add(self.0[2]),
        )
    }
}

#[inline(never)]
pub fn captured(seed: u64) -> (usize, u64) {
    let value = Align256([seed, 71, 73]);
    let callback: Box<dyn FnOnce() -> (usize, u64)> = Box::new(move || value.inspect());
    callback()
}
