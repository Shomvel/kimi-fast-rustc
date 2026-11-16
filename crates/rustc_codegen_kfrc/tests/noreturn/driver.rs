extern crate noreturn_representation;

use std::sync::Mutex;
static EVENTS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

#[no_mangle]
pub extern "C" fn record_event(value: u64) -> u64 {
    EVENTS.lock().unwrap().push(value);
    value ^ 42
}

#[no_mangle]
pub extern "C" fn stop_process(value: u64) -> ! {
    let events = EVENTS.lock().unwrap();
    assert_eq!(&*events, &[18, 19]);
    assert_eq!(value, 20);
    println!("stop={value} events={events:?}");
    std::process::exit(17)
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("inline") => {
            noreturn_representation::inline_exit(19);
            panic!("returned");
        }
        Some("closed") => {
            noreturn_representation::closed_exit(19);
            panic!("returned");
        }
        Some("defined") => {
            let mut checksum = 0u64;
            for index in 0..4096u64 {
                let value = index.wrapping_mul(0x9e3779b97f4a7c15) | 1;
                let expected = value.wrapping_mul(0x5851f42d4c957f2d).rotate_left(11);
                let actual = unsafe { noreturn_representation::defined_guard(value) };
                assert_eq!(actual, expected);
                checksum = checksum.rotate_left(3) ^ actual;
            }
            assert!(EVENTS.lock().unwrap().is_empty());
            println!("defined={checksum:016x}");
        }
        _ => panic!("expected defined, inline or closed"),
    }
}
