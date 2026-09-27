use whim_bytecode::instruction::operands::Comparison;
use whim_bytecode::instruction::operands::Register;
use whim_value::Value;
use whim_value::heap::Heap;
use whim_value::newtype::NewtypeValueId;
use whim_value::string::ByteStringObject;
use whim_value::vec::VecObject;

use super::NumericKind;
use super::NumericRegisters;
use super::NumericValue;
use super::Pins;
use super::arithmetic::{
    add, comparison_matches_numeric, equals_numeric, multiply, step_counter, subtract,
};
use super::assign;
use super::flush;
use super::string_length_operation;

#[test]
fn uint_shadow_flush_preserves_bits_and_releases_pinned_references() {
    let heap = Heap::new();
    let text =
        ByteStringObject::from_bytes(&heap, b"a retained string longer than an inline string");
    let vector = VecObject::with_elements(&heap, [Value::string(text.clone())]);
    let mut registers = [Value::uint(u64::MAX - 1), Value::vec(vector)];
    let pointer = registers.as_mut_ptr();
    // SAFETY: both registers remain initialized and live throughout this test.
    let (mut values, mut dirty) = unsafe { NumericRegisters::from_registers(pointer, 2) }.unwrap();
    assert_eq!(dirty, 1);
    assert_eq!(values.uint(0), u64::MAX - 1);
    let mut pins = Pins::new();
    // SAFETY: register one owns the vector for the lifetime of this pin.
    assert!(unsafe { pins.for_write(pointer, Register::new(1)) }.is_some());
    assert_eq!(pins.pinned, 2);
    assert_eq!(pins.writable, 2);
    assert!(!text.is_unique());
    // SAFETY: both destinations are live registers tracked by the shadow and pins.
    unsafe {
        assign(
            pointer,
            &mut values,
            &mut dirty,
            &mut pins,
            Register::new(0),
            NumericValue::uint(u64::MAX),
        );
        assign(
            pointer,
            &mut values,
            &mut dirty,
            &mut pins,
            Register::new(1),
            NumericValue::uint(1 << 63),
        );
        flush(pointer, &values, dirty);
    }
    assert_eq!(registers[0].as_uint(), Some(u64::MAX));
    assert_eq!(registers[1].as_uint(), Some(1 << 63));
    assert!(!registers[0].is_int());
    assert_eq!(dirty, 3);
    assert_eq!(pins.pinned, 0);
    assert_eq!(pins.writable, 0);
    assert!(text.is_unique());
}

#[test]
fn string_length_can_replace_its_own_reference_with_a_uint_shadow() {
    let heap = Heap::new();
    let text =
        ByteStringObject::from_bytes(&heap, b"a retained string longer than an inline string");
    let mut registers = [Value::string(text.clone())];
    let pointer = registers.as_mut_ptr();
    // SAFETY: the single register remains initialized and live throughout this test.
    let (mut values, mut dirty) = unsafe { NumericRegisters::from_registers(pointer, 1) }.unwrap();
    let mut pins = Pins::new();
    // SAFETY: the source and destination are the same live register.
    assert!(unsafe {
        string_length_operation(
            pointer,
            &mut values,
            &mut dirty,
            &mut pins,
            Register::new(0),
            Register::new(0),
        )
    });
    assert!(values.kind(0) == NumericKind::Uint);
    assert!(text.is_unique());
    // SAFETY: the dirty mask names only the live numeric register.
    unsafe { flush(pointer, &values, dirty) };
    assert_eq!(registers[0].as_uint(), Some(text.len() as u64));
}

#[test]
fn uint_shadows_reject_tags_and_unsupported_operations() {
    let mut registers = [Value::uint(u64::MAX).with_newtype(Some(NewtypeValueId(0)))];
    // SAFETY: the register is initialized and live throughout the copy attempt.
    assert!(unsafe { NumericRegisters::from_registers(registers.as_mut_ptr(), 1) }.is_none());
    assert_eq!(registers[0].newtype_id(), Some(NewtypeValueId(0)));
    let uint = NumericValue::uint(u64::MAX);
    for other in [
        uint,
        NumericValue::int(-1),
        NumericValue::float(1.0),
        NumericValue::bool(true),
    ] {
        assert!(equals_numeric(uint, other).is_none());
        assert!(equals_numeric(other, uint).is_none());
        assert!(comparison_matches_numeric(Comparison::LessThan, uint, other).is_none());
        assert!(comparison_matches_numeric(Comparison::GreaterThan, other, uint).is_none());
        assert!(add(uint, other).is_none());
        assert!(subtract(uint, other).is_none());
        assert!(multiply(uint, other).is_none());
        assert!(step_counter(Comparison::LessThan, uint, other).is_none());
    }
}
