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
