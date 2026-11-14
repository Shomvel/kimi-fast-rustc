//! Repeated length guards from real str equality, including UTF-8 and bytes.
#[inline(never)]
pub fn unit_code(input: &str) -> u8 {
    match input {
        "nanos" | "nsec" | "ns" => 1,
        "usec" | "us" | "µs" => 2,
        "millis" | "msec" | "ms" => 3,
        "seconds" | "second" | "secs" | "sec" | "s" => 4,
        "minutes" | "minute" | "min" | "mins" | "m" => 5,
        "hours" | "hour" | "hr" | "hrs" | "h" => 6,
        "days" | "day" | "d" => 7,
        "weeks" | "week" | "wk" | "wks" | "w" => 8,
        "months" | "month" | "M" => 9,
        "years" | "year" | "yr" | "yrs" | "y" => 10,
        _ => 0,
    }
}

#[inline(never)]
pub fn byte_code(input: &[u8]) -> u8 {
    if input == b"alpha" {
        1
    } else if input == b"xy" {
        2
    } else if input == b"bravo" {
        3
    } else if input == b"z" {
        4
    } else if input == b"rs" {
        5
    } else if input == b"delta" {
        6
    } else if input == b"q" {
        7
    } else {
        0
    }
}
