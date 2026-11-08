//! Workload shapes the P0 backend rejects today, with the reason. Each moves
//! into `scalar.rs` when its phase lands.
//!
//! - `t01_guard_loop`: match-guard bindings are shared references (`&i64`);
//!   references to locals need stack slots and memory partitions (P1).

#[inline(never)]
pub fn t01_guard_loop(v: i64) -> i64 {
    let mut x = 1;
    let y = match v {
        n if {
            loop {
                x = 4;
                break n > 5;
            }
        } =>
        {
            10
        }
        _ => 20,
    };
    x + y
}

