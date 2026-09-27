use crate::Value;
use crate::array::ArrayTypeCheck;
use crate::array::ArrayTypeCheckId;
use crate::dict::DictObject;
use crate::dict::keys::Key;
use crate::heap::Heap;
use crate::heap::handle::ManagedRef;
use crate::newtype::NewtypeValueId;
use crate::string::ByteStringObject;
use crate::string::short::ShortString;

fn short(bytes: &[u8]) -> ShortString {
    ShortString::from_bytes(bytes).unwrap()
}

fn key(position: usize) -> ShortString {
    short(format!("key-{position}").as_bytes())
}

fn insert(
    dict: &mut DictObject,
    key: ShortString,
    value: Value,
    specialized: bool,
) -> Option<Value> {
    if specialized {
        dict.insert_short_string(key, value)
    } else {
        dict.insert(Key::ShortString(key), value)
    }
}

fn full_dict(heap: &Heap, count: usize, specialized: bool) -> ManagedRef<DictObject> {
    let mut dict = DictObject::new(heap);
    for position in 0..count {
        assert!(
            insert(
                dict.make_mut(),
                key(position),
                Value::int(i64::try_from(position).unwrap()),
                specialized,
            )
            .is_none()
        );
    }
    assert_eq!(dict.index.capacity(), count);
    dict
}

fn keys(dict: &DictObject) -> Vec<Key> {
    dict.iter().map(|(key, _)| key.to_owned()).collect()
}

#[test]
fn replacements_keep_full_capacity_and_order_but_new_keys_grow() {
    let heap = Heap::new();
    for specialized in [false, true] {
        for count in [3, 7] {
            let mut dict = full_dict(&heap, count, specialized);
            let before = keys(&dict);
            let previous = insert(dict.make_mut(), key(1), Value::int(99), specialized);

            assert_eq!(previous.and_then(|value| value.as_int()), Some(1));
            assert_eq!(dict.index.capacity(), count);
            assert_eq!(dict.len(), count);
            assert_eq!(dict.entries.len(), count);
            assert!(keys(&dict) == before);
            assert_eq!(
                dict.get_short_string(key(1)).and_then(Value::as_int),
                Some(99)
            );

            assert!(insert(dict.make_mut(), key(count), Value::int(100), specialized).is_none());
            assert!(dict.index.capacity() > count);
            assert_eq!(dict.len(), count + 1);
            let mut expected = before;
            expected.push(Key::ShortString(key(count)));
            assert!(keys(&dict) == expected);
        }
    }
}

#[test]
fn full_replacements_keep_key_kinds_and_match_both_string_forms() {
    let heap = Heap::new();
    let original_keys = [
        Key::Int(0),
        Key::Uint(0),
        Key::Bool(false),
        Key::String(ByteStringObject::from_bytes(&heap, b"0")),
        Key::ShortString(short(b"x")),
        Key::Int(1),
        Key::Uint(1),
    ];
    let mut dict = DictObject::new(&heap);
    for (position, key) in original_keys.iter().enumerate() {
        assert!(
            dict.make_mut()
                .insert(key.clone(), Value::int(i64::try_from(position).unwrap()))
                .is_none()
        );
    }
    assert_eq!(dict.index.capacity(), 7);

    for (position, key) in original_keys.iter().enumerate() {
        let replacement_key = match position {
            3 => Key::ShortString(short(b"0")),
            4 => Key::String(ByteStringObject::from_bytes(&heap, b"x")),
            _ => key.clone(),
        };
        let position = i64::try_from(position).unwrap();
        let previous = dict
            .make_mut()
            .insert(replacement_key, Value::int(100 + position));
        assert_eq!(previous.and_then(|value| value.as_int()), Some(position));
        assert_eq!(dict.index.capacity(), 7);
        assert_eq!(dict.len(), 7);
    }
    for (position, key) in original_keys.iter().enumerate() {
        assert_eq!(
            dict.get(key).and_then(Value::as_int),
            Some(100 + i64::try_from(position).unwrap())
        );
    }
    assert!(keys(&dict) == original_keys);

    for (key, expected) in [(short(b"0"), 103), (short(b"x"), 104)] {
        let previous = dict.make_mut().insert_short_string(key, Value::int(200));
        assert_eq!(previous.and_then(|value| value.as_int()), Some(expected));
        assert_eq!(dict.index.capacity(), 7);
        assert_eq!(dict.len(), 7);
    }
    assert_eq!(dict.get(&Key::Int(0)).and_then(Value::as_int), Some(100));
    assert_eq!(dict.get(&Key::Uint(0)).and_then(Value::as_int), Some(101));
    assert_eq!(
        dict.get(&Key::Bool(false)).and_then(Value::as_int),
        Some(102)
    );
}

#[test]
fn full_replacements_keep_snapshots_tags_lifetimes_and_cache_state() {
    let heap = Heap::new();
    for specialized in [false, true] {
        let mut original = full_dict(&heap, 3, specialized);
        let child = ByteStringObject::from_bytes(&heap, b"a retained dictionary value");
        let tag = NewtypeValueId(0);
        drop(insert(
            original.make_mut(),
            key(1),
            Value::string(child.clone()).with_newtype(Some(tag)),
            specialized,
        ));
        let first = ArrayTypeCheckId::new(17);
        let second = ArrayTypeCheckId::new(18);
        original.mark_type_checked(first);
        original.mark_type_checked(second);
        let mut changed = original.clone();
        let previous = insert(changed.make_mut(), key(1), Value::int(99), specialized).unwrap();

        assert_eq!(original.index.capacity(), 3);
        assert_eq!(changed.index.capacity(), 3);
        assert_eq!(previous.newtype_id(), Some(tag));
        assert_eq!(
            previous.as_string_bytes(),
            Some(b"a retained dictionary value".as_slice())
        );
        assert_eq!(
            original.get_short_string(key(1)).unwrap().newtype_id(),
            Some(tag)
        );
        assert_eq!(
            changed.get_short_string(key(1)).and_then(Value::as_int),
            Some(99)
        );
        assert_eq!(original.type_check(first), ArrayTypeCheck::Clean(first));
        assert_eq!(original.type_check(second), ArrayTypeCheck::Clean(second));
        assert_eq!(
            changed.type_check(first),
            ArrayTypeCheck::Dirty { id: first, slot: 1 }
        );
        assert_eq!(changed.type_check(second), ArrayTypeCheck::Unknown);

        drop(original);
        assert!(!child.is_unique());
        drop(previous);
        assert!(child.is_unique());
    }
}

#[test]
fn replacements_and_new_keys_keep_tombstone_order() {
    let heap = Heap::new();
    for specialized in [false, true] {
        let mut dict = full_dict(&heap, 7, specialized);
        assert_eq!(
            dict.make_mut()
                .remove(&Key::ShortString(key(2)))
                .and_then(|value| value.as_int()),
            Some(2)
        );
        let capacity = dict.index.capacity();
        let check = ArrayTypeCheckId::new(19);
        dict.mark_type_checked(check);
        let previous = insert(dict.make_mut(), key(4), Value::int(44), specialized);
        assert_eq!(previous.and_then(|value| value.as_int()), Some(4));
        assert_eq!(dict.index.capacity(), capacity);
        assert_eq!(
            dict.type_check(check),
            ArrayTypeCheck::Dirty { id: check, slot: 4 }
        );

        assert!(insert(dict.make_mut(), key(7), Value::int(77), specialized).is_none());
        assert!(dict.get_short_string(key(2)).is_none());
        assert_eq!(dict.type_check(check), ArrayTypeCheck::Unknown);
        assert!(insert(dict.make_mut(), key(2), Value::int(22), specialized).is_none());
        assert_eq!(dict.len(), 8);
        let expected = [0, 1, 3, 4, 5, 6, 7, 2]
            .into_iter()
            .map(|position| Key::ShortString(key(position)))
            .collect::<Vec<_>>();
        assert!(keys(&dict) == expected);
        assert_eq!(
            dict.get_short_string(key(2)).and_then(Value::as_int),
            Some(22)
        );
        assert_eq!(
            dict.get_short_string(key(4)).and_then(Value::as_int),
            Some(44)
        );
        assert_eq!(
            dict.get_short_string(key(7)).and_then(Value::as_int),
            Some(77)
        );
    }
}
