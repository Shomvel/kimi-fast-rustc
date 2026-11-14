// W01: `?` chain + From conversion; the error is 40 bytes. parse_config_match is the hand-written control.
use std::hint::black_box;
pub struct FieldError { pub msg: String, pub pos: usize, pub code: u32 }
pub enum Error { Field(FieldError), Missing(&'static str) }
impl From<FieldError> for Error { fn from(e: FieldError) -> Self { Error::Field(e) } }
pub struct Reader<'a> { pub toks: &'a [&'a str], pub pos: usize }
impl Reader<'_> {
    #[inline(never)] pub fn next_value(&mut self) -> Result<u32, FieldError> {
        let pos = self.pos; self.pos += 1;
        let Some(t) = self.toks.get(pos) else { return Err(FieldError { msg: String::new(), pos, code: 2 }) };
        t.parse().map_err(|_| FieldError { msg: t.to_string(), pos, code: 1 })
    }
}
pub struct Config { pub a: u32, pub b: u32, pub c: u32, pub d: u32 }
#[inline(never)] pub fn parse_config(r: &mut Reader) -> Result<Config, Error> {
    let a = r.next_value()?;
    let b = r.next_value()?;
    let c = r.next_value()?;
    let d = r.next_value()?;
    Ok(Config { a, b, c, d })
}
macro_rules! field { ($r:expr) => { match $r.next_value() { Ok(v) => v, Err(e) => return Err(Error::Field(e)) } } }
#[inline(never)] pub fn parse_config_match(r: &mut Reader) -> Result<Config, Error> {
    Ok(Config { a: field!(r), b: field!(r), c: field!(r), d: field!(r) })
}
fn main() {
    let toks = ["1", "2", "x", "4"]; let r = parse_config(black_box(&mut Reader { toks: &toks, pos: 0 }));
    let m = parse_config_match(black_box(&mut Reader { toks: &toks, pos: 0 }));
    println!("{} {} {} {}", size_of::<Result<u32, FieldError>>(), size_of::<Result<Config, Error>>(), matches!(r, Err(Error::Field(ref e)) if e.code == 1 && e.pos == 2), matches!(m, Err(Error::Field(ref e)) if e.pos == 2));
}
