use whim_macros::whim_constant;

// `#[whim_constant]` declares a constant; applying it to a function must be
// refused rather than silently producing a broken registration.
#[whim_constant("Ui\\NOPE", "int")]
fn not_a_constant() {}

fn main() {}
