use semantic_values::heap_niche_probe::*;

pub fn check() {
    for selector in 0..10u8 {
        for unit in ["", "ys", "fortnight"] {
            let error = make_unit_error(selector, unit);
            let variant = unit_error_variant(&error);
            let guard = unit_error_guard(&error);
            let message = unit_error_message(&error);
            assert_eq!(variant, u32::from(selector % 5));
            assert_eq!(message, error.to_string());
            assert!(!message.is_empty());
            println!("heap-niche {selector} {unit:?} {variant} {guard} {message} | {error:?}");
        }
    }
    for text in ["", "s", "min", "4ys", "fortnight", "x"] {
        println!("heap-niche-parse {text:?} {}", describe_parse(text));
    }
}
