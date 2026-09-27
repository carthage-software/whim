use whim_bytecode::instruction::operands::IndexUpdateOperation;
use whim_value::Value;
use whim_value::dict::DictObject;
use whim_value::dict::keys::Key;
use whim_value::heap::Heap;
use whim_value::vec::VecObject;

use super::index_get;
use super::index_set_path;
use super::index_update_path;

#[test]
fn nested_write_keeps_unique_containers_and_separates_snapshots() {
    let heap = Heap::new();
    let leaf = VecObject::with_elements(&heap, [Value::int(1)]);
    let leaf_address = leaf.erased();
    let mut middle = DictObject::new(&heap);
    middle.make_mut().insert(Key::Int(0), Value::vec(leaf));
    let middle_address = middle.erased();
    let root = VecObject::with_elements(&heap, [Value::dict(middle)]);
    let root_address = root.erased();
    let mut root = Value::vec(root);
    let indexes = [Value::int(0), Value::int(0), Value::uint(0)];

    assert!(index_set_path(&heap, &mut root, &indexes, Value::int(2)).is_ok());
    assert_eq!(root.as_vec().unwrap().erased(), root_address);
    assert!(
        index_update_path(
            &heap,
            &mut root,
            &indexes,
            &Value::int(1),
            IndexUpdateOperation::Add
        )
        .is_ok()
    );

    assert!(
        index_update_path(
            &heap,
            &mut root,
            &indexes,
            &Value::int(1),
            IndexUpdateOperation::Subtract
        )
        .is_ok()
    );

    assert_eq!(root.as_vec().unwrap().erased(), root_address);
    let middle = root.as_vec().unwrap().get(0).unwrap().as_dict().unwrap();
    assert_eq!(middle.erased(), middle_address);
    assert_eq!(
        middle.get_int(0).unwrap().as_vec().unwrap().erased(),
        leaf_address
    );

    let snapshot = root.clone();
    assert!(
        index_update_path(
            &heap,
            &mut root,
            &indexes,
            &Value::int(1),
            IndexUpdateOperation::Add
        )
        .is_ok()
    );

    assert_ne!(root.as_vec().unwrap().erased(), root_address);
    for (value, expected) in [(&root, 3), (&snapshot, 2)] {
        let middle = index_get(&heap, value, &indexes[0])
            .map_err(|fault| fault.message)
            .unwrap();
        let leaf = index_get(&heap, &middle, &indexes[1])
            .map_err(|fault| fault.message)
            .unwrap();
        assert_eq!(
            index_get(&heap, &leaf, &indexes[2])
                .map_err(|fault| fault.message)
                .unwrap()
                .as_int(),
            Some(expected)
        );
    }
}
