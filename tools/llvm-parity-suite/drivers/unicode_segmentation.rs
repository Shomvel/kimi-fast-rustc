//! unicode-segmentation 1.13.3 driver. Timed workloads use grapheme segmentation, whose work is in
//! the library's non-inline GraphemeCursor::{next,prev}_boundary and table lookups. Word and sentence
//! iterators are #[inline] (instantiated in this LLVM driver), so they appear only in `corpus`.
use std::{hint::black_box, time::Instant};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

const MIXED: [&str; 12] = [
    "Hello, world! ", "e\u{301}cole na\u{ef}ve caf\u{e9} ", "\u{d55c}\u{ad6d}\u{c5b4} \u{1100}\u{1161}\u{11a8} ",
    "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466} ", "\u{1f1e9}\u{1f1ea}\u{1f1eb}\u{1f1f7} ",
    "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947} ", "a\r\nb\r\n", "\u{1f44d}\u{1f3fd} ", "\u{395}\u{3bb}\u{3bb}\u{3b7}\u{3bd}\u{3b9}\u{3ba}\u{3ac} ",
    "\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{6587}\u{7ae0}\u{3002}", "\u{627}\u{644}\u{639}\u{631}\u{628}\u{64a}\u{629} ", "Z\u{351}\u{36b}\u{343}\u{36a}a\u{30a} ",
];
const ASCII: &str = "The quick brown fox jumps over the lazy dog; 0123456789 (sphinx of black quartz, judge my vow).\r\n";

fn text(parts: &[&str], target: usize) -> String {
    let mut s = String::new();
    let mut i = 0;
    while s.len() < target { s.push_str(parts[i % parts.len()]); i += 1; }
    s
}

fn run(mode: &str, iterations: u64) -> u64 {
    let mixed = text(&MIXED, 8192);
    let ascii = text(&[ASCII], 8192);
    let mut checksum = 0u64;
    for i in 0..iterations {
        let value = match mode {
            "graphemes-mixed" => black_box(mixed.as_str()).graphemes(true).fold(0u64, |a, g| a.wrapping_mul(31).wrapping_add(g.len() as u64)),
            "graphemes-ascii" => black_box(ascii.as_str()).graphemes(true).fold(0u64, |a, g| a.wrapping_mul(31).wrapping_add(g.len() as u64)),
            "graphemes-rev" => black_box(mixed.as_str()).graphemes(true).rev().fold(0u64, |a, g| a.wrapping_mul(31).wrapping_add(g.len() as u64)),
            _ => panic!("unknown workload {mode}"),
        };
        checksum = checksum.wrapping_add(black_box(value) ^ i);
    }
    checksum
}

fn fnv(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

fn summarize<'a>(label: &str, name: &str, items: impl Iterator<Item = &'a str>) {
    let mut n = 0usize;
    let mut h = 0xcbf29ce484222325u64;
    let mut first = Vec::new();
    for item in items {
        if n < 6 { first.push(item.to_string()); }
        n += 1;
        h = fnv(h, item.as_bytes());
        h = fnv(h, &[0xff]);
    }
    println!("{label} {name} n={n} h={h:016x} first={first:?}");
}

fn corpus() {
    let mut inputs: Vec<(String, String)> = MIXED.iter().enumerate().map(|(i, s)| (format!("mixed{i}"), s.to_string())).collect();
    inputs.push(("ascii".into(), ASCII.into()));
    inputs.push(("mixed-all".into(), text(&MIXED, 4000)));
    inputs.push(("sentences".into(), "Mr. Smith went to Washington. He said \"Hi!\" Then left... 3.14 is pi? Yes.\n\nNew para. \u{5f88}\u{597d}\u{3002}\u{518d}\u{89c1}\u{ff01}".into()));
    inputs.push(("words".into(), "can't won't it's 3.14 1,000,000 e.g. U.S.A. foo_bar foo-bar \u{3053}\u{3093}\u{306b}\u{3061}\u{306f} \u{1f600}\u{1f600} a\u{200d}b".into()));
    inputs.push(("empty".into(), String::new()));
    for (label, s) in &inputs {
        summarize(label, "graphemes-ext", s.graphemes(true));
        summarize(label, "graphemes-legacy", s.graphemes(false));
        summarize(label, "graphemes-rev", s.graphemes(true).rev());
        summarize(label, "words", s.unicode_words());
        summarize(label, "word-bounds", s.split_word_bounds());
        summarize(label, "word-bounds-rev", s.split_word_bounds().rev());
        summarize(label, "sentences", s.unicode_sentences());
        summarize(label, "sentence-bounds", s.split_sentence_bounds());
        let gi: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).take(12).collect();
        let wi: Vec<usize> = s.split_word_bound_indices().map(|(i, _)| i).take(12).collect();
        let mut cursor = GraphemeCursor::new(0, s.len(), true);
        let mut boundaries = Vec::new();
        while let Ok(Some(b)) = cursor.next_boundary(s, 0) { boundaries.push(b); if boundaries.len() == 12 { break; } }
        let mut is_b = String::new();
        for off in 0..s.len().min(40) {
            if s.is_char_boundary(off) {
                let mut c = GraphemeCursor::new(off, s.len(), true);
                is_b.push(match c.is_boundary(s, 0) { Ok(true) => '1', Ok(false) => '0', Err(_) => 'e' });
            }
        }
        println!("{label} indices g={gi:?} w={wi:?} cursor={boundaries:?} is_boundary={is_b}");
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
