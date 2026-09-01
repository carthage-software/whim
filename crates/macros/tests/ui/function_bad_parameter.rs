use whim_macros::whim_function;

#[whim_function("Ui\\unsupported(mixed $value): int")]
fn unsupported(value: char) -> i64 {
    let _ = value;
    0
}

fn main() {}
