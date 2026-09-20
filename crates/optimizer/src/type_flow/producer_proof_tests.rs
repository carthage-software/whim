use std::ptr;

use whim_bytecode::chunk::Chunk;
use whim_bytecode::chunk::descriptors::Literal;
use whim_bytecode::chunk::descriptors::TypeDescriptor;
use whim_bytecode::unit::CompiledAttribute;
use whim_bytecode::unit::CompiledConstant;
use whim_bytecode::unit::CompiledUnit;
use whim_bytecode::unit::ConstantInitializer;
use whim_bytecode::unit::STUB_ATTRIBUTE;
use whim_span::Span;
use whim_value::heap::Heap;

use crate::type_flow::IndexedUnit;
use crate::type_flow::World;

#[test]
fn stub_constant_lookup_falls_back_to_a_real_world_definition() {
    let heap = Heap::new();
    let name = heap.intern(b"ANSWER");
    let real = CompiledUnit {
        path: heap.intern(b"/real.whim"),
        files: Vec::new(),
        main: Chunk::new(),
        functions: Vec::new(),
        classes: Vec::new(),
        constants: vec![CompiledConstant {
            name: name.clone(),
            span: Span::zero(),
            attributes: Vec::new(),
            initializer: ConstantInitializer::Literal(Literal::String(heap.intern(b"real"))),
        }],
        type_aliases: Vec::new(),
        newtypes: Vec::new(),
    };
    let mut stub = real.clone();
    stub.path = heap.intern(b"/stub.whim");
    stub.constants[0].initializer = ConstantInitializer::Literal(Literal::Null);
    stub.constants[0].attributes.push(CompiledAttribute {
        class: heap.intern(STUB_ATTRIBUTE),
        span: Span::zero(),
        arguments: Vec::new(),
        named_arguments: Vec::new(),
    });

    let empty_world = World::new(&[], &[]);
    let unresolved = IndexedUnit::with_world(&stub, &empty_world);
    assert!(unresolved.constant_by_name(&name).is_none());
    assert!(
        unresolved
            .descriptor_mask(
                &TypeDescriptor::Named {
                    name: name.clone(),
                    arguments: None,
                    recursive: false,
                },
                0
            )
            .is_none()
    );

    let units = [&stub, &real];
    let world = World::new(&units, &[]);
    let resolved = IndexedUnit::with_world(&stub, &world);
    assert!(ptr::eq(
        resolved.constant_by_name(&name).unwrap(),
        &real.constants[0],
    ));
}
