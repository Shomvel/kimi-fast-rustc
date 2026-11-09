use semantic_values::rs5_permissions_probe::*;
use std::cell::{Cell, RefCell};

fn nothing() {}
fn ignore_word(_: &u32) {}
fn bump_cell(c: &Cell<u32>) {
    c.set(c.get().wrapping_add(100));
}
fn bump_mixed(m: &Mixed) {
    m.c.set(m.c.get().wrapping_add(1000));
}
fn bump_refcell(c: &RefCell<u32>) {
    let mut value = c.borrow_mut();
    *value = value.wrapping_add(10);
}

pub fn check() {
    for value in [0u32, 5, u32::MAX] {
        assert_eq!(frozen_reload(&value, nothing), value.wrapping_mul(2));
        assert_eq!(observed_local(value), value.wrapping_mul(3).wrapping_mul(8));
        let cell = Cell::new(value);
        assert_eq!(cell_reload(&cell, bump_cell), value.wrapping_mul(2).wrapping_add(100));
        let mixed = Mixed {
            n: 3,
            c: Cell::new(value),
        };
        assert_eq!(
            mixed_reload(&mixed, bump_mixed),
            value.wrapping_mul(2).wrapping_add(1006)
        );
        let refcell = RefCell::new(value);
        assert_eq!(
            refcell_reload(&refcell, bump_refcell),
            value.wrapping_mul(2).wrapping_add(10)
        );
        let mut unique = value;
        assert_eq!(raw_from_unique(&mut unique), 2);
        assert_eq!(captured_then_written(value), value);
        assert_eq!(generic_reload(&value, ignore_word), value.wrapping_mul(2));
        let cell = Cell::new(value);
        assert_eq!(
            generic_reload(&cell, bump_cell),
            value.wrapping_mul(2).wrapping_add(100)
        );
        println!("rs5-permissions {value}");
    }
    let words = [9u64, 4, 7];
    assert_eq!(frozen_slice(&words, nothing), 18);
    for rounds in [0u32, 1, 5] {
        assert_eq!(frozen_loop(&words[1], rounds, nothing), 4 * (u64::from(rounds) + 1));
    }
    let text = b"Az-09.~";
    let classes: Vec<u32> = (0..=text.len()).map(|index| classify(text, index)).collect();
    assert_eq!(classes, [1, 1, 1, 2, 2, 3, 3, 0]);
    println!("rs5-permissions classify {classes:?}");
}
