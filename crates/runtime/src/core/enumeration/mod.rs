//! Enum protocols implemented by every Whim enum.

use whim_macros::whim_interface;

#[whim_interface("Whim\\Enum\\UnitEnum")]
#[whim_property("public readonly string $name")]
trait UnitEnum {
    #[whim_method("cases(): vec<static>", static)]
    fn cases();
}

#[whim_interface("Whim\\Enum\\BackedEnum<T: int|string>")]
#[whim_extends("Whim\\Enum\\UnitEnum")]
#[whim_property("public readonly T $value")]
trait BackedEnum {
    #[whim_method("from(int|string $value): static", static)]
    fn from();

    #[whim_method("tryFrom(int|string $value): static|null", static)]
    fn try_from();
}
