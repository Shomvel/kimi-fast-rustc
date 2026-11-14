//! Generic definitions whose call sites resolve, per instance, to callees
//! that R's direct call cannot express: a `#[track_caller]` implementation
//! (regex-syntax `Literal::exact` calls the blanket `Into::into`), and
//! dynamic dispatch when the type parameter is a trait object. Such an
//! instance compiles through the concrete path; others stay on R.
use core::panic::Location;

pub struct Literal {
    bytes: Vec<u8>,
    exact: bool,
}

impl Literal {
    #[inline(never)]
    pub fn exact<B: Into<Vec<u8>>>(bytes: B) -> Literal {
        Literal {
            bytes: bytes.into(),
            exact: true,
        }
    }
    #[inline(never)]
    pub fn describe(&self) -> (usize, bool, u32) {
        let sum = self.bytes.iter().map(|&b| u32::from(b)).sum();
        (self.bytes.len(), self.exact, sum)
    }
}

pub trait CallSite {
    fn line(&self) -> u32;
}

impl CallSite for u8 {
    #[track_caller]
    fn line(&self) -> u32 {
        Location::caller().line() * 1000 + u32::from(*self)
    }
}

impl CallSite for u16 {
    fn line(&self) -> u32 {
        u32::from(*self)
    }
}

#[inline(never)]
pub fn call_site_line<T: CallSite + ?Sized>(value: &T) -> u32 {
    value.line()
}

#[inline(never)]
pub fn call_site_line_of_byte(value: u8) -> u32 {
    call_site_line(&value)
}

#[inline(never)]
pub fn call_site_line_of_short(value: u16) -> u32 {
    call_site_line(&value)
}

#[inline(never)]
pub fn call_site_line_dynamic(value: &dyn CallSite) -> u32 {
    call_site_line(value)
}

#[inline(never)]
pub fn exact_from_vec(bytes: Vec<u8>) -> (usize, bool, u32) {
    Literal::exact(bytes).describe()
}

#[inline(never)]
pub fn exact_from_str(text: &str) -> (usize, bool, u32) {
    Literal::exact(text).describe()
}
