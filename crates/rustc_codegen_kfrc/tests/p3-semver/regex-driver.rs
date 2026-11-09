//! Public-API frontier for regex-syntax; this is not its internal unit suite.
fn main() {
    for pattern in [
        "",
        "abc",
        "a|bc",
        "(?:ab)+",
        "(?P<name>[a-z]+)",
        "^a{2,5}$",
        ".*?",
        "\\d+",
        "\\p{Greek}+",
        "(?i)straße",
        "[a-z&&[^aeiou]]+",
        "(?:foo|bar){0,3}",
        "[\\x00-\\x7F]",
        "(?-u:[\\x80-\\xFF])",
        "^\\bword\\b$",
        "[",
        "a{2,1}",
        "(?P<1>bad)",
    ] {
        match regex_syntax::parse(pattern) {
            Ok(hir) => {
                let text = format!("{hir:?}");
                let digest = text.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
                });
                println!("{pattern:?}: hir {} {digest:016x}", text.len());
            }
            Err(error) => println!("{pattern:?}: error {error}"),
        }
    }
}
