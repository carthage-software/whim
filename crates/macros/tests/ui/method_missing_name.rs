use whim_macros::whim_interface;

#[whim_interface("Ui\\Speaker")]
trait Speaker {
    #[whim_method()]
    fn speak(&self) -> i64;
}

fn main() {}
