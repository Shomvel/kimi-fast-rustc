// W03: Option<&V> lookup + derive(PartialEq) on fieldless and data-carrying enums.
use std::hint::black_box;
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum Kind { Bin, Lib, Rlib, Dylib, Test }
#[derive(PartialEq, Debug)]
pub enum CrateType { Bin, Lib, Rlib, Dylib, Cdylib, Staticlib, ProcMacro, Other(String) }
pub struct Map { keys: Vec<u32>, vals: Vec<CrateType> }
impl Map {
    pub fn find(&self, k: u32) -> Option<&CrateType> {
        let i = self.keys.iter().position(|&x| x == k)?;
        self.vals.get(i)
    }
}
#[inline(never)]
pub fn doctestable(m: &Map, k: u32) -> bool {
    if let Some(v) = m.find(k) { *v == CrateType::Rlib || *v == CrateType::ProcMacro } else { false }
}
#[inline(never)] // cargo Target::doctestable 的 any 闭包
pub fn any_lib(ts: &[CrateType]) -> bool {
    ts.iter().any(|k| *k == CrateType::Rlib || *k == CrateType::Lib || *k == CrateType::ProcMacro)
}
#[inline(never)] pub fn kind_eq(a: Kind, b: Kind) -> bool { a == b }
#[inline(never)] pub fn same(a: &CrateType, b: &CrateType) -> bool { a == b }
fn main() {
    let m = black_box(Map { keys: vec![1, 2, 3], vals: vec![CrateType::Lib, CrateType::ProcMacro, CrateType::Other("x".into())] });
    let r = [doctestable(&m, black_box(2)), doctestable(&m, black_box(3)), doctestable(&m, black_box(9)), any_lib(black_box(&m.vals))];
    let s = [kind_eq(black_box(Kind::Lib), black_box(Kind::Lib)), kind_eq(black_box(Kind::Bin), black_box(Kind::Test)), same(&m.vals[2], black_box(&CrateType::Other("x".into())))];
    assert_eq!((r, s), ([true, false, false, true], [true, false, true]));
}
