use whim_macros::whim_interface;

#[whim_interface("Ui\\Speaker")]
trait Speaker {
    #[whim_method("speak(): void", static = true)]
    fn speak();
}

fn main() {}
