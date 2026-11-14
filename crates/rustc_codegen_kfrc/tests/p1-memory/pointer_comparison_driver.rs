//! Pointer order is checked using indices and lengths in the LLVM driver.

fn relation(left: (usize, usize), right: (usize, usize)) -> u8 {
    (left == right) as u8
        | (((left != right) as u8) << 1)
        | (((left < right) as u8) << 2)
        | (((left <= right) as u8) << 3)
        | (((left > right) as u8) << 4)
        | (((left >= right) as u8) << 5)
}

pub fn run() {
    for index in 0..8 {
        for other in 0..8 {
            let expected = relation((index, 0), (other, 0));
            assert_eq!(
                memory::pointer_comparisons::thin(index, other),
                (expected, expected, expected, expected, true, true),
            );
        }
    }
    for left_index in 0..3 {
        for right_index in 0..3 {
            for left_len in 0..4 {
                for right_len in 0..4 {
                    let expected = relation((left_index, left_len), (right_index, right_len));
                    assert_eq!(
                        memory::pointer_comparisons::wide(
                            left_index,
                            left_len,
                            right_index,
                            right_len
                        ),
                        (expected, expected, true),
                    );
                }
            }
        }
    }
    println!("pointer-comparisons 64 thin and 144 wide pairs verified");
}
