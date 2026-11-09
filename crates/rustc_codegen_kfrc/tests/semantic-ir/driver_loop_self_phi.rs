use semantic_values::loop_self_phi_probe::*;

pub fn check() {
    let cases: [&[u8]; 8] = [
        &[],
        &[0],
        &[0, 0, 0],
        &[1],
        &[0, 1, 1, 0],
        &[2, 1, 0, 1, 2, 2, 3],
        &[1, 1, 2, 2, 1, 0, 0, 3, 4, 3],
        &[5, 4, 3, 2, 1, 0, 1, 2],
    ];
    for raw in cases {
        println!("loop-self-phi {raw:?} {:?}", reorder_visual(&levels(raw)));
    }
}
