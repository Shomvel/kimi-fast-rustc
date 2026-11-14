fn example_result() -> i64 {
    let negated = 0 - (-40);
    let multiplied = 1 * -2;
    let inverted = 0 ^ -1;
    return negated + multiplied + inverted + 5;
}

fn main() { println!("{}", example_result()); }
