//! regex-syntax 0.8.8 driver: AST parsing, HIR translation and Unicode class construction.
use regex_syntax::ast::parse::Parser as AstParser;
use regex_syntax::ParserBuilder;
use std::{hint::black_box, time::Instant};

const PLAIN: [&str; 8] = ["abc", "a|bc", "(?:ab)+", "(?P<name>[a-z]+)", "^a{2,5}$", ".*?", "[a-z&&[^aeiou]]+", "(?:foo|bar){0,3}"];
const LONG: &str = r"(?x)
    (?P<scheme>[a-zA-Z][a-zA-Z0-9+.-]*)://
    (?:(?P<user>[^:@/]+)(?::(?P<pass>[^@/]*))?@)?
    (?P<host>[a-zA-Z0-9.-]+|\[[0-9a-fA-F:]+\])
    (?::(?P<port>[0-9]{1,5}))?
    (?P<path>/[^?\#]*)?(?:\?(?P<query>[^\#]*))?(?:\#(?P<frag>.*))?";
const UNICODE: [&str; 6] = [r"\w+", r"\p{Greek}+", "(?i)stra\u{df}e", r"[\p{L}&&\p{Lu}]", r"\pN\s\d", r"(?i)\p{Cyrillic}{2,}"];

fn run(mode: &str, iterations: u64) -> u64 {
    let mut checksum = 0u64;
    for i in 0..iterations {
        let value = match mode {
            "hir" => {
                let p = PLAIN[i as usize % PLAIN.len()];
                let hir = ParserBuilder::new().unicode(false).utf8(false).build().parse(black_box(p)).unwrap();
                hir.properties().minimum_len().unwrap_or(9) as u64 + hir.properties().explicit_captures_len() as u64
            }
            "ast" => {
                let ast = AstParser::new().parse(black_box(LONG)).unwrap();
                ast.span().end.offset as u64
            }
            "unicode" => {
                let p = UNICODE[i as usize % UNICODE.len()];
                let hir = regex_syntax::parse(black_box(p)).unwrap();
                hir.properties().minimum_len().unwrap_or(9) as u64 + hir.properties().maximum_len().unwrap_or(1) as u64
            }
            _ => panic!("unknown workload {mode}"),
        };
        checksum = checksum.wrapping_add(black_box(value));
    }
    checksum
}

fn corpus() {
    let extra = ["", "\\d+", "(?i)straße", "[\\x00-\\x7F]", "(?-u:[\\x80-\\xFF])", "^\\bword\\b$", "[", "a{2,1}",
        "(?P<1>bad)", "a**", "(?m)^$", "(?s).", "[[:alpha:]]", "[[:bogus:]]", "\\p{Bogus}", "x{1000000}", "\\Q", "(", ")",
        "a{,5}", "\\b{start}x", "[^\\s\\S]", "(?<n>a)(?<n>b)", "\\u{10FFFF}", "\\x{110000}", "[z-a]"];
    for pattern in PLAIN.iter().chain(UNICODE.iter()).chain([LONG].iter()).chain(extra.iter()) {
        match regex_syntax::parse(pattern) {
            Ok(hir) => {
                let text = format!("{hir:?}");
                let digest = text.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100000001b3));
                let p = hir.properties();
                println!("{pattern:?}: hir {} {digest:016x} min={:?} max={:?} caps={} lit={} display={}", text.len(),
                         p.minimum_len(), p.maximum_len(), p.explicit_captures_len(), p.is_literal(), hir);
            }
            Err(error) => println!("{pattern:?}: error {error}"),
        }
        match AstParser::new().parse(pattern) {
            Ok(ast) => println!("{pattern:?}: ast span {:?}", ast.span()),
            Err(error) => println!("{pattern:?}: ast error {:?}", error.kind()),
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("corpus") => corpus(),
        Some("run") => {
            let mode = args[2].as_str();
            let iterations: u64 = args[3].parse().unwrap();
            let start = Instant::now();
            let checksum = run(mode, iterations);
            let elapsed = start.elapsed().as_nanos();
            println!("{{\"workload\":\"{mode}\",\"iterations\":{iterations},\"checksum\":{checksum},\"elapsed_ns\":{elapsed}}}");
        }
        _ => panic!("usage: driver corpus | driver run <workload> <iterations>"),
    }
}
