use crate::bytecode::chunk::Chunk;
use crate::bytecode::instruction::Instruction;
use crate::bytecode::rewrite::compact;
use crate::optimizer::cfg::relative_target;
use crate::optimizer::cfg::successors;

pub(in crate::optimizer) fn optimize_chunk(chunk: &mut Chunk) {
    if chunk.code.len() < 2 {
        return;
    }

    let mut reachable = vec![false; chunk.code.len()];
    let mut pending = vec![0];
    pending.extend(chunk.catch_table.iter().map(|entry| entry.handler as usize));
    let mut edges = Vec::new();
    while let Some(index) = pending.pop() {
        if index >= chunk.code.len() || reachable[index] {
            continue;
        }

        reachable[index] = true;
        edges.clear();
        successors(chunk, index, &mut edges);
        pending.extend(edges.iter().copied());
    }

    let mut remove = reachable
        .into_iter()
        .map(|reachable| !reachable)
        .collect::<Vec<_>>();

    if let Some(last) = remove.last_mut() {
        *last = false;
    }

    let mut next = chunk.code.len();
    for index in (0..chunk.code.len()).rev() {
        if remove[index] {
            continue;
        }
        if let Instruction::Jump { offset } = chunk.code[index] {
            let previous = index.checked_sub(1).and_then(|previous| {
                if matches!(chunk.code[previous], Instruction::DrainFinalizers) {
                    previous.checked_sub(1)
                } else {
                    Some(previous)
                }
            });
            if previous.is_some_and(|previous| {
                matches!(
                    chunk.code[previous],
                    Instruction::ForeachNext { .. }
                        | Instruction::VecForeachNext { .. }
                        | Instruction::DictForeachNext { .. }
                )
            }) {
                next = index;
                continue;
            }
            let target = relative_target(index, offset.offset());
            if target > index && target <= next {
                remove[index] = true;
                continue;
            }
        }
        next = index;
    }

    if remove.iter().all(|remove| !*remove) {
        return;
    }

    compact(chunk, &remove);
}
