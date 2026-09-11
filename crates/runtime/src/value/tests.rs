//! Owned result construction and storage lifetime regressions.

use crate::value::Value;
use crate::value::ValueView;
use crate::value::heap::Heap;

#[test]
fn owned_string_buffers_preserve_binary_bytes_and_storage_boundaries() {
    let heap = Heap::new();
    for length in [0, 7, 8, 23, 24, 64] {
        let mut bytes = Vec::with_capacity(length);
        bytes.extend((0..length).map(|index| if index % 2 == 0 { 0 } else { 0xff }));
        let expected = bytes.clone();
        let pointer = bytes.as_ptr();
        let value = Value::from_string_vec(&heap, bytes);
        assert_eq!(value.as_string_bytes(), Some(expected.as_slice()));
        assert_eq!(
            matches!(value.transparent(), ValueView::ShortString(_)),
            length <= 7
        );
        if length > 23 {
            assert_eq!(value.as_string_bytes().unwrap().as_ptr(), pointer);
        }

        let alias = value.clone();
        drop(value);
        assert_eq!(alias.as_string_bytes(), Some(expected.as_slice()));
    }
}

#[test]
fn cloning_preserves_tags_and_owns_one_reference() {
    use crate::value::dict::DictObject;
    use crate::value::newtype::NewtypeValueId;
    use crate::value::tuple::TupleObject;
    use crate::value::vec::VecObject;

    let heap = Heap::new();
    for value in [
        Value::uninitialized(),
        Value::null(),
        Value::bool(true),
        Value::int(i64::MIN),
        Value::float(-0.0),
        Value::from_string_bytes(&heap, b"short"),
        Value::from_string_bytes(&heap, b"a heap string"),
        Value::vec(VecObject::new(&heap)),
        Value::dict(DictObject::new(&heap)),
        Value::tuple(TupleObject::with_pair(&heap, Value::int(1), Value::int(2))),
    ] {
        let value = value.with_newtype(Some(NewtypeValueId(17)));
        let alias = value.clone();
        assert!(alias.kind == value.kind);
        assert_eq!(alias.newtype_id(), value.newtype_id());
        assert_eq!(alias.as_bool(), value.as_bool());
        assert_eq!(alias.as_int(), value.as_int());
        assert_eq!(
            alias.as_float().map(f64::to_bits),
            value.as_float().map(f64::to_bits)
        );
        assert_eq!(alias.as_string_bytes(), value.as_string_bytes());
        if value.is_reference_counted() {
            assert!(value.has_other_strong_references());
            drop(value);
            assert!(!alias.has_other_strong_references());
        }
    }
}
