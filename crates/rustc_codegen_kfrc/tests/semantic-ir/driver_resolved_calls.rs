use semantic_values::resolved_calls_probe::*;

pub fn check() {
    let from_vec = exact_from_vec(vec![1, 2, 3, 250]);
    let from_str = exact_from_str("kfrc");
    assert_eq!(from_vec, (4, true, 256));
    assert_eq!(from_str, (4, true, 422));
    let byte = call_site_line_of_byte(7);
    let short = call_site_line_of_short(9);
    let dynamic_byte = call_site_line_dynamic(&5u8);
    let dynamic_short = call_site_line_dynamic(&11u16);
    assert_eq!(byte % 1000, 7);
    assert_eq!(dynamic_byte % 1000, 5);
    assert_eq!((short, dynamic_short), (9, 11));
    println!(
        "resolved-calls {from_vec:?} {from_str:?} {byte} {short} {dynamic_byte} {dynamic_short}"
    );
}
