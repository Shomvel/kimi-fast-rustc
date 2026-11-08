// W05 对照（--crate-type=lib）：无 noalias 的裸指针版本；扩容函数体可见、可推断 captures(none) 的自制 Vec。
#[inline(never)]
pub unsafe fn append_raw(dst: *mut Vec<u8>, src: &[u8]) {
    for &b in src {
        unsafe { (*dst).push(b) }
    }
}

pub struct MyVec { cap: usize, ptr: *mut u8, len: usize }
impl MyVec {
    #[inline(never)]
    fn grow(&mut self) {
        let new = if self.cap == 0 { 8 } else { self.cap * 2 };
        let mut v = std::mem::ManuallyDrop::new(Vec::<u8>::with_capacity(new));
        unsafe { std::ptr::copy_nonoverlapping(self.ptr, v.as_mut_ptr(), self.len) };
        (self.ptr, self.cap) = (v.as_mut_ptr(), new);
    }
    #[inline]
    fn push(&mut self, b: u8) {
        let len = self.len; // 与 Vec::push_mut 相同的写法
        if len == self.cap { self.grow() }
        unsafe { *self.ptr.add(len) = b }
        self.len = len + 1;
    }
}
#[inline(never)]
pub fn append_my(dst: &mut MyVec, src: &[u8]) {
    for &b in src { dst.push(b) }
}
