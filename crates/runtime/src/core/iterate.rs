//! Iteration protocols used by `foreach`.

use whim_macros::whim_interface;

#[whim_interface("Whim\\Iterate\\Iterator<out K, out V>")]
trait IterateIterator {
    #[whim_method("next(): null|(K, V)")]
    fn next();
}

#[whim_interface("Whim\\Iterate\\ToIterator<out K, out V>")]
trait IterateToIterator {
    #[whim_method("toIterator(): Whim\\Iterate\\Iterator<K, V>")]
    fn to_iterator();
}
