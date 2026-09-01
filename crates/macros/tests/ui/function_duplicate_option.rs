use whim_macros::whim_function;

#[whim_function("Ui\\value(): int", must_use, must_use)]
fn value() -> i64 {
    1
}

fn main() {}
