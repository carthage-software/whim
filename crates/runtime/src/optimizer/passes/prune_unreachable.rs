use crate::bytecode::chunk::Chunk;
use crate::bytecode::rewrite::compact;
use crate::optimizer::cfg::successors;

pub(super) fn optimize_chunk(chunk: &mut Chunk) {
    if chunk.code.len() < 2 || !chunk.catch_table.is_empty() {
        return;
    }

    let mut reachable = vec![false; chunk.code.len()];
    let mut pending = vec![0];
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

    if reachable.iter().all(|reachable| *reachable) {
        return;
    }
    let mut remove = reachable
        .into_iter()
        .map(|reachable| !reachable)
        .collect::<Vec<_>>();
    if let Some(last) = remove.last_mut() {
        *last = false;
    }
    if remove.iter().all(|remove| !*remove) {
        return;
    }
    compact(chunk, &remove);
}
