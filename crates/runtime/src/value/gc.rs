//! The cycle collector: synchronous Bacon-Rajan trial deletion.

use std::collections::HashSet;
use std::ptr::NonNull;

use crate::unwrap_option_invariant;
use crate::value::heap::Heap;
use crate::value::heap::Roots;
use crate::value::heap::metadata::Color;
use crate::value::heap::metadata::Header;
use crate::value::heap::metadata::HeapBox;
use crate::value::heap::metadata::TeardownMode;
use crate::value::heap::metadata::TypeTag;
use crate::value::heap::visit_children_erased;
use crate::value::object::InstanceObject;

type ErasedBox = NonNull<HeapBox<()>>;

/// # Safety
///
/// `box_pointer` must reference a live box for as long as the returned
/// reference is used.
const unsafe fn header(box_pointer: ErasedBox) -> &'static Header {
    // SAFETY: the tag and managed handle prove the payload type and lifetime.
    unsafe { box_pointer.as_ref() }.header_ref()
}

pub(in crate::value) fn collect(heap: &Heap) -> usize {
    if heap.is_collecting() {
        return 0;
    }

    heap.set_collecting(true);
    let mut roots = heap.take_roots();
    mark_roots(&mut roots);
    scan_roots(&roots);
    let whites = collect_roots(&roots);
    heap.recycle_roots(roots);
    let whites = retain_finalizable_subgraphs(heap, whites);
    let freed = whites.len();
    free_whites(heap, &whites);
    heap.drain_pending();
    heap.set_collecting(false);
    freed
}

fn retain_finalizable_subgraphs(heap: &Heap, whites: Vec<ErasedBox>) -> Vec<ErasedBox> {
    let white_addresses: HashSet<usize> = whites.iter().map(|node| node.addr().get()).collect();
    let mut retained = HashSet::new();
    let mut stack: Vec<ErasedBox> = whites
        .iter()
        .copied()
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        .filter(|node| unsafe { header(*node) }.type_tag() == TypeTag::FinalizableObject)
        .collect();

    while let Some(node) = stack.pop() {
        if !retained.insert(node.addr().get()) {
            continue;
        }

        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            visit_children_erased(node, &mut |child| {
                if white_addresses.contains(&child.addr().get()) {
                    stack.push(child);
                }
            });
        }
    }

    if retained.is_empty() {
        return whites;
    }

    for &node in &whites {
        if !retained.contains(&node.addr().get()) {
            continue;
        }

        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            visit_children_erased(node, &mut |child| {
                header(child).increment();
            });
        }
    }

    let mut finalizers = whites
        .iter()
        .copied()
        .filter(|node| {
            retained.contains(&node.addr().get())
                // SAFETY: the tag and managed handle prove the payload type and lifetime.
                && unsafe { header(*node) }.type_tag() == TypeTag::FinalizableObject
        })
        .map(NonNull::cast)
        .collect::<Vec<NonNull<HeapBox<InstanceObject>>>>();
    // SAFETY: the surrounding invariant proves this option contains a value.
    finalizers.sort_unstable_by_key(|object| unsafe {
        unwrap_option_invariant(
            heap.finalizer_sequence(*object),
            "a finalizable object retains its allocation sequence",
        )
    });

    for object in finalizers {
        heap.schedule_finalizer(object, heap.cycle_finalizer_origin());
    }

    whites
        .into_iter()
        .filter(|node| !retained.contains(&node.addr().get()))
        .collect()
}

/// Keeps the roots that are still cycle candidates and grays their
/// subgraphs; every other root leaves the buffer.
fn mark_roots(roots: &mut Roots) {
    let mut stack = Vec::new();
    roots.retain(|&root| {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let root_header = unsafe { header(root) };
        if root_header.color() == Color::Purple && root_header.reference_count() > 0 {
            mark_gray(root, &mut stack);
            true
        } else {
            root_header.set_buffered(false);
            false
        }
    });
}

/// Paints a subgraph gray, trial-decrementing the target of every edge
/// traversed, so counts reflect only external references.
fn mark_gray(start: ErasedBox, stack: &mut Vec<ErasedBox>) {
    debug_assert!(stack.is_empty());
    stack.push(start);
    while let Some(node) = stack.pop() {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let node_header = unsafe { header(node) };
        if node_header.color() == Color::Gray {
            continue;
        }

        node_header.set_color(Color::Gray);
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            visit_children_erased(node, &mut |child| {
                let child_header = header(child);
                child_header.decrement();
                stack.push(child);
            });
        }
    }
}

fn scan_roots(roots: &[ErasedBox]) {
    let mut stack = Vec::new();
    let mut black_stack = Vec::new();
    for &root in roots {
        scan(root, &mut stack, &mut black_stack);
    }
}

fn scan(start: ErasedBox, stack: &mut Vec<ErasedBox>, black_stack: &mut Vec<ErasedBox>) {
    debug_assert!(stack.is_empty());
    stack.push(start);
    while let Some(node) = stack.pop() {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let node_header = unsafe { header(node) };
        if node_header.color() != Color::Gray {
            continue;
        }

        if node_header.reference_count() > 0 {
            scan_black(node, black_stack);
        } else {
            node_header.set_color(Color::White);
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            unsafe {
                visit_children_erased(node, &mut |child| stack.push(child));
            }
        }
    }
}

fn scan_black(start: ErasedBox, stack: &mut Vec<ErasedBox>) {
    debug_assert!(stack.is_empty());
    stack.push(start);
    while let Some(node) = stack.pop() {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let node_header = unsafe { header(node) };
        if node_header.color() == Color::Black {
            continue;
        }

        node_header.set_color(Color::Black);
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe {
            visit_children_erased(node, &mut |child| {
                let child_header = header(child);
                child_header.increment();
                if child_header.color() != Color::Black {
                    stack.push(child);
                }
            });
        }
    }
}

fn collect_roots(roots: &[ErasedBox]) -> Vec<ErasedBox> {
    let mut whites = Vec::new();
    let mut stack = Vec::new();
    for &root in roots {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { header(root) }.set_buffered(false);
        stack.push(root);
        while let Some(node) = stack.pop() {
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            let node_header = unsafe { header(node) };
            if node_header.color() != Color::White || node_header.is_buffered() {
                continue;
            }

            node_header.set_color(Color::Black);
            // SAFETY: the tag and managed handle prove the payload type and lifetime.
            unsafe {
                visit_children_erased(node, &mut |child| stack.push(child));
            }

            whites.push(node);
        }
    }

    whites
}

/// Frees the gathered white set with the cycle-member teardown mode, marking
/// every member with the buffered flag first so weak notification can tell a
/// dying box from a live one.
fn free_whites(heap: &Heap, whites: &[ErasedBox]) {
    for &white in whites {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { header(white) }.set_buffered(true);
    }

    for &white in whites {
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        let tag = unsafe { header(white) }.type_tag();
        // SAFETY: the tag and managed handle prove the payload type and lifetime.
        unsafe { heap.teardown_in_mode(white, tag, TeardownMode::CycleMember) };
    }
}
