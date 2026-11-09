pub fn example_result() -> i64 {
    return 40 + 2;
}

pub(crate) fn crate_visible() -> i64 {
    return 42;
}

pub(self) fn self_visible() -> i64 {
    return 42;
}

mod nested {
    pub(super) fn super_visible() -> i64 {
        return 42;
    }
}

fn main() { println!("{}", example_result()); }
