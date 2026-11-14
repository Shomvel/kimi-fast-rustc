//! Compile with plain rustc; this probe does not link rustc_private or LLVM.
#[path = "../counter.rs"]
mod counter;

use counter::{Measurement, Window};
use std::io;
use std::sync::mpsc;

const ITERATIONS: u64 = 4_000_000;

#[inline(never)]
fn burn() -> u64 {
    let mut value = 7u64;
    for index in 0..ITERATIONS {
        value = std::hint::black_box(
            value
                .wrapping_mul(6364136223846793005)
                .rotate_left(7)
                .wrapping_add(index),
        );
    }
    std::hint::black_box(value)
}

fn report(mode: &str, measurement: &Measurement) {
    let roots = measurement
        .roots
        .iter()
        .map(|root| {
            format!(
                "{{\"tid\":{},\"instructions\":{},\"enabled_ns\":{},\"running_ns\":{}}}",
                root.tid, root.instructions, root.enabled_ns, root.running_ns,
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"mode\":\"{mode}\",\"instructions\":{},\"elapsed_ns\":{},\"roots\":[{roots}]}}",
        measurement.instructions, measurement.elapsed_ns
    );
}

fn run() -> io::Result<()> {
    // The first window measures the same work on the initial thread.
    let window = Window::start()?;
    let expected = burn();
    let current = window.finish()?;
    report("current_thread", &current);

    // These threads did not exist when the events were opened. Join before
    // reading proves that terminated descendants are added to their root.
    let window = Window::start()?;
    let worker = std::thread::spawn(|| {
        let child = std::thread::spawn(burn);
        child.join().unwrap()
    });
    assert_eq!(worker.join().unwrap(), expected);
    let spawned = window.finish()?;
    report("new_thread_and_grandchild", &spawned);

    // A sleeping worker exists before arming, so inheritance alone from the
    // caller is insufficient. It must be one of the enumerated root events.
    let (go, begin) = mpsc::channel();
    let (ready, waiting) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        ready.send(()).unwrap();
        begin.recv().unwrap();
        burn()
    });
    waiting.recv().unwrap();
    let window = Window::start()?;
    go.send(()).unwrap();
    assert_eq!(worker.join().unwrap(), expected);
    let existing = window.finish()?;
    report("existing_thread", &existing);

    if current.instructions < ITERATIONS
        || spawned.instructions < ITERATIONS
        || existing.instructions < ITERATIONS
    {
        return Err(io::Error::other(
            "busy-loop instructions are missing from at least one window",
        ));
    }
    for measurement in [&spawned, &existing] {
        let ratio = measurement.instructions as f64 / current.instructions as f64;
        if !(0.8..=1.5).contains(&ratio) {
            return Err(io::Error::other(format!(
                "thread coverage ratio {ratio:.4} outside 0.8..1.5"
            )));
        }
    }
    println!("{{\"status\":\"ok\",\"coverage\":\"current,new,grandchild,existing\"}}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("inherited counter probe unavailable or invalid: {error}");
        std::process::exit(1);
    }
}
